//! Redis-backed storage for sink service state
//!
//! Provides persistent storage for:
//! - Sink configurations (cached from API)
//! - Last triggered timestamps
//! - Active schedule state

#![cfg_attr(not(any(feature = "discord", feature = "telegram")), allow(dead_code))]

use redis::aio::ConnectionManager;
use redis::AsyncCommands;
use serde::{de::DeserializeOwned, Serialize};

const KEY_PREFIX: &str = "sink:";

#[derive(Clone)]
pub struct RedisStorage {
    conn: ConnectionManager,
}

impl RedisStorage {
    pub async fn new(redis_url: &str) -> Result<Self, RedisStorageError> {
        let client = redis::Client::open(redis_url)
            .map_err(|e| RedisStorageError::Connection(e.to_string()))?;

        let conn = ConnectionManager::new(client)
            .await
            .map_err(|e| RedisStorageError::Connection(e.to_string()))?;

        Ok(Self { conn })
    }

    fn key(parts: &[&str]) -> String {
        format!("{}{}", KEY_PREFIX, parts.join(":"))
    }

    // Generic JSON value storage

    pub async fn set_json<T: Serialize>(
        &self,
        key: &str,
        value: &T,
        ttl_secs: Option<u64>,
    ) -> Result<(), RedisStorageError> {
        let json = serde_json::to_string(value)
            .map_err(|e| RedisStorageError::Serialization(e.to_string()))?;

        let mut conn = self.conn.clone();

        if let Some(ttl) = ttl_secs {
            conn.set_ex::<_, _, ()>(key, &json, ttl)
                .await
                .map_err(|e| RedisStorageError::Redis(e.to_string()))?;
        } else {
            conn.set::<_, _, ()>(key, &json)
                .await
                .map_err(|e| RedisStorageError::Redis(e.to_string()))?;
        }

        Ok(())
    }

    pub async fn get_json<T: DeserializeOwned>(
        &self,
        key: &str,
    ) -> Result<Option<T>, RedisStorageError> {
        let mut conn = self.conn.clone();

        let result: Option<String> = conn
            .get(key)
            .await
            .map_err(|e| RedisStorageError::Redis(e.to_string()))?;

        match result {
            Some(json) => {
                let value = serde_json::from_str(&json)
                    .map_err(|e| RedisStorageError::Serialization(e.to_string()))?;
                Ok(Some(value))
            }
            None => Ok(None),
        }
    }

    pub async fn delete(&self, key: &str) -> Result<(), RedisStorageError> {
        let mut conn = self.conn.clone();
        conn.del::<_, ()>(key)
            .await
            .map_err(|e| RedisStorageError::Redis(e.to_string()))?;
        Ok(())
    }

    // Cron-specific operations

    pub async fn set_cron_schedule(
        &self,
        schedule: &CronScheduleState,
    ) -> Result<(), RedisStorageError> {
        let key = Self::key(&["cron", "schedule", &schedule.event_id]);
        self.set_json(&key, schedule, None).await
    }

    pub async fn get_cron_schedule(
        &self,
        event_id: &str,
    ) -> Result<Option<CronScheduleState>, RedisStorageError> {
        let key = Self::key(&["cron", "schedule", event_id]);
        self.get_json(&key).await
    }

    pub async fn delete_cron_schedule(&self, event_id: &str) -> Result<(), RedisStorageError> {
        let key = Self::key(&["cron", "schedule", event_id]);
        self.delete(&key).await
    }

    pub async fn get_all_cron_schedules(
        &self,
    ) -> Result<Vec<CronScheduleState>, RedisStorageError> {
        let mut conn = self.conn.clone();
        let pattern = Self::key(&["cron", "schedule", "*"]);

        let keys: Vec<String> = conn
            .keys(&pattern)
            .await
            .map_err(|e| RedisStorageError::Redis(e.to_string()))?;

        let mut schedules = Vec::with_capacity(keys.len());
        for key in keys {
            if let Some(schedule) = self.get_json::<CronScheduleState>(&key).await? {
                schedules.push(schedule);
            }
        }

        Ok(schedules)
    }

    pub async fn update_cron_last_triggered(
        &self,
        event_id: &str,
        timestamp: i64,
    ) -> Result<(), RedisStorageError> {
        if let Some(mut schedule) = self.get_cron_schedule(event_id).await? {
            schedule.last_triggered = Some(timestamp);
            self.set_cron_schedule(&schedule).await?;
        }
        Ok(())
    }

    // Discord-specific operations

    pub async fn set_discord_config(
        &self,
        config: &DiscordConfigState,
    ) -> Result<(), RedisStorageError> {
        let key = Self::key(&["discord", "config", &config.event_id]);
        self.set_json(&key, config, None).await
    }

    pub async fn set_telegram_config(
        &self,
        config: &TelegramConfigState,
    ) -> Result<(), RedisStorageError> {
        let key = Self::key(&["telegram", "config", &config.event_id]);
        self.set_json(&key, config, None).await
    }

    // Batch operations for sync

    pub async fn sync_cron_schedules(
        &self,
        schedules: Vec<CronScheduleState>,
    ) -> Result<(), RedisStorageError> {
        let existing = self.get_all_cron_schedules().await?;
        let existing_ids: std::collections::HashSet<_> =
            existing.iter().map(|s| s.event_id.clone()).collect();
        let new_ids: std::collections::HashSet<_> =
            schedules.iter().map(|s| s.event_id.clone()).collect();

        // Add/update schedules
        for schedule in schedules {
            let is_new = !existing_ids.contains(&schedule.event_id);

            // Preserve last_triggered if updating
            let schedule_to_save = if is_new {
                schedule
            } else if let Some(existing) = self.get_cron_schedule(&schedule.event_id).await? {
                CronScheduleState {
                    last_triggered: existing.last_triggered,
                    ..schedule
                }
            } else {
                schedule
            };

            self.set_cron_schedule(&schedule_to_save).await?;
        }

        // Remove orphaned schedules
        for id in existing_ids.difference(&new_ids) {
            self.delete_cron_schedule(id).await?;
        }

        Ok(())
    }

    // Bot state management

    pub async fn set_discord_bot(&self, bot: &DiscordBotState) -> Result<(), RedisStorageError> {
        let key = Self::key(&["discord", "bot", &bot.bot_id]);
        self.set_json(&key, bot, None).await
    }

    pub async fn get_discord_bot(
        &self,
        bot_id: &str,
    ) -> Result<Option<DiscordBotState>, RedisStorageError> {
        let key = Self::key(&["discord", "bot", bot_id]);
        self.get_json(&key).await
    }

    pub async fn delete_discord_bot(&self, bot_id: &str) -> Result<(), RedisStorageError> {
        let key = Self::key(&["discord", "bot", bot_id]);
        self.delete(&key).await
    }

    pub async fn set_telegram_bot(&self, bot: &TelegramBotState) -> Result<(), RedisStorageError> {
        let key = Self::key(&["telegram", "bot", &bot.bot_id]);
        self.set_json(&key, bot, None).await
    }

    pub async fn get_telegram_bot(
        &self,
        bot_id: &str,
    ) -> Result<Option<TelegramBotState>, RedisStorageError> {
        let key = Self::key(&["telegram", "bot", bot_id]);
        self.get_json(&key).await
    }

    pub async fn delete_telegram_bot(&self, bot_id: &str) -> Result<(), RedisStorageError> {
        let key = Self::key(&["telegram", "bot", bot_id]);
        self.delete(&key).await
    }

    pub fn hash_token(token: &str) -> String {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};
        let mut hasher = DefaultHasher::new();
        token.hash(&mut hasher);
        format!("{:x}", hasher.finish())
    }
}

// State structs

#[derive(Debug, Clone, Serialize, serde::Deserialize)]
pub struct CronScheduleState {
    pub event_id: String,
    pub cron_expression: String,
    pub enabled: bool,
    pub last_triggered: Option<i64>,
    pub next_trigger: Option<i64>,
}

#[derive(Debug, Clone, Serialize, serde::Deserialize)]
pub struct DiscordConfigState {
    pub event_id: String,
    pub bot_id: String,
    pub guild_id: Option<u64>,
    pub channel_id: Option<u64>,
    pub command_prefix: Option<String>,
    pub respond_to_mentions: bool,
    pub respond_to_dms: bool,
}

#[derive(Debug, Clone, Serialize, serde::Deserialize)]
pub struct DiscordBotState {
    pub bot_id: String,
    pub token_hash: String, // Store hash, not the token itself
    pub connected: bool,
    pub last_seen: Option<i64>,
    pub handler_count: usize,
}

#[derive(Debug, Clone, Serialize, serde::Deserialize)]
pub struct TelegramConfigState {
    pub event_id: String,
    pub bot_id: String,
    pub chat_id: Option<i64>,
    pub command: Option<String>,
}

#[derive(Debug, Clone, Serialize, serde::Deserialize)]
pub struct TelegramBotState {
    pub bot_id: String,
    pub token_hash: String,
    pub connected: bool,
    pub last_seen: Option<i64>,
    pub handler_count: usize,
}

// Errors

#[derive(Debug)]
pub enum RedisStorageError {
    Connection(String),
    Redis(String),
    Serialization(String),
}

impl std::fmt::Display for RedisStorageError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Connection(e) => write!(f, "Redis connection error: {}", e),
            Self::Redis(e) => write!(f, "Redis error: {}", e),
            Self::Serialization(e) => write!(f, "Serialization error: {}", e),
        }
    }
}

impl std::error::Error for RedisStorageError {}
