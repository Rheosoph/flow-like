//! The Discord provider against a local fake of Discord's REST API: the reply (send, edit,
//! split at 2,000 characters), notices, and the one bounded history read of a run's payload.
//! The gateway has no fake; the live tests at the end need a real bot token.
#![cfg(feature = "discord")]

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use axum::Router;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, Method, StatusCode, Uri};
use axum::response::{IntoResponse, Json, Response};
use flow_like_bots::discord::{DiscordConnection, DiscordReply, facts, notice};
use flow_like_bots::limits::{HourBudget, ImageBudget};
use flow_like_bots::render::utf16_len;
use flow_like_bots::reply::{Answer, Plan, ReplySink};
use flow_like_bots::state::Status;
use flow_like_bots::{
    Bot, BotHost, BotSpec, BotToken, BotsState, Connection, Gate, Provider, Revisions, RunEnd,
    StreamEvent,
};
use serde_json::{Value, json};
use serenity::http::{Http, HttpBuilder};
use serenity::model::channel::Message;
use serenity::model::id::{ChannelId, MessageId, UserId};
use tokio::sync::{mpsc, watch};
use tokio_util::sync::CancellationToken;
use tracing::field::{Field, Visit};
use tracing::span::{Attributes, Id, Record};
use tracing::{Metadata, Subscriber};

/// Shaped like a token of the bot with the id 900 (`OTAw`); Discord never sees it.
const TOKEN: &str = "OTAw.GqZ3xY.c2VjcmV0LXRva2VuLXZhbHVlLWZvci10ZXN0cw";
const HANDLE: &str = "device-bot:evt_dc";
/// Discord's limit for one message.
const MESSAGE_LIMIT: usize = 2_000;
const BOT: u64 = 900;

#[derive(Clone, Debug)]
struct Seen {
    method: Method,
    path: String,
    query: String,
    authorization: String,
    body: Value,
}

#[derive(Clone, Default)]
struct Fake {
    seen: Arc<Mutex<Vec<Seen>>>,
    deleted: Arc<Mutex<Vec<u64>>>,
    refusal: Arc<Mutex<Option<StatusCode>>>,
}

impl Fake {
    fn seen(&self) -> Vec<Seen> {
        self.seen.lock().unwrap().clone()
    }

    /// Someone deleted the message: an edit of it is answered 404.
    fn delete(&self, id: u64) {
        self.deleted.lock().unwrap().push(id);
    }

    /// Every later request is answered with `status`.
    fn refuse(&self, status: StatusCode) {
        *self.refusal.lock().unwrap() = Some(status);
    }
}

fn message_json(id: u64, extra: Value) -> Value {
    let mut message = json!({
        "id": id.to_string(),
        "channel_id": "300",
        "guild_id": "400",
        "author": {"id": "500", "username": "alice", "discriminator": "0"},
        "content": "hello",
        "timestamp": "2026-10-02T10:00:00.000000+00:00",
        "edited_timestamp": null,
        "tts": false,
        "mention_everyone": false,
        "mentions": [],
        "mention_roles": [],
        "attachments": [],
        "embeds": [],
        "pinned": false,
        "type": 0
    });
    for (key, value) in extra.as_object().expect("extra keys are an object") {
        message[key] = value.clone();
    }
    message
}

/// Answers like Discord: a sent or edited message comes back as the bot's message; a history
/// read returns the ten messages before the asked one, newest first.
async fn answer(
    State(fake): State<Fake>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let body: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
    let mut seen = fake.seen.lock().unwrap();
    seen.push(Seen {
        method: method.clone(),
        path: uri.path().to_string(),
        query: uri.query().unwrap_or_default().to_string(),
        authorization: headers
            .get("authorization")
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default()
            .to_string(),
        body: body.clone(),
    });
    if let Some(status) = *fake.refusal.lock().unwrap() {
        let refused = json!({"code": 0, "message": "refused"});
        return (status, Json(refused)).into_response();
    }
    let bot = json!({"id": BOT.to_string(), "username": "helper", "bot": true});
    if method == Method::GET {
        let history: Vec<Value> = (990..1000)
            .rev()
            .map(|id| message_json(id, json!({"content": format!("message {id}")})))
            .collect();
        return Json(Value::Array(history)).into_response();
    }
    let id = match method {
        Method::PATCH => uri
            .path()
            .rsplit('/')
            .next()
            .and_then(|id| id.parse().ok())
            .unwrap_or(0),
        _ => 5000 + seen.len() as u64,
    };
    if method == Method::PATCH && fake.deleted.lock().unwrap().contains(&id) {
        let unknown = json!({"code": 10008, "message": "Unknown Message"});
        return (StatusCode::NOT_FOUND, Json(unknown)).into_response();
    }
    let content = body["content"].as_str().unwrap_or_default();
    Json(message_json(id, json!({"author": bot, "content": content}))).into_response()
}

async fn fake() -> (String, Fake) {
    let fake = Fake::default();
    let app = Router::new().fallback(answer).with_state(fake.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (url, fake)
}

fn http(url: &str) -> Http {
    HttpBuilder::new(TOKEN)
        .proxy(url)
        .ratelimiter_disabled(true)
        .build()
}

fn plain(text: &str) -> Answer {
    Answer {
        text: text.to_string(),
        plan: None,
        links: Vec::new(),
    }
}

fn no_token_outside_the_header(seen: &[Seen]) {
    for request in seen {
        assert_eq!(request.authorization, format!("Bot {TOKEN}"));
        let shown = format!("{} {} {}", request.path, request.query, request.body);
        assert!(!shown.contains(TOKEN), "{shown}");
    }
}

const MESSAGES: &str = "/api/v10/channels/300/messages";
/// The reply the fake gives the first message it receives.
const REPLY: &str = "/api/v10/channels/300/messages/5001";

fn reply_at(url: &str) -> DiscordReply {
    DiscordReply::new(
        Arc::new(http(url)),
        ChannelId::new(300),
        MessageId::new(1000),
    )
}

#[tokio::test]
async fn reply_is_sent_with_its_plan_then_edited() {
    let (url, fake) = fake().await;
    let mut reply = reply_at(&url);
    let plan = Plan {
        steps: vec![(1, "Suchen".into()), (2, "Antworten".into())],
        current_step: 1,
        current_message: "lese".into(),
    };
    let working = Answer {
        plan: Some(plan),
        ..plain("Ich schaue nach …")
    };
    reply.show(&working, false).await.unwrap();
    reply.show(&plain("Gefunden"), false).await.unwrap();

    let seen = fake.seen();
    assert_eq!(seen.len(), 2);
    assert_eq!(
        (&seen[0].method, seen[0].path.as_str()),
        (&Method::POST, MESSAGES)
    );
    let first = &seen[0].body;
    assert_eq!(first["content"], "Ich schaue nach …");
    assert_eq!(first["message_reference"]["message_id"], "1000");
    assert_eq!(first["message_reference"]["fail_if_not_exists"], false);
    assert_eq!(first["allowed_mentions"]["parse"], json!(["users"]));
    assert_eq!(first["allowed_mentions"]["replied_user"], true);
    assert_eq!(first["embeds"][0]["title"], "🧠 Thinking...");
    assert_eq!(first["embeds"][0]["fields"][0]["name"], "🔄 Step 1");
    assert_eq!(
        (&seen[1].method, seen[1].path.as_str()),
        (&Method::PATCH, REPLY)
    );
    assert_eq!(seen[1].body["content"], "Gefunden");
    assert_eq!(seen[1].body["embeds"], json!([]), "the plan leaves");
    no_token_outside_the_header(&seen);

    let pauses = [1, 5, 9].map(|shown| reply.edit_interval(shown));
    assert_eq!(pauses, [5, 10, 15].map(Duration::from_secs));
}

#[tokio::test]
async fn final_answer_edits_the_reply_and_sends_the_rest() {
    let (url, fake) = fake().await;
    let mut reply = reply_at(&url);
    reply
        .show(&plain("Ich schaue nach …"), false)
        .await
        .unwrap();
    let text = "Grüße 😀 中文 ".repeat(400);
    reply.show(&plain(&text), true).await.unwrap();

    let seen = fake.seen();
    let methods: Vec<&Method> = seen.iter().map(|request| &request.method).collect();
    let expected = [&Method::POST, &Method::PATCH, &Method::POST, &Method::POST];
    assert_eq!(methods, expected);
    assert_eq!(seen[1].path, REPLY, "the first part edits the reply");
    let parts: Vec<&str> = seen[1..]
        .iter()
        .map(|request| request.body["content"].as_str().unwrap())
        .collect();
    assert!(parts.iter().all(|part| utf16_len(part) <= MESSAGE_LIMIT));
    assert_eq!(parts.concat(), text.trim_end());
    for later in &seen[2..] {
        assert_eq!(later.path, MESSAGES);
        assert!(later.body.get("message_reference").is_none());
        assert_eq!(later.body["allowed_mentions"]["parse"], json!(["users"]));
    }
    no_token_outside_the_header(&seen);
}

#[tokio::test]
async fn a_failed_send_is_reported_and_the_final_answer_is_sent_new() {
    let mut reply = reply_at("http://127.0.0.1:9");
    assert!(reply.show(&plain("first"), false).await.is_err());

    let (url, fake) = fake().await;
    let mut reply = reply_at(&url);
    reply
        .show(&plain("only the final answer"), true)
        .await
        .unwrap();
    let seen = fake.seen();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].method, Method::POST);
    assert_eq!(seen[0].body["message_reference"]["message_id"], "1000");
}

#[tokio::test]
async fn a_deleted_reply_is_sent_anew() {
    let (url, fake) = fake().await;
    let mut reply = reply_at(&url);
    reply
        .show(&plain("Ich schaue nach …"), false)
        .await
        .unwrap();
    fake.delete(5001);
    reply.show(&plain("Gefunden"), true).await.unwrap();

    let seen = fake.seen();
    let methods: Vec<&Method> = seen.iter().map(|request| &request.method).collect();
    assert_eq!(methods, [&Method::POST, &Method::PATCH, &Method::POST]);
    assert_eq!(seen[2].body["content"], "Gefunden");
    assert_eq!(seen[2].body["message_reference"]["message_id"], "1000");
}

#[tokio::test]
async fn notice_pings_nobody() {
    let (url, fake) = fake().await;
    notice(
        &http(&url),
        ChannelId::new(300),
        MessageId::new(1000),
        "I'm restarting. Please send that again in a moment.",
    )
    .await;
    let seen = fake.seen();
    assert_eq!(seen.len(), 1);
    assert_eq!(
        seen[0].body["content"],
        "I'm restarting. Please send that again in a moment."
    );
    assert_eq!(seen[0].body["message_reference"]["message_id"], "1000");
    assert_eq!(seen[0].body["allowed_mentions"]["parse"], json!([]));
    assert!(
        seen[0].body["allowed_mentions"]
            .get("replied_user")
            .is_none()
    );
    no_token_outside_the_header(&seen);
}

fn token() -> BotToken {
    BotToken::parse(Provider::Discord, TOKEN.as_bytes()).expect("a token of Discord's shape")
}

fn spec() -> BotSpec {
    BotSpec::from_config("evt_dc", "discord", b"{}").expect("valid settings")
}

fn current() -> Message {
    serde_json::from_value(message_json(
        1000,
        json!({"content": "What changed <@900>?", "mentions": [{"id": BOT.to_string(), "username": "helper", "bot": true}]}),
    ))
    .unwrap()
}

fn images() -> ImageBudget {
    ImageBudget::new(Arc::new(Mutex::new(HourBudget::new())), 0)
}

#[tokio::test]
async fn payload_reads_the_history_once_and_carries_the_handle() {
    let (url, fake) = fake().await;
    let connection = DiscordConnection::with_http(&token(), &[spec()], http(&url));
    let message = facts(current(), Some(UserId::new(BOT)));

    let payload = connection.payload(&message, HANDLE, &mut images()).await;

    let seen = fake.seen();
    assert_eq!(seen.len(), 1, "one history read");
    assert_eq!(seen[0].method, Method::GET);
    assert_eq!(seen[0].path, "/api/v10/channels/300/messages");
    let mut query: Vec<&str> = seen[0]
        .query
        .split('&')
        .filter(|pair| !pair.is_empty())
        .collect();
    query.sort_unstable();
    assert_eq!(query, vec!["before=1000", "limit=10"]);

    let messages = payload["messages"].as_array().unwrap();
    assert_eq!(messages.len(), 11);
    assert_eq!(
        messages[0]["content"][0]["text"],
        "alice[id: 500]: message 990"
    );
    assert_eq!(
        messages[10]["content"][0]["text"],
        "alice[id: 500]: What changed <@900>?"
    );
    assert_eq!(payload["local_session"]["bot_token"], HANDLE);
    assert_eq!(payload["local_session"]["bot_user_id"], "900");
    assert!(!payload.to_string().contains(TOKEN));
    no_token_outside_the_header(&seen);
}

#[tokio::test]
async fn payload_without_reachable_history_still_has_the_message() {
    let connection = DiscordConnection::with_http(&token(), &[spec()], http("http://127.0.0.1:9"));
    let message = facts(current(), Some(UserId::new(BOT)));
    let payload = tokio::time::timeout(
        Duration::from_secs(10),
        connection.payload(&message, HANDLE, &mut images()),
    )
    .await
    .expect("the history read is bounded");
    assert_eq!(payload["messages"].as_array().unwrap().len(), 1);
    assert_eq!(payload["local_session"]["bot_token"], HANDLE);
}

#[tokio::test]
async fn connection_replies_and_notices_in_the_message_channel() {
    let (url, fake) = fake().await;
    let connection = DiscordConnection::with_http(&token(), &[spec()], http(&url));
    let message = facts(current(), Some(UserId::new(BOT)));

    connection
        .reply(&message)
        .show(&plain("Nothing changed."), true)
        .await
        .unwrap();
    connection
        .notice(
            &message,
            "I'm getting too many messages. Try again in a minute.",
        )
        .await;

    let seen = fake.seen();
    assert_eq!(seen.len(), 2);
    assert!(seen.iter().all(|request| request.method == Method::POST
        && request.path == "/api/v10/channels/300/messages"
        && request.body["message_reference"]["message_id"] == "1000"));
    no_token_outside_the_header(&seen);
}

/// Every span and event of this thread, at every level, as text.
#[derive(Clone, Default)]
struct Recorder {
    lines: Arc<Mutex<Vec<String>>>,
    spans: Arc<AtomicU64>,
}

struct Fields<'a>(&'a mut String);

impl Visit for Fields<'_> {
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        self.0.push_str(&format!(" {}={value:?}", field.name()));
    }
}

impl Recorder {
    fn push(&self, line: String) {
        self.lines.lock().unwrap().push(line);
    }
}

impl Subscriber for Recorder {
    fn enabled(&self, _metadata: &Metadata<'_>) -> bool {
        true
    }

    fn new_span(&self, span: &Attributes<'_>) -> Id {
        let mut line = span.metadata().name().to_string();
        span.record(&mut Fields(&mut line));
        self.push(line);
        Id::from_u64(self.spans.fetch_add(1, Ordering::SeqCst) + 1)
    }

    fn record(&self, _span: &Id, values: &Record<'_>) {
        let mut line = String::new();
        values.record(&mut Fields(&mut line));
        self.push(line);
    }

    fn record_follows_from(&self, _span: &Id, _follows: &Id) {}

    fn event(&self, event: &tracing::Event<'_>) {
        let metadata = event.metadata();
        let mut line = format!("{} {}", metadata.level(), metadata.target());
        event.record(&mut Fields(&mut line));
        self.push(line);
    }

    fn enter(&self, _span: &Id) {}

    fn exit(&self, _span: &Id) {}
}

/// A run's whole way through the provider: history read, reply, edit, split answer, notice.
async fn answer_once(connection: &DiscordConnection, message: &flow_like_bots::Message) {
    connection.payload(message, HANDLE, &mut images()).await;
    let mut reply = connection.reply(message);
    let _ = reply.show(&plain("Zwischenstand"), false).await;
    let _ = reply.show(&plain(&"Antwort ".repeat(400)), true).await;
    connection.notice(message, "I'm restarting.").await;
}

#[tokio::test]
async fn no_log_line_carries_the_token() {
    // Global, not scoped to this thread: tracing caches a callsite's interest from the thread
    // that first reaches it, and the other tests of this binary run on other threads. So the
    // lines of every test of the binary are checked.
    let recorder = Recorder::default();
    tracing::subscriber::set_global_default(recorder.clone()).expect("one global subscriber");
    let (url, fake) = fake().await;
    let connection = DiscordConnection::with_http(&token(), &[spec()], http(&url));
    let message = facts(current(), Some(UserId::new(BOT)));

    answer_once(&connection, &message).await;
    fake.refuse(StatusCode::UNAUTHORIZED);
    answer_once(&connection, &message).await;
    fake.refuse(StatusCode::INTERNAL_SERVER_ERROR);
    answer_once(&connection, &message).await;
    let unreachable = http("http://127.0.0.1:9");
    let unreachable = DiscordConnection::with_http(&token(), &[spec()], unreachable);
    answer_once(&unreachable, &message).await;

    let lines = recorder.lines.lock().unwrap();
    assert!(lines.iter().any(|line| line.contains("status=401")));
    assert!(lines.iter().any(|line| line.contains("status=500")));
    let client = "the REST client is formatted into serenity's request spans, its token redacted";
    assert!(
        lines.iter().any(|line| line.contains("REDACTED")),
        "{client}"
    );
    assert!(lines.iter().any(|line| line.contains("status=0")));
    let secret = TOKEN.split('.').next_back().expect("the token's last part");
    for line in lines.iter() {
        assert!(!line.contains(secret), "{line}");
    }
}

/// A host for the live tests: every gate open, the state kept in memory, no run expected.
struct LiveHost {
    gate: watch::Sender<Gate>,
    state: Mutex<Option<BotsState>>,
}

#[async_trait]
impl BotHost for LiveHost {
    async fn run(
        &self,
        _event_id: &str,
        _payload: Value,
        _events: mpsc::Sender<StreamEvent>,
        _cancel: CancellationToken,
    ) -> RunEnd {
        RunEnd::Succeeded
    }

    fn gate(&self, _event_id: &str) -> watch::Receiver<Gate> {
        self.gate.subscribe()
    }

    fn load(&self) -> Option<BotsState> {
        None
    }

    fn save(&self, state: &BotsState) -> std::io::Result<()> {
        *self.state.lock().unwrap() = Some(state.clone());
        Ok(())
    }

    fn now(&self) -> i64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.as_secs() as i64)
    }

    fn handle(&self, event_id: &str) -> String {
        format!("device-bot:{event_id}")
    }
}

/// Connects with `token` until the bot's state settles, stops, and returns the last state
/// document with its text.
async fn live(token: BotToken, settled: impl Fn(Status) -> bool) -> (Status, BotsState, String) {
    let spec = BotSpec::from_config(
        "evt_live",
        "discord",
        br#"{"intents":["Guilds","GuildMessages"],"respond_to_dms":false}"#,
    )
    .expect("valid settings");
    let host = Arc::new(LiveHost {
        gate: watch::channel(Gate::Open).0,
        state: Mutex::new(None),
    });
    let stop = CancellationToken::new();
    let running = tokio::spawn(flow_like_bots::run(
        vec![Bot { spec, token }],
        Revisions {
            config_revision: 1,
            intent_revision: 1,
        },
        host.clone(),
        async {},
        stop.clone(),
    ));
    let mut status = Status::Waiting;
    for _ in 0..120 {
        tokio::time::sleep(Duration::from_millis(500)).await;
        status = host
            .state
            .lock()
            .unwrap()
            .as_ref()
            .and_then(|state| state.bots.get("evt_live").map(|entry| entry.state))
            .unwrap_or(Status::Waiting);
        if settled(status) {
            break;
        }
    }
    stop.cancel();
    tokio::time::timeout(Duration::from_secs(20), running)
        .await
        .expect("the bots stop")
        .unwrap();
    let state = host.state.lock().unwrap().clone().expect("a saved state");
    let saved = serde_json::to_string(&state).unwrap();
    (status, state, saved)
}

#[tokio::test]
#[ignore = "needs FLOW_LIKE_TEST_DISCORD_TOKEN and Discord's gateway"]
async fn live_bot_connects_and_stops() {
    let secret = std::env::var("FLOW_LIKE_TEST_DISCORD_TOKEN")
        .expect("FLOW_LIKE_TEST_DISCORD_TOKEN holds a bot token");
    let token = BotToken::parse(Provider::Discord, secret.as_bytes()).expect("a bot token");
    let (status, state, saved) = live(token, |status| status == Status::Connected).await;
    assert_eq!(status, Status::Connected);
    assert!(state.bots["evt_live"].bot_id.is_some());
    assert!(!saved.contains(secret.trim()));
}

#[tokio::test]
#[ignore = "needs Discord's gateway"]
async fn live_refused_token_stops_the_bot() {
    let (status, state, saved) = live(token(), |status| status == Status::TokenRefused).await;
    assert_eq!(status, Status::TokenRefused);
    assert_eq!(state.bots["evt_live"].state, Status::TokenRefused);
    assert!(!saved.contains(TOKEN));
}
