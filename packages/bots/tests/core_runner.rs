//! The runner with a fake provider and a fake host, on tokio's paused clock.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use flow_like_bots::limits::ImageBudget;
use flow_like_bots::reply::{Answer, ReplySink, SendError};
use flow_like_bots::state::{Outcome, Status, utc_date};
use flow_like_bots::{
    Bot, BotHost, BotSpec, BotToken, BotsState, Connection, Connector, End, Gate, Intake,
    LinkState, Message, Provider, Refusal, Resume, Revisions, RunEnd, StreamEvent, Taken, run_with,
};
use serde_json::{Value, json};
use tokio::sync::{Notify, mpsc, oneshot, watch};
use tokio::task::JoinHandle;
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

const TOKEN: &str = "123456789:AAHdqTcvCH1vGWJxfSeofSAs0K5PALDsaw";
const OTHER_TOKEN: &str = "987654321:BBHdqTcvCH1vGWJxfSeofSAs0K5PALDsaw";
const BASE: i64 = 1_790_035_200;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Behaviour {
    Succeed,
    Fail,
    Hang,
    Ask,
}

struct RunRecord {
    event_id: String,
    payload: Value,
    watermark: Option<i64>,
}

struct Host {
    gates: Mutex<HashMap<String, watch::Sender<Gate>>>,
    stored: Mutex<Option<BotsState>>,
    saves: Mutex<Vec<String>>,
    origin: Instant,
    base: i64,
    behaviour: Mutex<HashMap<String, Behaviour>>,
    runs: Mutex<Vec<RunRecord>>,
    release: Notify,
    refuse_saves: AtomicBool,
}

impl Host {
    fn new() -> Arc<Self> {
        Self::at(BASE)
    }

    fn at(base: i64) -> Arc<Self> {
        Arc::new(Self {
            gates: Mutex::new(HashMap::new()),
            stored: Mutex::new(None),
            saves: Mutex::new(Vec::new()),
            origin: Instant::now(),
            base,
            behaviour: Mutex::new(HashMap::new()),
            runs: Mutex::new(Vec::new()),
            release: Notify::new(),
            refuse_saves: AtomicBool::new(false),
        })
    }

    fn sender(&self, event_id: &str) -> watch::Sender<Gate> {
        self.gates
            .lock()
            .unwrap()
            .entry(event_id.to_string())
            .or_insert_with(|| watch::channel(Gate::Undecided).0)
            .clone()
    }

    fn set_gate(&self, event_id: &str, gate: Gate) {
        self.sender(event_id).send_replace(gate);
    }

    fn behave(&self, event_id: &str, behaviour: Behaviour) {
        self.behaviour
            .lock()
            .unwrap()
            .insert(event_id.to_string(), behaviour);
    }

    fn state(&self) -> BotsState {
        self.stored.lock().unwrap().clone().expect("a saved state")
    }

    fn runs(&self) -> Vec<String> {
        self.runs
            .lock()
            .unwrap()
            .iter()
            .map(|run| run.event_id.clone())
            .collect()
    }
}

#[async_trait]
impl BotHost for Host {
    async fn run(
        &self,
        event_id: &str,
        payload: Value,
        events: mpsc::Sender<StreamEvent>,
        cancel: CancellationToken,
    ) -> RunEnd {
        let watermark = self
            .stored
            .lock()
            .unwrap()
            .as_ref()
            .and_then(|state| state.bots.get(event_id)?.watermark);
        self.runs.lock().unwrap().push(RunRecord {
            event_id: event_id.to_string(),
            payload,
            watermark,
        });
        let behaviour = self
            .behaviour
            .lock()
            .unwrap()
            .get(event_id)
            .copied()
            .unwrap_or(Behaviour::Succeed);
        let _ = events
            .send(StreamEvent::new(
                "chat_stream_partial",
                json!({"chunk":{"choices":[{"delta":{"content":"answer"}}]}}),
            ))
            .await;
        match behaviour {
            Behaviour::Succeed => RunEnd::Succeeded,
            Behaviour::Fail => RunEnd::Failed,
            Behaviour::Hang => tokio::select! {
                () = cancel.cancelled() => RunEnd::Cancelled,
                () = self.release.notified() => RunEnd::Succeeded,
            },
            Behaviour::Ask => {
                let _ = events
                    .send(StreamEvent::new("interaction_request", json!({})))
                    .await;
                cancel.cancelled().await;
                RunEnd::Cancelled
            }
        }
    }

    fn gate(&self, event_id: &str) -> watch::Receiver<Gate> {
        self.sender(event_id).subscribe()
    }

    fn load(&self) -> Option<BotsState> {
        self.stored.lock().unwrap().clone()
    }

    fn save(&self, state: &BotsState) -> std::io::Result<()> {
        if self.refuse_saves.load(Ordering::SeqCst) {
            return Err(std::io::Error::other("disk full"));
        }
        self.saves
            .lock()
            .unwrap()
            .push(serde_json::to_string(state).unwrap());
        *self.stored.lock().unwrap() = Some(state.clone());
        Ok(())
    }

    fn now(&self) -> i64 {
        self.base + i64::try_from(self.origin.elapsed().as_secs()).unwrap()
    }

    fn handle(&self, event_id: &str) -> String {
        format!("device-bot:{event_id}")
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Serve {
    Hold,
    End,
    Refuse,
}

struct Conn {
    mode: Mutex<Serve>,
    bot_id: Mutex<u64>,
    bot_name: Option<String>,
    serves: AtomicUsize,
    stopped: AtomicUsize,
    intake: Mutex<Option<Intake>>,
    resumes: Mutex<Vec<Resume>>,
    notices: Mutex<Vec<(String, String)>>,
    replies: Mutex<Vec<(String, bool, String)>>,
    specs: Vec<String>,
}

impl Conn {
    fn intake(&self) -> Intake {
        self.intake
            .lock()
            .unwrap()
            .clone()
            .expect("a connected intake")
    }

    fn notices(&self) -> Vec<(String, String)> {
        self.notices.lock().unwrap().clone()
    }
}

struct ChatSink {
    conn: Arc<Conn>,
    chat: String,
}

#[async_trait]
impl ReplySink for ChatSink {
    async fn show(&mut self, answer: &Answer, done: bool) -> Result<(), SendError> {
        self.conn
            .replies
            .lock()
            .unwrap()
            .push((self.chat.clone(), done, answer.text.clone()));
        Ok(())
    }

    fn edit_interval(&self, _shown: u32) -> Duration {
        Duration::from_secs(1)
    }
}

struct Shared(Arc<Conn>);

#[async_trait]
impl Connection for Shared {
    async fn serve(&self, intake: Intake, stop: CancellationToken) -> End {
        let conn = &self.0;
        conn.serves.fetch_add(1, Ordering::SeqCst);
        let mode = *conn.mode.lock().unwrap();
        match mode {
            Serve::End => End::Ended,
            Serve::Refuse => End::Refused(Refusal::Token),
            Serve::Hold => {
                let bot_id = *conn.bot_id.lock().unwrap();
                let resume = intake.connected(bot_id, conn.bot_name.as_deref());
                conn.resumes.lock().unwrap().push(resume);
                *conn.intake.lock().unwrap() = Some(intake);
                stop.cancelled().await;
                *conn.intake.lock().unwrap() = None;
                conn.stopped.fetch_add(1, Ordering::SeqCst);
                End::Stopped
            }
        }
    }

    async fn payload(&self, message: &Message, handle: &str, images: &mut ImageBudget) -> Value {
        let size = message.native::<u64>().copied().unwrap_or(0);
        if size > 0 {
            images.take(size);
        }
        json!({"local_session": {"bot_token": handle, "chat_id": message.chat},
               "messages": [{"role": "user", "content": message.text}]})
    }

    fn reply(&self, message: &Message) -> Box<dyn ReplySink> {
        Box::new(ChatSink {
            conn: self.0.clone(),
            chat: message.chat.clone(),
        })
    }

    async fn notice(&self, message: &Message, text: &str) {
        self.0
            .notices
            .lock()
            .unwrap()
            .push((message.chat.clone(), text.to_string()));
    }
}

struct Fake {
    mode: Serve,
    bot_name: Option<String>,
    conns: Mutex<Vec<Arc<Conn>>>,
}

impl Fake {
    fn new(mode: Serve) -> Arc<Self> {
        Arc::new(Self {
            mode,
            bot_name: Some("helper_bot".into()),
            conns: Mutex::new(Vec::new()),
        })
    }

    fn conn(&self, index: usize) -> Arc<Conn> {
        self.conns.lock().unwrap()[index].clone()
    }
}

impl Connector for Fake {
    fn connect(
        &self,
        _provider: Provider,
        token: &BotToken,
        specs: &[BotSpec],
    ) -> Option<Arc<dyn Connection>> {
        let conn = Arc::new(Conn {
            mode: Mutex::new(self.mode),
            bot_id: Mutex::new(token.bot_id().unwrap_or(1)),
            bot_name: self.bot_name.clone(),
            serves: AtomicUsize::new(0),
            stopped: AtomicUsize::new(0),
            intake: Mutex::new(None),
            resumes: Mutex::new(Vec::new()),
            notices: Mutex::new(Vec::new()),
            replies: Mutex::new(Vec::new()),
            specs: specs.iter().map(|spec| spec.event_id.clone()).collect(),
        });
        self.conns.lock().unwrap().push(conn.clone());
        Some(Arc::new(Shared(conn)))
    }
}

fn bot(event_id: &str, config: Value, token: &str) -> Bot {
    Bot {
        spec: BotSpec::from_config(event_id, "telegram", config.to_string().as_bytes()).unwrap(),
        token: BotToken::parse(Provider::Telegram, token.as_bytes()).unwrap(),
    }
}

struct Running {
    host: Arc<Host>,
    fake: Arc<Fake>,
    stop: CancellationToken,
    ready: Option<oneshot::Sender<()>>,
    task: JoinHandle<()>,
}

impl Running {
    fn start(bots: Vec<Bot>, host: Arc<Host>, fake: Arc<Fake>) -> Self {
        let (ready, waiting) = oneshot::channel::<()>();
        let stop = CancellationToken::new();
        let task = tokio::spawn(run_with(
            bots,
            Revisions {
                config_revision: 12,
                intent_revision: 7,
            },
            host.clone(),
            fake.clone(),
            async move {
                if waiting.await.is_err() {
                    std::future::pending::<()>().await;
                }
            },
            stop.clone(),
        ));
        Self {
            host,
            fake,
            stop,
            ready: Some(ready),
            task,
        }
    }

    fn ready(&mut self) {
        let _ = self.ready.take().unwrap().send(());
    }

    async fn stop(self) -> (Arc<Host>, Arc<Fake>) {
        self.stop.cancel();
        tokio::time::timeout(Duration::from_secs(30), self.task)
            .await
            .expect("run returns after a stop")
            .unwrap();
        (self.host, self.fake)
    }
}

async fn settle() {
    tokio::time::sleep(Duration::from_millis(50)).await;
}

fn message(chat: &str, update_id: i64) -> Message {
    let mut message = Message::new(chat, ());
    message.update_id = Some(update_id);
    message.private = true;
    message.text = "hello".into();
    message
}

fn entry(host: &Host, event_id: &str) -> flow_like_bots::state::BotEntry {
    host.state().bots[event_id].clone()
}

#[tokio::test(start_paused = true)]
async fn nothing_connects_before_ready_and_an_open_gate() {
    let host = Host::new();
    let fake = Fake::new(Serve::Hold);
    let mut running = Running::start(
        vec![bot("evt_helper", json!({}), TOKEN)],
        host.clone(),
        fake.clone(),
    );
    settle().await;
    host.set_gate("evt_helper", Gate::Open);
    settle().await;
    assert_eq!(
        fake.conn(0).serves.load(Ordering::SeqCst),
        0,
        "connected before ready"
    );
    let first = entry(&host, "evt_helper");
    assert_eq!(first.state, Status::Waiting);
    assert!(!host.state().decided);

    host.set_gate("evt_helper", Gate::Undecided);
    running.ready();
    settle().await;
    assert_eq!(
        fake.conn(0).serves.load(Ordering::SeqCst),
        0,
        "connected while undecided"
    );
    assert!(!host.state().decided);

    host.set_gate("evt_helper", Gate::Held("not_released"));
    settle().await;
    assert_eq!(fake.conn(0).serves.load(Ordering::SeqCst), 0);
    let held = entry(&host, "evt_helper");
    assert_eq!(
        (held.state, held.hold.as_deref()),
        (Status::Waiting, Some("not_released"))
    );
    assert!(host.state().decided);

    host.set_gate("evt_helper", Gate::Open);
    settle().await;
    assert_eq!(fake.conn(0).serves.load(Ordering::SeqCst), 1);
    let connected = entry(&host, "evt_helper");
    assert_eq!((connected.state, connected.hold), (Status::Connected, None));
    assert_eq!(connected.bot_id, Some(123_456_789));
    assert_eq!(connected.bot_name.as_deref(), Some("helper_bot"));
    assert!(connected.connected_at.is_some());
    let saved = host.saves.lock().unwrap().clone();
    assert!(
        saved
            .iter()
            .any(|state| state.contains("\"state\":\"connecting\""))
    );
    running.stop().await;
}

#[tokio::test(start_paused = true)]
async fn a_closed_gate_disconnects_and_an_open_one_connects_again() {
    let host = Host::new();
    host.set_gate("evt_helper", Gate::Open);
    let fake = Fake::new(Serve::Hold);
    let mut running = Running::start(
        vec![bot("evt_helper", json!({}), TOKEN)],
        host.clone(),
        fake.clone(),
    );
    running.ready();
    settle().await;
    let conn = fake.conn(0);
    assert_eq!(conn.serves.load(Ordering::SeqCst), 1);

    host.set_gate("evt_helper", Gate::Held("runs_elsewhere"));
    settle().await;
    assert_eq!(
        conn.stopped.load(Ordering::SeqCst),
        1,
        "the closed gate did not disconnect"
    );
    let held = entry(&host, "evt_helper");
    assert_eq!(
        (held.state, held.hold.as_deref()),
        (Status::Waiting, Some("runs_elsewhere"))
    );
    assert_eq!(held.connected_at, None);

    host.set_gate("evt_helper", Gate::Open);
    settle().await;
    assert_eq!(conn.serves.load(Ordering::SeqCst), 2);
    assert_eq!(entry(&host, "evt_helper").state, Status::Connected);
    running.stop().await;
}

#[tokio::test(start_paused = true)]
async fn a_message_becomes_one_run_with_the_handle_after_its_watermark() {
    let host = Host::new();
    host.set_gate("evt_helper", Gate::Open);
    let fake = Fake::new(Serve::Hold);
    let mut running = Running::start(
        vec![bot("evt_helper", json!({}), TOKEN)],
        host.clone(),
        fake.clone(),
    );
    running.ready();
    settle().await;
    let intake = fake.conn(0).intake();
    assert_eq!(intake.message(message("42", 1_000)).await, Taken::Run);
    settle().await;
    {
        let runs = host.runs.lock().unwrap();
        assert_eq!(runs.len(), 1);
        assert_eq!(
            runs[0].payload["local_session"]["bot_token"],
            "device-bot:evt_helper"
        );
        assert_eq!(
            runs[0].watermark,
            Some(1_000),
            "the watermark was not written before the run"
        );
    }
    assert_eq!(intake.message(message("42", 1_000)).await, Taken::Seen);
    assert_eq!(intake.message(message("42", 999)).await, Taken::Seen);
    intake.ignore(Some(1_001));
    intake.pass(Some(1_002));
    assert_eq!(intake.watermark(), Some(1_002));
    tokio::time::sleep(Duration::from_secs(11)).await;
    let replies = fake.conn(0).replies.lock().unwrap().clone();
    assert!(replies.contains(&("42".into(), true, "answer".into())));
    let done = entry(&host, "evt_helper");
    assert_eq!(
        (done.messages, done.runs, done.ignored, done.failed),
        (2, 1, 1, 0)
    );
    assert_eq!(done.last.unwrap().outcome, Outcome::Succeeded);
    assert_eq!(done.day.unwrap().runs, 1);
    assert_eq!(done.running, 0);
    running.stop().await;
}

#[tokio::test(start_paused = true)]
async fn one_running_and_four_waiting_per_chat_then_dropped_with_one_notice() {
    let host = Host::new();
    host.set_gate("evt_helper", Gate::Open);
    host.behave("evt_helper", Behaviour::Hang);
    let fake = Fake::new(Serve::Hold);
    let mut running = Running::start(
        vec![bot("evt_helper", json!({}), TOKEN)],
        host.clone(),
        fake.clone(),
    );
    running.ready();
    settle().await;
    let intake = fake.conn(0).intake();
    assert_eq!(intake.message(message("7", 1)).await, Taken::Run);
    for id in 2..=5 {
        assert_eq!(intake.message(message("7", id)).await, Taken::Queued);
    }
    assert_eq!(intake.message(message("7", 6)).await, Taken::Busy);
    assert_eq!(intake.message(message("7", 7)).await, Taken::Busy);
    settle().await;
    assert_eq!(host.runs().len(), 1);
    let busy: Vec<_> = fake
        .conn(0)
        .notices()
        .into_iter()
        .filter(|(_, text)| text.contains("too many"))
        .collect();
    assert_eq!(busy.len(), 1, "one notice per chat and minute");
    host.release.notify_one();
    settle().await;
    assert_eq!(
        host.runs().len(),
        2,
        "the next waiting message runs after the first"
    );
    assert_eq!(entry(&host, "evt_helper").dropped_busy, 2);
    for _ in 0..4 {
        host.release.notify_one();
        settle().await;
    }
    assert_eq!(host.runs().len(), 5);
    running.stop().await;
}

#[tokio::test(start_paused = true)]
async fn ten_runs_a_minute_per_chat() {
    let host = Host::new();
    host.set_gate("evt_helper", Gate::Open);
    let fake = Fake::new(Serve::Hold);
    let mut running = Running::start(
        vec![bot("evt_helper", json!({}), TOKEN)],
        host.clone(),
        fake.clone(),
    );
    running.ready();
    settle().await;
    let intake = fake.conn(0).intake();
    for id in 1..=10 {
        assert_eq!(
            intake.message(message("7", id)).await,
            Taken::Run,
            "message {id}"
        );
        settle().await;
    }
    assert_eq!(intake.message(message("7", 11)).await, Taken::Flood);
    assert_eq!(intake.message(message("8", 12)).await, Taken::Run);
    settle().await;
    let flooded = entry(&host, "evt_helper");
    assert_eq!((flooded.runs, flooded.dropped_flood), (11, 1));
    tokio::time::sleep(Duration::from_secs(60)).await;
    assert_eq!(intake.message(message("7", 13)).await, Taken::Run);
    running.stop().await;
}

#[tokio::test(start_paused = true)]
async fn eight_runs_at_once() {
    let host = Host::new();
    host.set_gate("evt_helper", Gate::Open);
    host.behave("evt_helper", Behaviour::Hang);
    let fake = Fake::new(Serve::Hold);
    let mut running = Running::start(
        vec![bot("evt_helper", json!({}), TOKEN)],
        host.clone(),
        fake.clone(),
    );
    running.ready();
    settle().await;
    let intake = fake.conn(0).intake();
    for chat in 0..9 {
        assert_eq!(
            intake.message(message(&chat.to_string(), chat + 1)).await,
            Taken::Run
        );
    }
    settle().await;
    assert_eq!(host.runs().len(), 8);
    assert_eq!(entry(&host, "evt_helper").running, 8);
    host.release.notify_one();
    settle().await;
    assert_eq!(host.runs().len(), 9);
    running.stop().await;
}

#[tokio::test(start_paused = true)]
async fn two_events_of_one_bot_start_one_run() {
    let host = Host::new();
    host.set_gate("evt_first", Gate::Open);
    host.set_gate("evt_second", Gate::Open);
    let fake = Fake::new(Serve::Hold);
    let mut running = Running::start(
        vec![
            bot("evt_first", json!({}), TOKEN),
            bot("evt_second", json!({}), TOKEN),
            bot("evt_other", json!({}), OTHER_TOKEN),
        ],
        host.clone(),
        fake.clone(),
    );
    host.set_gate("evt_other", Gate::Open);
    running.ready();
    settle().await;
    assert_eq!(
        fake.conns.lock().unwrap().len(),
        2,
        "one connection per token"
    );
    assert_eq!(fake.conn(0).specs, ["evt_first", "evt_second"]);
    let intake = fake.conn(0).intake();
    assert_eq!(intake.message(message("1", 1)).await, Taken::Run);
    settle().await;
    assert_eq!(host.runs(), ["evt_first"]);

    host.set_gate("evt_first", Gate::Held("other_service"));
    settle().await;
    assert_eq!(
        fake.conn(0).serves.load(Ordering::SeqCst),
        1,
        "kept while one event may connect"
    );
    assert_eq!(intake.message(message("1", 2)).await, Taken::Run);
    settle().await;
    assert_eq!(host.runs(), ["evt_first", "evt_second"]);
    let first = entry(&host, "evt_first");
    assert_eq!(
        (first.state, first.hold.as_deref()),
        (Status::Waiting, Some("other_service"))
    );
    assert_eq!(entry(&host, "evt_second").state, Status::Connected);
    running.stop().await;
}

#[tokio::test(start_paused = true)]
async fn the_first_event_whose_filter_passes_takes_the_message() {
    let host = Host::new();
    host.set_gate("evt_admins", Gate::Open);
    host.set_gate("evt_everyone", Gate::Open);
    let fake = Fake::new(Serve::Hold);
    let mut running = Running::start(
        vec![
            bot("evt_admins", json!({"chat_whitelist": ["1"]}), TOKEN),
            bot("evt_everyone", json!({"respond_to_private": true}), TOKEN),
        ],
        host.clone(),
        fake.clone(),
    );
    running.ready();
    settle().await;
    let intake = fake.conn(0).intake();
    assert_eq!(intake.message(message("1", 1)).await, Taken::Run);
    assert_eq!(intake.message(message("2", 2)).await, Taken::Run);
    let mut group = message("3", 3);
    group.private = false;
    assert_eq!(intake.message(group).await, Taken::Ignored);
    settle().await;
    assert_eq!(host.runs(), ["evt_admins", "evt_everyone"]);
    assert_eq!(entry(&host, "evt_admins").ignored, 1);
    assert_eq!(entry(&host, "evt_everyone").ignored, 0);
    running.stop().await;
}

#[tokio::test(start_paused = true)]
async fn a_failing_run_and_a_refused_token_do_not_end_the_runner() {
    let host = Host::new();
    host.set_gate("evt_helper", Gate::Open);
    host.behave("evt_helper", Behaviour::Fail);
    let fake = Fake::new(Serve::Hold);
    let mut running = Running::start(
        vec![bot("evt_helper", json!({}), TOKEN)],
        host.clone(),
        fake.clone(),
    );
    running.ready();
    settle().await;
    let conn = fake.conn(0);
    assert_eq!(conn.intake().message(message("1", 1)).await, Taken::Run);
    settle().await;
    let failed = entry(&host, "evt_helper");
    assert_eq!((failed.runs, failed.failed), (1, 1));
    assert_eq!(failed.last.unwrap().outcome, Outcome::Failed);
    assert!(!running.task.is_finished());

    *conn.mode.lock().unwrap() = Serve::Refuse;
    host.set_gate("evt_helper", Gate::Held("hub_unreachable"));
    settle().await;
    host.set_gate("evt_helper", Gate::Open);
    settle().await;
    assert_eq!(entry(&host, "evt_helper").state, Status::TokenRefused);
    let serves = conn.serves.load(Ordering::SeqCst);
    tokio::time::sleep(Duration::from_secs(3_600)).await;
    assert_eq!(
        conn.serves.load(Ordering::SeqCst),
        serves,
        "a refused token is tried again"
    );
    host.set_gate("evt_helper", Gate::Held("not_released"));
    settle().await;
    host.set_gate("evt_helper", Gate::Open);
    settle().await;
    assert_eq!(conn.serves.load(Ordering::SeqCst), serves);
    assert_eq!(entry(&host, "evt_helper").state, Status::TokenRefused);
    assert!(!running.task.is_finished());
    running.stop().await;
}

#[tokio::test(start_paused = true)]
async fn a_question_fails_the_run_at_once() {
    let host = Host::new();
    host.set_gate("evt_helper", Gate::Open);
    host.behave("evt_helper", Behaviour::Ask);
    let fake = Fake::new(Serve::Hold);
    let mut running = Running::start(
        vec![bot("evt_helper", json!({}), TOKEN)],
        host.clone(),
        fake.clone(),
    );
    running.ready();
    settle().await;
    fake.conn(0).intake().message(message("1", 1)).await;
    settle().await;
    let asked = entry(&host, "evt_helper");
    assert_eq!((asked.failed, asked.running), (1, 0));
    assert_eq!(asked.last.unwrap().outcome, Outcome::Failed);
    running.stop().await;
}

#[tokio::test(start_paused = true)]
async fn a_provider_that_keeps_ending_is_started_at_most_twenty_times_an_hour() {
    let host = Host::new();
    host.set_gate("evt_helper", Gate::Open);
    let fake = Fake::new(Serve::End);
    let mut running = Running::start(
        vec![bot("evt_helper", json!({}), TOKEN)],
        host.clone(),
        fake.clone(),
    );
    running.ready();
    tokio::time::sleep(Duration::from_secs(3_600)).await;
    let starts = fake.conn(0).serves.load(Ordering::SeqCst);
    assert!((10..=20).contains(&starts), "{starts} starts in an hour");
    let ended = entry(&host, "evt_helper");
    assert_eq!(ended.state, Status::Reconnecting);
    assert!(ended.reconnects >= 1);
    tokio::time::sleep(Duration::from_secs(3_600)).await;
    let later = fake.conn(0).serves.load(Ordering::SeqCst);
    assert!(
        later - starts <= 20,
        "{} starts in the second hour",
        later - starts
    );
    assert!(!running.task.is_finished());
    running.stop().await;
}

#[tokio::test(start_paused = true)]
async fn a_flapping_gate_starts_at_most_twenty_connections_an_hour() {
    let host = Host::new();
    host.set_gate("evt_helper", Gate::Open);
    let fake = Fake::new(Serve::Hold);
    let mut running = Running::start(
        vec![bot("evt_helper", json!({}), TOKEN)],
        host.clone(),
        fake.clone(),
    );
    running.ready();
    settle().await;
    for _ in 0..30 {
        host.set_gate("evt_helper", Gate::Held("hub_unreachable"));
        settle().await;
        host.set_gate("evt_helper", Gate::Open);
        settle().await;
    }
    let conn = fake.conn(0);
    assert_eq!(conn.serves.load(Ordering::SeqCst), 20);
    assert_eq!(entry(&host, "evt_helper").state, Status::Reconnecting);
    host.set_gate("evt_helper", Gate::Held("not_released"));
    settle().await;
    let held = entry(&host, "evt_helper");
    assert_eq!(
        (held.state, held.hold.as_deref()),
        (Status::Waiting, Some("not_released"))
    );
    host.set_gate("evt_helper", Gate::Open);
    tokio::time::sleep(Duration::from_secs(3_600)).await;
    assert_eq!(conn.serves.load(Ordering::SeqCst), 21);
    assert_eq!(entry(&host, "evt_helper").state, Status::Connected);
    running.stop().await;
}

#[tokio::test(start_paused = true)]
async fn another_bot_does_not_inherit_the_watermark() {
    let host = Host::new();
    let mut stored = BotsState::prepare(
        None,
        [&bot("evt_helper", json!({}), TOKEN).spec],
        Revisions {
            config_revision: 11,
            intent_revision: 7,
        },
        BASE,
    );
    let entry_before = stored.bots.get_mut("evt_helper").unwrap();
    entry_before.bot_id = Some(111);
    entry_before.watermark = Some(5_000);
    entry_before.watermark_at = Some(BASE - 60);
    entry_before.webhook_cleared = true;
    *host.stored.lock().unwrap() = Some(stored.clone());
    host.set_gate("evt_helper", Gate::Open);

    let fake = Fake::new(Serve::Hold);
    let mut running = Running::start(
        vec![bot("evt_helper", json!({}), TOKEN)],
        host.clone(),
        fake.clone(),
    );
    running.ready();
    settle().await;
    let conn = fake.conn(0);
    assert_eq!(conn.resumes.lock().unwrap()[0], Resume::default());
    assert_eq!(conn.intake().message(message("1", 100)).await, Taken::Run);
    settle().await;
    let adopted = entry(&host, "evt_helper");
    assert_eq!(
        (adopted.bot_id, adopted.watermark),
        (Some(123_456_789), Some(100))
    );
    assert!(!adopted.webhook_cleared);
    running.stop().await;

    let host = Host::new();
    let same = stored.bots.get_mut("evt_helper").unwrap();
    same.bot_id = Some(123_456_789);
    *host.stored.lock().unwrap() = Some(stored);
    host.set_gate("evt_helper", Gate::Open);
    let fake = Fake::new(Serve::Hold);
    let mut running = Running::start(
        vec![bot("evt_helper", json!({}), TOKEN)],
        host.clone(),
        fake.clone(),
    );
    running.ready();
    settle().await;
    let conn = fake.conn(0);
    assert_eq!(
        conn.resumes.lock().unwrap()[0],
        Resume {
            watermark: Some(5_000),
            webhook_cleared: true
        }
    );
    assert_eq!(conn.intake().message(message("1", 100)).await, Taken::Seen);
    running.stop().await;
}

#[tokio::test(start_paused = true)]
async fn old_messages_are_skipped_after_a_connect() {
    let host = Host::new();
    host.set_gate("evt_helper", Gate::Open);
    let fake = Fake::new(Serve::Hold);
    let mut running = Running::start(
        vec![bot("evt_helper", json!({}), TOKEN)],
        host.clone(),
        fake.clone(),
    );
    running.ready();
    settle().await;
    let intake = fake.conn(0).intake();
    let mut old = message("1", 1);
    old.sent_at = Some(BASE - 901);
    assert_eq!(intake.message(old).await, Taken::Stale);
    let mut recent = message("1", 2);
    recent.sent_at = Some(BASE - 899);
    assert_eq!(intake.message(recent).await, Taken::Run);
    let mut later = message("1", 3);
    later.sent_at = Some(BASE - 2_000);
    tokio::time::sleep(Duration::from_secs(3_000)).await;
    assert_eq!(
        intake.message(later).await,
        Taken::Stale,
        "the cut is fixed at the connect"
    );
    tokio::time::sleep(Duration::from_secs(11)).await;
    let skipped = entry(&host, "evt_helper");
    assert_eq!(skipped.skipped_stale, 2);
    assert_eq!(intake.watermark(), Some(3));
    running.stop().await;
}

#[tokio::test(start_paused = true)]
async fn images_over_the_budget_are_counted_and_told() {
    let host = Host::new();
    host.set_gate("evt_helper", Gate::Open);
    let fake = Fake::new(Serve::Hold);
    let mut running = Running::start(
        vec![bot("evt_helper", json!({}), TOKEN)],
        host.clone(),
        fake.clone(),
    );
    running.ready();
    settle().await;
    let intake = fake.conn(0).intake();
    let mut big = Message::new("5", 3 * 1024 * 1024_u64);
    big.update_id = Some(1);
    big.private = true;
    assert_eq!(intake.message(big).await, Taken::Run);
    settle().await;
    assert_eq!(host.runs().len(), 1, "the run starts without the image");
    assert_eq!(entry(&host, "evt_helper").images_left_out, 1);
    assert!(
        fake.conn(0)
            .notices()
            .contains(&("5".into(), "I can't take that image.".into()))
    );
    running.stop().await;
}

#[tokio::test(start_paused = true)]
async fn a_stop_tells_every_chat_it_leaves_without_an_answer() {
    let host = Host::new();
    host.set_gate("evt_helper", Gate::Open);
    host.behave("evt_helper", Behaviour::Hang);
    let fake = Fake::new(Serve::Hold);
    let mut running = Running::start(
        vec![bot("evt_helper", json!({}), TOKEN)],
        host.clone(),
        fake.clone(),
    );
    running.ready();
    settle().await;
    let conn = fake.conn(0);
    let intake = conn.intake();
    assert_eq!(intake.message(message("a", 1)).await, Taken::Run);
    assert_eq!(intake.message(message("a", 2)).await, Taken::Queued);
    assert_eq!(intake.message(message("b", 3)).await, Taken::Run);
    settle().await;
    let stopping = Instant::now();
    let (host, fake) = running.stop().await;
    assert!(
        stopping.elapsed() >= Duration::from_secs(12),
        "runs got their grace"
    );
    assert!(stopping.elapsed() < Duration::from_secs(15));
    let conn = fake.conn(0);
    let restart: Vec<String> = conn
        .notices()
        .into_iter()
        .filter(|(_, text)| text.contains("restarting"))
        .map(|(chat, _)| chat)
        .collect();
    let chats: HashSet<&str> = restart.iter().map(String::as_str).collect();
    assert_eq!(restart.len(), 2, "{restart:?}");
    assert_eq!(chats, HashSet::from(["a", "b"]));
    assert_eq!(conn.stopped.load(Ordering::SeqCst), 1);
    let stopped = entry(&host, "evt_helper");
    assert_eq!(stopped.dropped_busy, 1);
    assert_eq!(stopped.running, 0);
    assert_eq!(stopped.last.unwrap().outcome, Outcome::Cancelled);
    assert_eq!(intake.message(message("c", 4)).await, Taken::Stopping);
}

#[tokio::test(start_paused = true)]
async fn the_state_never_holds_the_token_and_drops_odd_names() {
    let host = Host::new();
    host.set_gate("evt_helper", Gate::Open);
    let fake = Arc::new(Fake {
        mode: Serve::Hold,
        bot_name: Some("Grüße <bot>".into()),
        conns: Mutex::new(Vec::new()),
    });
    let mut running = Running::start(
        vec![bot("evt_helper", json!({}), TOKEN)],
        host.clone(),
        fake.clone(),
    );
    running.ready();
    settle().await;
    let intake = fake.conn(0).intake();
    intake.message(message("1", 1)).await;
    for state in [
        LinkState::Reconnecting,
        LinkState::Conflict,
        LinkState::Connected,
    ] {
        intake.state(state);
        settle().await;
    }
    intake.webhook_cleared();
    settle().await;
    let (host, _) = running.stop().await;
    let saves = host.saves.lock().unwrap().clone();
    assert!(saves.len() > 3);
    for state in &saves {
        assert!(
            !state.contains("AAHdq") && !state.contains(TOKEN),
            "{state}"
        );
        assert!(!state.contains("hello"), "message text in the state");
    }
    let last: Value = serde_json::from_str(saves.last().unwrap()).unwrap();
    assert_eq!(last["bots"]["evt_helper"]["bot_name"], Value::Null);
    assert_eq!(last["bots"]["evt_helper"]["webhook_cleared"], true);
    assert!(last["bots"]["evt_helper"]["reconnects"].as_u64().unwrap() >= 1);
    assert!(
        saves
            .iter()
            .any(|state| state.contains("\"state\":\"conflict\""))
    );
}

#[tokio::test(start_paused = true)]
async fn runs_today_roll_over_at_midnight() {
    let host = Host::at(BASE + 86_400 - 30);
    host.set_gate("evt_helper", Gate::Open);
    let fake = Fake::new(Serve::Hold);
    let mut running = Running::start(
        vec![bot("evt_helper", json!({}), TOKEN)],
        host.clone(),
        fake.clone(),
    );
    running.ready();
    settle().await;
    let intake = fake.conn(0).intake();
    intake.message(message("1", 1)).await;
    settle().await;
    intake.message(message("1", 2)).await;
    settle().await;
    let evening = entry(&host, "evt_helper").day.unwrap();
    assert_eq!((evening.date.as_str(), evening.runs), ("2026-09-22", 2));
    tokio::time::sleep(Duration::from_secs(60)).await;
    intake.message(message("1", 3)).await;
    settle().await;
    let morning = entry(&host, "evt_helper");
    assert_eq!(
        morning.day.unwrap(),
        flow_like_bots::state::Day {
            date: utc_date(BASE + 86_400),
            runs: 1
        }
    );
    assert_eq!(morning.runs, 3);
    running.stop().await;
}

#[tokio::test(start_paused = true)]
async fn a_run_does_not_start_when_its_watermark_cannot_be_written() {
    let host = Host::new();
    host.set_gate("evt_helper", Gate::Open);
    let fake = Fake::new(Serve::Hold);
    let mut running = Running::start(
        vec![bot("evt_helper", json!({}), TOKEN)],
        host.clone(),
        fake.clone(),
    );
    running.ready();
    settle().await;
    host.refuse_saves.store(true, Ordering::SeqCst);
    assert_eq!(
        fake.conn(0).intake().message(message("1", 1)).await,
        Taken::Run
    );
    settle().await;
    assert!(host.runs().is_empty());
    host.refuse_saves.store(false, Ordering::SeqCst);
    let (host, _) = running.stop().await;
    let refused = entry(&host, "evt_helper");
    assert_eq!((refused.runs, refused.failed, refused.running), (0, 1, 0));
}
