use crate::{
    account::{AccountConfig, AccountHost},
    host::{Alert, RunError, TurnEvents, TurnInfo, TurnJob, TurnRunner},
    hub::{AskResult, CODE_LEASE_LOST, Claimed, HubFailure, TurnHub, TurnSession},
};
use async_trait::async_trait;
use chrono::Utc;
use flow_like_local_bots::InboxEntry;
use flow_like_types::{
    bot_message::ApprovalRequest,
    bot_turn::{BotTurn, MAX_ATTEMPTS, TurnAudience, TurnHostKind, TurnStatus, reply_message_id},
    inbox::{InboxEntryKind, plain_envelope},
};
use serde_json::{Value, json};
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::sync::Semaphore;

const HOST: &str = "device:test";

mod checkpoint_encryption;
mod events;
mod participation;
mod recovery;

#[derive(Clone, Debug, PartialEq)]
enum Call {
    Claim,
    Heartbeat,
    Send(Value),
    Park(String),
    Silent,
    WaitFor(String),
    Release(String),
    Abandon(String),
}

#[derive(Default)]
struct DeviceHub {
    recovery_enabled: bool,
    original_trigger: Mutex<Option<InboxEntry>>,
    trigger_reads: Mutex<Vec<Vec<(&'static str, String)>>>,
    checkpoint: Mutex<Option<flow_like_types::bot_turn::BotRunCheckpoint>>,
    checkpoint_calls: Mutex<Vec<(String, String, String, u32)>>,
    checkpoint_lease_lost: bool,
    release_refused: bool,
    cancel_refused: bool,
    room: bool,
    origin_room: bool,
    memory_ready: bool,
    origin_unavailable: bool,
    origin_only: bool,
    asked: Mutex<Option<AskResult>>,
    origin: Mutex<Vec<InboxEntry>>,
    read_conversations: Mutex<Vec<String>>,
    sent_conversations: Mutex<Vec<String>>,
    turns: Mutex<Vec<BotTurn>>,
    entries: Mutex<Vec<InboxEntry>>,
    calls: Mutex<Vec<Call>>,
    lease_lost_after: Mutex<Option<usize>>,
    polls: Mutex<usize>,
    attempts: Mutex<u32>,
    wait_outcome: Option<crate::hub::WaitOutcome>,
    grants: Mutex<Vec<(String, String)>>,
}

impl DeviceHub {
    fn record(&self, call: Call) {
        self.calls.lock().unwrap().push(call);
    }

    fn calls(&self) -> Vec<Call> {
        self.calls.lock().unwrap().clone()
    }

    fn has(&self, wanted: impl Fn(&Call) -> bool) -> bool {
        self.calls().iter().any(wanted)
    }
}

#[async_trait]
impl TurnHub for DeviceHub {
    fn supports_recovery(&self) -> bool {
        self.recovery_enabled
    }
    async fn checkpoint_load(
        &self,
        bot: &str,
        turn: &str,
        host: &str,
        attempt: u32,
    ) -> Result<Option<flow_like_types::bot_turn::BotRunCheckpoint>, HubFailure> {
        self.checkpoint_calls
            .lock()
            .unwrap()
            .push((bot.into(), turn.into(), host.into(), attempt));
        if self.checkpoint_lease_lost {
            return Err(HubFailure::answered(
                409,
                Some(CODE_LEASE_LOST),
                "Lease lost",
            ));
        }
        Ok(self.checkpoint.lock().unwrap().clone())
    }
    async fn checkpoint_save(
        &self,
        bot: &str,
        turn: &str,
        host: &str,
        attempt: u32,
        checkpoint: &flow_like_types::bot_turn::BotRunCheckpoint,
    ) -> Result<(), HubFailure> {
        self.checkpoint_calls
            .lock()
            .unwrap()
            .push((bot.into(), turn.into(), host.into(), attempt));
        if self.checkpoint_lease_lost {
            return Err(HubFailure::answered(
                409,
                Some(CODE_LEASE_LOST),
                "Lease lost",
            ));
        }
        let mut stored = self.checkpoint.lock().unwrap();
        if checkpoint.revision != stored.as_ref().map_or(1, |prior| prior.revision + 1) {
            return Err(HubFailure::answered(
                409,
                Some("CHECKPOINT_CHANGED"),
                "Checkpoint changed",
            ));
        }
        *stored = Some(checkpoint.clone());
        Ok(())
    }
    async fn tool_keys(&self, _: &str) -> Result<Vec<(String, String)>, HubFailure> {
        Ok(self.grants.lock().unwrap().clone())
    }

    async fn delete_tool_key(&self, _: &str, tool: &str, scope: &str) -> Result<bool, HubFailure> {
        let mut grants = self.grants.lock().unwrap();
        let before = grants.len();
        grants.retain(|(known_tool, known_scope)| known_tool != tool || known_scope != scope);
        Ok(grants.len() != before)
    }

    async fn list(&self) -> Result<Vec<BotTurn>, HubFailure> {
        *self.polls.lock().unwrap() += 1;
        Ok(std::mem::take(&mut *self.turns.lock().unwrap()))
    }

    fn next_poll(&self) -> Option<Duration> {
        Some(Duration::from_millis(10))
    }

    async fn claim(&self, turn_id: &str, host_id: &str) -> Result<BotTurn, HubFailure> {
        Ok(self.claim_turn(turn_id, host_id).await?.turn)
    }

    async fn claim_turn(&self, turn_id: &str, _: &str) -> Result<Claimed, HubFailure> {
        self.record(Call::Claim);
        let mut claimed = turn(turn_id);
        if self.room {
            claimed.audience = TurnAudience::Room;
        }
        if let Some(asked) = self.asked.lock().unwrap().as_ref() {
            claimed.requester_id = Some(asked.callee_id.clone());
            claimed.requester_name = Some(asked.callee_name.clone());
        }
        claimed.attempts = *self.attempts.lock().unwrap();
        Ok(Claimed {
            turn: claimed,
            session: Some(TurnSession {
                token: Some("bot-token".into()),
                hub: None,
                run: Some(Arc::new(42u32)),
            }),
        })
    }

    async fn heartbeat(&self, turn_id: &str, _: &str) -> Result<BotTurn, HubFailure> {
        self.record(Call::Heartbeat);
        let beats = self
            .calls()
            .iter()
            .filter(|c| **c == Call::Heartbeat)
            .count();
        if self
            .lease_lost_after
            .lock()
            .unwrap()
            .is_some_and(|after| beats >= after)
        {
            return Err(HubFailure::answered(409, Some(CODE_LEASE_LOST), "taken"));
        }
        Ok(turn(turn_id))
    }

    async fn entries(&self, _: &str, conversation: &str) -> Result<Vec<InboxEntry>, HubFailure> {
        self.read_conversations
            .lock()
            .unwrap()
            .push(conversation.to_owned());
        if self.origin_only && conversation != "origin" {
            return Err(HubFailure::answered(
                403,
                None,
                "the token only permits the origin",
            ));
        }
        if conversation == "origin" {
            if self.origin_unavailable {
                return Err(HubFailure::offline("the origin is unavailable"));
            }
            return Ok(self.origin.lock().unwrap().clone());
        }
        Ok(self.entries.lock().unwrap().clone())
    }

    async fn ask_result(&self, _: &str, _: &str) -> Result<Option<AskResult>, HubFailure> {
        Ok(self.asked.lock().unwrap().clone())
    }

    async fn read(
        &self,
        path: &str,
        query: Vec<(&'static str, String)>,
    ) -> Result<Value, HubFailure> {
        if self.origin_only && path.starts_with("inbox/conversations/conv") {
            return Err(HubFailure::answered(
                403,
                None,
                "the token only permits the origin",
            ));
        }
        if path == "inbox/conversations/conv/entries"
            && query.iter().any(|(key, _)| *key == "message_id")
        {
            self.trigger_reads.lock().unwrap().push(query);
            return Ok(
                json!({"entries": self.original_trigger.lock().unwrap().clone().into_iter().collect::<Vec<_>>() }),
            );
        }
        if path == "inbox/conversations/conv/memory" && self.memory_ready {
            return Ok(json!({
                "room": self.room,
                "use_owner_memory": !self.room,
                "shared_room_keys": [],
            }));
        }
        let group = match path {
            "inbox/conversations/origin" => self.origin_room,
            "inbox/conversations/conv" if self.memory_ready => self.room,
            _ => return Err(HubFailure::offline("unavailable")),
        };
        let mut members = vec![
            json!({ "principal_id": "bot", "kind": "bot" }),
            json!({ "principal_id": "owner", "kind": "user" }),
        ];
        if group {
            members.push(json!({ "principal_id": "peer", "kind": "bot" }));
        }
        Ok(json!({ "kind": if group { "group" } else { "direct" }, "members": members }))
    }

    async fn send(
        &self,
        _: &str,
        conversation: &str,
        message: Value,
        _: &str,
        _: &str,
    ) -> Result<(), HubFailure> {
        self.sent_conversations
            .lock()
            .unwrap()
            .push(conversation.to_owned());
        if self.origin_only && conversation != "origin" {
            return Err(HubFailure::answered(
                403,
                None,
                "the token only permits the origin",
            ));
        }
        self.record(Call::Send(message));
        Ok(())
    }

    async fn silent(&self, _: &str, _: &str) -> Result<(), HubFailure> {
        self.record(Call::Silent);
        Ok(())
    }

    async fn wait_for(
        &self,
        _: &str,
        _: &str,
        participant: &str,
    ) -> Result<crate::hub::WaitOutcome, HubFailure> {
        self.record(Call::WaitFor(participant.to_owned()));
        Ok(self.wait_outcome.unwrap_or(crate::hub::WaitOutcome::Parked))
    }

    async fn park(&self, _: &str, _: &str, card: &ApprovalRequest) -> Result<(), HubFailure> {
        self.record(Call::Park(card.tool.clone()));
        Ok(())
    }

    async fn release(&self, _: &str, _: &str, code: &str) -> Result<(), HubFailure> {
        self.record(Call::Release(code.to_owned()));
        if self.release_refused {
            return Err(HubFailure::answered(
                409,
                Some("BOT_TURN_NOT_RETRYABLE"),
                "The recovery budget is exhausted",
            ));
        }
        Ok(())
    }

    async fn abandon(&self, _: &str, _: &str, code: &str) -> Result<(), HubFailure> {
        self.record(Call::Abandon(code.to_owned()));
        Ok(())
    }

    async fn desktop_bots(&self) -> Result<Vec<String>, HubFailure> {
        Ok(Vec::new())
    }

    async fn report_host(&self, _: &str, _: &[String]) -> Result<Vec<String>, HubFailure> {
        Ok(Vec::new())
    }

    async fn delete_host(&self, _: &str) -> Result<(), HubFailure> {
        Ok(())
    }

    async fn cancel_turn(&self, _: &str, _: &str) -> Result<(), HubFailure> {
        if self.cancel_refused {
            return Err(HubFailure::offline(
                "The cancellation could not reach the hub",
            ));
        }
        Ok(())
    }
}

fn turn(id: &str) -> BotTurn {
    BotTurn {
        id: id.into(),
        bot_id: "bot".into(),
        conversation_id: "conv".into(),
        trigger_message_id: "m1".into(),
        host_kind: TurnHostKind::Device,
        status: TurnStatus::Queued,
        attempts: 0,
        host_id: None,
        lease_until: None,
        reply_message_id: None,
        error_code: None,
        hops: 0,
        owner_id: "owner".into(),
        requester_id: Some("owner".into()),
        requester_name: None,
        audience: TurnAudience::Owner,
        created_at: Utc::now().to_rfc3339(),
        updated_at: Utc::now().to_rfc3339(),
        batched: Vec::new(),
    }
}

fn entry(envelope: Value) -> InboxEntry {
    InboxEntry {
        id: "e1".into(),
        message_id: "m1".into(),
        conversation_id: "conv".into(),
        sender_id: "owner".into(),
        kind: InboxEntryKind::Message,
        envelope,
        created_at: Utc::now().fixed_offset(),
        seq: Some(1),
        cursor: String::new(),
    }
}

fn plain(text: &str) -> InboxEntry {
    entry(plain_envelope(json!({ "text": text })))
}

fn sealed() -> InboxEntry {
    entry(json!({ "v": 1, "enc": "x25519", "key_id": "k", "enc_key": "e", "ct": "c" }))
}

fn spoken(mentions: &[&str]) -> InboxEntry {
    let mut entry = entry(plain_envelope(json!({
        "call": "call_1", "text": "What is the plan?", "mentions": mentions,
        "offset_ms": 1200, "duration_ms": 1500,
    })));
    entry.kind = InboxEntryKind::Transcript;
    entry.sender_id = flow_like_local_bots::LOCAL_OWNER_ID.into();
    entry
}

#[test]
fn a_local_call_transcript_wakes_the_direct_bot_or_the_named_group_bot() {
    use flow_like_types::inbox::ConversationKind;
    let check = |kind, mentions: &[&str], bot| {
        let line = spoken(mentions);
        crate::rule::waiting(kind, bot, &[line.clone()], line.created_at, false)
    };
    assert!(
        check(ConversationKind::Direct, &[], "bot")
            .unwrap()
            .addressed
    );
    assert!(check(ConversationKind::Group, &[], "bot").is_none());
    assert!(
        check(ConversationKind::Group, &["bot"], "bot")
            .unwrap()
            .addressed
    );
    assert!(check(ConversationKind::Group, &["bot"], "other").is_none());
}

#[test]
fn an_answered_local_call_transcript_does_not_start_another_turn() {
    let line = spoken(&["bot"]);
    let mut answer = plain("Here is the plan.");
    answer.sender_id = "bot".into();
    answer.message_id = reply_message_id("bot", &line.message_id);
    answer.envelope["body"]["reply_to"] = json!(line.message_id);
    let at = line.created_at;
    assert!(
        crate::rule::waiting(
            flow_like_types::inbox::ConversationKind::Direct,
            "bot",
            &[answer, line],
            at,
            false,
        )
        .is_none()
    );
}

#[derive(Clone)]
enum Mode {
    Reply(&'static str),
    Retry,
    Park,
    Hang,
}

struct Runner {
    mode: Mode,
    jobs: Mutex<Vec<TurnJob>>,
    seen_claim: Mutex<Option<u32>>,
    cancelled: Mutex<Vec<String>>,
    started: Mutex<usize>,
    room_updates: Mutex<Option<Arc<dyn flow_like::flow::copilot::platform::RoomUpdates>>>,
}

impl Runner {
    fn new(mode: Mode) -> Arc<Self> {
        Arc::new(Self {
            mode,
            jobs: Mutex::default(),
            seen_claim: Mutex::default(),
            cancelled: Mutex::default(),
            started: Mutex::default(),
            room_updates: Mutex::default(),
        })
    }
}

#[async_trait]
impl TurnRunner for Runner {
    async fn run(&self, job: TurnJob) -> Result<String, RunError> {
        self.jobs.lock().unwrap().push(job.clone());
        *self.started.lock().unwrap() += 1;
        *self.seen_claim.lock().unwrap() = job.claim_as::<u32>().map(|claim| *claim);
        *self.room_updates.lock().unwrap() = job.room_updates.clone();
        match &self.mode {
            Mode::Reply(text) => Ok((*text).to_owned()),
            Mode::Retry => Err(RunError::Retry {
                code: "BOT_TURN_FAILED".into(),
                message: "the model was unreachable".into(),
            }),
            Mode::Park => {
                let outcome = json!({
                    "code": flow_like_bot_tools::error::code::NEEDS_APPROVAL,
                    "message": "above the cost limits",
                });
                job.gate
                    .park_for_cost("file_summarize", &json!({}), outcome);
                Ok(String::new())
            }
            Mode::Hang => {
                std::future::pending::<()>().await;
                Ok(String::new())
            }
        }
    }

    fn cancel(&self, run_id: &str) {
        self.cancelled.lock().unwrap().push(run_id.to_owned());
    }
}

struct Quiet;

impl TurnEvents for Quiet {
    fn turn(&self, _: &TurnInfo) {}
    fn alert(&self, _: &Alert) {}
}

fn config() -> AccountConfig {
    AccountConfig {
        host_id: HOST.into(),
        poll: Duration::from_secs(60),
        max_backoff: Duration::from_secs(60),
        heartbeat: Duration::from_millis(15),
        deadline: Duration::from_secs(5),
        send_retries: 1,
        send_retry_delay: Duration::from_millis(5),
        presence: Duration::from_secs(60),
        fire_tasks: false,
    }
}

struct Fixture {
    hub: Arc<DeviceHub>,
    runner: Arc<Runner>,
    host: AccountHost,
}

impl Fixture {
    fn start(mode: Mode, entries: Vec<InboxEntry>) -> Self {
        Self::with_config(mode, entries, config())
    }

    fn with_config(mode: Mode, entries: Vec<InboxEntry>, config: AccountConfig) -> Self {
        let hub = Arc::new(DeviceHub::default());
        *hub.entries.lock().unwrap() = entries;
        Self::with_hub(mode, hub, config)
    }

    fn with_hub(mode: Mode, hub: Arc<DeviceHub>, config: AccountConfig) -> Self {
        hub.turns.lock().unwrap().push(turn("bt_1"));
        let runner = Runner::new(mode);
        let host = AccountHost::new(
            hub.clone(),
            runner.clone(),
            Arc::new(Quiet),
            Arc::new(Semaphore::new(2)),
            config,
        );
        tokio::spawn(host.clone().run());
        Self { hub, runner, host }
    }

    fn status(&self) -> Option<TurnStatus> {
        self.host.turns().first().map(|turn| turn.status)
    }

    async fn until(&self, what: &str, check: impl Fn(&Self) -> bool) {
        for _ in 0..600 {
            if check(self) {
                return;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        panic!("timed out waiting for {what}: {:?}", self.hub.calls());
    }
}

#[test]
fn the_default_deadline_allows_long_bot_turns() {
    assert_eq!(
        AccountConfig::new(HOST.into()).deadline,
        Duration::from_secs(1_800)
    );
}

fn delayed_answer_hub(round: u32, origin_room: bool) -> Arc<DeviceHub> {
    let hub = Arc::new(DeviceHub {
        room: true,
        origin_room,
        memory_ready: true,
        ..Default::default()
    });
    let mut request = plain("Prepare the quarterly report.");
    request.message_id = "request".into();
    request.conversation_id = "origin".into();
    let mut waiting = plain("I am waiting for Ledger's total.");
    waiting.message_id = "waiting".into();
    waiting.conversation_id = "origin".into();
    waiting.sender_id = "bot".into();
    waiting.seq = Some(2);
    *hub.origin.lock().unwrap() = vec![waiting, request];
    let mut answer = plain("42");
    answer.sender_id = "peer".into();
    *hub.entries.lock().unwrap() = vec![answer];
    *hub.asked.lock().unwrap() = Some(AskResult {
        origin_conversation: "origin".into(),
        callee_id: "peer".into(),
        callee_name: "Ledger".into(),
        question: "What is the total?".into(),
        answer: "42".into(),
        failed: false,
        round,
    });
    hub
}

#[tokio::test]
async fn a_delayed_answer_preserves_the_origin_after_the_bot_already_replied() {
    use flow_like::flow::copilot::ChatRole;

    for round in [0, 2] {
        let hub = delayed_answer_hub(round, false);
        let question = format!(
            "What is the total? {} Exclude cancelled orders and report EUR only.",
            "Include the invoice details. ".repeat(16),
        );
        hub.asked.lock().unwrap().as_mut().unwrap().question = question.clone();
        let memory_hub: Arc<dyn TurnHub> = hub.clone();
        assert!(
            crate::memory::hub_source_for(
                &memory_hub,
                &memory_hub,
                "bot",
                "conv",
                Some(1),
                Some("owner"),
            )
            .await
            .is_some(),
            "the exchange otherwise provides conversation memory"
        );
        let f = Fixture::with_hub(Mode::Reply("The report includes 42."), hub, config());
        f.until("the resumed answer", |f| {
            f.status() == Some(TurnStatus::Done)
        })
        .await;

        let jobs = f.runner.jobs.lock().unwrap();
        assert_eq!(jobs.len(), 1);
        let job = &jobs[0];
        assert_eq!(job.conversation_id, "origin");
        assert!(job.prompt.contains("Earlier you asked Ledger"));
        assert!(
            job.prompt.contains(&question),
            "the continuation must keep constraints beyond a question preview"
        );
        assert!(job.prompt.contains("Ledger answered:\n42"));
        assert_eq!(job.history.len(), 2);
        assert!(matches!(job.history[0].role, ChatRole::User));
        assert_eq!(job.history[0].content, "Prepare the quarterly report.");
        assert!(matches!(job.history[1].role, ChatRole::Assistant));
        assert_eq!(job.history[1].content, "I am waiting for Ledger's total.");
        assert!(
            job.memory.is_none(),
            "exchange memory must not replace the origin transcript"
        );
        assert!(job.room_updates.is_none());
        assert_eq!(
            job.requester.as_ref().unwrap().audience,
            TurnAudience::Owner
        );
        assert_eq!(
            job.answered_round.as_ref().map(|answered| answered.round),
            (round > 0).then_some(round)
        );
    }
}

#[tokio::test]
async fn a_delayed_answer_keeps_group_speakers_and_room_scope() {
    let hub = delayed_answer_hub(0, true);
    hub.origin.lock().unwrap()[1].sender_id = "guest".into();
    let f = Fixture::with_hub(Mode::Reply("The report includes 42."), hub, config());
    f.until("the resumed room answer", |f| {
        f.status() == Some(TurnStatus::Done)
    })
    .await;

    let jobs = f.runner.jobs.lock().unwrap();
    let job = &jobs[0];
    assert_eq!(job.conversation_id, "origin");
    assert_eq!(
        job.history[0].content,
        "guest (member, not your owner) wrote: Prepare the quarterly report."
    );
    assert_eq!(job.history[1].content, "I am waiting for Ledger's total.");
    assert_eq!(job.requester.as_ref().unwrap().audience, TurnAudience::Room);
    assert!(job.memory.is_none());
    assert!(
        job.room_updates.is_none(),
        "legacy results with round=0 are continuations too"
    );
}

#[tokio::test]
async fn a_delayed_answer_reads_and_posts_only_in_its_tokens_origin_scope() {
    for origin_room in [false, true] {
        for fails in [false, true] {
            let mut hub = delayed_answer_hub(2, origin_room);
            Arc::get_mut(&mut hub).unwrap().origin_only = true;
            let response = if fails { "" } else { "The report includes 42." };
            let expected = if fails {
                TurnStatus::Failed
            } else {
                TurnStatus::Done
            };
            let f = Fixture::with_hub(Mode::Reply(response), hub, config());
            f.until("the origin-scoped response", |f| {
                f.status() == Some(expected)
            })
            .await;

            assert_eq!(*f.hub.read_conversations.lock().unwrap(), ["origin"]);
            assert_eq!(*f.hub.sent_conversations.lock().unwrap(), ["origin"]);
            let sent: Vec<Value> = f
                .hub
                .calls()
                .into_iter()
                .filter_map(|call| match call {
                    Call::Send(message) => Some(message),
                    _ => None,
                })
                .collect();
            assert_eq!(sent.len(), 1);
            let body = &sent[0]["envelope"]["body"];
            assert!(
                body.get("reply_to").is_none(),
                "the exchange's trigger does not exist in the origin"
            );
            assert_eq!(body.get("error").is_some(), fails);
        }
    }
}

#[tokio::test]
async fn an_unavailable_origin_does_not_run_the_delayed_answer_as_an_exchange_message() {
    let mut hub = delayed_answer_hub(2, false);
    Arc::get_mut(&mut hub).unwrap().origin_unavailable = true;
    let f = Fixture::with_hub(Mode::Reply("should not run"), hub, config());
    f.until("the failed read", |f| {
        f.status() == Some(TurnStatus::Failed)
    })
    .await;

    assert!(f.runner.jobs.lock().unwrap().is_empty());
    assert!(!f.hub.has(|call| matches!(call, Call::Send(_))));
}

#[tokio::test]
async fn an_explicit_shorter_deadline_cancels_the_run_and_reports_timeout() {
    let f = Fixture::with_config(
        Mode::Hang,
        vec![plain("hi")],
        AccountConfig {
            deadline: Duration::from_millis(50),
            ..config()
        },
    );
    f.until("the timeout", |f| f.status() == Some(TurnStatus::Failed))
        .await;
    assert_eq!(f.runner.cancelled.lock().unwrap().as_slice(), ["bt_1"]);
    let sent: Vec<Value> = f
        .hub
        .calls()
        .into_iter()
        .filter_map(|call| match call {
            Call::Send(message) => Some(message),
            _ => None,
        })
        .collect();
    assert_eq!(sent.len(), 1);
    assert_eq!(
        sent[0]["envelope"]["body"]["error"]["code"],
        crate::host::CODE_TIMEOUT
    );
}

#[tokio::test]
async fn a_claim_hands_its_run_spec_to_the_runner_and_the_reply_is_posted() {
    let f = Fixture::start(Mode::Reply("hello"), vec![plain("hi")]);
    f.until("the reply", |f| f.status() == Some(TurnStatus::Done))
        .await;

    assert_eq!(*f.runner.seen_claim.lock().unwrap(), Some(42));
    let sent: Vec<Value> = f
        .hub
        .calls()
        .into_iter()
        .filter_map(|call| match call {
            Call::Send(message) => Some(message),
            _ => None,
        })
        .collect();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0]["message_id"], reply_message_id("bot", "m1"));
    assert_eq!(sent[0]["envelope"]["body"]["text"], "hello");
    assert!(f.runner.room_updates.lock().unwrap().is_none());
}

#[tokio::test]
async fn a_call_transcript_turn_posts_a_text_reply_linked_to_the_spoken_line() {
    let f = Fixture::start(Mode::Reply("Here is the plan."), vec![spoken(&["bot"])]);
    f.until("the spoken reply", |f| f.status() == Some(TurnStatus::Done))
        .await;
    let sent: Vec<Value> = f
        .hub
        .calls()
        .into_iter()
        .filter_map(|call| match call {
            Call::Send(message) => Some(message),
            _ => None,
        })
        .collect();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0]["envelope"]["body"]["reply_to"], "m1");
    assert_eq!(sent[0]["envelope"]["body"]["text"], "Here is the plan.");
}

#[tokio::test]
async fn a_running_room_turn_reads_peer_messages_already_after_its_trigger_and_later_owner_messages()
 {
    let hub = Arc::new(DeviceHub {
        room: true,
        ..Default::default()
    });
    let trigger = plain("introduce yourselves");
    let mut peer = plain("I am Flux");
    peer.id = "peer-entry".into();
    peer.message_id = "m2".into();
    peer.sender_id = "peer".into();
    peer.seq = Some(2);
    *hub.entries.lock().unwrap() = vec![peer, trigger];
    let mut queued = turn("bt_1");
    queued.audience = TurnAudience::Room;
    hub.turns.lock().unwrap().push(queued);
    let runner = Runner::new(Mode::Hang);
    let host = AccountHost::new(
        hub.clone(),
        runner.clone(),
        Arc::new(Quiet),
        Arc::new(Semaphore::new(1)),
        config(),
    );
    tokio::spawn(host.clone().run());
    let f = Fixture { hub, runner, host };
    f.until("room updates attached to the running turn", |f| {
        f.runner.room_updates.lock().unwrap().is_some()
    })
    .await;
    let updates = f.runner.room_updates.lock().unwrap().clone().unwrap();
    assert_eq!(
        updates.drain().await,
        ["[Message m2] peer (member, not your owner) wrote: I am Flux"]
    );
    assert!(updates.drain().await.is_empty());
    let mut later = plain("keep it short");
    later.id = "later-entry".into();
    later.message_id = "m3".into();
    later.seq = Some(3);
    f.hub.entries.lock().unwrap().insert(0, later);
    assert_eq!(
        updates.drain().await,
        ["[Message m3] owner (your owner) wrote: keep it short"]
    );
    assert!(updates.drain().await.is_empty());
    f.host.cancel("bt_1").await.unwrap();
}

#[tokio::test]
async fn the_hubs_poll_rhythm_replaces_the_hosts_interval() {
    let f = Fixture::start(Mode::Reply("hello"), vec![plain("hi")]);
    f.until("three polls", |f| *f.hub.polls.lock().unwrap() >= 3)
        .await;
}

#[tokio::test]
async fn a_slow_turn_is_kept_alive_by_heartbeats() {
    let f = Fixture::start(Mode::Hang, vec![plain("hi")]);
    f.until("two heartbeats", |f| {
        f.hub
            .calls()
            .iter()
            .filter(|c| **c == Call::Heartbeat)
            .count()
            >= 2
    })
    .await;
    assert_eq!(f.status(), Some(TurnStatus::Running));
}

#[tokio::test]
async fn a_lost_lease_stops_the_run_and_posts_nothing() {
    let f = Fixture::start(Mode::Hang, vec![plain("hi")]);
    *f.hub.lease_lost_after.lock().unwrap() = Some(2);
    f.until("the run stops", |f| {
        f.status() == Some(TurnStatus::Cancelled)
    })
    .await;
    assert_eq!(f.runner.cancelled.lock().unwrap().as_slice(), ["bt_1"]);
    assert!(!f.hub.has(|call| matches!(call, Call::Send(_))));
}

#[tokio::test]
async fn an_approval_parks_the_turn_instead_of_answering() {
    let f = Fixture::start(Mode::Park, vec![plain("summarize the report")]);
    f.until("the park", |f| {
        f.hub.has(|call| matches!(call, Call::Park(_)))
    })
    .await;
    assert!(
        f.hub
            .has(|call| *call == Call::Park("file_summarize".into()))
    );
    assert!(!f.hub.has(|call| matches!(call, Call::Send(_))));
}

#[tokio::test]
async fn a_transient_failure_before_any_mutation_releases_the_turn() {
    let f = Fixture::start(Mode::Retry, vec![plain("hi")]);
    f.until("the release", |f| {
        f.hub.has(|call| matches!(call, Call::Release(_)))
    })
    .await;
    assert!(
        f.hub
            .has(|call| *call == Call::Release("BOT_TURN_FAILED".into()))
    );
    assert!(!f.hub.has(|call| matches!(call, Call::Send(_))));
}

#[tokio::test]
async fn an_exhausted_hub_recovery_budget_reports_the_failure() {
    let hub = Arc::new(DeviceHub {
        release_refused: true,
        ..Default::default()
    });
    *hub.entries.lock().unwrap() = vec![plain("hi")];
    let f = Fixture::with_hub(Mode::Retry, hub, config());
    f.until("the report", |f| {
        f.hub.has(|call| matches!(call, Call::Send(_)))
    })
    .await;
    assert!(f.hub.has(|call| matches!(call, Call::Release(_))));
    let Some(Call::Send(message)) = f
        .hub
        .calls()
        .into_iter()
        .find(|c| matches!(c, Call::Send(_)))
    else {
        unreachable!()
    };
    assert_eq!(
        message["envelope"]["body"]["error"]["code"],
        "BOT_TURN_FAILED"
    );
}

#[tokio::test]
async fn claim_generations_do_not_exhaust_the_hosts_recovery_budget() {
    let hub = Arc::new(DeviceHub::default());
    *hub.entries.lock().unwrap() = vec![plain("hi")];
    *hub.attempts.lock().unwrap() = MAX_ATTEMPTS + 10;
    let f = Fixture::with_hub(Mode::Retry, hub, config());
    f.until("the hub retry decision", |f| {
        f.hub.has(|call| matches!(call, Call::Release(_)))
    })
    .await;
    assert!(!f.hub.has(|call| matches!(call, Call::Send(_))));
}

#[tokio::test]
async fn an_end_to_end_encrypted_conversation_is_refused_without_a_reply() {
    let f = Fixture::start(Mode::Reply("never"), vec![sealed()]);
    f.until("the abandon", |f| {
        f.hub.has(|call| matches!(call, Call::Abandon(_)))
    })
    .await;
    assert_eq!(*f.runner.started.lock().unwrap(), 0);
    assert!(!f.hub.has(|call| matches!(call, Call::Send(_))));
}
