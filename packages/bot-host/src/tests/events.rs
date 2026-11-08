use super::*;
use flow_like::flow::copilot::decision_event::{QUESTION, REPLY_DELIVERY_FIELD};
use flow_like::flow_like_model_provider::systemone::{
    SystemOneAnswer, SystemOneQuestion, SystemOneRequest, SystemOneResponse,
};
use flow_like_types::inbox::{TriggerBody, TriggerSource};
use std::{
    collections::BTreeMap,
    sync::atomic::{AtomicUsize, Ordering},
};

struct EventRunner {
    choice: &'static str,
    mode: Mode,
    classified: AtomicUsize,
    ran: AtomicUsize,
    suppressed: AtomicUsize,
    prompts: Mutex<Vec<String>>,
}

#[async_trait]
impl TurnRunner for EventRunner {
    async fn classify(
        &self,
        _: &TurnJob,
        request: &SystemOneRequest,
    ) -> Result<Option<SystemOneResponse>, RunError> {
        self.classified.fetch_add(1, Ordering::SeqCst);
        if !request.state["completed_result"].is_null() {
            assert_eq!(
                self.ran.load(Ordering::SeqCst),
                1,
                "a monitor must execute before triage"
            );
        }
        let SystemOneQuestion::Choice { criteria, .. } = &request.questions[QUESTION] else {
            panic!()
        };
        Ok(Some(SystemOneResponse {
            model: "decision-bit".into(),
            answers: BTreeMap::from([(
                QUESTION.into(),
                SystemOneAnswer::Choice {
                    choice: self.choice.into(),
                    confidence: 1.0,
                    probabilities: criteria
                        .keys()
                        .map(|choice| {
                            (
                                choice.clone(),
                                if choice == self.choice { 1.0 } else { 0.0 },
                            )
                        })
                        .collect(),
                },
            )]),
            usage: None,
            extra: BTreeMap::new(),
        }))
    }

    async fn run(&self, job: TurnJob) -> Result<String, RunError> {
        self.prompts.lock().unwrap().push(job.prompt.clone());
        self.ran.fetch_add(1, Ordering::SeqCst);
        if job.suppress_owner_notifications {
            self.suppressed.fetch_add(1, Ordering::SeqCst);
        }
        if matches!(self.choice, "process_quietly" | "respond" | "notify")
            && job.prompt.contains("Host delivery instruction")
        {
            assert!(job.suppress_owner_notifications);
        }
        match self.mode {
            Mode::Reply(text) => Ok(text.into()),
            Mode::Retry => Err(RunError::Failed {
                code: "EVENT_FAILED".into(),
                message: "The check failed".into(),
            }),
            Mode::Park => {
                job.gate.park_for_cost(
                    "file_summarize",
                    &json!({}),
                    json!({
                        "code":flow_like_bot_tools::error::code::NEEDS_APPROVAL,
                        "message":"Approval required",
                    }),
                );
                Ok(String::new())
            }
            Mode::Hang => unreachable!(),
        }
    }

    fn cancel(&self, _: &str) {}
}

async fn event_turn(
    choice: &'static str,
    mode: Mode,
    source: TriggerSource,
    monitoring: bool,
) -> (Arc<DeviceHub>, Arc<EventRunner>, TurnStatus) {
    let hub = Arc::new(DeviceHub {
        memory_ready: true,
        ..Default::default()
    });
    event_on_hub(hub, choice, mode, source, monitoring).await
}

async fn event_on_hub(
    hub: Arc<DeviceHub>,
    choice: &'static str,
    mode: Mode,
    source: TriggerSource,
    monitoring: bool,
) -> (Arc<DeviceHub>, Arc<EventRunner>, TurnStatus) {
    let mut entry = plain("Event");
    entry.kind = InboxEntryKind::Trigger;
    entry.envelope = TriggerBody {
        source,
        monitoring,
        ref_id: "event".into(),
        title: "Check".into(),
        text: "Check this event".into(),
        payload: None,
        payload_handle: None,
        bot_id: None,
    }
    .envelope();
    *hub.entries.lock().unwrap() = vec![entry];
    hub.turns.lock().unwrap().push(turn("bt_event"));
    let runner = Arc::new(EventRunner {
        choice,
        mode,
        classified: AtomicUsize::new(0),
        ran: AtomicUsize::new(0),
        suppressed: AtomicUsize::new(0),
        prompts: Mutex::new(Vec::new()),
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
    .expect("the event turn finishes");
    task.abort();
    (hub, runner, status)
}

#[tokio::test]
async fn background_choices_control_execution_and_delivery() {
    for choice in ["ignore", "process_quietly", "respond", "notify"] {
        let (hub, runner, status) = event_turn(
            choice,
            Mode::Reply("Processed"),
            TriggerSource::Webhook,
            false,
        )
        .await;
        assert_eq!(status, TurnStatus::Done);
        assert_eq!(runner.classified.load(Ordering::SeqCst), 1);
        assert_eq!(
            runner.ran.load(Ordering::SeqCst),
            usize::from(choice != "ignore")
        );
        if matches!(choice, "ignore" | "process_quietly") {
            assert!(hub.has(|call| *call == Call::Silent));
            assert!(!hub.has(|call| matches!(call, Call::Send(_))));
        } else {
            assert!(hub.has(|call| matches!(call, Call::Send(message) if
                (message["envelope"]["body"][REPLY_DELIVERY_FIELD].as_str() == Some("respond")) == (choice == "respond"))));
        }
    }
}

#[tokio::test]
async fn quiet_processing_preserves_failures_and_approval_cards() {
    for (mode, expected) in [
        (Mode::Retry, TurnStatus::Failed),
        (Mode::Park, TurnStatus::Waiting),
    ] {
        let (hub, runner, status) =
            event_turn("process_quietly", mode, TriggerSource::Email, false).await;
        assert_eq!(status, expected);
        assert_eq!(runner.ran.load(Ordering::SeqCst), 1);
        assert!(!hub.has(|call| *call == Call::Silent));
        if expected == TurnStatus::Failed {
            assert!(hub.has(|call| matches!(call, Call::Send(message) if message["envelope"]["body"]["error"]["code"] == "EVENT_FAILED")));
        } else {
            assert!(hub.has(|call| matches!(call, Call::Park(_))));
        }
    }
}

#[tokio::test]
async fn reminders_and_explicit_tasks_bypass_optional_triage() {
    for source in [TriggerSource::Schedule, TriggerSource::Event] {
        let (hub, runner, status) = event_turn("ignore", Mode::Reply("Done"), source, false).await;
        assert_eq!(status, TurnStatus::Done);
        assert_eq!(runner.classified.load(Ordering::SeqCst), 0);
        assert_eq!(runner.ran.load(Ordering::SeqCst), 1);
        assert!(hub.has(|call| matches!(call, Call::Send(_))));
    }
}

#[tokio::test]
async fn monitoring_runs_the_check_then_suppresses_an_unchanged_result() {
    let (hub, runner, status) = event_turn(
        "ignore",
        Mode::Reply("No meaningful change"),
        TriggerSource::Schedule,
        true,
    )
    .await;
    assert_eq!(status, TurnStatus::Done);
    assert_eq!(runner.ran.load(Ordering::SeqCst), 1);
    assert_eq!(runner.classified.load(Ordering::SeqCst), 1);
    assert!(
        runner.prompts.lock().unwrap()[0]
            .contains("No prior successful monitoring result is available")
    );
    assert!(hub.has(|call| *call == Call::Silent));
}

#[tokio::test]
async fn a_malformed_decision_keeps_normal_event_processing() {
    let (hub, runner, status) = event_turn(
        "unknown",
        Mode::Reply("Processed"),
        TriggerSource::Email,
        false,
    )
    .await;
    assert_eq!(status, TurnStatus::Done);
    assert_eq!(runner.ran.load(Ordering::SeqCst), 1);
    assert!(hub.has(|call| matches!(call, Call::Send(_))));
}

#[tokio::test]
async fn recovered_and_approved_events_keep_their_original_quiet_delivery() {
    for choice in ["process_quietly", "respond"] {
        for approved in [false, true] {
            let hub = Arc::new(DeviceHub {
                memory_ready: true,
                recovery_enabled: true,
                ..Default::default()
            });
            let now = Utc::now().timestamp_millis();
            *hub.checkpoint.lock().unwrap() = Some(flow_like_types::bot_turn::BotRunCheckpoint {
                revision: 1,
                started_at_ms: now - 1_000,
                deadline_at_ms: now + 10_000,
                state: json!({"phase":"ready", "event_delivery":{"choice":choice,"monitoring":false}}),
            });
            if approved {
                hub.grants.lock().unwrap().push((
                    "file_summarize".into(),
                    format!("{}approved", flow_like_types::tool_grant::ONCE_PREFIX),
                ));
            }
            let (hub, runner, status) = event_on_hub(
                hub,
                "notify",
                Mode::Reply("Processed"),
                TriggerSource::Webhook,
                false,
            )
            .await;
            assert_eq!(status, TurnStatus::Done);
            assert_eq!(runner.classified.load(Ordering::SeqCst), 0);
            assert_eq!(runner.ran.load(Ordering::SeqCst), 1);
            assert_eq!(runner.suppressed.load(Ordering::SeqCst), 1);
            if choice == "process_quietly" {
                assert!(hub.has(|call| *call == Call::Silent));
                assert!(!hub.has(|call| matches!(call, Call::Send(_))));
            } else {
                assert!(hub.has(|call| matches!(call, Call::Send(message) if message["envelope"]["body"][REPLY_DELIVERY_FIELD] == "respond")));
            }
        }
    }
}

#[tokio::test]
async fn a_recovered_monitor_records_its_final_delivery_before_posting() {
    let hub = Arc::new(DeviceHub {
        memory_ready: true,
        recovery_enabled: true,
        ..Default::default()
    });
    let now = Utc::now().timestamp_millis();
    *hub.checkpoint.lock().unwrap() = Some(flow_like_types::bot_turn::BotRunCheckpoint {
        revision: 1,
        started_at_ms: now - 1_000,
        deadline_at_ms: now + 10_000,
        state: json!({"phase":"ready", "event_delivery":{"choice":null,"monitoring":true}}),
    });
    let (hub, runner, status) = event_on_hub(
        hub,
        "respond",
        Mode::Reply("A nonurgent change"),
        TriggerSource::Schedule,
        true,
    )
    .await;
    assert_eq!(status, TurnStatus::Done);
    assert_eq!(runner.classified.load(Ordering::SeqCst), 1);
    let saved = hub.checkpoint.lock().unwrap().clone().unwrap();
    assert_eq!(saved.revision, 2);
    assert_eq!(saved.state["event_delivery"]["choice"], "respond");
}

#[tokio::test]
async fn a_quiet_checkpoint_with_a_runtime_handoff_stays_visible() {
    let hub = Arc::new(DeviceHub {
        memory_ready: true,
        recovery_enabled: true,
        ..Default::default()
    });
    let now = Utc::now().timestamp_millis();
    *hub.checkpoint.lock().unwrap() = Some(flow_like_types::bot_turn::BotRunCheckpoint {
        revision: 1,
        started_at_ms: now - 1_000,
        deadline_at_ms: now + 10_000,
        state: json!({"answer":{"stop_reason":"blocked"}, "event_delivery":{"choice":"process_quietly","monitoring":false}}),
    });
    let (hub, _, status) = event_on_hub(
        hub,
        "ignore",
        Mode::Reply("The action outcome is unknown; please inspect it"),
        TriggerSource::Webhook,
        false,
    )
    .await;
    assert_eq!(status, TurnStatus::Done);
    assert!(hub.has(|call| matches!(call, Call::Send(_))));
    assert!(!hub.has(|call| *call == Call::Silent));
}
