//! One Discord gateway client per bot: admitted messages are relayed to the hub, the reply of
//! the turn is posted back into the channel.

use super::hub::Hub;
use super::store::DiscordSettings;
use flow_like_bots::discord::{ClientEnd, DiscordReply, facts_of, notice, startable};
use flow_like_bots::reply::{Answer, ReplySink, SendError};
use flow_like_bots::BotSpec;
use reqwest::Method;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use serenity::all::{
    ChannelId, Client, Context, EventHandler, GatewayIntents, Http, Message, MessageId,
    MessageType, Permissions, Ready, UserId,
};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use tokio_util::sync::CancellationToken;

const CONCURRENT_RELAYS: usize = 4;
const TURN_DEADLINE: Duration = Duration::from_secs(15 * 60);
const POLL_WAIT: &str = "25";
const WAITING_NOTICE_AFTER: Duration = Duration::from_secs(30);
const NOTICE_WAIT: Duration = Duration::from_secs(5);
const CLOSE_WAIT: Duration = Duration::from_secs(5);
const MAX_TEXT_CHARS: usize = 8000;
const MAX_ATTACHMENTS: usize = 10;
const MAX_TITLE_CHARS: usize = 100;

pub const ERROR_TOKEN: &str = "Discord rejected the token";
pub const ERROR_INTENTS: &str =
    "Message Content intent is not enabled for this bot in the Discord Developer Portal";
pub const ERROR_ENDED: &str = "The Discord connection ended and will be retried";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    Stopped,
    Connecting,
    Connected,
    Error,
}

#[derive(Debug, Clone, Serialize)]
pub struct BotUser {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone)]
pub struct LinkState {
    pub status: Status,
    pub error: Option<String>,
    pub bot_user: Option<BotUser>,
    pub invite_url: Option<String>,
}

impl LinkState {
    fn connecting() -> Self {
        Self {
            status: Status::Connecting,
            error: None,
            bot_user: None,
            invite_url: None,
        }
    }
}

type SharedState = Arc<Mutex<LinkState>>;

fn update(state: &SharedState, change: impl FnOnce(&mut LinkState)) {
    change(&mut state.lock().unwrap_or_else(PoisonError::into_inner));
}

pub struct Running {
    pub fingerprint: String,
    pub state: SharedState,
    stop: CancellationToken,
    ended: Arc<AtomicBool>,
    refused: Arc<AtomicBool>,
}

impl Running {
    pub fn start(
        hub: Arc<Hub>,
        bot_id: &str,
        settings: &DiscordSettings,
        token: String,
        fingerprint: String,
    ) -> Self {
        let state = Arc::new(Mutex::new(LinkState::connecting()));
        let running = Self {
            fingerprint,
            state: state.clone(),
            stop: CancellationToken::new(),
            ended: Arc::default(),
            refused: Arc::default(),
        };
        let shared = Arc::new(Shared {
            bot_id: bot_id.to_owned(),
            spec: settings.spec(bot_id),
            hub,
            state,
            slots: Arc::new(Semaphore::new(CONCURRENT_RELAYS)),
        });
        tauri::async_runtime::spawn(serve(
            shared,
            token,
            intents(settings.respond_to_dms),
            running.stop.clone(),
            running.ended.clone(),
            running.refused.clone(),
        ));
        running
    }

    pub fn stop(&self) {
        self.stop.cancel();
    }

    /// The client ended for a reason a restart can fix.
    pub fn needs_restart(&self) -> bool {
        self.ended.load(Ordering::SeqCst) && !self.refused.load(Ordering::SeqCst)
    }
}

pub fn intents(direct_messages: bool) -> GatewayIntents {
    let base =
        GatewayIntents::GUILDS | GatewayIntents::GUILD_MESSAGES | GatewayIntents::MESSAGE_CONTENT;
    if direct_messages {
        base | GatewayIntents::DIRECT_MESSAGES
    } else {
        base
    }
}

pub fn invite_permissions() -> Permissions {
    Permissions::VIEW_CHANNEL
        | Permissions::SEND_MESSAGES
        | Permissions::READ_MESSAGE_HISTORY
        | Permissions::SEND_MESSAGES_IN_THREADS
        | Permissions::ATTACH_FILES
}

pub fn invite_url(application_id: u64) -> String {
    format!(
        "https://discord.com/oauth2/authorize?client_id={application_id}&scope=bot&permissions={}",
        invite_permissions().bits()
    )
}

struct Shared {
    bot_id: String,
    spec: BotSpec,
    hub: Arc<Hub>,
    state: SharedState,
    slots: Arc<Semaphore>,
}

async fn serve(
    shared: Arc<Shared>,
    token: String,
    intents: GatewayIntents,
    stop: CancellationToken,
    ended: Arc<AtomicBool>,
    refused: Arc<AtomicBool>,
) {
    let state = shared.state.clone();
    let built = Client::builder(&token, intents)
        .event_handler(Handler(shared))
        .await;
    drop(token);
    let mut client = match built {
        Ok(client) => client,
        Err(_) => {
            update(&state, |link| {
                link.status = Status::Error;
                link.error = Some(ERROR_TOKEN.into());
            });
            refused.store(true, Ordering::SeqCst);
            ended.store(true, Ordering::SeqCst);
            return;
        }
    };
    let shards = client.shard_manager.clone();
    let mut running = tokio::spawn(async move { ClientEnd::of(&client.start().await) });
    tokio::select! {
        finished = &mut running => {
            let end = finished.unwrap_or(ClientEnd::Ended);
            tracing::warn!(code = end.code(), "Bot channel Discord client ended");
            let (message, final_end) = match end {
                ClientEnd::TokenRefused => (ERROR_TOKEN, true),
                ClientEnd::IntentsRefused => (ERROR_INTENTS, true),
                ClientEnd::Ended => (ERROR_ENDED, false),
            };
            update(&state, |link| {
                link.status = Status::Error;
                link.error = Some(message.into());
            });
            refused.store(final_end, Ordering::SeqCst);
            ended.store(true, Ordering::SeqCst);
        }
        () = stop.cancelled() => {
            shards.shutdown_all().await;
            if tokio::time::timeout(CLOSE_WAIT, &mut running).await.is_err() {
                running.abort();
            }
            update(&state, |link| link.status = Status::Stopped);
        }
    }
}

struct Handler(Arc<Shared>);

#[serenity::async_trait]
impl EventHandler for Handler {
    async fn ready(&self, _ctx: Context, ready: Ready) {
        let user = BotUser {
            id: ready.user.id.to_string(),
            name: ready.user.name.clone(),
        };
        update(&self.0.state, |link| {
            link.status = Status::Connected;
            link.error = None;
            link.invite_url = Some(invite_url(ready.application.id.get()));
            link.bot_user = Some(user);
        });
        tracing::info!("Bot channel Discord client connected");
    }

    async fn message(&self, ctx: Context, msg: Message) {
        let bot = ctx.cache.current_user().id;
        if !admits(&self.0.spec, &msg, bot) {
            return;
        }
        let http = ctx.http.clone();
        let Ok(permit) = self.0.slots.clone().try_acquire_owned() else {
            let _ = tokio::time::timeout(
                NOTICE_WAIT,
                notice(&http, msg.channel_id, msg.id, BUSY_NOTICE),
            )
            .await;
            return;
        };
        let title = title_of(&ctx, &msg);
        let body = post_body(&msg, bot, title);
        let shared = self.0.clone();
        tokio::spawn(relay(
            shared,
            http,
            msg.channel_id,
            msg.id,
            body,
            permit,
        ));
    }
}

const BUSY_NOTICE: &str = "I'm answering several messages right now. Please try again in a moment.";
const UNREACHABLE_NOTICE: &str =
    "I couldn't reach my owner's Flow-Like account right now. Please try again later.";
const WAITING_NOTICE: &str = "Waiting for my owner's approval...";
const FAILED_NOTICE: &str = "Sorry, I couldn't answer that.";
const CANCELLED_NOTICE: &str = "That request was cancelled.";
const LOST_NOTICE: &str = "I lost track of that request.";

/// Whether a gateway message starts a relay: a person's plain message or reply that the
/// bot's settings admit.
pub fn admits(spec: &BotSpec, msg: &Message, bot: UserId) -> bool {
    msg.author.id != bot
        && matches!(msg.kind, MessageType::Regular | MessageType::InlineReply)
        && startable(msg)
        && spec.admits(&facts_of(msg, Some(bot)))
}

/// `<@id>` and `<@!id>` of the bot removed, the rest trimmed.
pub fn strip_mention(text: &str, bot: u64) -> String {
    text.replace(&format!("<@{bot}>"), "")
        .replace(&format!("<@!{bot}>"), "")
        .trim()
        .to_owned()
}

pub fn chat_title(channel: Option<&str>, guild: Option<&str>, dm_with: Option<&str>) -> String {
    let title = match (guild, channel, dm_with) {
        (None, _, Some(name)) => format!("DM with {name}"),
        (Some(guild), Some(channel), _) => format!("#{channel} ({guild})"),
        (None, Some(channel), None) => format!("#{channel}"),
        (Some(guild), None, _) => format!("Discord ({guild})"),
        (None, None, None) => "Discord chat".to_owned(),
    };
    title.chars().take(MAX_TITLE_CHARS).collect()
}

fn title_of(ctx: &Context, msg: &Message) -> String {
    let Some(guild_id) = msg.guild_id else {
        let name = msg
            .author
            .global_name
            .as_deref()
            .unwrap_or(&msg.author.name);
        return chat_title(None, None, Some(name));
    };
    let (channel, guild) = ctx
        .cache
        .guild(guild_id)
        .map(|guild| {
            let channel = guild
                .channels
                .get(&msg.channel_id)
                .or_else(|| guild.threads.iter().find(|thread| thread.id == msg.channel_id))
                .map(|channel| channel.name.clone());
            (channel, Some(guild.name.clone()))
        })
        .unwrap_or_default();
    chat_title(channel.as_deref(), guild.as_deref(), None)
}

/// The body of `POST /bots/{bot}/channels/discord/messages`. A thread is its own chat: its
/// channel id is the chat id.
pub fn post_body(msg: &Message, bot: UserId, chat_title: String) -> Value {
    let mut text = strip_mention(&msg.content, bot.get());
    if text.chars().count() > MAX_TEXT_CHARS {
        text = text.chars().take(MAX_TEXT_CHARS).collect();
    }
    let attachments: Vec<Value> = msg
        .attachments
        .iter()
        .filter(|file| file.url.starts_with("https://"))
        .take(MAX_ATTACHMENTS)
        .map(|file| {
            json!({
                "name": file.filename,
                "url": file.url,
                "content_type": file.content_type,
            })
        })
        .collect();
    if text.is_empty() && attachments.is_empty() {
        text = "(mentioned without a message)".into();
    }
    let sender = msg
        .author
        .global_name
        .as_deref()
        .unwrap_or(&msg.author.name);
    json!({
        "chat_id": msg.channel_id.to_string(),
        "guild_id": msg.guild_id.map(|guild| guild.to_string()),
        "chat_title": chat_title,
        "message_id": msg.id.to_string(),
        "sender": {"id": msg.author.id.to_string(), "name": sender},
        "text": text,
        "attachments": attachments,
    })
}

#[derive(Debug, Deserialize)]
struct Posted {
    turn_id: Option<String>,
    status: String,
    notice: Option<String>,
}

#[derive(Debug, Deserialize)]
struct TurnReply {
    text: String,
}

#[derive(Debug, Deserialize)]
pub struct TurnView {
    pub status: String,
    reply: Option<TurnReply>,
    pub error_code: Option<String>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Settled {
    Pending,
    Reply(String),
    Notice(&'static str),
    Silent,
}

/// What a polled turn means for the channel.
pub fn settle(turn: &TurnView) -> Settled {
    let reply = turn
        .reply
        .as_ref()
        .map(|reply| reply.text.trim())
        .filter(|text| !text.is_empty());
    match (turn.status.as_str(), reply) {
        ("done" | "cancelled", Some(text)) => Settled::Reply(text.to_owned()),
        ("done", None) => Settled::Silent,
        ("failed", _) => Settled::Notice(FAILED_NOTICE),
        ("cancelled", None) if turn.error_code.as_deref() == Some("BATCHED") => Settled::Silent,
        ("cancelled", None) => Settled::Notice(CANCELLED_NOTICE),
        _ => Settled::Pending,
    }
}

async fn say(http: &Http, channel: ChannelId, message: MessageId, text: &str) {
    let _ = tokio::time::timeout(NOTICE_WAIT, notice(http, channel, message, text)).await;
}

async fn relay(
    shared: Arc<Shared>,
    http: Arc<Http>,
    channel: ChannelId,
    message: MessageId,
    body: Value,
    _permit: OwnedSemaphorePermit,
) {
    let path = format!("bots/{}/channels/discord/messages", shared.bot_id);
    let posted = shared.hub.call(Method::POST, &path, &[], Some(&body)).await;
    let posted: Posted = match posted.map(serde_json::from_value) {
        Ok(Ok(posted)) => posted,
        Ok(Err(error)) => {
            tracing::warn!(%error, "Discord relay: unreadable answer to a posted message");
            return;
        }
        Err(error) => {
            tracing::warn!(%error, "Discord relay: posting a message to the hub failed");
            say(&http, channel, message, UNREACHABLE_NOTICE).await;
            return;
        }
    };
    match (posted.status.as_str(), posted.turn_id) {
        ("limited", _) => {
            if let Some(text) = posted.notice.filter(|text| !text.trim().is_empty()) {
                say(&http, channel, message, &text).await;
            }
        }
        ("queued", Some(turn_id)) => {
            let typing = http.start_typing(channel);
            let settled = await_turn(&shared, &http, channel, message, &turn_id).await;
            drop(typing);
            deliver(&http, channel, message, settled).await;
        }
        _ => {}
    }
}

async fn await_turn(
    shared: &Shared,
    http: &Http,
    channel: ChannelId,
    message: MessageId,
    turn_id: &str,
) -> Settled {
    let path = format!("bots/{}/channels/turns/{turn_id}", shared.bot_id);
    let deadline = Instant::now() + TURN_DEADLINE;
    let mut waiting_since: Option<Instant> = None;
    let mut warned = false;
    while Instant::now() < deadline {
        let asked = Instant::now();
        let answer = shared
            .hub
            .call(Method::GET, &path, &[("wait", POLL_WAIT.into())], None)
            .await;
        match answer.map(serde_json::from_value::<TurnView>) {
            Ok(Ok(turn)) => {
                let settled = settle(&turn);
                if settled != Settled::Pending {
                    return settled;
                }
                if turn.status == "waiting" {
                    let since = *waiting_since.get_or_insert_with(Instant::now);
                    if !warned && since.elapsed() > WAITING_NOTICE_AFTER {
                        warned = true;
                        say(http, channel, message, WAITING_NOTICE).await;
                    }
                } else {
                    waiting_since = None;
                }
            }
            Ok(Err(error)) => {
                tracing::warn!(%error, "Discord relay: unreadable turn status");
                return Settled::Silent;
            }
            Err(error) if error.is_transient() => {
                tokio::time::sleep(Duration::from_secs(3)).await;
            }
            Err(error) => {
                tracing::warn!(%error, "Discord relay: polling the turn failed");
                return Settled::Notice(LOST_NOTICE);
            }
        }
        if asked.elapsed() < Duration::from_secs(1) {
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
    }
    tracing::warn!("Discord relay: gave up waiting for a turn after 15 minutes");
    Settled::Silent
}

async fn deliver(http: &Arc<Http>, channel: ChannelId, message: MessageId, settled: Settled) {
    match settled {
        Settled::Reply(text) => {
            let answer = Answer {
                text,
                ..Answer::default()
            };
            let mut reply = DiscordReply::new(http.clone(), channel, message);
            if let Err(SendError::RetryAfter(wait)) = reply.show(&answer, true).await {
                tokio::time::sleep(wait.min(Duration::from_secs(30))).await;
                let _ = reply.show(&answer, true).await;
            }
        }
        Settled::Notice(text) => say(http, channel, message, text).await,
        Settled::Pending | Settled::Silent => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn turn(status: &str, reply: Option<&str>, code: Option<&str>) -> TurnView {
        TurnView {
            status: status.into(),
            reply: reply.map(|text| TurnReply { text: text.into() }),
            error_code: code.map(str::to_owned),
        }
    }

    #[test]
    fn mentions_of_the_bot_are_stripped() {
        assert_eq!(strip_mention("<@42> hello <@!42>", 42), "hello");
        assert_eq!(strip_mention("<@43> hello", 42), "<@43> hello");
        assert_eq!(strip_mention("  hi  ", 42), "hi");
    }

    #[test]
    fn titles_name_the_channel_or_the_person() {
        assert_eq!(chat_title(Some("general"), Some("Acme"), None), "#general (Acme)");
        assert_eq!(chat_title(None, None, Some("Ann")), "DM with Ann");
        assert_eq!(chat_title(None, Some("Acme"), None), "Discord (Acme)");
        assert_eq!(chat_title(None, None, None), "Discord chat");
        let long = "x".repeat(300);
        assert_eq!(chat_title(Some(&long), Some("G"), None).chars().count(), 100);
    }

    #[test]
    fn a_turn_settles_by_status_and_reply() {
        assert_eq!(settle(&turn("done", Some(" hi "), None)), Settled::Reply("hi".into()));
        assert_eq!(settle(&turn("done", None, None)), Settled::Silent);
        assert_eq!(settle(&turn("done", Some("  "), None)), Settled::Silent);
        assert_eq!(settle(&turn("failed", None, Some("X"))), Settled::Notice(FAILED_NOTICE));
        assert_eq!(settle(&turn("cancelled", None, Some("BATCHED"))), Settled::Silent);
        assert_eq!(
            settle(&turn("cancelled", None, None)),
            Settled::Notice(CANCELLED_NOTICE)
        );
        for status in ["queued", "running", "waiting"] {
            assert_eq!(settle(&turn(status, None, None)), Settled::Pending);
        }
    }

    #[test]
    fn the_invite_url_carries_the_computed_permissions() {
        let bits = invite_permissions().bits();
        assert_eq!(bits, 1024 + 2048 + 32768 + 65536 + (1u64 << 38));
        assert_eq!(
            invite_url(7),
            format!("https://discord.com/oauth2/authorize?client_id=7&scope=bot&permissions={bits}")
        );
    }

    #[test]
    fn direct_messages_add_their_intent() {
        assert!(intents(true).contains(GatewayIntents::DIRECT_MESSAGES));
        assert!(!intents(false).contains(GatewayIntents::DIRECT_MESSAGES));
        assert!(intents(false).contains(GatewayIntents::MESSAGE_CONTENT));
    }

    #[test]
    fn the_post_body_names_chat_sender_and_files() {
        let message: Message = serde_json::from_value(json!({
            "id": "100", "channel_id": "200", "guild_id": "300",
            "author": {"id": "5", "username": "ann", "discriminator": "0", "avatar": null, "global_name": "Ann"},
            "content": "<@42> what is up",
            "timestamp": "2026-01-01T00:00:00.000000+00:00",
            "edited_timestamp": null, "tts": false, "mention_everyone": false,
            "mentions": [], "mention_roles": [], "embeds": [], "pinned": false, "type": 0,
            "attachments": [
                {"id": "1", "filename": "a.png", "size": 1, "url": "https://cdn.discordapp.com/a.png", "proxy_url": "https://x/a.png", "content_type": "image/png"},
                {"id": "2", "filename": "b.txt", "size": 1, "url": "http://insecure/b.txt", "proxy_url": "http://x/b.txt"}
            ]
        }))
        .unwrap();
        let body = post_body(&message, UserId::new(42), "#general (Acme)".into());
        assert_eq!(body["chat_id"], "200");
        assert_eq!(body["guild_id"], "300");
        assert_eq!(body["message_id"], "100");
        assert_eq!(body["sender"], json!({"id": "5", "name": "Ann"}));
        assert_eq!(body["text"], "what is up");
        assert_eq!(body["attachments"].as_array().unwrap().len(), 1);
        assert_eq!(body["attachments"][0]["name"], "a.png");
    }
}
