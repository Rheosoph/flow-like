use super::*;
use crate::hub::WaitOutcome;
use flow_like::flow::copilot::decision::{Decision, QUESTION};
use flow_like::flow_like_model_provider::systemone::{SystemOneQuestion, SystemOneRequest};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

struct DecisionRunner {
    decision: Option<Decision>,
    classified: AtomicUsize,
    ran: AtomicUsize,
    prompt: Mutex<Option<String>>,
    acknowledgement: AtomicBool,
    acknowledgement_offered: AtomicBool,
    live_call: AtomicBool,
    gated: AtomicBool,
}

#[async_trait]
impl TurnRunner for DecisionRunner {
    async fn decide(&self, _: &TurnJob, request: &SystemOneRequest) -> Result<Decision, RunError> {
        self.classified.fetch_add(1, Ordering::SeqCst);
        let SystemOneQuestion::Choice { criteria, .. } = &request.questions[QUESTION] else {
            panic!()
        };
        assert!(criteria.contains_key("wait_for:owner"));
        assert!(!criteria.contains_key("wait_for:bot"));
        self.acknowledgement_offered
            .store(criteria.contains_key("acknowledge"), Ordering::SeqCst);
        self.decision.clone().ok_or(RunError::SessionChanged)
    }

    async fn run(&self, job: TurnJob) -> Result<String, RunError> {
        self.live_call.store(job.live_call, Ordering::SeqCst);
        self.gated
            .fetch_or(job.tool_gate.is_some(), Ordering::SeqCst);
        if let Some(gate) = &job.tool_gate
            && !gate.passes().await
        {
            return Err(RunError::SessionChanged);
        }
        self.ran.fetch_add(1, Ordering::SeqCst);
        self.acknowledgement
            .store(job.acknowledgement, Ordering::SeqCst);
        *self.prompt.lock().unwrap() = Some(job.prompt);
        Ok("The primary model answered.".into())
    }

    fn cancel(&self, _: &str) {}
}

async fn classified_turn(
    room: bool,
    decision: Option<Decision>,
    wait_outcome: Option<WaitOutcome>,
) -> (Arc<DeviceHub>, Arc<DecisionRunner>, TurnStatus) {
    classified_entry(room, decision, wait_outcome, plain("What should we do?")).await
}

async fn classified_entry(
    room: bool,
    decision: Option<Decision>,
    wait_outcome: Option<WaitOutcome>,
    entry: InboxEntry,
) -> (Arc<DeviceHub>, Arc<DecisionRunner>, TurnStatus) {
    let hub = Arc::new(DeviceHub {
        room,
        memory_ready: true,
        wait_outcome,
        ..Default::default()
    });
    classified_on_hub(hub, decision, entry).await
}

async fn classified_on_hub(
    hub: Arc<DeviceHub>,
    decision: Option<Decision>,
    entry: InboxEntry,
) -> (Arc<DeviceHub>, Arc<DecisionRunner>, TurnStatus) {
    *hub.entries.lock().unwrap() = vec![entry];
    hub.turns.lock().unwrap().push(turn("bt_decision"));
    let runner = Arc::new(DecisionRunner {
        decision,
        classified: AtomicUsize::new(0),
        ran: AtomicUsize::new(0),
        prompt: Mutex::new(None),
        acknowledgement: AtomicBool::new(false),
        acknowledgement_offered: AtomicBool::new(false),
        live_call: AtomicBool::new(false),
        gated: AtomicBool::new(false),
    });
    let host = AccountHost::new(
        hub.clone(),
        runner.clone(),
        Arc::new(Quiet),
        Arc::new(Semaphore::new(1)),
        config(),
    );
    let task = tokio::spawn(host.clone().run());
    let status = tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            if let Some(turn) = host
                .turns()
                .into_iter()
                .find(|turn| turn.finished_at.is_some())
            {
                break turn.status;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("the classified turn finishes");
    task.abort();
    (hub, runner, status)
}

#[tokio::test]
async fn no_answer_and_wait_skip_primary_inference_and_tools() {
    for decision in [Decision::NoAnswer, Decision::WaitFor("peer".into())] {
        let waiting = matches!(decision, Decision::WaitFor(_));
        let (hub, runner, status) = classified_turn(true, Some(decision), None).await;
        assert_eq!(status, TurnStatus::Done);
        assert_eq!(runner.classified.load(Ordering::SeqCst), 1);
        assert_eq!(runner.ran.load(Ordering::SeqCst), 0);
        assert!(!hub.has(|call| matches!(call, Call::Send(_))));
        assert!(hub.has(|call| if waiting {
            matches!(call, Call::WaitFor(id) if id == "peer")
        } else {
            *call == Call::Silent
        }));
    }
}

#[tokio::test]
async fn an_already_received_target_reply_continues_the_primary_turn() {
    let (hub, runner, status) = classified_turn(
        true,
        Some(Decision::WaitFor("peer".into())),
        Some(WaitOutcome::AlreadyAnswered),
    )
    .await;
    assert_eq!(status, TurnStatus::Done);
    assert_eq!(runner.ran.load(Ordering::SeqCst), 1);
    assert!(hub.has(|call| matches!(call, Call::Send(_))));
}

#[tokio::test]
async fn direct_chats_bypass_group_classification() {
    let (_, runner, status) = classified_turn(false, Some(Decision::NoAnswer), None).await;
    assert_eq!(status, TurnStatus::Done);
    assert_eq!(runner.classified.load(Ordering::SeqCst), 0);
    assert_eq!(runner.ran.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn an_approved_resume_runs_the_primary_model_without_reclassifying_participation() {
    let hub = Arc::new(DeviceHub {
        room: true,
        memory_ready: true,
        ..Default::default()
    });
    hub.grants.lock().unwrap().push((
        "file_summarize".into(),
        format!("{}approved-call", flow_like_types::tool_grant::ONCE_PREFIX),
    ));
    let (hub, runner, status) = classified_on_hub(
        hub,
        Some(Decision::NoAnswer),
        plain("Please summarize this file"),
    )
    .await;
    assert_eq!(status, TurnStatus::Done);
    assert_eq!(runner.classified.load(Ordering::SeqCst), 0);
    assert_eq!(runner.ran.load(Ordering::SeqCst), 1);
    assert!(
        runner
            .prompt
            .lock()
            .unwrap()
            .as_deref()
            .unwrap()
            .contains("They approved: file_summarize")
    );
    assert!(hub.has(|call| matches!(call, Call::Send(_))));
    assert!(!hub.has(|call| matches!(call, Call::Silent | Call::WaitFor(_))));
    assert!(hub.grants.lock().unwrap().is_empty());
}

#[tokio::test]
async fn explicit_bot_asks_bypass_group_no_answer_and_wait_decisions() {
    for decision in [Decision::NoAnswer, Decision::WaitFor("peer".into())] {
        let mut request = plain("What is the total?");
        request.sender_id = "peer".into();
        request.envelope["body"]["mentions"] = json!(["bot"]);
        request.envelope["body"]["ask"] = json!({
            "id": request.message_id,
            "origin_conversation": "origin",
            "hops": 1,
            "reply_by": (Utc::now() + chrono::Duration::days(1)).to_rfc3339(),
            "chain": ["peer"],
            "wait_secs": 30,
            "round": 1,
        });
        let (hub, runner, status) = classified_entry(true, Some(decision), None, request).await;

        assert_eq!(status, TurnStatus::Done);
        assert_eq!(runner.classified.load(Ordering::SeqCst), 0);
        assert_eq!(runner.ran.load(Ordering::SeqCst), 1);
        assert!(hub.has(|call| matches!(call, Call::Send(_))));
        assert!(!hub.has(|call| matches!(call, Call::Silent | Call::WaitFor(_))));
    }
}

#[tokio::test]
async fn mismatched_ask_metadata_does_not_bypass_group_classification() {
    let mut request = plain("What is the total?");
    request.sender_id = "peer".into();
    request.envelope["body"]["mentions"] = json!(["bot"]);
    request.envelope["body"]["ask"] = json!({
        "id": "another-question",
        "origin_conversation": "origin",
        "hops": 1,
        "reply_by": (Utc::now() + chrono::Duration::days(1)).to_rfc3339(),
        "chain": ["peer"],
    });
    let (hub, runner, status) =
        classified_entry(true, Some(Decision::NoAnswer), None, request).await;

    assert_eq!(status, TurnStatus::Done);
    assert_eq!(runner.classified.load(Ordering::SeqCst), 1);
    assert_eq!(
        runner.ran.load(Ordering::SeqCst),
        1,
        "the message mentions the bot, so no_answer does not silence it"
    );
    assert!(!hub.has(|call| *call == Call::Silent));
}

#[tokio::test]
async fn direct_call_waits_keep_listening_without_a_group_dependency() {
    for decision in [Decision::NoAnswer, Decision::WaitFor("owner".into())] {
        let (hub, runner, status) =
            classified_entry(false, Some(decision), None, spoken(&[])).await;
        assert_eq!(status, TurnStatus::Done);
        assert_eq!(runner.classified.load(Ordering::SeqCst), 1);
        assert_eq!(runner.ran.load(Ordering::SeqCst), 0);
        assert!(hub.has(|call| *call == Call::Silent));
        assert!(!hub.has(|call| matches!(call, Call::WaitFor(_))));
    }
}

#[tokio::test]
async fn completed_direct_and_group_utterances_request_a_brief_acknowledgement() {
    for room in [false, true] {
        let (hub, runner, status) =
            classified_entry(room, Some(Decision::Acknowledge), None, spoken(&["bot"])).await;
        assert_eq!(status, TurnStatus::Done);
        assert_eq!(runner.classified.load(Ordering::SeqCst), 1);
        assert_eq!(runner.ran.load(Ordering::SeqCst), 1);
        assert!(runner.acknowledgement_offered.load(Ordering::SeqCst));
        assert!(runner.acknowledgement.load(Ordering::SeqCst));
        assert!(hub.has(|call| matches!(call, Call::Send(message) if message["envelope"]["body"]["system_one_acknowledgement"] == true)));
    }
}

#[tokio::test]
async fn typed_messages_cannot_enable_acknowledgement_rendering() {
    for room in [false, true] {
        let (_, runner, status) = classified_turn(room, Some(Decision::Acknowledge), None).await;
        assert_eq!(status, TurnStatus::Done);
        assert_eq!(runner.ran.load(Ordering::SeqCst), 1);
        assert!(!runner.acknowledgement_offered.load(Ordering::SeqCst));
        assert!(!runner.acknowledgement.load(Ordering::SeqCst));
    }
}

#[tokio::test]
async fn bot_acknowledgements_never_reach_classification_but_user_messages_do() {
    for (sender, should_run) in [("peer", false), ("owner", true)] {
        let mut entry = plain("Understood.");
        entry.sender_id = sender.into();
        entry.envelope["body"]["system_one_acknowledgement"] = json!(true);
        let (hub, runner, status) =
            classified_entry(true, Some(Decision::AnswerNow), None, entry).await;
        assert_eq!(status, TurnStatus::Done);
        assert_eq!(
            runner.classified.load(Ordering::SeqCst),
            usize::from(should_run)
        );
        assert_eq!(runner.ran.load(Ordering::SeqCst), usize::from(should_run));
        assert_eq!(hub.has(|call| matches!(call, Call::Send(_))), should_run);
    }
}

#[test]
fn bot_acknowledgements_do_not_wake_a_local_bot_or_satisfy_a_participant_wait() {
    use flow_like_types::inbox::ConversationKind;
    let since = (Utc::now() - chrono::Duration::hours(1)).fixed_offset();
    let mut entry = plain("Understood.");
    entry.sender_id = "peer".into();
    entry.seq = Some(2);
    entry.envelope["body"]["mentions"] = json!(["bot"]);
    entry.envelope["body"]["system_one_acknowledgement"] = json!(true);
    assert!(
        crate::rule::waiting(
            ConversationKind::Group,
            "bot",
            &[entry.clone()],
            since,
            false, false)
        .is_none()
    );
    assert!(
        crate::rule::waiting_participant(
            ConversationKind::Group,
            "bot",
            "peer",
            1,
            &[entry.clone()],
            since,
            None
        )
        .is_none()
    );
    entry.sender_id = flow_like_local_bots::LOCAL_OWNER_ID.into();
    assert!(
        crate::rule::waiting(
            ConversationKind::Group,
            "bot",
            &[entry.clone()],
            since,
            false, false
        )
        .is_some()
    );
    assert!(
        crate::rule::waiting_participant(
            ConversationKind::Group,
            "bot",
            flow_like_local_bots::LOCAL_OWNER_ID,
            1,
            &[entry],
            since,
            None
        )
        .is_some()
    );
}

#[tokio::test]
async fn a_changed_session_during_classification_never_starts_the_primary_model() {
    let (_, runner, status) = classified_turn(true, None, None).await;
    assert_eq!(status, TurnStatus::Cancelled);
    assert_eq!(runner.ran.load(Ordering::SeqCst), 0);
}

#[test]
fn a_named_bot_reply_wakes_without_a_mention_and_respects_the_checkpoint() {
    use flow_like_types::inbox::ConversationKind;
    let mut message = plain("Here is my answer");
    message.sender_id = "peer".into();
    message.seq = Some(2);
    let entries = [message];
    let since = (Utc::now() - chrono::Duration::hours(1)).fixed_offset();
    assert!(crate::rule::waiting(ConversationKind::Group, "bot", &entries, since, false, false).is_none());
    let found = crate::rule::waiting_participant(
        ConversationKind::Group,
        "bot",
        "peer",
        1,
        &entries,
        since,
        None,
    )
    .unwrap();
    assert_eq!(found.message_id, "m1");
    assert!(!found.addressed);
    assert!(
        crate::rule::waiting_participant(
            ConversationKind::Group,
            "bot",
            "peer",
            2,
            &entries,
            since,
            None
        )
        .is_none()
    );
    let mut deleted = entries;
    deleted[0].envelope = flow_like_types::inbox::tombstone_envelope();
    assert!(
        crate::rule::waiting_participant(
            ConversationKind::Group,
            "bot",
            "peer",
            1,
            &deleted,
            since,
            None
        )
        .is_none()
    );
}

#[tokio::test]
async fn live_call_is_set_for_transcript_turns_only() {
    let (_, runner, _) = classified_turn(true, Some(Decision::AnswerNow), None).await;
    assert!(!runner.live_call.load(Ordering::SeqCst));
    let (_, runner, status) =
        classified_entry(false, Some(Decision::AnswerNow), None, spoken(&[])).await;
    assert_eq!(status, TurnStatus::Done);
    assert!(runner.live_call.load(Ordering::SeqCst));
}

#[tokio::test]
async fn the_reply_starts_with_the_decision_for_direct_or_addressed_utterances_only() {
    let (_, runner, _) = classified_turn(true, Some(Decision::AnswerNow), None).await;
    assert!(!runner.gated.load(Ordering::SeqCst));
    let (_, runner, _) =
        classified_entry(true, Some(Decision::AnswerNow), None, spoken(&[])).await;
    assert!(!runner.gated.load(Ordering::SeqCst), "an unaddressed group utterance stays sequential");
    for (room, mentions) in [(false, vec![]), (true, vec!["bot"])] {
        let (hub, runner, status) =
            classified_entry(room, Some(Decision::AnswerNow), None, spoken(&mentions)).await;
        assert_eq!(status, TurnStatus::Done);
        assert!(runner.gated.load(Ordering::SeqCst));
        assert_eq!(runner.ran.load(Ordering::SeqCst), 1);
        assert!(hub.has(|call| matches!(call, Call::Send(_))));
    }
}

#[tokio::test]
async fn a_parallel_no_answer_runs_no_tool_and_posts_nothing() {
    let (hub, runner, status) =
        classified_entry(false, Some(Decision::NoAnswer), None, spoken(&[])).await;
    assert_eq!(status, TurnStatus::Done);
    assert_eq!(runner.ran.load(Ordering::SeqCst), 0);
    assert!(!hub.has(|call| matches!(call, Call::Send(_))));
    assert!(hub.has(|call| *call == Call::Silent));
}

#[tokio::test]
async fn poke_brings_the_next_poll_forward() {
    let hub = Arc::new(DeviceHub {
        memory_ready: true,
        ..Default::default()
    });
    *hub.entries.lock().unwrap() = vec![plain("Hello")];
    let runner = Arc::new(DecisionRunner {
        decision: None,
        classified: AtomicUsize::new(0),
        ran: AtomicUsize::new(0),
        prompt: Mutex::new(None),
        acknowledgement: AtomicBool::new(false),
        acknowledgement_offered: AtomicBool::new(false),
        live_call: AtomicBool::new(false),
        gated: AtomicBool::new(false),
    });
    let host = AccountHost::new(
        hub.clone(),
        runner,
        Arc::new(Quiet),
        Arc::new(Semaphore::new(1)),
        config(),
    );
    let task = tokio::spawn(host.clone().run());
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(host.turns().is_empty());
    hub.turns.lock().unwrap().push(turn("bt_poked"));
    host.poke();
    tokio::time::timeout(Duration::from_secs(3), async {
        while !host.turns().iter().any(|turn| turn.finished_at.is_some()) {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("the poked poll claims the turn long before the 60 s poll");
    task.abort();
}

#[test]
fn history_cuts_an_interrupted_reply_at_the_heard_share() {
    use flow_like_types::inbox::ConversationKind;
    let reply = "The weather is sunny today and tomorrow.";
    let mut bot_reply = plain(reply);
    bot_reply.message_id = "m_bot".into();
    bot_reply.sender_id = "bot".into();
    let mut stop = spoken(&[]);
    stop.message_id = "m_stop".into();
    stop.envelope["body"]["interrupted"] =
        json!({ "message_id": "m_bot", "heard_chars": 10, "spoken_chars": 40 });
    let found = crate::transcript::transcript(
        "bot",
        ConversationKind::Direct,
        "m_stop",
        &[bot_reply, stop],
        &Default::default(),
        "owner",
    )
    .unwrap();
    let heard = &found.history[0].content;
    assert!(heard.starts_with("The"), "{heard}");
    assert!(!heard.contains("tomorrow") && !heard.contains("sunny"), "{heard}");
    assert!(heard.contains("cut off"), "{heard}");
}
