use super::{
    account::{AccountConfig, AccountHost, CODE_LEASE_LOST, CODE_TAKEN, backoff},
    host::TurnEvents,
    hub::{HubFailure, TurnHub},
    tests::{FakeRunner, Mode, Recorded, settle, until},
};
use async_trait::async_trait;
use chrono::Utc;
use flow_like_local_bots::InboxEntry;
use flow_like_types::{
    bot_turn::{BotTurn, TurnHostKind, TurnStatus, failure_message_id, reply_message_id},
    inbox::{InboxEntryKind, plain_envelope},
};
use serde_json::{Value, json};
use std::{
    collections::VecDeque,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use tokio::{sync::Semaphore, time::Instant};

const HOST: &str = "desktop:test";

#[derive(Debug, Clone)]
struct Sent {
    bot_id: String,
    conversation_id: String,
    message: Value,
    turn_id: String,
    host_id: String,
}

#[derive(Default)]
struct FakeHub {
    listed: Mutex<Vec<BotTurn>>,
    lead: Mutex<Option<BotTurn>>,
    claim_failure: Mutex<Option<HubFailure>>,
    claims: Mutex<Vec<(String, String)>>,
    lease_lost_from: Mutex<Option<usize>>,
    heartbeats: Mutex<Vec<(String, String)>>,
    entries: Arc<Mutex<Vec<InboxEntry>>>,
    send_failures: Arc<Mutex<VecDeque<HubFailure>>>,
    sent: Arc<Mutex<Vec<Sent>>>,
    calls: Arc<Mutex<Vec<(String, &'static str)>>>,
    lists: Mutex<Vec<Instant>>,
    offline: AtomicBool,
    bots: Mutex<Vec<String>>,
    reports: Mutex<Vec<(String, Vec<String>, Instant)>>,
    deleted: Mutex<Vec<String>>,
    cancelled: Mutex<Vec<(String, String)>>,
    token_requests: Mutex<Vec<(String, String)>>,
    token_failure: AtomicBool,
}

struct BotView {
    token: String,
    entries: Arc<Mutex<Vec<InboxEntry>>>,
    sent: Arc<Mutex<Vec<Sent>>>,
    send_failures: Arc<Mutex<VecDeque<HubFailure>>>,
    calls: Arc<Mutex<Vec<(String, &'static str)>>>,
}

#[async_trait]
impl TurnHub for BotView {
    async fn list(&self) -> Result<Vec<BotTurn>, HubFailure> {
        Err(HubFailure::answered(403, None, "a bot token lists no turns"))
    }

    async fn claim(&self, _turn_id: &str, _host_id: &str) -> Result<BotTurn, HubFailure> {
        Err(HubFailure::answered(403, None, "a bot token claims no turns"))
    }

    async fn heartbeat(&self, _turn_id: &str, _host_id: &str) -> Result<BotTurn, HubFailure> {
        Err(HubFailure::answered(403, None, "a bot token keeps no lease"))
    }

    async fn desktop_bots(&self) -> Result<Vec<String>, HubFailure> {
        Err(HubFailure::answered(403, None, "a bot token lists no bots"))
    }

    async fn report_host(&self, _: &str, _: &[String]) -> Result<Vec<String>, HubFailure> {
        Err(HubFailure::answered(403, None, "a bot token reports no host"))
    }

    async fn delete_host(&self, _: &str) -> Result<(), HubFailure> {
        Err(HubFailure::answered(403, None, "a bot token reports no host"))
    }

    async fn cancel_turn(&self, _: &str, _: &str) -> Result<(), HubFailure> {
        Err(HubFailure::answered(403, None, "a bot token cancels no turns"))
    }

    async fn entries(&self, _: &str, _: &str) -> Result<Vec<InboxEntry>, HubFailure> {
        self.calls.lock().unwrap().push((self.token.clone(), "entries"));
        Ok(self.entries.lock().unwrap().clone())
    }

    async fn send(
        &self,
        bot_id: &str,
        conversation_id: &str,
        message: Value,
        turn_id: &str,
        host_id: &str,
    ) -> Result<(), HubFailure> {
        if let Some(failure) = self.send_failures.lock().unwrap().pop_front() {
            return Err(failure);
        }
        self.calls.lock().unwrap().push((self.token.clone(), "send"));
        self.sent.lock().unwrap().push(Sent {
            bot_id: bot_id.to_owned(),
            conversation_id: conversation_id.to_owned(),
            message,
            turn_id: turn_id.to_owned(),
            host_id: host_id.to_owned(),
        });
        Ok(())
    }
}

#[async_trait]
impl TurnHub for FakeHub {
    fn with_runtime_token(&self, token: &str) -> Option<Arc<dyn TurnHub>> {
        Some(Arc::new(BotView {
            token: token.to_owned(),
            entries: self.entries.clone(),
            sent: self.sent.clone(),
            send_failures: self.send_failures.clone(),
            calls: self.calls.clone(),
        }))
    }

    async fn runtime_token(
        &self,
        bot_id: &str,
        turn_id: &str,
    ) -> Result<Option<String>, HubFailure> {
        self.token_requests
            .lock()
            .unwrap()
            .push((bot_id.to_owned(), turn_id.to_owned()));
        if self.token_failure.load(Ordering::SeqCst) {
            return Err(HubFailure::offline("the hub could not be reached"));
        }
        Ok(Some(format!("bot-token:{bot_id}:{turn_id}")))
    }

    async fn desktop_bots(&self) -> Result<Vec<String>, HubFailure> {
        if self.offline.load(Ordering::SeqCst) {
            return Err(HubFailure::answered(
                401,
                Some("ACCOUNT_REQUIRED"),
                "Sign in",
            ));
        }
        Ok(self.bots.lock().unwrap().clone())
    }

    async fn report_host(
        &self,
        host_id: &str,
        bot_ids: &[String],
    ) -> Result<Vec<String>, HubFailure> {
        self.reports
            .lock()
            .unwrap()
            .push((host_id.to_owned(), bot_ids.to_vec(), Instant::now()));
        Ok(Vec::new())
    }

    async fn delete_host(&self, host_id: &str) -> Result<(), HubFailure> {
        self.deleted.lock().unwrap().push(host_id.to_owned());
        Ok(())
    }

    async fn cancel_turn(&self, bot_id: &str, turn_id: &str) -> Result<(), HubFailure> {
        self.cancelled
            .lock()
            .unwrap()
            .push((bot_id.to_owned(), turn_id.to_owned()));
        Ok(())
    }

    async fn list(&self) -> Result<Vec<BotTurn>, HubFailure> {
        self.lists.lock().unwrap().push(Instant::now());
        if self.offline.load(Ordering::SeqCst) {
            return Err(HubFailure::offline("the hub could not be reached"));
        }
        Ok(self.listed.lock().unwrap().clone())
    }

    async fn claim(&self, turn_id: &str, host_id: &str) -> Result<BotTurn, HubFailure> {
        self.claims
            .lock()
            .unwrap()
            .push((turn_id.to_owned(), host_id.to_owned()));
        let asked = {
            let mut listed = self.listed.lock().unwrap();
            let asked = listed.iter().find(|turn| turn.id == turn_id).cloned();
            listed.clear();
            asked
        };
        if let Some(failure) = self.claim_failure.lock().unwrap().clone() {
            return Err(failure);
        }
        let lead = self.lead.lock().unwrap().clone().or(asked);
        lead.ok_or_else(|| HubFailure::answered(404, None, "not yours"))
    }

    async fn heartbeat(&self, turn_id: &str, host_id: &str) -> Result<BotTurn, HubFailure> {
        let count = {
            let mut beats = self.heartbeats.lock().unwrap();
            beats.push((turn_id.to_owned(), host_id.to_owned()));
            beats.len()
        };
        if self
            .lease_lost_from
            .lock()
            .unwrap()
            .is_some_and(|from| count >= from)
        {
            return Err(HubFailure::answered(
                409,
                Some(CODE_LEASE_LOST),
                "another host took the turn",
            ));
        }
        Ok(turn(turn_id, "bot", "conv", "m1", 0, &[]))
    }

    async fn entries(
        &self,
        _bot: &str,
        _conversation: &str,
    ) -> Result<Vec<InboxEntry>, HubFailure> {
        self.calls.lock().unwrap().push(("owner".into(), "entries"));
        Ok(self.entries.lock().unwrap().clone())
    }

    async fn send(
        &self,
        bot_id: &str,
        conversation_id: &str,
        message: Value,
        turn_id: &str,
        host_id: &str,
    ) -> Result<(), HubFailure> {
        if let Some(failure) = self.send_failures.lock().unwrap().pop_front() {
            return Err(failure);
        }
        self.calls.lock().unwrap().push(("owner".into(), "send"));
        self.sent.lock().unwrap().push(Sent {
            bot_id: bot_id.to_owned(),
            conversation_id: conversation_id.to_owned(),
            message,
            turn_id: turn_id.to_owned(),
            host_id: host_id.to_owned(),
        });
        Ok(())
    }
}

fn turn(
    id: &str,
    bot: &str,
    conversation: &str,
    trigger: &str,
    hops: u32,
    batched: &[&str],
) -> BotTurn {
    BotTurn {
        id: id.into(),
        bot_id: bot.into(),
        conversation_id: conversation.into(),
        trigger_message_id: trigger.into(),
        host_kind: TurnHostKind::Desktop,
        status: TurnStatus::Queued,
        attempts: 0,
        host_id: None,
        lease_until: None,
        reply_message_id: None,
        error_code: None,
        hops,
        owner_id: "owner".into(),
        requester_id: Some("owner".into()),
        requester_name: None,
        audience: flow_like_types::bot_turn::TurnAudience::Owner,
        created_at: Utc::now().to_rfc3339(),
        updated_at: Utc::now().to_rfc3339(),
        batched: batched.iter().map(|id| (*id).to_owned()).collect(),
    }
}

fn entry(seq: i64, message_id: &str, sender: &str, text: &str) -> InboxEntry {
    InboxEntry {
        id: format!("e{seq}"),
        message_id: message_id.into(),
        conversation_id: "conv".into(),
        sender_id: sender.into(),
        kind: InboxEntryKind::Message,
        envelope: plain_envelope(json!({ "text": text })),
        created_at: Utc::now().fixed_offset(),
        seq: Some(seq),
        cursor: String::new(),
    }
}

fn config() -> AccountConfig {
    AccountConfig {
        host_id: HOST.into(),
        poll: Duration::from_millis(20),
        max_backoff: Duration::from_millis(80),
        heartbeat: Duration::from_millis(15),
        deadline: Duration::from_secs(5),
        send_retries: 2,
        send_retry_delay: Duration::from_millis(5),
        presence: Duration::from_millis(40),
    }
}

struct Fixture {
    hub: Arc<FakeHub>,
    runner: Arc<FakeRunner>,
    host: AccountHost,
}

impl Fixture {
    fn start(open: bool, config: AccountConfig) -> Self {
        let hub = Arc::new(FakeHub::default());
        let runner = FakeRunner::new(open);
        let host = AccountHost::new(
            hub.clone(),
            runner.clone(),
            Arc::new(Recorded::default()) as Arc<dyn TurnEvents>,
            Arc::new(Semaphore::new(4)),
            config,
        );
        tokio::spawn(host.clone().run());
        Self { hub, runner, host }
    }

    fn queue(&self, queued: BotTurn, entries: Vec<InboxEntry>) {
        *self.hub.entries.lock().unwrap() = entries;
        self.hub.listed.lock().unwrap().push(queued);
    }

    fn status(&self, bot: &str) -> Option<TurnStatus> {
        self.host
            .turns()
            .into_iter()
            .find(|turn| turn.bot_id == bot)
            .map(|turn| turn.status)
    }

    fn info(&self, bot: &str) -> super::host::TurnInfo {
        self.host
            .turns()
            .into_iter()
            .find(|turn| turn.bot_id == bot)
            .unwrap()
    }
}

fn newest_first(entries: &[(i64, &str, &str, &str)]) -> Vec<InboxEntry> {
    let mut all: Vec<_> = entries
        .iter()
        .map(|(seq, id, sender, text)| entry(*seq, id, sender, text))
        .collect();
    all.reverse();
    all
}

#[test]
fn polls_back_off_by_doubling_up_to_a_cap() {
    let (poll, max) = (Duration::from_secs(30), Duration::from_secs(300));
    let waits: Vec<_> = (0..6).map(|n| backoff(poll, max, n).as_secs()).collect();
    assert_eq!(waits, [30, 60, 120, 240, 300, 300]);
    assert_eq!(backoff(poll, max, 40).as_secs(), 300);
}

#[tokio::test]
async fn a_claimed_turn_is_answered_with_the_turn_it_holds() {
    let f = Fixture::start(true, config());
    f.queue(
        turn("bt_a", "botA", "conv", "m1", 1, &[]),
        newest_first(&[(1, "m1", "user", "hello")]),
    );
    until("the reply", || f.status("botA") == Some(TurnStatus::Done)).await;

    assert_eq!(
        *f.hub.claims.lock().unwrap(),
        [("bt_a".to_owned(), HOST.to_owned())]
    );
    let sent = f.hub.sent.lock().unwrap().clone();
    assert_eq!(sent.len(), 1);
    assert_eq!(
        (sent[0].bot_id.as_str(), sent[0].conversation_id.as_str()),
        ("botA", "conv")
    );
    assert_eq!(
        (sent[0].turn_id.as_str(), sent[0].host_id.as_str()),
        ("bt_a", HOST)
    );
    assert_eq!(sent[0].message["kind"], "message");
    assert_eq!(
        sent[0].message["message_id"],
        reply_message_id("botA", "m1")
    );
    let body = &sent[0].message["envelope"]["body"];
    assert_eq!(body["text"], "answer to: hello");
    assert_eq!(body["reply_to"], "m1");
    assert_eq!(body["hops"], 2);
    assert_eq!(f.runner.jobs.lock().unwrap()[0].run_id, "bt_a");
}

#[tokio::test]
async fn an_account_turn_runs_with_a_bot_runtime_token_minted_for_that_turn() {
    let f = Fixture::start(true, config());
    f.queue(
        turn("bt_a", "botA", "conv", "m1", 1, &[]),
        newest_first(&[(1, "m1", "user", "hello")]),
    );
    until("the reply", || f.status("botA") == Some(TurnStatus::Done)).await;

    assert_eq!(
        *f.hub.token_requests.lock().unwrap(),
        [("botA".to_owned(), "bt_a".to_owned())]
    );
    let job = f.runner.jobs.lock().unwrap()[0].clone();
    assert_eq!(
        job.hub_token.map(|token| token.0).as_deref(),
        Some("bot-token:botA:bt_a")
    );
}

#[tokio::test]
async fn an_account_turn_reads_and_answers_with_only_the_runtime_token() {
    let f = Fixture::start(true, config());
    f.queue(
        turn("bt_a", "botA", "conv", "m1", 1, &[]),
        newest_first(&[(1, "m1", "user", "hello")]),
    );
    until("the reply", || f.status("botA") == Some(TurnStatus::Done)).await;

    let calls = f.hub.calls.lock().unwrap().clone();
    let token = "bot-token:botA:bt_a".to_owned();
    assert_eq!(
        calls,
        [(token.clone(), "entries"), (token, "send")],
        "the owner's session made no in-turn read or send"
    );
    assert_eq!(f.hub.sent.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn a_turn_whose_bot_token_cannot_be_minted_runs_nothing() {
    let f = Fixture::start(true, config());
    f.hub.token_failure.store(true, Ordering::SeqCst);
    f.queue(
        turn("bt_a", "botA", "conv", "m1", 1, &[]),
        newest_first(&[(1, "m1", "user", "hello")]),
    );
    until("the failure", || f.status("botA") == Some(TurnStatus::Failed)).await;
    assert_eq!(f.runner.started(), 0);
}

#[tokio::test]
async fn a_lost_claim_race_runs_nothing_and_posts_nothing() {
    let f = Fixture::start(true, config());
    *f.hub.claim_failure.lock().unwrap() = Some(HubFailure::answered(
        409,
        Some("BOT_TURN_BUSY"),
        "another turn of this bot is running",
    ));
    f.queue(
        turn("bt_a", "botA", "conv", "m1", 0, &[]),
        newest_first(&[(1, "m1", "user", "hello")]),
    );
    until("the lost race", || {
        f.status("botA") == Some(TurnStatus::Cancelled)
    })
    .await;
    settle().await;

    assert_eq!(f.info("botA").error_code.as_deref(), Some(CODE_TAKEN));
    assert_eq!(f.runner.started(), 0);
    assert!(f.hub.sent.lock().unwrap().is_empty());
    assert!(f.hub.heartbeats.lock().unwrap().is_empty());
}

#[tokio::test]
async fn folded_triggers_are_answered_by_the_lead_turn_once() {
    let f = Fixture::start(true, config());
    *f.hub.lead.lock().unwrap() = Some(turn("bt_lead", "botA", "conv", "m3", 0, &["m1", "m2"]));
    f.queue(
        turn("bt_old", "botA", "conv", "m1", 0, &[]),
        newest_first(&[
            (1, "m1", "user", "one"),
            (2, "m2", "user", "two"),
            (3, "m3", "user", "three"),
        ]),
    );
    until("the reply", || f.status("botA") == Some(TurnStatus::Done)).await;
    settle().await;

    let jobs = f.runner.jobs.lock().unwrap().clone();
    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[0].prompt, "three");
    assert_eq!(jobs[0].history.len(), 2);
    let sent = f.hub.sent.lock().unwrap().clone();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].turn_id, "bt_lead");
    assert_eq!(
        sent[0].message["message_id"],
        reply_message_id("botA", "m3")
    );
    assert_eq!(sent[0].message["envelope"]["body"]["reply_to"], "m3");
    assert_eq!(f.info("botA").batched, 2);
}

#[tokio::test]
async fn the_lease_is_kept_alive_while_the_turn_runs_and_not_after() {
    let f = Fixture::start(false, config());
    f.queue(
        turn("bt_a", "botA", "conv", "m1", 0, &[]),
        newest_first(&[(1, "m1", "user", "hello")]),
    );
    until("three heartbeats", || {
        f.hub.heartbeats.lock().unwrap().len() >= 3
    })
    .await;
    assert!(
        f.hub
            .heartbeats
            .lock()
            .unwrap()
            .iter()
            .all(|beat| beat == &("bt_a".to_owned(), HOST.to_owned()))
    );

    f.runner.gate.add_permits(1);
    until("the reply", || f.status("botA") == Some(TurnStatus::Done)).await;
    let beats = f.hub.heartbeats.lock().unwrap().len();
    settle().await;
    assert_eq!(f.hub.heartbeats.lock().unwrap().len(), beats);
}

#[tokio::test]
async fn a_turn_whose_lease_was_lost_is_dropped_without_a_reply() {
    let f = Fixture::start(false, config());
    *f.hub.lease_lost_from.lock().unwrap() = Some(2);
    f.queue(
        turn("bt_a", "botA", "conv", "m1", 0, &[]),
        newest_first(&[(1, "m1", "user", "hello")]),
    );
    until("the drop", || {
        f.status("botA") == Some(TurnStatus::Cancelled)
    })
    .await;

    f.runner.gate.add_permits(1);
    settle().await;
    assert_eq!(f.info("botA").error_code.as_deref(), Some(CODE_LEASE_LOST));
    assert_eq!(*f.runner.cancelled.lock().unwrap(), ["bt_a".to_owned()]);
    assert!(f.hub.sent.lock().unwrap().is_empty());
}

#[tokio::test]
async fn a_reply_the_hub_refuses_for_a_lost_lease_is_dropped_silently() {
    let f = Fixture::start(true, config());
    f.hub
        .send_failures
        .lock()
        .unwrap()
        .push_back(HubFailure::answered(
            409,
            Some("BOT_TURN_NOT_CLAIMED"),
            "not running under this host",
        ));
    f.queue(
        turn("bt_a", "botA", "conv", "m1", 0, &[]),
        newest_first(&[(1, "m1", "user", "hello")]),
    );
    until("the drop", || {
        f.status("botA") == Some(TurnStatus::Cancelled)
    })
    .await;
    assert_eq!(f.info("botA").error_code.as_deref(), Some(CODE_LEASE_LOST));
}

#[tokio::test]
async fn a_reply_is_retried_while_the_hub_is_unreachable() {
    let f = Fixture::start(true, config());
    for _ in 0..2 {
        f.hub
            .send_failures
            .lock()
            .unwrap()
            .push_back(HubFailure::offline("the hub could not be reached"));
    }
    f.queue(
        turn("bt_a", "botA", "conv", "m1", 0, &[]),
        newest_first(&[(1, "m1", "user", "hello")]),
    );
    until("the reply", || f.status("botA") == Some(TurnStatus::Done)).await;
    assert_eq!(f.hub.sent.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn a_failed_run_posts_the_failure_under_the_turn() {
    let f = Fixture::start(true, config());
    *f.runner.mode.lock().unwrap() = Mode::Fail;
    f.queue(
        turn("bt_a", "botA", "conv", "m1", 0, &[]),
        newest_first(&[(1, "m1", "user", "hello")]),
    );
    until("the failure", || {
        f.status("botA") == Some(TurnStatus::Failed)
    })
    .await;
    let sent = f.hub.sent.lock().unwrap().clone();
    assert_eq!(
        sent[0].message["message_id"],
        failure_message_id("botA", "m1")
    );
    assert_eq!(
        sent[0].message["envelope"]["body"]["error"]["code"],
        "BOT_TURN_FAILED"
    );
    assert_eq!(sent[0].turn_id, "bt_a");
}

#[tokio::test(start_paused = true)]
async fn an_unreachable_hub_is_polled_less_and_less_often_until_it_answers() {
    let f = Fixture::start(true, {
        let mut config = config();
        config.poll = Duration::from_millis(100);
        config.max_backoff = Duration::from_millis(400);
        config
    });
    f.hub.offline.store(true, Ordering::SeqCst);
    tokio::time::sleep(Duration::from_millis(1_050)).await;
    let gaps = |lists: &[Instant]| -> Vec<u128> {
        lists
            .windows(2)
            .map(|pair| (pair[1] - pair[0]).as_millis())
            .collect()
    };
    assert_eq!(gaps(&f.hub.lists.lock().unwrap()), [200, 400, 400]);

    f.hub.offline.store(false, Ordering::SeqCst);
    tokio::time::sleep(Duration::from_millis(900)).await;
    let lists = f.hub.lists.lock().unwrap().clone();
    let recovered = gaps(&lists);
    assert_eq!(recovered.last(), Some(&100));
}

#[tokio::test]
async fn the_bots_that_live_here_are_reported_on_a_cadence_and_withdrawn_on_sign_out() {
    let f = Fixture::start(true, config());
    *f.hub.bots.lock().unwrap() = vec!["botA".into(), "botB".into()];
    until("three reports", || f.hub.reports.lock().unwrap().len() >= 3).await;
    {
        let reports = f.hub.reports.lock().unwrap();
        assert!(
            reports
                .iter()
                .all(|(host, ids, _)| host == HOST && ids == &["botA", "botB"])
        );
        let gap = reports[2].2 - reports[1].2;
        assert!(gap >= Duration::from_millis(35), "{gap:?}");
    }
    assert!(f.hub.deleted.lock().unwrap().is_empty());

    f.hub.offline.store(true, Ordering::SeqCst);
    until("the delete", || !f.hub.deleted.lock().unwrap().is_empty()).await;
    settle().await;
    assert_eq!(*f.hub.deleted.lock().unwrap(), [HOST.to_owned()]);
}

#[tokio::test]
async fn withdrawing_deletes_the_host_only_when_it_reported() {
    let f = Fixture::start(true, config());
    f.host.withdraw().await;
    assert!(f.hub.deleted.lock().unwrap().is_empty());

    *f.hub.bots.lock().unwrap() = vec!["botA".into()];
    until("a report", || !f.hub.reports.lock().unwrap().is_empty()).await;
    f.host.withdraw().await;
    assert_eq!(*f.hub.deleted.lock().unwrap(), [HOST.to_owned()]);
}

#[tokio::test]
async fn cancelling_a_turn_goes_through_the_hubs_cancel_endpoint() {
    let f = Fixture::start(false, config());
    f.queue(
        turn("bt_a", "botA", "conv", "m1", 0, &[]),
        newest_first(&[(1, "m1", "user", "hello")]),
    );
    until("the run", || f.runner.started() == 1).await;

    assert!(f.host.cancel("bt_a").await);
    until("the cancellation", || {
        f.status("botA") == Some(TurnStatus::Cancelled)
    })
    .await;
    assert_eq!(
        *f.hub.cancelled.lock().unwrap(),
        [("botA".to_owned(), "bt_a".to_owned())]
    );
    assert!(f.hub.sent.lock().unwrap().is_empty());
    assert!(!f.host.cancel("bt_a").await);
}
