//! Discord: one gateway client (serenity) per token, the filter of §5.5, the run's payload with
//! one bounded history read, and the reply. Every decision is a pure function of the submodules;
//! the handler only forwards what the gateway sent.

mod connection;
mod filter;
mod payload;
mod sender;

use std::future::IntoFuture;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use serde_json::{Value, json};
use serenity::builder::GetMessages;
use serenity::cache::Settings as CacheSettings;
use serenity::client::{Client, Context, EventHandler, RawEventHandler};
use serenity::gateway::{ConnectionStage, ShardManager, ShardStageUpdateEvent};
use serenity::http::Http;
use serenity::model::channel::Message as DiscordMessage;
use serenity::model::event::Event;
use serenity::model::gateway::GatewayIntents;
use serenity::model::id::UserId;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use crate::config::{BotSpec, BotToken};
use crate::limits::ImageBudget;
use crate::reply::ReplySink;
use crate::runner::{Connection, End, Intake, LinkState, Message};

pub use connection::{ClientEnd, intent, intents, stage_link};
pub use filter::{Verdict, addressed, facts, is_image, private, startable, verdict};
pub use payload::{HISTORY, chat_payload, display_name};
pub use sender::{DiscordReply, NoReply, answer_text, final_parts, notice, progress_text};

/// Gateway events waiting for the connection's loop.
const EVENTS: usize = 512;
/// The one history read of a run.
const HISTORY_WAIT: Duration = Duration::from_secs(5);
const NOTICE_WAIT: Duration = Duration::from_secs(5);
/// What `serve` waits for the client to close at a stop; the close goes on in the background.
const CLOSE_WAIT: Duration = Duration::from_millis(1_500);
/// A shard that boots after a stop is shut down by a background task, this often and this long.
const REAP_EVERY: Duration = Duration::from_secs(5);
const REAP_FOR: Duration = Duration::from_secs(600);

/// The connection of one token, serving the service's events that use it.
pub fn connect(token: &BotToken, specs: &[BotSpec]) -> Arc<dyn Connection> {
    Arc::new(DiscordConnection::new(token, specs))
}

/// What the gateway sent that the connection reads.
enum Seen {
    Gateway(Box<Event>),
    Stage(ConnectionStage),
}

/// Forwards gateway events and stage changes to the connection's loop. Serenity calls it in
/// a task of its own per event.
#[derive(Clone)]
struct Handler {
    seen: mpsc::Sender<Seen>,
}

#[async_trait]
impl RawEventHandler for Handler {
    async fn raw_event(&self, _context: Context, event: Event) {
        if matches!(
            event,
            Event::MessageCreate(_) | Event::Ready(_) | Event::Resumed(_)
        ) {
            let _ = self.seen.send(Seen::Gateway(Box::new(event))).await;
        }
    }
}

#[async_trait]
impl EventHandler for Handler {
    async fn shard_stage_update(&self, _context: Context, update: ShardStageUpdateEvent) {
        let _ = self.seen.send(Seen::Stage(update.new)).await;
    }
}

/// Serenity caches guilds, channels and users by default; the bot reads none of them.
fn cache_settings() -> CacheSettings {
    let mut settings = CacheSettings::default();
    settings.cache_guilds = false;
    settings.cache_channels = false;
    settings.cache_users = false;
    settings
}

pub struct DiscordConnection {
    token: BotToken,
    intents: GatewayIntents,
    /// REST calls of runs, notices and replies; it outlives every gateway client.
    http: Arc<Http>,
    /// The bot's own user: from the token until `READY` confirms it; 0 while unknown.
    bot: AtomicU64,
}

impl DiscordConnection {
    pub fn new(token: &BotToken, specs: &[BotSpec]) -> Self {
        Self::with_http(token, specs, Http::new(token.expose()))
    }

    /// With a REST client of the caller's, such as one pointed at a fake API.
    pub fn with_http(token: &BotToken, specs: &[BotSpec], http: Http) -> Self {
        Self {
            token: token.clone(),
            intents: intents(specs),
            http: Arc::new(http),
            bot: AtomicU64::new(token.bot_id().unwrap_or(0)),
        }
    }

    fn bot(&self) -> Option<UserId> {
        match self.bot.load(Ordering::SeqCst) {
            0 => None,
            id => Some(UserId::new(id)),
        }
    }

    async fn history(&self, message: &DiscordMessage) -> Vec<DiscordMessage> {
        let read = message.channel_id.messages(
            &*self.http,
            GetMessages::new().before(message.id).limit(HISTORY),
        );
        match tokio::time::timeout(HISTORY_WAIT, read).await {
            Ok(Ok(history)) => history,
            _ => {
                tracing::debug!(
                    code = "history_unavailable",
                    "Run starts without the channel's history"
                );
                Vec::new()
            }
        }
    }

    /// One forwarded event. `connected`: `READY` arrived on this client.
    async fn take(&self, intake: &Intake, seen: Seen, connected: &mut bool) {
        match seen {
            Seen::Stage(stage) => {
                if let Some(state) = stage_link(stage, *connected) {
                    intake.state(state);
                }
            }
            Seen::Gateway(event) => match *event {
                Event::Ready(ready) => {
                    let user = &ready.ready.user;
                    self.bot.store(user.id.get(), Ordering::SeqCst);
                    intake.connected(user.id.get(), Some(&user.name));
                    *connected = true;
                }
                Event::Resumed(_) => {
                    if *connected {
                        intake.state(LinkState::Connected);
                    }
                }
                event => {
                    let bot = self.bot();
                    let candidate = match verdict(&event, bot) {
                        Verdict::Candidate(_) => true,
                        Verdict::Ignored => {
                            intake.ignore(None);
                            false
                        }
                        Verdict::Unrelated => false,
                    };
                    if candidate && let Event::MessageCreate(created) = event {
                        intake.message(facts(created.message, bot)).await;
                    }
                }
            },
        }
    }
}

/// Shuts the client down; a shard still booting is shut down once it runs, for a while.
async fn close(shards: Arc<ShardManager>, mut running: JoinHandle<ClientEnd>) {
    shards.shutdown_all().await;
    let deadline = tokio::time::Instant::now() + REAP_FOR;
    loop {
        if tokio::time::timeout(REAP_EVERY, &mut running).await.is_ok() {
            return;
        }
        if tokio::time::Instant::now() >= deadline {
            tracing::warn!(code = "client_not_closed", "Discord client did not close");
            running.abort();
            return;
        }
        shards.shutdown_all().await;
    }
}

#[async_trait]
impl Connection for DiscordConnection {
    async fn serve(&self, intake: Intake, stop: CancellationToken) -> End {
        let (sender, mut seen) = mpsc::channel(EVENTS);
        let handler = Handler { seen: sender };
        let building = Client::builder(self.token.expose(), self.intents)
            .cache_settings(cache_settings())
            .raw_event_handler(handler.clone())
            .event_handler(handler)
            .into_future();
        let built = tokio::select! {
            built = building => built,
            () = stop.cancelled() => return End::Stopped,
        };
        let mut client = match built {
            Ok(client) => client,
            Err(error) => {
                let end = ClientEnd::of(&Err(error));
                tracing::warn!(code = end.code(), "Discord client could not be built");
                return end.end();
            }
        };
        let shards = client.shard_manager.clone();
        let mut running = tokio::spawn(async move { ClientEnd::of(&client.start().await) });
        let mut connected = false;
        loop {
            tokio::select! {
                biased;
                () = stop.cancelled() => {
                    let closing = tokio::spawn(close(shards, running));
                    let _ = tokio::time::timeout(CLOSE_WAIT, closing).await;
                    return End::Stopped;
                }
                finished = &mut running => {
                    let end = finished.unwrap_or(ClientEnd::Ended);
                    tracing::warn!(code = end.code(), "Discord client ended");
                    // The shard queue of an ended client waits for a shutdown that nobody sends.
                    tokio::spawn(async move { shards.shutdown_all().await });
                    return end.end();
                }
                Some(event) = seen.recv() => self.take(&intake, event, &mut connected).await,
            }
        }
    }

    async fn payload(&self, message: &Message, handle: &str, _images: &mut ImageBudget) -> Value {
        let Some(native) = message.native::<DiscordMessage>() else {
            return json!({"local_session": {"bot_token": handle}, "messages": [], "attachments": []});
        };
        let history = self.history(native).await;
        chat_payload(native, &history, handle, self.bot())
    }

    fn reply(&self, message: &Message) -> Box<dyn ReplySink> {
        match message.native::<DiscordMessage>() {
            Some(native) => Box::new(DiscordReply::new(
                self.http.clone(),
                native.channel_id,
                native.id,
            )),
            None => Box::new(NoReply),
        }
    }

    async fn notice(&self, message: &Message, text: &str) {
        let Some(native) = message.native::<DiscordMessage>() else {
            return;
        };
        let _ = tokio::time::timeout(
            NOTICE_WAIT,
            notice(&self.http, native.channel_id, native.id, text),
        )
        .await;
    }
}
