use crate::{
    account::{AccountConfig, AccountHost},
    host::{Alert, RunError, TurnEvents, TurnInfo, TurnJob, TurnRunner},
    hub::{CODE_LEASE_LOST, Claimed, HubFailure, TurnHub, TurnSession},
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

#[derive(Clone, Debug, PartialEq)]
enum Call {
    Claim,
    Heartbeat,
    Send(Value),
    Park(String),
    Silent,
    Release(String),
    Abandon(String),
}

#[derive(Default)]
struct DeviceHub {
    room: bool,
    turns: Mutex<Vec<BotTurn>>,
    entries: Mutex<Vec<InboxEntry>>,
    calls: Mutex<Vec<Call>>,
    lease_lost_after: Mutex<Option<usize>>,
    polls: Mutex<usize>,
    attempts: Mutex<u32>,
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

    async fn entries(&self, _: &str, _: &str) -> Result<Vec<InboxEntry>, HubFailure> {
        Ok(self.entries.lock().unwrap().clone())
    }

    async fn send(
        &self,
        _: &str,
        _: &str,
        message: Value,
        _: &str,
        _: &str,
    ) -> Result<(), HubFailure> {
        self.record(Call::Send(message));
        Ok(())
    }

    async fn silent(&self, _: &str, _: &str) -> Result<(), HubFailure> {
        self.record(Call::Silent);
        Ok(())
    }

    async fn park(&self, _: &str, _: &str, card: &ApprovalRequest) -> Result<(), HubFailure> {
        self.record(Call::Park(card.tool.clone()));
        Ok(())
    }

    async fn release(&self, _: &str, _: &str, code: &str) -> Result<(), HubFailure> {
        self.record(Call::Release(code.to_owned()));
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

#[derive(Clone)]
enum Mode {
    Reply(&'static str),
    Retry,
    Park,
    Hang,
}

struct Runner {
    mode: Mode,
    seen_claim: Mutex<Option<u32>>,
    cancelled: Mutex<Vec<String>>,
    started: Mutex<usize>,
    room_updates: Mutex<Option<Arc<dyn flow_like::flow::copilot::platform::RoomUpdates>>>,
}

impl Runner {
    fn new(mode: Mode) -> Arc<Self> {
        Arc::new(Self {
            mode,
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
        let hub = Arc::new(DeviceHub::default());
        *hub.entries.lock().unwrap() = entries;
        hub.turns.lock().unwrap().push(turn("bt_1"));
        let runner = Runner::new(mode);
        let host = AccountHost::new(
            hub.clone(),
            runner.clone(),
            Arc::new(Quiet),
            Arc::new(Semaphore::new(2)),
            config(),
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
    f.host.cancel("bt_1").await;
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
async fn a_failure_on_the_last_attempt_is_reported_instead_of_released() {
    let f = Fixture::start(Mode::Retry, vec![plain("hi")]);
    *f.hub.attempts.lock().unwrap() = MAX_ATTEMPTS;
    f.until("the report", |f| {
        f.hub.has(|call| matches!(call, Call::Send(_)))
    })
    .await;
    assert!(!f.hub.has(|call| matches!(call, Call::Release(_))));
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
async fn an_end_to_end_encrypted_conversation_is_refused_without_a_reply() {
    let f = Fixture::start(Mode::Reply("never"), vec![sealed()]);
    f.until("the abandon", |f| {
        f.hub.has(|call| matches!(call, Call::Abandon(_)))
    })
    .await;
    assert_eq!(*f.runner.started.lock().unwrap(), 0);
    assert!(!f.hub.has(|call| matches!(call, Call::Send(_))));
}
