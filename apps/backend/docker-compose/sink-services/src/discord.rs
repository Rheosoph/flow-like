//! Discord multi-bot implementation for Flow-Like sink services
//!
//! Supports running multiple Discord bots concurrently, each with their own
//! token and event handlers. Bots are synced from the API and managed dynamically.

#![cfg_attr(not(feature = "discord"), allow(dead_code))]

use crate::api_client::{ApiClient, BotConfig, BotHandler};
use crate::storage::{DiscordConfigState, RedisStorage};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{info, warn};

#[cfg(feature = "discord")]
use crate::storage::DiscordBotState;
#[cfg(feature = "discord")]
use tracing::{debug, error};

#[cfg(feature = "discord")]
use serenity::{
    all::{GatewayIntents, Message, Ready},
    async_trait,
    prelude::*,
};

/// Event handler configuration for a single Discord bot
#[derive(Debug, Clone)]
pub struct DiscordEventHandler {
    pub event_id: String,
    pub guild_id: Option<u64>,
    pub channel_id: Option<u64>,
    pub command_prefix: Option<String>,
    pub respond_to_mentions: bool,
    pub respond_to_dms: bool,
}

/// Live Discord bot connections keyed by bot id: `(token_hash, shutdown_sender)`.
type ActiveDiscordBots = Arc<RwLock<HashMap<String, (String, tokio::sync::oneshot::Sender<()>)>>>;

/// Manages multiple Discord bot instances
pub struct DiscordBotManager {
    api_client: Arc<ApiClient>,
    storage: Option<Arc<RedisStorage>>,
    /// Active bot connections: bot_id -> (token_hash, shutdown_sender)
    active_bots: ActiveDiscordBots,
    /// Event handlers per bot: bot_id -> handlers
    bot_handlers: Arc<RwLock<HashMap<String, Vec<DiscordEventHandler>>>>,
}

impl DiscordBotManager {
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
        let bot_configs = self.api_client.get_bot_configs("discord").await?;

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
                    // Same token, just update handlers
                    drop(active);
                    self.update_handlers(&config).await;
                    updated += 1;
                }
                Some(_) => {
                    // Token changed, restart bot
                    drop(active);
                    self.stop_bot(&config.bot_id).await.ok();
                    if self.start_bot(&config).await.is_ok() {
                        started += 1;
                    }
                }
                None => {
                    // New bot
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
        let handlers: Vec<DiscordEventHandler> = config
            .handlers
            .iter()
            .filter_map(Self::parse_handler)
            .collect();

        let mut bot_handlers = self.bot_handlers.write().await;
        bot_handlers.insert(config.bot_id.clone(), handlers.clone());

        // Update storage
        if let Some(ref storage) = self.storage {
            for handler in &handlers {
                let state = DiscordConfigState {
                    event_id: handler.event_id.clone(),
                    bot_id: config.bot_id.clone(),
                    guild_id: handler.guild_id,
                    channel_id: handler.channel_id,
                    command_prefix: handler.command_prefix.clone(),
                    respond_to_mentions: handler.respond_to_mentions,
                    respond_to_dms: handler.respond_to_dms,
                };
                if let Err(e) = storage.set_discord_config(&state).await {
                    warn!("Failed to save Discord config: {}", e);
                }
            }
        }
    }

    fn parse_handler(handler: &BotHandler) -> Option<DiscordEventHandler> {
        Some(DiscordEventHandler {
            event_id: handler.event_id.clone(),
            guild_id: handler.config.get("guild_id").and_then(|v| v.as_u64()),
            channel_id: handler.config.get("channel_id").and_then(|v| v.as_u64()),
            command_prefix: handler
                .config
                .get("command_prefix")
                .and_then(|v| v.as_str())
                .map(String::from),
            respond_to_mentions: handler
                .config
                .get("respond_to_mentions")
                .and_then(|v| v.as_bool())
                .unwrap_or(true),
            respond_to_dms: handler
                .config
                .get("respond_to_dms")
                .and_then(|v| v.as_bool())
                .unwrap_or(true),
        })
    }

    #[cfg(feature = "discord")]
    async fn start_bot(
        &self,
        config: &BotConfig,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let token_hash = RedisStorage::hash_token(&config.token);
        let bot_id = config.bot_id.clone();

        info!(bot_id = %bot_id, "Starting Discord bot");

        // Parse handlers
        let handlers: Vec<DiscordEventHandler> = config
            .handlers
            .iter()
            .filter_map(|h| Self::parse_handler(h))
            .collect();

        // Store handlers
        {
            let mut bot_handlers = self.bot_handlers.write().await;
            bot_handlers.insert(bot_id.clone(), handlers.clone());
        }

        // Create shutdown channel
        let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel();

        // Store active bot
        {
            let mut active = self.active_bots.write().await;
            active.insert(bot_id.clone(), (token_hash.clone(), shutdown_tx));
        }

        // Update storage
        if let Some(ref storage) = self.storage {
            let bot_state = DiscordBotState {
                bot_id: bot_id.clone(),
                token_hash: token_hash.clone(),
                connected: false,
                last_seen: None,
                handler_count: handlers.len(),
            };
            storage.set_discord_bot(&bot_state).await.ok();

            for handler in &handlers {
                let state = DiscordConfigState {
                    event_id: handler.event_id.clone(),
                    bot_id: bot_id.clone(),
                    guild_id: handler.guild_id,
                    channel_id: handler.channel_id,
                    command_prefix: handler.command_prefix.clone(),
                    respond_to_mentions: handler.respond_to_mentions,
                    respond_to_dms: handler.respond_to_dms,
                };
                storage.set_discord_config(&state).await.ok();
            }
        }

        // Spawn bot task
        let token = config.token.clone();
        let api_client = Arc::clone(&self.api_client);
        let bot_handlers = Arc::clone(&self.bot_handlers);
        let storage = self.storage.clone();

        tokio::spawn(async move {
            if let Err(e) = run_discord_bot(
                bot_id.clone(),
                token,
                api_client,
                bot_handlers,
                storage.clone(),
                shutdown_rx,
            )
            .await
            {
                error!(bot_id = %bot_id, error = %e, "Discord bot error");
            }

            // Mark as disconnected
            if let Some(ref storage) = storage {
                if let Ok(Some(mut state)) = storage.get_discord_bot(&bot_id).await {
                    state.connected = false;
                    storage.set_discord_bot(&state).await.ok();
                }
            }
        });

        Ok(())
    }

    #[cfg(not(feature = "discord"))]
    async fn start_bot(
        &self,
        _config: &BotConfig,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        warn!("Discord feature not enabled");
        Ok(())
    }

    async fn stop_bot(&self, bot_id: &str) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        info!(bot_id = %bot_id, "Stopping Discord bot");

        let mut active = self.active_bots.write().await;
        if let Some((_, shutdown_tx)) = active.remove(bot_id) {
            let _ = shutdown_tx.send(());
        }

        let mut handlers = self.bot_handlers.write().await;
        handlers.remove(bot_id);

        if let Some(ref storage) = self.storage {
            storage.delete_discord_bot(bot_id).await.ok();
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

/// Serializable Discord message data
#[derive(Debug, Clone, serde::Serialize)]
pub struct DiscordMessage {
    pub id: String,
    pub channel_id: String,
    pub guild_id: Option<String>,
    pub author_id: String,
    pub author_name: String,
    pub author_bot: bool,
    pub content: String,
    pub timestamp: String,
}

#[cfg(feature = "discord")]
impl From<&Message> for DiscordMessage {
    fn from(msg: &Message) -> Self {
        Self {
            id: msg.id.to_string(),
            channel_id: msg.channel_id.to_string(),
            guild_id: msg.guild_id.map(|g| g.to_string()),
            author_id: msg.author.id.to_string(),
            author_name: msg.author.name.clone(),
            author_bot: msg.author.bot,
            content: msg.content.clone(),
            timestamp: msg.timestamp.to_string(),
        }
    }
}

#[cfg(feature = "discord")]
struct BotEventHandler {
    bot_id: String,
    api_client: Arc<ApiClient>,
    handlers: Arc<RwLock<HashMap<String, Vec<DiscordEventHandler>>>>,
    storage: Option<Arc<RedisStorage>>,
}

#[cfg(feature = "discord")]
#[async_trait]
impl EventHandler for BotEventHandler {
    async fn message(&self, _ctx: Context, msg: Message) {
        if msg.author.bot {
            return;
        }

        let handlers = self.handlers.read().await;
        let Some(bot_handlers) = handlers.get(&self.bot_id) else {
            return;
        };

        let guild_id = msg.guild_id.map(|g| g.get());
        let channel_id = msg.channel_id.get();
        let is_dm = guild_id.is_none();

        for handler in bot_handlers {
            if is_dm && !handler.respond_to_dms {
                continue;
            }

            if let Some(required_guild) = handler.guild_id {
                if guild_id != Some(required_guild) {
                    continue;
                }
            }

            if let Some(required_channel) = handler.channel_id {
                if channel_id != required_channel {
                    continue;
                }
            }

            if let Some(ref prefix) = handler.command_prefix {
                if !msg.content.starts_with(prefix) {
                    continue;
                }
            }

            let discord_msg = DiscordMessage::from(&msg);
            let payload = serde_json::json!({
                "source": "discord",
                "bot_id": self.bot_id,
                "event_id": handler.event_id,
                "message": discord_msg,
            });

            match self
                .api_client
                .trigger_sink(&handler.event_id, "discord", payload)
                .await
            {
                Ok(_) => {
                    debug!(event_id = %handler.event_id, "Discord event triggered");
                }
                Err(e) => {
                    error!(event_id = %handler.event_id, error = %e, "Failed to trigger event");
                }
            }
        }
    }

    async fn ready(&self, _ctx: Context, ready: Ready) {
        info!(
            bot_id = %self.bot_id,
            user = %ready.user.name,
            guilds = ready.guilds.len(),
            "Discord bot connected"
        );

        if let Some(ref storage) = self.storage {
            if let Ok(Some(mut state)) = storage.get_discord_bot(&self.bot_id).await {
                state.connected = true;
                state.last_seen = Some(chrono::Utc::now().timestamp());
                storage.set_discord_bot(&state).await.ok();
            }
        }
    }
}

#[cfg(feature = "discord")]
async fn run_discord_bot(
    bot_id: String,
    token: String,
    api_client: Arc<ApiClient>,
    handlers: Arc<RwLock<HashMap<String, Vec<DiscordEventHandler>>>>,
    storage: Option<Arc<RedisStorage>>,
    mut shutdown_rx: tokio::sync::oneshot::Receiver<()>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let intents = GatewayIntents::GUILD_MESSAGES
        | GatewayIntents::DIRECT_MESSAGES
        | GatewayIntents::MESSAGE_CONTENT;

    let handler = BotEventHandler {
        bot_id: bot_id.clone(),
        api_client,
        handlers,
        storage,
    };

    let mut client = Client::builder(&token, intents)
        .event_handler(handler)
        .await?;

    tokio::select! {
        result = client.start() => {
            if let Err(e) = result {
                error!(bot_id = %bot_id, error = %e, "Discord client error");
            }
        }
        _ = &mut shutdown_rx => {
            info!(bot_id = %bot_id, "Received shutdown signal");
            client.shard_manager.shutdown_all().await;
        }
    }

    Ok(())
}

/// Start the Discord bot manager
#[cfg(feature = "discord")]
pub async fn start_discord_bot(
    api_client: Arc<ApiClient>,
    storage: Option<Arc<RedisStorage>>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let manager = Arc::new(DiscordBotManager::new(api_client, storage));

    info!("Starting Discord bot manager");

    match manager.sync_bots().await {
        Ok(result) => {
            info!(
                started = result.started,
                stopped = result.stopped,
                updated = result.updated,
                "Initial Discord bot sync complete"
            );
        }
        Err(e) => {
            warn!("Initial Discord bot sync failed: {}", e);
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
                        "Discord bot sync"
                    );
                }
            }
            Err(e) => {
                warn!("Discord bot sync failed: {}", e);
            }
        }
    }
}

#[cfg(not(feature = "discord"))]
pub async fn start_discord_bot(
    _api_client: Arc<ApiClient>,
    _storage: Option<Arc<RedisStorage>>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    warn!("Discord bot feature not enabled - running as stub");
    loop {
        tokio::time::sleep(std::time::Duration::from_secs(60)).await;
    }
}
