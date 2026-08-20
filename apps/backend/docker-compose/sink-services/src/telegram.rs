//! Telegram multi-bot implementation for Flow-Like sink services
//!
//! Supports running multiple Telegram bots concurrently, each with their own
//! token and event handlers. Bots are synced from the API and managed dynamically.

#![cfg_attr(not(feature = "telegram"), allow(dead_code))]

use crate::api_client::{ApiClient, BotConfig, BotHandler};
use crate::storage::{RedisStorage, TelegramConfigState};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{info, warn};

#[cfg(feature = "telegram")]
use teloxide::{prelude::*, types::Message};

/// Event handler configuration for a Telegram bot
#[derive(Debug, Clone)]
pub struct TelegramEventHandler {
    pub event_id: String,
    pub chat_id: Option<i64>,
    pub command: Option<String>,
}

/// Live Telegram bot connections keyed by bot id: `(token_hash, shutdown_sender)`.
type ActiveTelegramBots = Arc<RwLock<HashMap<String, (String, tokio::sync::watch::Sender<bool>)>>>;

/// Manages multiple Telegram bot instances
pub struct TelegramBotManager {
    api_client: Arc<ApiClient>,
    storage: Option<Arc<RedisStorage>>,
    /// Active bot connections: bot_id -> (token_hash, shutdown_sender)
    active_bots: ActiveTelegramBots,
    /// Event handlers per bot: bot_id -> handlers
    bot_handlers: Arc<RwLock<HashMap<String, Vec<TelegramEventHandler>>>>,
}

impl TelegramBotManager {
    pub fn new(api_client: Arc<ApiClient>, storage: Option<Arc<RedisStorage>>) -> Self {
        Self {
            api_client,
            storage,
            active_bots: Arc::new(RwLock::new(HashMap::new())),
            bot_handlers: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Sync bot configurations from API
    pub async fn sync_bots(&self) -> Result<SyncResult, Box<dyn std::error::Error + Send + Sync>> {
        let bot_configs = self.api_client.get_bot_configs("telegram").await?;

        let mut started = 0;
        let mut stopped = 0;
        let mut updated = 0;

        let new_bot_ids: std::collections::HashSet<_> =
            bot_configs.iter().map(|b| b.bot_id.clone()).collect();

        // Stop bots that are no longer configured
        {
            let active = self.active_bots.read().await;
            let to_stop: Vec<_> = active
                .keys()
                .filter(|id| !new_bot_ids.contains(*id))
                .cloned()
                .collect();
            drop(active);

            for bot_id in to_stop {
                if self.stop_bot(&bot_id).await.is_ok() {
                    stopped += 1;
                }
            }
        }

        // Start or update bots
        for config in bot_configs {
            let token_hash = RedisStorage::hash_token(&config.token);

            let active = self.active_bots.read().await;
            let existing = active.get(&config.bot_id);

            match existing {
                Some((existing_hash, _)) if existing_hash == &token_hash => {
                    drop(active);
                    self.update_handlers(&config).await;
                    updated += 1;
                }
                Some(_) => {
                    drop(active);
                    self.stop_bot(&config.bot_id).await.ok();
                    if self.start_bot(&config).await.is_ok() {
                        started += 1;
                    }
                }
                None => {
                    drop(active);
                    if self.start_bot(&config).await.is_ok() {
                        started += 1;
                    }
                }
            }
        }

        Ok(SyncResult {
            started,
            stopped,
            updated,
        })
    }

    async fn update_handlers(&self, config: &BotConfig) {
        let handlers: Vec<TelegramEventHandler> = config
            .handlers
            .iter()
            .filter_map(Self::parse_handler)
            .collect();

        let mut bot_handlers = self.bot_handlers.write().await;
        bot_handlers.insert(config.bot_id.clone(), handlers.clone());

        if let Some(ref storage) = self.storage {
            for handler in &handlers {
                let state = TelegramConfigState {
                    event_id: handler.event_id.clone(),
                    bot_id: config.bot_id.clone(),
                    chat_id: handler.chat_id,
                    command: handler.command.clone(),
                };
                if let Err(e) = storage.set_telegram_config(&state).await {
                    warn!("Failed to save Telegram config: {}", e);
                }
            }
        }
    }

    fn parse_handler(handler: &BotHandler) -> Option<TelegramEventHandler> {
        Some(TelegramEventHandler {
            event_id: handler.event_id.clone(),
            chat_id: handler.config.get("chat_id").and_then(|v| v.as_i64()),
            command: handler
                .config
                .get("command")
                .and_then(|v| v.as_str())
                .map(String::from),
        })
    }

    #[cfg(feature = "telegram")]
    async fn start_bot(
        &self,
        config: &BotConfig,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let token_hash = RedisStorage::hash_token(&config.token);
        let bot_id = config.bot_id.clone();

        info!(bot_id = %bot_id, "Starting Telegram bot");

        let handlers: Vec<TelegramEventHandler> = config
            .handlers
            .iter()
            .filter_map(|h| Self::parse_handler(h))
            .collect();

        {
            let mut bot_handlers = self.bot_handlers.write().await;
            bot_handlers.insert(bot_id.clone(), handlers.clone());
        }

        // Use watch channel for shutdown (can be cloned)
        let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);

        {
            let mut active = self.active_bots.write().await;
            active.insert(bot_id.clone(), (token_hash.clone(), shutdown_tx));
        }

        if let Some(ref storage) = self.storage {
            let bot_state = TelegramBotState {
                bot_id: bot_id.clone(),
                token_hash: token_hash.clone(),
                connected: false,
                last_seen: None,
                handler_count: handlers.len(),
            };
            storage.set_telegram_bot(&bot_state).await.ok();

            for handler in &handlers {
                let state = TelegramConfigState {
                    event_id: handler.event_id.clone(),
                    bot_id: bot_id.clone(),
                    chat_id: handler.chat_id,
                    command: handler.command.clone(),
                };
                storage.set_telegram_config(&state).await.ok();
            }
        }

        let token = config.token.clone();
        let api_client = Arc::clone(&self.api_client);
        let bot_handlers = Arc::clone(&self.bot_handlers);
        let storage = self.storage.clone();

        tokio::spawn(async move {
            if let Err(e) = run_telegram_bot(
                bot_id.clone(),
                token,
                api_client,
                bot_handlers,
                storage.clone(),
                shutdown_rx,
            )
            .await
            {
                error!(bot_id = %bot_id, error = %e, "Telegram bot error");
            }

            if let Some(ref storage) = storage {
                if let Ok(Some(mut state)) = storage.get_telegram_bot(&bot_id).await {
                    state.connected = false;
                    storage.set_telegram_bot(&state).await.ok();
                }
            }
        });

        Ok(())
    }

    #[cfg(not(feature = "telegram"))]
    async fn start_bot(
        &self,
        _config: &BotConfig,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        warn!("Telegram feature not enabled");
        Ok(())
    }

    async fn stop_bot(&self, bot_id: &str) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        info!(bot_id = %bot_id, "Stopping Telegram bot");

        let mut active = self.active_bots.write().await;
        if let Some((_, shutdown_tx)) = active.remove(bot_id) {
            let _ = shutdown_tx.send(true);
        }

        let mut handlers = self.bot_handlers.write().await;
        handlers.remove(bot_id);

        if let Some(ref storage) = self.storage {
            storage.delete_telegram_bot(bot_id).await.ok();
        }

        Ok(())
    }

    pub async fn get_active_bot_count(&self) -> usize {
        self.active_bots.read().await.len()
    }
}

#[derive(Debug, Default)]
pub struct SyncResult {
    pub started: usize,
    pub stopped: usize,
    pub updated: usize,
}

/// Serializable Telegram message data
#[derive(Debug, Clone, serde::Serialize)]
pub struct TelegramMessage {
    pub message_id: i32,
    pub chat_id: i64,
    pub chat_type: String,
    pub chat_title: Option<String>,
    pub from_id: Option<i64>,
    pub from_username: Option<String>,
    pub from_first_name: Option<String>,
    pub from_is_bot: bool,
    pub text: Option<String>,
    pub date: i64,
}

#[cfg(feature = "telegram")]
impl From<&Message> for TelegramMessage {
    fn from(msg: &Message) -> Self {
        Self {
            message_id: msg.id.0,
            chat_id: msg.chat.id.0,
            chat_type: format!("{:?}", msg.chat.kind),
            chat_title: msg.chat.title().map(String::from),
            from_id: msg.from.as_ref().map(|u| u.id.0 as i64),
            from_username: msg.from.as_ref().and_then(|u| u.username.clone()),
            from_first_name: msg.from.as_ref().map(|u| u.first_name.clone()),
            from_is_bot: msg.from.as_ref().is_some_and(|u| u.is_bot),
            text: msg.text().map(String::from),
            date: msg.date.timestamp(),
        }
    }
}

#[cfg(feature = "telegram")]
async fn run_telegram_bot(
    bot_id: String,
    token: String,
    api_client: Arc<ApiClient>,
    handlers: Arc<RwLock<HashMap<String, Vec<TelegramEventHandler>>>>,
    storage: Option<Arc<RedisStorage>>,
    mut shutdown_rx: tokio::sync::watch::Receiver<bool>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let bot = Bot::new(token);

    info!(bot_id = %bot_id, "Telegram bot starting long polling");

    // Mark as connected
    if let Some(ref storage) = storage {
        if let Ok(Some(mut state)) = storage.get_telegram_bot(&bot_id).await {
            state.connected = true;
            state.last_seen = Some(chrono::Utc::now().timestamp());
            storage.set_telegram_bot(&state).await.ok();
        }
    }

    let bot_id_clone = bot_id.clone();
    let api_client_clone = api_client.clone();
    let handlers_clone = handlers.clone();

    let handler = Update::filter_message().endpoint(move |_bot: Bot, msg: Message| {
        let bot_id = bot_id_clone.clone();
        let api_client = api_client_clone.clone();
        let handlers = handlers_clone.clone();

        async move {
            handle_telegram_message(&bot_id, &api_client, &handlers, &msg).await;
            respond(())
        }
    });

    let mut dispatcher = Dispatcher::builder(bot, handler).build();

    // Get shutdown token before dispatching
    let shutdown_token = dispatcher.shutdown_token();

    // Run dispatcher with shutdown check
    tokio::select! {
        _ = dispatcher.dispatch() => {}
        _ = shutdown_rx.changed() => {
            info!(bot_id = %bot_id, "Received shutdown signal");
            shutdown_token.shutdown().ok();
        }
    }

    Ok(())
}

#[cfg(feature = "telegram")]
async fn handle_telegram_message(
    bot_id: &str,
    api_client: &ApiClient,
    handlers: &Arc<RwLock<HashMap<String, Vec<TelegramEventHandler>>>>,
    msg: &Message,
) {
    let Some(text) = msg.text() else {
        return;
    };

    if msg.from.as_ref().is_some_and(|u| u.is_bot) {
        return;
    }

    let chat_id = msg.chat.id.0;

    let handlers_guard = handlers.read().await;
    let Some(bot_handlers) = handlers_guard.get(bot_id) else {
        return;
    };

    for handler in bot_handlers {
        // Check chat filter
        if let Some(required_chat) = handler.chat_id {
            if chat_id != required_chat {
                continue;
            }
        }

        // Check command filter
        if let Some(ref cmd) = handler.command {
            if !text.starts_with(cmd) {
                continue;
            }
        }

        let telegram_msg = TelegramMessage::from(msg);
        let payload = serde_json::json!({
            "source": "telegram",
            "bot_id": bot_id,
            "event_id": handler.event_id,
            "message": telegram_msg,
        });

        match api_client
            .trigger_sink(&handler.event_id, "telegram", payload)
            .await
        {
            Ok(_) => {
                debug!(event_id = %handler.event_id, "Telegram event triggered");
            }
            Err(e) => {
                error!(event_id = %handler.event_id, error = %e, "Failed to trigger event");
            }
        }
    }
}

/// Start the Telegram bot manager
#[cfg(feature = "telegram")]
pub async fn start_telegram_bot(
    api_client: Arc<ApiClient>,
    storage: Option<Arc<RedisStorage>>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let manager = Arc::new(TelegramBotManager::new(api_client, storage));

    info!("Starting Telegram bot manager");

    match manager.sync_bots().await {
        Ok(result) => {
            info!(
                started = result.started,
                stopped = result.stopped,
                updated = result.updated,
                "Initial Telegram bot sync complete"
            );
        }
        Err(e) => {
            warn!("Initial Telegram bot sync failed: {}", e);
        }
    }

    loop {
        tokio::time::sleep(std::time::Duration::from_secs(60)).await;

        match manager.sync_bots().await {
            Ok(result) => {
                if result.started > 0 || result.stopped > 0 {
                    let active = manager.get_active_bot_count().await;
                    info!(
                        started = result.started,
                        stopped = result.stopped,
                        updated = result.updated,
                        active = active,
                        "Telegram bot sync"
                    );
                }
            }
            Err(e) => {
                warn!("Telegram bot sync failed: {}", e);
            }
        }
    }
}

#[cfg(not(feature = "telegram"))]
pub async fn start_telegram_bot(
    _api_client: Arc<ApiClient>,
    _storage: Option<Arc<RedisStorage>>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    warn!("Telegram bot feature not enabled - running as stub");
    loop {
        tokio::time::sleep(std::time::Duration::from_secs(60)).await;
    }
}
