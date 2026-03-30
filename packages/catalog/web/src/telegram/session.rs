//! Telegram session types and management

use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_types::async_trait;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[cfg(feature = "execute")]
use {
    flow_like_types::{Cacheable, json::json},
    std::{
        any::Any,
        collections::HashMap,
        sync::{Arc, OnceLock},
    },
    teloxide::prelude::*,
    teloxide::types::ChatId,
    tokio::sync::{RwLock, broadcast},
};

// ============================================================================
// Telegram Update Broadcaster
// ============================================================================
// Provides a global pub/sub mechanism so that the Telegram dispatcher (which
// owns the single `getUpdates` long-poll connection) can fan out incoming
// updates to any interaction nodes that are waiting for replies or callbacks.

/// Events broadcast from the dispatcher to interaction node subscribers.
#[cfg(feature = "execute")]
#[derive(Clone, Debug)]
pub enum TelegramBroadcastEvent {
    Message(Box<teloxide::types::Message>),
    CallbackQuery(Box<teloxide::types::CallbackQuery>),
}

#[cfg(feature = "execute")]
fn tg_broadcasters() -> &'static RwLock<HashMap<String, broadcast::Sender<TelegramBroadcastEvent>>>
{
    static INSTANCE: OnceLock<
        RwLock<HashMap<String, broadcast::Sender<TelegramBroadcastEvent>>>,
    > = OnceLock::new();
    INSTANCE.get_or_init(|| RwLock::new(HashMap::new()))
}

/// Subscribe to Telegram updates for a specific bot (keyed by token).
#[cfg(feature = "execute")]
pub async fn subscribe_tg_updates(
    bot_token: &str,
) -> broadcast::Receiver<TelegramBroadcastEvent> {
    {
        let read = tg_broadcasters().read().await;
        if let Some(sender) = read.get(bot_token) {
            return sender.subscribe();
        }
    }
    let mut write = tg_broadcasters().write().await;
    let sender = write
        .entry(bot_token.to_string())
        .or_insert_with(|| broadcast::channel(512).0);
    sender.subscribe()
}

/// Publish a Telegram update so that all interaction node subscribers receive it.
#[cfg(feature = "execute")]
pub async fn broadcast_tg_event(bot_token: &str, event: TelegramBroadcastEvent) {
    {
        let read = tg_broadcasters().read().await;
        if let Some(sender) = read.get(bot_token) {
            let _ = sender.send(event);
            return;
        }
    }
    let mut write = tg_broadcasters().write().await;
    let sender = write
        .entry(bot_token.to_string())
        .or_insert_with(|| broadcast::channel(512).0);
    let _ = sender.send(event);
}

/// Telegram user information
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Default)]
pub struct TelegramUser {
    pub id: String,
    pub name: String,
    pub username: Option<String>,
    pub is_bot: bool,
}

/// Telegram session data stored in global_session
/// Contains all information needed to reconstruct a Telegram bot client
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct TelegramSessionData {
    pub bot_token: String,
    pub chat_id: String,
    pub message_id: String,
    pub chat_type: String,
    pub chat_title: Option<String>,
    pub bot_username: Option<String>,
    #[serde(default)]
    pub user: Option<TelegramUser>,
}

/// A typed Telegram session with a reference to the cached bot
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct TelegramSession {
    pub ref_id: String,
    pub chat_id: String,
    pub message_id: String,
    pub chat_type: String,
    pub chat_title: Option<String>,
    pub bot_username: Option<String>,
    #[serde(default)]
    pub user: Option<TelegramUser>,
}

#[cfg(feature = "execute")]
impl TelegramSession {
    pub fn chat_id(&self) -> flow_like_types::Result<ChatId> {
        Ok(ChatId(self.chat_id.parse()?))
    }

    pub fn message_id(&self) -> flow_like_types::Result<teloxide::types::MessageId> {
        Ok(teloxide::types::MessageId(self.message_id.parse()?))
    }
}

/// Cached Telegram bot for making API calls
#[cfg(feature = "execute")]
pub struct CachedTelegramBot {
    pub bot: Bot,
    pub bot_username: Option<String>,
}

#[cfg(feature = "execute")]
impl Cacheable for CachedTelegramBot {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

#[cfg(feature = "execute")]
impl CachedTelegramBot {
    pub fn new(token: &str) -> Self {
        let bot = Bot::new(token);
        Self {
            bot,
            bot_username: None,
        }
    }

    pub async fn with_bot_info(token: &str) -> flow_like_types::Result<Self> {
        let bot = Bot::new(token);
        let me = bot.get_me().await?;
        Ok(Self {
            bot,
            bot_username: me.username.clone(),
        })
    }
}

/// Helper to get the cached Telegram bot from context
#[cfg(feature = "execute")]
pub async fn get_telegram_bot(
    context: &ExecutionContext,
    ref_id: &str,
) -> flow_like_types::Result<Arc<CachedTelegramBot>> {
    let cache = context.cache.read().await;
    let bot = cache
        .get(ref_id)
        .ok_or_else(|| flow_like_types::anyhow!("Telegram bot not found in cache: {}", ref_id))?
        .clone();

    let bot = bot
        .as_any()
        .downcast_ref::<CachedTelegramBot>()
        .ok_or_else(|| flow_like_types::anyhow!("Failed to downcast Telegram bot"))?;

    Ok(Arc::new(CachedTelegramBot {
        bot: bot.bot.clone(),
        bot_username: bot.bot_username.clone(),
    }))
}

// ============================================================================
// To Telegram Session Node
// ============================================================================

#[flow_like_catalog_macros::register_node]
#[derive(Default)]
pub struct ToTelegramSessionNode;

impl ToTelegramSessionNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for ToTelegramSessionNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "telegram_to_session",
            "To Telegram Session",
            "Creates a Telegram session from local_session data for use with other Telegram nodes",
            "Telegram",
        );
        node.add_icon("/flow/icons/telegram.svg");

        node.add_input_pin("exec_in", "Input", "Trigger", VariableType::Execution);

        node.add_input_pin(
            "local_session",
            "Local Session",
            "The local_session from a Chat Event containing Telegram session data",
            VariableType::Struct,
        );

        node.add_output_pin(
            "exec_out",
            "Output",
            "Continues after session is created",
            VariableType::Execution,
        );

        node.add_output_pin(
            "session",
            "Session",
            "Telegram session for use with other Telegram nodes",
            VariableType::Struct,
        )
        .set_schema::<TelegramSession>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let session_data: TelegramSessionData = context.evaluate_pin("local_session").await?;

        let ref_id = format!("telegram_bot_{}", flow_like_types::create_id());

        let bot = CachedTelegramBot::with_bot_info(&session_data.bot_token)
            .await
            .unwrap_or_else(|_| CachedTelegramBot::new(&session_data.bot_token));

        let cacheable: Arc<dyn Cacheable> = Arc::new(bot);
        context.set_cache(&ref_id, cacheable).await;

        let session = TelegramSession {
            ref_id,
            chat_id: session_data.chat_id,
            message_id: session_data.message_id,
            chat_type: session_data.chat_type,
            chat_title: session_data.chat_title,
            bot_username: session_data.bot_username,
            user: session_data.user,
        };

        context.set_pin_value("session", json!(session)).await?;

        let exec_out = context.get_pin_by_name("exec_out").await?;
        context.activate_exec_pin_ref(&exec_out).await?;

        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "Telegram functionality requires the 'execute' feature"
        ))
    }
}
