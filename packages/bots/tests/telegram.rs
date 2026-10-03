//! The Telegram provider against a local fake Bot API (design §7.7 step 4).

#![cfg(feature = "telegram")]

use std::collections::HashMap;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use flow_like_bots::limits::{HourBudget, IMAGE_BYTES, IMAGE_BYTES_PER_HOUR, ImageBudget};
use flow_like_bots::state::{BotEntry, Status};
use flow_like_bots::telegram::{Timing, connect_to};
use flow_like_bots::{
    Bot, BotHost, BotSpec, BotToken, BotsState, Connection, Connector, Gate, Message, Provider,
    Revisions, RunEnd, StreamEvent, run_with,
};
use serde_json::{Value, json};
use tokio::sync::{Notify, mpsc, watch};
use tokio::task::JoinHandle;
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

const BOT_ID: u64 = 7_123_456_789;
const EVENT: &str = "evt_helper";
const ANSWER: &str = "Hi Ada";
const CONFLICT: &str = "Conflict: terminated by other getUpdates request; make sure that only one bot instance is running";
const WEBHOOK: &str = "Conflict: can\u{27}t use getUpdates method while webhook is active; use deleteWebhook to delete the webhook first";

fn token(name: &str) -> String {
    format!("{BOT_ID}:AA{name}-secret-part-0123456789")
}

fn with<T, R>(mutex: &Mutex<T>, change: impl FnOnce(&mut T) -> R) -> R {
    let mut guard = mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    change(&mut guard)
}

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs() as i64)
}

mod logs {
    use std::fmt::{self, Write};
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::{Mutex, OnceLock};

    use tracing::field::{Field, Visit};
    use tracing::span::{Attributes, Id, Record};
    use tracing::{Event, Metadata, Subscriber};

    static LINES: Mutex<Vec<String>> = Mutex::new(Vec::new());
    static NEXT_SPAN: AtomicU64 = AtomicU64::new(1);

    struct Line(String);

    impl Visit for Line {
        fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
            let _ = write!(self.0, " {}={value:?}", field.name());
        }
    }

    fn keep(line: Line) {
        super::with(&LINES, |lines| lines.push(line.0));
    }

    /// Every event and span of every crate, at every level.
    struct Capture;

    impl Subscriber for Capture {
        fn enabled(&self, _metadata: &Metadata) -> bool {
            true
        }

        fn new_span(&self, span: &Attributes) -> Id {
            let mut line = Line(format!("span {}", span.metadata().name()));
            span.record(&mut line);
            keep(line);
            Id::from_u64(NEXT_SPAN.fetch_add(1, Ordering::Relaxed))
        }

        fn record(&self, _span: &Id, values: &Record) {
            let mut line = Line(String::from("record"));
            values.record(&mut line);
            keep(line);
        }

        fn record_follows_from(&self, _span: &Id, _follows: &Id) {}

        fn event(&self, event: &Event) {
            let metadata = event.metadata();
            let mut line = Line(format!("{} {}", metadata.level(), metadata.target()));
            event.record(&mut line);
            keep(line);
        }

        fn enter(&self, _span: &Id) {}

        fn exit(&self, _span: &Id) {}
    }

    pub fn capture() {
        static INSTALLED: OnceLock<()> = OnceLock::new();
        INSTALLED.get_or_init(|| {
            let _ = tracing::subscriber::set_global_default(Capture);
        });
    }

    pub fn lines() -> Vec<String> {
        super::with(&LINES, |lines| lines.clone())
    }
}

/// What the fake Bot API is told to do.
#[derive(Default)]
struct Script {
    refuse: bool,
    webhook: bool,
    conflicts: u32,
    server_errors: u32,
    poll_retry_after: Option<u32>,
    send_retry_after: Option<u32>,
    ignore_offset: bool,
    pending: Vec<Value>,
    files: HashMap<String, Vec<u8>>,
}

#[derive(Clone, Debug)]
struct Call {
    method: String,
    body: Value,
    at: Instant,
}

struct Api {
    token: String,
    script: Mutex<Script>,
    calls: Mutex<Vec<Call>>,
    arrived: Notify,
    next_message: AtomicI64,
}

impl Api {
    fn new(token: &str) -> Arc<Self> {
        Arc::new(Self {
            token: token.to_string(),
            script: Mutex::new(Script::default()),
            calls: Mutex::new(Vec::new()),
            arrived: Notify::new(),
            next_message: AtomicI64::new(5_000),
        })
    }

    fn script<R>(&self, change: impl FnOnce(&mut Script) -> R) -> R {
        with(&self.script, change)
    }

    fn push(&self, update: Value) {
        self.script(|script| script.pending.push(update));
        self.arrived.notify_waiters();
    }

    fn calls(&self, method: &str) -> Vec<Call> {
        with(&self.calls, |calls| {
            calls
                .iter()
                .filter(|call| call.method == method)
                .cloned()
                .collect()
        })
    }

    fn all_calls(&self) -> usize {
        with(&self.calls, |calls| calls.len())
    }

    /// The failure the script asks for in place of the next poll.
    fn scripted_failure(&self) -> Option<Response> {
        self.script(|script| {
            if script.webhook {
                return Some(failure(StatusCode::CONFLICT, WEBHOOK, None));
            }
            if script.conflicts > 0 {
                script.conflicts -= 1;
                return Some(failure(StatusCode::CONFLICT, CONFLICT, None));
            }
            if let Some(wait) = script.poll_retry_after.take() {
                return Some(too_many_requests(wait));
            }
            if script.server_errors > 0 {
                script.server_errors -= 1;
                let error = (StatusCode::INTERNAL_SERVER_ERROR, "Internal Server Error");
                return Some(error.into_response());
            }
            None
        })
    }

    /// The pending updates from `offset`; Telegram forgets those below it.
    fn batch(&self, offset: Option<i64>, limit: usize) -> Vec<Value> {
        self.script(|script| {
            if script.ignore_offset {
                return std::mem::take(&mut script.pending);
            }
            if let Some(offset) = offset {
                script
                    .pending
                    .retain(|update| update["update_id"].as_i64() >= Some(offset));
            }
            script.pending.iter().take(limit).cloned().collect()
        })
    }

    async fn updates(&self, body: &Value) -> Response {
        if let Some(response) = self.scripted_failure() {
            return response;
        }
        let limit = body["limit"]
            .as_u64()
            .and_then(|limit| usize::try_from(limit).ok())
            .unwrap_or(100);
        let wait = Duration::from_secs(body["timeout"].as_u64().unwrap_or(0))
            .min(Duration::from_millis(300));
        let deadline = Instant::now() + wait;
        loop {
            let arrived = self.arrived.notified();
            let batch = self.batch(body["offset"].as_i64(), limit);
            if !batch.is_empty() || Instant::now() >= deadline {
                return success(json!(batch));
            }
            tokio::select! {
                () = arrived => {}
                () = tokio::time::sleep_until(deadline) => {}
            }
        }
    }

    fn sent(&self, body: &Value) -> Response {
        if let Some(wait) = self.script(|script| script.send_retry_after.take()) {
            return too_many_requests(wait);
        }
        let id = body["message_id"]
            .as_i64()
            .unwrap_or_else(|| self.next_message.fetch_add(1, Ordering::SeqCst));
        success(json!({
            "message_id": id,
            "date": now(),
            "chat": {"id": body["chat_id"], "type": "private", "first_name": "Ada"},
            "from": {"id": BOT_ID, "is_bot": true, "first_name": "Helper", "username": "helper_bot"},
            "text": body["text"],
        }))
    }
}

fn success(result: Value) -> Response {
    Json(json!({"ok": true, "result": result})).into_response()
}

fn failure(status: StatusCode, description: &str, retry_after: Option<u32>) -> Response {
    let mut body = json!({"ok": false, "error_code": status.as_u16(), "description": description});
    if let Some(wait) = retry_after {
        body["parameters"] = json!({"retry_after": wait});
    }
    (status, Json(body)).into_response()
}

fn too_many_requests(wait: u32) -> Response {
    let description = format!("Too Many Requests: retry after {wait}");
    failure(StatusCode::TOO_MANY_REQUESTS, &description, Some(wait))
}

fn me() -> Value {
    json!({"id": BOT_ID, "is_bot": true, "first_name": "Helper", "username": "helper_bot",
        "can_join_groups": true, "can_read_all_group_messages": false,
        "supports_inline_queries": false, "has_main_web_app": false})
}

async fn method(
    State(api): State<Arc<Api>>,
    Path((bot, method)): Path<(String, String)>,
    body: String,
) -> Response {
    let method = method.to_ascii_lowercase();
    let body: Value = serde_json::from_str(&body).unwrap_or(Value::Null);
    let call = Call {
        method: method.clone(),
        body: body.clone(),
        at: Instant::now(),
    };
    with(&api.calls, |calls| calls.push(call));
    if bot != format!("bot{}", api.token) || api.script(|script| script.refuse) {
        return failure(StatusCode::UNAUTHORIZED, "Unauthorized", None);
    }
    match method.as_str() {
        "getme" => success(me()),
        "getupdates" => api.updates(&body).await,
        "sendmessage" | "editmessagetext" => api.sent(&body),
        "getfile" => {
            let id = body["file_id"].as_str().unwrap_or_default().to_string();
            let size = api.script(|script| script.files.get(&id).map_or(0, Vec::len));
            success(
                json!({"file_id": id, "file_unique_id": id, "file_size": size, "file_path": id}),
            )
        }
        "deletewebhook" => {
            api.script(|script| script.webhook = false);
            success(json!(true))
        }
        _ => success(json!(true)),
    }
}

async fn file(State(api): State<Arc<Api>>, Path((bot, path)): Path<(String, String)>) -> Response {
    if bot != format!("bot{}", api.token) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    match api.script(|script| script.files.get(&path).cloned()) {
        Some(bytes) => bytes.into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn local_listener() -> (tokio::net::TcpListener, String) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("a local port");
    let address = listener.local_addr().expect("its address");
    (listener, format!("http://{address}/"))
}

async fn serve(api: Arc<Api>) -> String {
    let (listener, url) = local_listener().await;
    let app = Router::new()
        .route("/{bot}/{method}", post(method))
        .route("/file/{bot}/{*path}", get(file))
        .with_state(api);
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    url
}

/// A server that takes every connection and closes it without an answer.
async fn hang_up() -> String {
    let (listener, url) = local_listener().await;
    tokio::spawn(async move {
        while let Ok((socket, _)) = listener.accept().await {
            drop(socket);
        }
    });
    url
}

#[derive(Clone)]
struct Seen {
    payload: Value,
    watermark: Option<i64>,
}

struct Host {
    gate: watch::Sender<Gate>,
    initial: Option<BotsState>,
    saved: Mutex<Vec<BotsState>>,
    runs: Mutex<Vec<Seen>>,
    observed: Mutex<Vec<Value>>,
    takes: fn(&Value) -> bool,
    hold_runs: Notify,
    slow: bool,
    answer: Mutex<String>,
}

impl Host {
    fn new(initial: Option<BotsState>) -> Arc<Self> {
        Self::with(initial, |_| false, false)
    }

    fn with(initial: Option<BotsState>, takes: fn(&Value) -> bool, slow: bool) -> Arc<Self> {
        Arc::new(Self {
            gate: watch::channel(Gate::Open).0,
            initial,
            saved: Mutex::new(Vec::new()),
            runs: Mutex::new(Vec::new()),
            observed: Mutex::new(Vec::new()),
            takes,
            hold_runs: Notify::new(),
            slow,
            answer: Mutex::new(ANSWER.to_string()),
        })
    }

    fn last(&self) -> Option<BotsState> {
        with(&self.saved, |saved| saved.last().cloned())
    }

    fn entry(&self) -> Option<BotEntry> {
        self.last()?.bots.get(EVENT).cloned()
    }

    fn seen(&self) -> Vec<Seen> {
        with(&self.runs, |runs| runs.clone())
    }

    fn runs(&self) -> usize {
        with(&self.runs, |runs| runs.len())
    }
}

#[async_trait]
impl BotHost for Host {
    async fn run(
        &self,
        _event_id: &str,
        payload: Value,
        events: mpsc::Sender<StreamEvent>,
        _cancel: CancellationToken,
    ) -> RunEnd {
        let watermark = self
            .last()
            .and_then(|state| state.bots.get(EVENT)?.watermark);
        with(&self.runs, |runs| runs.push(Seen { payload, watermark }));
        if self.slow {
            self.hold_runs.notified().await;
        }
        let content = with(&self.answer, |answer| answer.clone());
        let answer = json!({"response": {"choices": [{"message": {"content": content}}]}});
        let _ = events.send(StreamEvent::new("chat_out", answer)).await;
        RunEnd::Succeeded
    }

    fn gate(&self, _event_id: &str) -> watch::Receiver<Gate> {
        self.gate.subscribe()
    }

    fn load(&self) -> Option<BotsState> {
        self.initial.clone()
    }

    fn save(&self, state: &BotsState) -> std::io::Result<()> {
        with(&self.saved, |saved| saved.push(state.clone()));
        Ok(())
    }

    fn now(&self) -> i64 {
        now()
    }

    fn handle(&self, event_id: &str) -> String {
        format!("device-bot:{event_id}")
    }

    fn observed(&self, _event_id: &str, update: &Value) -> bool {
        with(&self.observed, |observed| observed.push(update.clone()));
        (self.takes)(update)
    }
}

struct FakeTelegram {
    api_url: String,
    timing: Timing,
}

impl Connector for FakeTelegram {
    fn connect(
        &self,
        provider: Provider,
        token: &BotToken,
        specs: &[BotSpec],
    ) -> Option<Arc<dyn Connection>> {
        (provider == Provider::Telegram)
            .then(|| connect_to(Some(&self.api_url), self.timing, token, specs))
    }
}

fn timing() -> Timing {
    Timing {
        long_poll_secs: 1,
        blocked_pause: Duration::from_millis(400),
    }
}

fn spec() -> BotSpec {
    BotSpec::from_config(EVENT, "telegram", b"{}").expect("valid settings")
}

fn bot_token(token: &str) -> BotToken {
    BotToken::parse(Provider::Telegram, token.as_bytes()).expect("a token")
}

struct Running {
    stop: CancellationToken,
    task: JoinHandle<()>,
}

impl Running {
    async fn stop(self) {
        self.stop.cancel();
        tokio::time::timeout(Duration::from_secs(30), self.task)
            .await
            .expect("the bots stop in time")
            .expect("the bots task ends cleanly");
    }
}

fn start(api_url: String, token: &str, host: Arc<Host>) -> Running {
    logs::capture();
    let bot = Bot {
        spec: spec(),
        token: bot_token(token),
    };
    let connector = Arc::new(FakeTelegram {
        api_url,
        timing: timing(),
    });
    let stop = CancellationToken::new();
    let revisions = Revisions {
        config_revision: 1,
        intent_revision: 1,
    };
    let task = tokio::spawn(run_with(
        vec![bot],
        revisions,
        host,
        connector,
        async {},
        stop.clone(),
    ));
    Running { stop, task }
}

async fn until(what: &str, mut condition: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(20);
    while !condition() {
        assert!(Instant::now() < deadline, "timed out waiting until {what}");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

fn state_is(host: &Host, state: Status) -> bool {
    host.entry().is_some_and(|entry| entry.state == state)
}

fn text_update(id: i64, chat: i64, text: &str, sent: i64) -> Value {
    json!({"update_id": id, "message": {
        "message_id": id + 10, "date": sent,
        "chat": {"id": chat, "type": "private", "first_name": "Ada"},
        "from": {"id": chat, "is_bot": false, "first_name": "Ada"},
        "text": text}})
}

/// A state document that knows the bot of the tests with `watermark`.
fn known_bot(bot_id: u64, watermark: i64) -> BotsState {
    let mut state = BotsState::default();
    let mut entry = BotEntry::new(Provider::Telegram);
    entry.bot_id = Some(bot_id);
    entry.webhook_cleared = true;
    entry.watermark = Some(watermark);
    entry.watermark_at = Some(now());
    state.bots.insert(EVENT.to_string(), entry);
    state
}

fn assert_no_token(token: &str, texts: impl IntoIterator<Item = String>) {
    let secret = token.split_once(":").map_or(token, |(_, secret)| secret);
    for text in texts {
        assert!(
            !text.contains(token) && !text.contains(secret),
            "the token leaked into: {}",
            text.replace(secret, "<secret>")
        );
    }
}

fn assert_secrets_kept(token: &str, host: &Host) {
    let documents = with(&host.saved, |saved| {
        saved
            .iter()
            .map(|state| serde_json::to_string(state).expect("a document"))
            .collect::<Vec<_>>()
    });
    let payloads = host.seen().into_iter().map(|seen| seen.payload.to_string());
    let lines = logs::lines();
    assert!(
        lines.iter().any(|line| line.contains("Bot state changed")),
        "no log line was captured"
    );
    assert_no_token(token, documents);
    assert_no_token(token, payloads);
    assert_no_token(token, lines);
}

#[tokio::test]
async fn first_connect_removes_the_webhook_and_a_later_one_leaves_it() {
    let token = token("webhook");
    let api = Api::new(&token);
    api.script(|script| script.webhook = true);
    let url = serve(api.clone()).await;

    let host = Host::new(None);
    let running = start(url.clone(), &token, host.clone());
    until("the first connect polls", || {
        !api.calls("getupdates").is_empty()
    })
    .await;
    running.stop().await;
    let removed = api.calls("deletewebhook");
    assert_eq!(removed.len(), 1);
    assert_eq!(removed[0].body["drop_pending_updates"], json!(false));
    let entry = host.entry().expect("a state entry");
    assert!(entry.webhook_cleared);
    assert_eq!(entry.bot_id, Some(BOT_ID));
    assert_eq!(entry.bot_name.as_deref(), Some("helper_bot"));

    api.script(|script| script.webhook = true);
    let later = Host::new(host.last());
    let running = start(url, &token, later.clone());
    until("the webhook is reported", || {
        state_is(&later, Status::WebhookSet)
    })
    .await;
    running.stop().await;
    assert_eq!(
        api.calls("deletewebhook").len(),
        1,
        "a later connect removed the webhook"
    );
    assert_secrets_kept(&token, &later);
}

#[tokio::test]
async fn a_connect_as_another_bot_starts_over() {
    let token = token("another");
    let api = Api::new(&token);
    api.script(|script| script.webhook = true);
    api.push(text_update(100, 4242, "hello", now()));
    let url = serve(api.clone()).await;

    let host = Host::new(Some(known_bot(1_111, 500)));
    let running = start(url, &token, host.clone());
    until("the message below the old watermark runs", || {
        host.runs() == 1
    })
    .await;
    running.stop().await;
    assert_eq!(api.calls("deletewebhook").len(), 1);
    let first_poll = &api.calls("getupdates")[0];
    assert!(
        first_poll.body["offset"].is_null(),
        "the watermark of the old bot was used"
    );
    let entry = host.entry().expect("a state entry");
    assert_eq!(entry.bot_id, Some(BOT_ID));
    assert_eq!(entry.watermark, Some(100));
}

#[tokio::test]
async fn an_update_becomes_one_run() {
    let token = token("run");
    let api = Api::new(&token);
    api.push(text_update(300, 4242, "hello", now()));
    let url = serve(api.clone()).await;

    let host = Host::new(None);
    let running = start(url, &token, host.clone());
    until("the answer is sent", || {
        !api.calls("sendmessage").is_empty()
    })
    .await;
    running.stop().await;

    let seen = host.seen();
    assert_eq!(seen.len(), 1);
    assert_eq!(
        seen[0].watermark,
        Some(300),
        "the watermark was not written before the run"
    );
    let session = &seen[0].payload["local_session"];
    assert_eq!(session["bot_token"], "device-bot:evt_helper");
    assert_eq!(session["bot_username"], "helper_bot");
    assert_eq!(session["chat_type"], "private");
    assert_eq!(session["chat_id"], "4242");
    assert_eq!(session["message_id"], "310");
    let text = &seen[0].payload["messages"][0]["content"][0]["text"];
    assert_eq!(text, "Ada[id: 4242]: hello");

    let sent = api.calls("sendmessage");
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].body["text"], ANSWER);
    assert_eq!(sent[0].body["reply_parameters"]["message_id"], 310);
    assert_secrets_kept(&token, &host);
}

#[tokio::test]
async fn a_long_answer_is_split_on_character_boundaries() {
    let token = token("split");
    let api = Api::new(&token);
    api.push(text_update(320, 4242, "tell me more", now()));
    let url = serve(api.clone()).await;
    let answer = "😀ä".repeat(2_000);

    let host = Host::new(None);
    with(&host.answer, |text| *text = answer.clone());
    let running = start(url, &token, host.clone());
    until("both parts are sent", || {
        api.calls("sendmessage").len() == 2
    })
    .await;
    running.stop().await;

    let text = |call: &Call| call.body["text"].as_str().unwrap_or_default().to_string();
    let sent = api.calls("sendmessage");
    let first = api
        .calls("editmessagetext")
        .last()
        .unwrap_or(&sent[0])
        .clone();
    let parts = [text(&first), text(&sent[1])];
    for part in &parts {
        assert!(part.encode_utf16().count() <= 4_096);
    }
    assert_eq!(parts.concat(), answer);
    assert_eq!(sent[0].body["reply_parameters"]["message_id"], 330);
    assert!(sent[1].body["reply_parameters"].is_null());
}

#[tokio::test]
async fn the_read_position_is_confirmed_on_stop() {
    let token = token("confirm");
    let api = Api::new(&token);
    api.push(text_update(350, 4242, "hello", now()));
    let url = serve(api.clone()).await;

    let host = Host::new(None);
    let running = start(url, &token, host.clone());
    until("the answer is sent", || {
        !api.calls("sendmessage").is_empty()
    })
    .await;
    running.stop().await;

    let polls = api.calls("getupdates");
    assert!(
        polls
            .iter()
            .any(|poll| { poll.body["allowed_updates"] == json!(["message", "callback_query"]) })
    );
    let confirm = polls.last().expect("a confirming poll");
    assert_eq!(confirm.body["offset"], 351);
    assert_eq!(confirm.body["timeout"], 0);
    assert_eq!(confirm.body["limit"], 1);
}

#[tokio::test]
async fn an_update_at_or_below_the_watermark_is_not_handled_again() {
    let token = token("again");
    let api = Api::new(&token);
    api.script(|script| script.ignore_offset = true);
    api.push(text_update(300, 4242, "hello", now()));
    let url = serve(api.clone()).await;

    let host = Host::new(Some(known_bot(BOT_ID, 300)));
    let running = start(url, &token, host.clone());
    until("the redelivered update was polled", || {
        api.calls("getupdates").len() >= 2
    })
    .await;
    api.script(|script| script.ignore_offset = false);
    api.push(text_update(301, 4242, "again", now()));
    until("the new update runs", || host.runs() == 1).await;
    running.stop().await;

    assert_eq!(api.calls("getupdates")[0].body["offset"], 301);
    assert_eq!(
        host.runs(),
        1,
        "an update at or below the watermark ran again"
    );
    let session = &host.seen()[0].payload["local_session"];
    assert_eq!(session["message_id"], "311");
}

#[tokio::test]
async fn a_refused_token_stops_the_bot_and_its_requests() {
    let token = token("refused");
    let api = Api::new(&token);
    api.script(|script| script.refuse = true);
    let url = serve(api.clone()).await;

    let host = Host::new(None);
    let running = start(url, &token, host.clone());
    until("the token is refused", || {
        state_is(&host, Status::TokenRefused)
    })
    .await;
    let requests = api.all_calls();
    tokio::time::sleep(Duration::from_millis(1_500)).await;
    assert_eq!(
        api.all_calls(),
        requests,
        "the bot kept asking after a refused token"
    );
    assert!(
        !running.task.is_finished(),
        "a refused token ended the bots task"
    );
    running.stop().await;
    assert_secrets_kept(&token, &host);
}

#[tokio::test]
async fn another_consumer_is_a_conflict_tried_once_per_pause() {
    let token = token("conflict");
    let api = Api::new(&token);
    api.script(|script| script.conflicts = 3);
    let url = serve(api.clone()).await;

    let host = Host::new(None);
    let running = start(url, &token, host.clone());
    until("the conflict is reported", || {
        state_is(&host, Status::Conflict)
    })
    .await;
    until("three polls in a row connect it again", || {
        state_is(&host, Status::Connected) && api.calls("getupdates").len() >= 6
    })
    .await;
    running.stop().await;
    let polls = api.calls("getupdates");
    for pair in polls[..4].windows(2) {
        let pause = pair[1].at - pair[0].at;
        assert!(
            pause >= Duration::from_millis(350),
            "attempts {pause:?} apart during a conflict"
        );
    }
}

#[tokio::test]
async fn retry_after_is_waited_out() {
    let token = token("retry");
    let api = Api::new(&token);
    api.script(|script| {
        script.poll_retry_after = Some(1);
        script.send_retry_after = Some(1);
    });
    api.push(text_update(400, 4242, "hello", now()));
    let url = serve(api.clone()).await;

    let host = Host::new(None);
    let running = start(url, &token, host.clone());
    until("the answer is sent after the wait", || {
        api.calls("sendmessage").len() == 2
    })
    .await;
    running.stop().await;
    let polls = api.calls("getupdates");
    assert!(polls[1].at - polls[0].at >= Duration::from_millis(950));
    let sent = api.calls("sendmessage");
    assert!(sent[1].at - sent[0].at >= Duration::from_millis(950));
    assert_eq!(sent[1].body["text"], ANSWER);
}

#[tokio::test]
async fn old_messages_are_skipped_and_fresh_ones_never_are() {
    let token = token("stale");
    let api = Api::new(&token);
    api.push(text_update(500, 4242, "from last night", now() - 1_000));
    api.push(text_update(501, 4343, "ten minutes ago", now() - 600));
    let url = serve(api.clone()).await;

    let host = Host::new(None);
    let running = start(url, &token, host.clone());
    until("the fresh message runs", || host.runs() == 1).await;
    running.stop().await;
    let entry = host.entry().expect("a state entry");
    assert_eq!(entry.skipped_stale, 1);
    assert_eq!(entry.watermark, Some(501));
    assert_eq!(host.seen()[0].payload["local_session"]["chat_id"], "4343");
}

#[tokio::test]
async fn waiting_nodes_see_every_update_and_take_replies_in_a_busy_chat() {
    fn takes(update: &Value) -> bool {
        update["message"]["text"] == "yes" || update.get("callback_query").is_some()
    }
    let token = token("observed");
    let api = Api::new(&token);
    api.push(text_update(700, 4242, "start", now()));
    let url = serve(api.clone()).await;

    let host = Host::with(None, takes, true);
    let running = start(url, &token, host.clone());
    until("the first run starts", || host.runs() == 1).await;
    api.push(text_update(701, 4242, "yes", now()));
    api.push(
        json!({"update_id": 702, "callback_query": {"id": "cb1", "chat_instance": "c1",
        "from": {"id": 4242, "is_bot": false, "first_name": "Ada"}, "data": "confirm"}}),
    );
    until("both updates reached the host", || {
        with(&host.observed, |observed| observed.len()) == 3
    })
    .await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    host.hold_runs.notify_waiters();
    until("the answer is sent", || {
        !api.calls("sendmessage").is_empty()
    })
    .await;
    running.stop().await;
    assert_eq!(host.runs(), 1, "a reply a waiting node took started a run");
    assert_eq!(host.entry().expect("a state entry").watermark, Some(702));
}

fn photo(file: &str, size: u64) -> Message {
    let native: teloxide::types::Message = serde_json::from_str(
        &json!({"message_id": 10, "date": now(),
            "chat": {"id": 4242, "type": "private", "first_name": "Ada"},
            "from": {"id": 4242, "is_bot": false, "first_name": "Ada"},
            "caption": "what is this",
            "photo": [{"file_id": file, "file_unique_id": file, "width": 90, "height": 90, "file_size": size}]})
        .to_string(),
    )
    .expect("a photo message");
    Message::new("4242", native)
}

fn image_of(payload: &Value) -> &Value {
    &payload["messages"][0]["content"][1]["image_url"]["url"]
}

async fn images_api(token: &str) -> (Arc<Api>, Arc<dyn Connection>) {
    let api = Api::new(token);
    api.script(|script| {
        script.files.insert("small".into(), vec![7_u8; 1_000]);
        script
            .files
            .insert("liar".into(), vec![7_u8; 3 * 1024 * 1024]);
    });
    let url = serve(api.clone()).await;
    let connection = connect_to(Some(&url), timing(), &bot_token(token), &[spec()]);
    (api, connection)
}

#[tokio::test]
async fn an_image_becomes_a_data_url() {
    let token = token("image");
    let (_api, connection) = images_api(&token).await;
    let mut images = ImageBudget::fresh();
    let message = photo("small", 1_000);
    let payload = connection
        .payload(&message, "device-bot:evt_helper", &mut images)
        .await;
    let url = image_of(&payload).as_str().unwrap_or_default();
    assert!(url.starts_with("data:image/jpeg;base64,"));
    assert_eq!(
        payload["messages"][0]["content"][0]["text"],
        "Ada[id: 4242]: what is this"
    );
    assert_eq!(images.left_out(), 0);
    assert_no_token(&token, [payload.to_string()]);
}

#[tokio::test]
async fn images_over_a_limit_are_left_out_and_counted() {
    let token = token("limits");
    let (api, connection) = images_api(&token).await;
    let handle = "device-bot:evt_helper";

    let mut images = ImageBudget::fresh();
    let payload = connection
        .payload(&photo("huge", IMAGE_BYTES + 1), handle, &mut images)
        .await;
    assert!(
        image_of(&payload).is_null(),
        "an image over 2 MiB reached the flow"
    );
    assert_eq!(images.left_out(), 1);
    assert!(
        api.calls("getfile").is_empty(),
        "an image over its limit was fetched"
    );

    let mut images = ImageBudget::fresh();
    let payload = connection
        .payload(&photo("liar", 1_000), handle, &mut images)
        .await;
    assert!(
        image_of(&payload).is_null(),
        "a download over its limit reached the flow"
    );
    assert_eq!(images.left_out(), 1);

    let hour = Arc::new(Mutex::new(HourBudget::new()));
    assert!(with(&hour, |hour| hour.take(IMAGE_BYTES_PER_HOUR - 500, 0)));
    let mut images = ImageBudget::new(hour, 0);
    let payload = connection
        .payload(&photo("small", 1_000), handle, &mut images)
        .await;
    assert!(
        image_of(&payload).is_null(),
        "an image over the budget of the hour reached the flow"
    );
    assert_eq!(images.left_out(), 1);
}

#[tokio::test]
async fn nothing_carries_the_token_when_the_api_fails_or_hangs_up() {
    let token = token("failing");
    let api = Api::new(&token);
    api.script(|script| script.server_errors = 1);
    api.push(text_update(800, 4242, "hello", now()));
    let url = serve(api.clone()).await;
    let host = Host::new(None);
    let running = start(url, &token, host.clone());
    until("the run after a server error answers", || {
        !api.calls("sendmessage").is_empty()
    })
    .await;
    running.stop().await;
    assert_secrets_kept(&token, &host);

    let hung_up = Host::new(None);
    let running = start(hang_up().await, &token, hung_up.clone());
    until("a hung-up connection reconnects", || {
        state_is(&hung_up, Status::Reconnecting)
    })
    .await;
    running.stop().await;
    assert_secrets_kept(&token, &hung_up);
}
