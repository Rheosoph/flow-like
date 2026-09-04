use anyhow::{Context, Result};
use flow_like::flow::event::EventExecutionMode;
use flow_like::flow::oauth::OAuthToken;
use flow_like_types::intercom::BufferedInterComHandler;
use rusqlite::{Connection, params};
use serde_json;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Manager};

use crate::event_sink::cron::CronSchedule;

use super::cron::CronSink;
use super::{EventConfig, EventRegistration, EventSink};

pub type DbConnection = Arc<Mutex<Connection>>;

/// Internal storage for event registrations
/// Handles database persistence of event sink configurations
struct RegistrationStorage {
    conn: DbConnection,
}

impl RegistrationStorage {
    fn new(db_path: PathBuf) -> Result<Self> {
        let conn = Connection::open(db_path).context("Failed to open database")?;
        let storage = Self {
            conn: Arc::new(Mutex::new(conn)),
        };
        storage.init_schema()?;
        Ok(storage)
    }

    fn ensure_registration_column(conn: &Connection, column: &str, definition: &str) -> Result<()> {
        let has_column: bool = conn
            .prepare(
                "SELECT COUNT(*) FROM pragma_table_info('event_registrations') WHERE name = ?1",
            )?
            .query_row([column], |row| row.get::<_, i32>(0))
            .map(|count| count > 0)
            .unwrap_or(false);

        if !has_column {
            conn.execute(
                &format!("ALTER TABLE event_registrations ADD COLUMN {definition}"),
                [],
            )?;
        }

        Ok(())
    }

    fn connection(&self) -> DbConnection {
        Arc::clone(&self.conn)
    }

    /// Parse config JSON with backwards compatibility
    /// Injects sink_type if missing, based on the registration type
    fn parse_config_json(
        config_json: &str,
        reg_type: &str,
    ) -> Result<EventConfig, serde_json::Error> {
        // First try to parse directly (new format with sink_type)
        if let Ok(config) = serde_json::from_str::<EventConfig>(config_json) {
            return Ok(config);
        }

        // Backwards compatibility: inject sink_type based on registration type
        let mut config_value: serde_json::Value = serde_json::from_str(config_json)?;
        if let Some(obj) = config_value.as_object_mut() {
            // Map registration type to sink_type
            let sink_type = match reg_type {
                "api" | "http" => "http",
                "mail" | "email" => "email",
                _ => reg_type,
            };
            obj.insert(
                "sink_type".to_string(),
                serde_json::Value::String(sink_type.to_string()),
            );
        }

        serde_json::from_value(config_value)
    }

    fn init_schema(&self) -> Result<()> {
        let conn = self.conn.lock().unwrap();

        // Use event_id as the primary key since each event can only be attached to one sink
        conn.execute(
            "CREATE TABLE IF NOT EXISTS event_registrations (
                event_id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                type TEXT NOT NULL,
                updated_at INTEGER NOT NULL,
                created_at INTEGER NOT NULL,
                config TEXT NOT NULL,
                offline INTEGER NOT NULL,
                app_id TEXT NOT NULL,
                default_payload TEXT,
                personal_access_token TEXT,
                oauth_tokens TEXT
            )",
            [],
        )?;

        Self::ensure_registration_column(&conn, "default_payload", "default_payload TEXT")?;
        Self::ensure_registration_column(
            &conn,
            "personal_access_token",
            "personal_access_token TEXT",
        )?;
        Self::ensure_registration_column(&conn, "oauth_tokens", "oauth_tokens TEXT")?;

        Ok(())
    }

    fn save_registration(&self, registration: &EventRegistration) -> Result<()> {
        let config_json = serde_json::to_string(&registration.config)?;
        let default_payload_json = registration
            .default_payload
            .as_ref()
            .map(serde_json::to_string)
            .transpose()?;
        let oauth_tokens_json = if registration.oauth_tokens.is_empty() {
            None
        } else {
            Some(serde_json::to_string(&registration.oauth_tokens)?)
        };

        let updated_at = registration
            .updated_at
            .duration_since(std::time::UNIX_EPOCH)?
            .as_secs() as i64;
        let created_at = registration
            .created_at
            .duration_since(std::time::UNIX_EPOCH)?
            .as_secs() as i64;

        tracing::info!(
            "Saving registration for event {} with PAT present: {}",
            registration.event_id,
            registration.personal_access_token.is_some()
        );

        // Acquire lock in limited scope to minimize lock duration
        {
            let conn = self.conn.lock().unwrap();

            conn.execute(
                "INSERT OR REPLACE INTO event_registrations
                 (event_id, name, type, updated_at, created_at, config, offline, app_id, default_payload, personal_access_token, oauth_tokens)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
                params![
                    registration.event_id,
                    registration.name,
                    registration.r#type,
                    updated_at,
                    created_at,
                    config_json,
                    registration.offline as i32,
                    registration.app_id,
                    default_payload_json,
                    registration.personal_access_token,
                    oauth_tokens_json,
                ],
            )?;
        }

        tracing::info!(
            "Successfully saved registration for event {}",
            registration.event_id
        );

        Ok(())
    }

    fn get_registration(&self, event_id: &str) -> Result<Option<EventRegistration>> {
        let conn = self.conn.lock().unwrap();

        let mut stmt = conn.prepare(
            "SELECT event_id, name, type, updated_at, created_at, config, offline, app_id, default_payload, personal_access_token, oauth_tokens
             FROM event_registrations WHERE event_id = ?1"
        )?;

        let result = stmt.query_row(params![event_id], |row| {
            let config_json: String = row.get(5)?;
            let reg_type: String = row.get(2)?;
            let config: EventConfig =
                Self::parse_config_json(&config_json, &reg_type).map_err(|e| {
                    rusqlite::Error::FromSqlConversionFailure(
                        5,
                        rusqlite::types::Type::Text,
                        Box::new(e),
                    )
                })?;

            let default_payload_json: Option<String> = row.get(8)?;
            let default_payload = default_payload_json
                .map(|json| serde_json::from_str(&json))
                .transpose()
                .map_err(|e| {
                    rusqlite::Error::FromSqlConversionFailure(
                        8,
                        rusqlite::types::Type::Text,
                        Box::new(e),
                    )
                })?;

            let oauth_tokens_json: Option<String> = row.get(10)?;
            let oauth_tokens: HashMap<String, OAuthToken> = oauth_tokens_json
                .map(|json| serde_json::from_str(&json))
                .transpose()
                .map_err(|e| {
                    rusqlite::Error::FromSqlConversionFailure(
                        10,
                        rusqlite::types::Type::Text,
                        Box::new(e),
                    )
                })?
                .unwrap_or_default();

            let updated_at_secs: i64 = row.get(3)?;
            let created_at_secs: i64 = row.get(4)?;

            Ok(EventRegistration {
                event_id: row.get(0)?,
                name: row.get(1)?,
                r#type: row.get(2)?,
                updated_at: std::time::UNIX_EPOCH
                    + std::time::Duration::from_secs(updated_at_secs as u64),
                created_at: std::time::UNIX_EPOCH
                    + std::time::Duration::from_secs(created_at_secs as u64),
                config,
                offline: row.get::<_, i32>(6)? != 0,
                app_id: row.get(7)?,
                default_payload,
                personal_access_token: row.get(9)?,
                oauth_tokens,
            })
        });

        match result {
            Ok(reg) => Ok(Some(reg)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    fn list_registrations(&self) -> Result<Vec<EventRegistration>> {
        let conn = self.conn.lock().unwrap();

        let mut stmt = conn.prepare(
            "SELECT event_id, name, type, updated_at, created_at, config, offline, app_id, default_payload, personal_access_token, oauth_tokens
             FROM event_registrations ORDER BY created_at DESC"
        )?;

        let registrations = stmt
            .query_map([], |row| {
                let config_json: String = row.get(5)?;
                let reg_type: String = row.get(2)?;
                let config: EventConfig = Self::parse_config_json(&config_json, &reg_type)
                    .map_err(|e| {
                        rusqlite::Error::FromSqlConversionFailure(
                            5,
                            rusqlite::types::Type::Text,
                            Box::new(e),
                        )
                    })?;

                let default_payload_json: Option<String> = row.get(8)?;
                let default_payload = default_payload_json
                    .map(|json| serde_json::from_str(&json))
                    .transpose()
                    .map_err(|e| {
                        rusqlite::Error::FromSqlConversionFailure(
                            8,
                            rusqlite::types::Type::Text,
                            Box::new(e),
                        )
                    })?;

                let oauth_tokens_json: Option<String> = row.get(10)?;
                let oauth_tokens: HashMap<String, OAuthToken> = oauth_tokens_json
                    .map(|json| serde_json::from_str(&json))
                    .transpose()
                    .map_err(|e| {
                        rusqlite::Error::FromSqlConversionFailure(
                            10,
                            rusqlite::types::Type::Text,
                            Box::new(e),
                        )
                    })?
                    .unwrap_or_default();

                let updated_at_secs: i64 = row.get(3)?;
                let created_at_secs: i64 = row.get(4)?;

                Ok(EventRegistration {
                    event_id: row.get(0)?,
                    name: row.get(1)?,
                    r#type: row.get(2)?,
                    updated_at: std::time::UNIX_EPOCH
                        + std::time::Duration::from_secs(updated_at_secs as u64),
                    created_at: std::time::UNIX_EPOCH
                        + std::time::Duration::from_secs(created_at_secs as u64),
                    config,
                    offline: row.get::<_, i32>(6)? != 0,
                    app_id: row.get(7)?,
                    default_payload,
                    personal_access_token: row.get(9)?,
                    oauth_tokens,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;

        Ok(registrations)
    }

    fn delete_registration(&self, event_id: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "DELETE FROM event_registrations WHERE event_id = ?1",
            params![event_id],
        )?;
        Ok(())
    }
}

/// Manager for all event sinks
/// Initializes database and coordinates sink lifecycle
pub struct EventSinkManager {
    db: DbConnection,
    storage: Arc<RegistrationStorage>,
    started_sinks: Arc<flow_like_types::tokio::sync::Mutex<HashSet<String>>>,
}

impl Clone for EventSinkManager {
    fn clone(&self) -> Self {
        Self {
            db: self.db.clone(),
            storage: Arc::clone(&self.storage),
            started_sinks: Arc::clone(&self.started_sinks),
        }
    }
}

impl EventSinkManager {
    /// Create a new event sink manager
    pub fn new(db_path: &str) -> Result<Self> {
        let storage = Arc::new(RegistrationStorage::new(PathBuf::from(db_path))?);
        let db = storage.connection();

        Ok(Self {
            db,
            storage,
            started_sinks: Arc::new(flow_like_types::tokio::sync::Mutex::new(HashSet::new())),
        })
    }

    /// Check if a sink type has been started, and mark it as started if not
    async fn ensure_sink_started(
        &self,
        sink_type: &str,
        app_handle: &AppHandle,
        sink: &dyn EventSink,
    ) -> Result<()> {
        let mut started_guard = self.started_sinks.lock().await;
        if started_guard.contains(sink_type) {
            println!(
                "[SINK_MANAGER] Sink {} already started or starting, skipping",
                sink_type
            );
            return Ok(());
        }

        println!("🚀 [SINK_MANAGER] Starting {} sink", sink_type);
        started_guard.insert(sink_type.to_string());
        drop(started_guard);

        if let Err(err) = sink.start(app_handle, self.db.clone()).await {
            let mut started_guard = self.started_sinks.lock().await;
            started_guard.remove(sink_type);
            println!(
                "❌ [SINK_MANAGER] Failed to start {} sink: {}",
                sink_type, err
            );
            return Err(err);
        }

        println!("✅ [SINK_MANAGER] {} sink ready", sink_type);
        Ok(())
    }

    /// Get database connection
    pub fn db(&self) -> DbConnection {
        self.db.clone()
    }

    /// Fire an event by retrieving its configuration and pushing it to the event bus
    /// This is a centralized method that handles offline status, personal_access_token, oauth_tokens, etc.
    pub fn fire_event(
        &self,
        app_handle: &AppHandle,
        event_id: &str,
        payload: Option<flow_like_types::Value>,
        callback: Option<Arc<BufferedInterComHandler>>,
    ) -> Result<bool> {
        tracing::info!("🔥 [FIRE_EVENT] Starting fire_event for: {}", event_id);

        let conn = self.db.lock().unwrap();

        let mut stmt = conn.prepare(
            "SELECT app_id, offline, personal_access_token, oauth_tokens FROM event_registrations WHERE event_id = ?1",
        )?;

        let query_result = stmt.query_row(params![event_id], |row| {
            let oauth_tokens_json: Option<String> = row.get(3)?;
            let oauth_tokens: HashMap<String, OAuthToken> = oauth_tokens_json
                .map(|json| serde_json::from_str(&json))
                .transpose()
                .map_err(|e| {
                    rusqlite::Error::FromSqlConversionFailure(
                        3,
                        rusqlite::types::Type::Text,
                        Box::new(e),
                    )
                })?
                .unwrap_or_default();

            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, bool>(1)?,
                row.get::<_, Option<String>>(2)?,
                oauth_tokens,
            ))
        });

        let (app_id, offline, personal_access_token, oauth_tokens) = match query_result {
            Ok(result) => result,
            Err(e) => {
                drop(stmt);
                drop(conn);
                return Err(e.into());
            }
        };

        drop(stmt);
        drop(conn);

        // Convert oauth_tokens to Option if empty
        let oauth_tokens_opt = if oauth_tokens.is_empty() {
            None
        } else {
            Some(oauth_tokens)
        };

        if let Some(event_bus_state) = app_handle.try_state::<crate::state::TauriEventBusState>() {
            let event_bus = &event_bus_state.0;

            let push_result = event_bus.push_event_with_token(
                payload,
                app_id.clone(),
                event_id.to_string(),
                offline,
                personal_access_token,
                callback,
                oauth_tokens_opt,
            );

            match push_result {
                Ok(_) => Ok(true),
                Err(e) => {
                    tracing::error!("Failed to push event {}: {:?}", event_id, e);
                    Ok(false)
                }
            }
        } else {
            tracing::error!("EventBus state not available for {}", event_id);
            Ok(false)
        }
    }

    /// Register a new event with its sink configuration
    pub async fn register_event(
        &self,
        app_handle: &AppHandle,
        registration: EventRegistration,
    ) -> Result<()> {
        tracing::info!(
            "Registering event {} with type {}",
            registration.event_id,
            registration.r#type
        );

        // Save registration to database
        self.storage.save_registration(&registration)?;

        // Get the appropriate sink and call on_register
        match &registration.config {
            EventConfig::Cron(sink) => {
                self.ensure_sink_started("cron", app_handle, sink).await?;
                sink.on_register(app_handle, &registration, self.db.clone())
                    .await?;
            }
            EventConfig::Discord(sink) => {
                self.ensure_sink_started("discord", app_handle, sink)
                    .await?;
                sink.on_register(app_handle, &registration, self.db.clone())
                    .await?;
            }
            EventConfig::Email(sink) => {
                self.ensure_sink_started("email", app_handle, sink).await?;
                sink.on_register(app_handle, &registration, self.db.clone())
                    .await?;
            }
            EventConfig::Http(sink) => {
                self.ensure_sink_started("http", app_handle, sink).await?;
                sink.on_register(app_handle, &registration, self.db.clone())
                    .await?;
            }
            EventConfig::Rss(sink) => {
                self.ensure_sink_started("rss", app_handle, sink).await?;
                sink.on_register(app_handle, &registration, self.db.clone())
                    .await?;
            }
            EventConfig::Slack(sink) => {
                self.ensure_sink_started("slack", app_handle, sink).await?;
                sink.on_register(app_handle, &registration, self.db.clone())
                    .await?;
            }
            EventConfig::Telegram(sink) => {
                self.ensure_sink_started("telegram", app_handle, sink)
                    .await?;
                sink.on_register(app_handle, &registration, self.db.clone())
                    .await?;
            }
            EventConfig::WebWatcher(sink) => {
                self.ensure_sink_started("web_watcher", app_handle, sink)
                    .await?;
                sink.on_register(app_handle, &registration, self.db.clone())
                    .await?;
            }
            EventConfig::File(sink) => {
                self.ensure_sink_started("file", app_handle, sink).await?;
                sink.on_register(app_handle, &registration, self.db.clone())
                    .await?;
            }
            EventConfig::Webhook(sink) => {
                self.ensure_sink_started("webhook", app_handle, sink)
                    .await?;
                sink.on_register(app_handle, &registration, self.db.clone())
                    .await?;
            }
            EventConfig::GitHub(sink) => {
                self.ensure_sink_started("github", app_handle, sink).await?;
                sink.on_register(app_handle, &registration, self.db.clone())
                    .await?;
            }
            EventConfig::Mqtt(sink) => {
                self.ensure_sink_started("mqtt", app_handle, sink).await?;
                sink.on_register(app_handle, &registration, self.db.clone())
                    .await?;
            }
            EventConfig::Notion(sink) => {
                self.ensure_sink_started("notion", app_handle, sink).await?;
                sink.on_register(app_handle, &registration, self.db.clone())
                    .await?;
            }
            EventConfig::GeoLocation(sink) => {
                self.ensure_sink_started("geolocation", app_handle, sink)
                    .await?;
                sink.on_register(app_handle, &registration, self.db.clone())
                    .await?;
            }
            EventConfig::Deeplink(sink) => {
                self.ensure_sink_started("deeplink", app_handle, sink)
                    .await?;
                sink.on_register(app_handle, &registration, self.db.clone())
                    .await?;
            }
            EventConfig::Daemon(sink) => {
                self.ensure_sink_started("daemon", app_handle, sink).await?;
                sink.on_register(app_handle, &registration, self.db.clone())
                    .await?;
            }
            EventConfig::Nfc(_sink) => {
                tracing::warn!("NFC sink not yet implemented");
                // TODO: Implement NFCSink
            }
            EventConfig::Shortcut(_sink) => {
                tracing::warn!("Shortcut sink not yet implemented");
                // TODO: Implement ShortcutSink
            }
            EventConfig::Mcp(_sink) => {
                tracing::warn!("MCP sink not yet implemented");
                // TODO: Implement MCPSink
            }
        }

        tracing::info!(
            "Registered event {} ({})",
            registration.event_id,
            registration.r#type
        );
        Ok(())
    }

    /// Unregister an event
    pub async fn unregister_event(&self, app_handle: &AppHandle, event_id: &str) -> Result<()> {
        // Get registration from database
        let registration = self
            .storage
            .get_registration(event_id)?
            .ok_or_else(|| anyhow::anyhow!("Registration not found: {}", event_id))?;

        // Call on_unregister for the sink
        match &registration.config {
            EventConfig::Cron(sink) => {
                sink.on_unregister(app_handle, &registration, self.db.clone())
                    .await?;
            }
            EventConfig::Discord(sink) => {
                sink.on_unregister(app_handle, &registration, self.db.clone())
                    .await?;
            }
            EventConfig::Email(sink) => {
                sink.on_unregister(app_handle, &registration, self.db.clone())
                    .await?;
            }
            EventConfig::Http(sink) => {
                sink.on_unregister(app_handle, &registration, self.db.clone())
                    .await?;
            }
            EventConfig::Rss(sink) => {
                sink.on_unregister(app_handle, &registration, self.db.clone())
                    .await?;
            }
            EventConfig::Slack(sink) => {
                sink.on_unregister(app_handle, &registration, self.db.clone())
                    .await?;
            }
            EventConfig::Telegram(sink) => {
                sink.on_unregister(app_handle, &registration, self.db.clone())
                    .await?;
            }
            EventConfig::WebWatcher(sink) => {
                sink.on_unregister(app_handle, &registration, self.db.clone())
                    .await?;
            }
            EventConfig::File(sink) => {
                sink.on_unregister(app_handle, &registration, self.db.clone())
                    .await?;
            }
            EventConfig::Webhook(sink) => {
                sink.on_unregister(app_handle, &registration, self.db.clone())
                    .await?;
            }
            EventConfig::GitHub(sink) => {
                sink.on_unregister(app_handle, &registration, self.db.clone())
                    .await?;
            }
            EventConfig::Mqtt(sink) => {
                sink.on_unregister(app_handle, &registration, self.db.clone())
                    .await?;
            }
            EventConfig::Notion(sink) => {
                sink.on_unregister(app_handle, &registration, self.db.clone())
                    .await?;
            }
            EventConfig::GeoLocation(sink) => {
                sink.on_unregister(app_handle, &registration, self.db.clone())
                    .await?;
            }
            EventConfig::Deeplink(sink) => {
                sink.on_unregister(app_handle, &registration, self.db.clone())
                    .await?;
            }
            EventConfig::Daemon(sink) => {
                sink.on_unregister(app_handle, &registration, self.db.clone())
                    .await?;
            }
            _ => {
                tracing::warn!("Unregister called for unimplemented sink type");
            }
        }

        // Delete registration from database
        self.storage.delete_registration(event_id)?;

        tracing::info!("Unregistered event: {}", event_id);
        Ok(())
    }

    /// Automatically register an event from a flow_like Event struct
    /// This parses the event.config bytes and event_type to determine which sink to use
    pub async fn register_from_flow_event(
        &self,
        app_handle: &AppHandle,
        app_id: &str,
        event: &flow_like::flow::event::Event,
        offline: Option<bool>,
        personal_access_token: Option<String>,
        oauth_tokens: Option<HashMap<String, OAuthToken>>,
    ) -> Result<()> {
        // Check if this event type supports sink registration
        if !Self::supports_sink_registration(&event.event_type) {
            // Clean up if it was previously registered (e.g., type changed)
            if self.storage.get_registration(&event.id)?.is_some() {
                self.unregister_event(app_handle, &event.id).await?;
            }
            return Ok(());
        }

        // Only register active events
        if !event.active {
            // If it was previously registered, unregister it
            if self.storage.get_registration(&event.id)?.is_some() {
                self.unregister_event(app_handle, &event.id).await?;
            }
            return Ok(());
        }

        if event.event_type == "daemon" && event.execution_mode != EventExecutionMode::Local {
            tracing::info!(
                "Event {} is a daemon but not local, skipping local daemon registration",
                event.id
            );
            if self.storage.get_registration(&event.id)?.is_some() {
                self.unregister_event(app_handle, &event.id).await?;
            }
            return Ok(());
        }

        // Determine which PAT to use based on existing registration
        let final_pat = if let Some(existing_reg) = self.storage.get_registration(&event.id)? {
            match (&existing_reg.personal_access_token, &personal_access_token) {
                (Some(existing), None) => Some(existing.clone()),
                (None, Some(new_pat)) => Some(new_pat.clone()),
                (Some(_), Some(new_pat)) => Some(new_pat.clone()),
                (None, None) => None,
            }
        } else {
            // No existing registration, use whatever was provided
            personal_access_token
        };

        // Parse config bytes to determine sink type and configuration
        let config_result = self.parse_event_config(&event.event_type, &event.config);

        println!("Registering event {} ({})", event.id, event.event_type);

        match config_result {
            Ok(event_config) => {
                // If the sink is configured for remote-only execution, skip local registration
                if event_config.is_remote_only() {
                    tracing::info!(
                        "Event {} configured for remote-only execution, skipping local registration",
                        event.id
                    );
                    // Clean up any existing local registration
                    if self.storage.get_registration(&event.id)?.is_some() {
                        self.unregister_event(app_handle, &event.id).await?;
                    }
                    return Ok(());
                }

                // Merge oauth_tokens from existing registration with new tokens
                let final_oauth_tokens =
                    if let Some(existing_reg) = self.storage.get_registration(&event.id)? {
                        let mut merged = existing_reg.oauth_tokens.clone();
                        if let Some(new_tokens) = oauth_tokens {
                            merged.extend(new_tokens);
                        }
                        merged
                    } else {
                        oauth_tokens.unwrap_or_default()
                    };

                let registration = EventRegistration {
                    event_id: event.id.clone(),
                    name: event.name.clone(),
                    r#type: event.event_type.clone(),
                    updated_at: event.updated_at,
                    created_at: event.created_at,
                    config: event_config,
                    offline: offline.unwrap_or(true),
                    app_id: app_id.to_string(),
                    default_payload: None, // TODO: Parse from event if needed
                    personal_access_token: final_pat.clone(),
                    oauth_tokens: final_oauth_tokens,
                };

                self.register_event(app_handle, registration).await?;
            }
            Err(_e) => {
                // If it was previously registered, unregister it
                if self.storage.get_registration(&event.id)?.is_some() {
                    self.unregister_event(app_handle, &event.id).await?;
                }
            }
        }

        Ok(())
    }

    /// Parse event config bytes based on event_type
    fn parse_event_config(&self, event_type: &str, config_bytes: &[u8]) -> Result<EventConfig> {
        // If config is empty, try to create default config based on type
        if config_bytes.is_empty() {
            return Err(anyhow::anyhow!(
                "Empty config for event type: {}",
                event_type
            ));
        }

        // Deserialize the config JSON
        let config_json: serde_json::Value =
            serde_json::from_slice(config_bytes).context("Failed to parse config as JSON")?;

        // Match event_type to sink type and parse appropriate config
        match event_type {
            "cron" => {
                let cron_config: super::cron::CronSink =
                    serde_json::from_value(config_json).context("Failed to parse cron config")?;
                Ok(EventConfig::Cron(cron_config))
            }
            "api" | "http" => {
                let http_config: super::http::HttpSink =
                    serde_json::from_value(config_json).context("Failed to parse HTTP config")?;
                Ok(EventConfig::Http(http_config))
            }
            "mail" | "email" => {
                let email_config: super::email::EmailSink =
                    serde_json::from_value(config_json).context("Failed to parse email config")?;
                Ok(EventConfig::Email(email_config))
            }
            "rss" => {
                let rss_config: super::rss::RSSSink =
                    serde_json::from_value(config_json).context("Failed to parse RSS config")?;
                Ok(EventConfig::Rss(rss_config))
            }
            "discord" => {
                let discord_config: super::discord::DiscordSink =
                    serde_json::from_value(config_json)
                        .context("Failed to parse Discord config")?;
                Ok(EventConfig::Discord(discord_config))
            }
            "telegram" => {
                let telegram_config: super::telegram::TelegramSink =
                    serde_json::from_value(config_json)
                        .context("Failed to parse Telegram config")?;
                Ok(EventConfig::Telegram(telegram_config))
            }
            "slack" => {
                let slack_config: super::slack::SlackSink =
                    serde_json::from_value(config_json).context("Failed to parse Slack config")?;
                Ok(EventConfig::Slack(slack_config))
            }
            "deeplink" => {
                let deeplink_config: super::deeplink::DeeplinkSink =
                    serde_json::from_value(config_json)
                        .context("Failed to parse deeplink config")?;
                Ok(EventConfig::Deeplink(deeplink_config))
            }
            "webhook" => {
                let webhook_config: super::webhook::WebhookSink =
                    serde_json::from_value(config_json)
                        .context("Failed to parse Webhook config")?;
                Ok(EventConfig::Webhook(webhook_config))
            }
            "mqtt" => {
                let mqtt_config: super::mqtt::MQTTSink =
                    serde_json::from_value(config_json).context("Failed to parse MQTT config")?;
                Ok(EventConfig::Mqtt(mqtt_config))
            }
            "github" => {
                let github_config: super::github::GitHubSink =
                    serde_json::from_value(config_json).context("Failed to parse GitHub config")?;
                Ok(EventConfig::GitHub(github_config))
            }
            "file" => {
                let file_config: super::file::FileSink =
                    serde_json::from_value(config_json).context("Failed to parse File config")?;
                Ok(EventConfig::File(file_config))
            }
            "web_watcher" => {
                let web_watcher_config: super::web_watcher::WebWatcherSink =
                    serde_json::from_value(config_json)
                        .context("Failed to parse WebWatcher config")?;
                Ok(EventConfig::WebWatcher(web_watcher_config))
            }
            "notion" => {
                let notion_config: super::notion::NotionSink =
                    serde_json::from_value(config_json).context("Failed to parse Notion config")?;
                Ok(EventConfig::Notion(notion_config))
            }
            "geolocation" => {
                let geolocation_config: super::geolocation::GeoLocationSink =
                    serde_json::from_value(config_json)
                        .context("Failed to parse GeoLocation config")?;
                Ok(EventConfig::GeoLocation(geolocation_config))
            }
            "nfc" => {
                let nfc_config: super::nfc::NFCSink =
                    serde_json::from_value(config_json).context("Failed to parse NFC config")?;
                Ok(EventConfig::Nfc(nfc_config))
            }
            "shortcut" => {
                let shortcut_config: super::shortcut::ShortcutSink =
                    serde_json::from_value(config_json)
                        .context("Failed to parse Shortcut config")?;
                Ok(EventConfig::Shortcut(shortcut_config))
            }
            "mcp" => {
                let mcp_config: super::mcp::MCPSink =
                    serde_json::from_value(config_json).context("Failed to parse MCP config")?;
                Ok(EventConfig::Mcp(mcp_config))
            }
            "daemon" => {
                let daemon_config: super::daemon::DaemonSink =
                    serde_json::from_value(config_json).context("Failed to parse daemon config")?;
                Ok(EventConfig::Daemon(daemon_config))
            }
            // Add more sink types as needed
            _ => Err(anyhow::anyhow!(
                "Unsupported event type for sink registration: {}",
                event_type
            )),
        }
    }

    /// Add or update an event sink registration
    /// If the event already exists with a different type, it will be unregistered first
    pub async fn add_event_sink(
        &self,
        app_handle: &AppHandle,
        registration: EventRegistration,
    ) -> Result<()> {
        let event_id = registration.event_id.clone();

        // Check if event already exists
        if let Some(existing_registration) = self.storage.get_registration(&event_id)? {
            let old_type = existing_registration.r#type.clone();
            let new_type = registration.r#type.clone();

            // If the sink type changed, unregister the old one first
            if old_type != new_type {
                tracing::info!(
                    "Event {} is switching from {} to {}, unregistering old sink",
                    event_id,
                    old_type,
                    new_type
                );

                // Unregister from the old sink
                self.unregister_event(app_handle, &event_id).await?;

                // Now register with the new sink
                tracing::info!(
                    "Registering event {} with new sink type {}",
                    event_id,
                    new_type
                );
                self.register_event(app_handle, registration).await?;
            } else {
                // Same sink type - unregister and re-register to update configuration
                tracing::info!(
                    "Updating event {} configuration for {} sink",
                    event_id,
                    new_type
                );

                // Unregister to clean up old configuration
                self.unregister_event(app_handle, &event_id).await?;

                // Register with new configuration
                self.register_event(app_handle, registration).await?;
            }
        } else {
            // New event - just register it
            tracing::info!(
                "Adding new event {} with sink type {}",
                event_id,
                registration.r#type
            );
            self.register_event(app_handle, registration).await?;
        }

        Ok(())
    }

    /// Remove an event sink
    /// This is an alias for unregister_event with clearer naming for external API
    pub async fn remove_event_sink(&self, app_handle: &AppHandle, event_id: &str) -> Result<()> {
        tracing::info!("Removing event sink for event: {}", event_id);
        self.unregister_event(app_handle, event_id).await
    }

    /// Get all registrations
    pub fn list_registrations(&self) -> Result<Vec<EventRegistration>> {
        self.storage.list_registrations()
    }

    /// Get a specific registration by event_id
    pub fn get_registration(&self, event_id: &str) -> Result<Option<EventRegistration>> {
        self.storage.get_registration(event_id)
    }

    /// Check if a sink is active for an event
    /// Returns true if the event is registered and has an active sink
    pub fn is_event_active(&self, event_id: &str) -> bool {
        let registration = self.storage.get_registration(event_id).ok().flatten();

        if let Some(reg) = registration {
            // An event is considered active if it is registered and not offline
            (!reg.offline && reg.personal_access_token.is_some()) || reg.offline
        } else {
            false
        }
    }

    /// Check if an event type supports sink registration
    /// Some event types (like quick_action, chat) don't need sink infrastructure
    pub fn supports_sink_registration(event_type: &str) -> bool {
        matches!(
            event_type,
            "cron"
                | "api"
                | "http"
                | "mail"
                | "email"
                | "rss"
                | "discord"
                | "webhook"
                | "slack"
                | "telegram"
                | "mqtt"
                | "github"
                | "notion"
                | "web_watcher"
                | "file"
                | "geolocation"
                | "deeplink"
                | "nfc"
                | "shortcut"
                | "mcp"
                | "daemon"
        )
    }

    /// Initialize all sinks on app startup
    /// This loads all registrations from the database and starts their infrastructure
    /// NOTE: We only need to start the sink workers, not re-register events
    /// (the database already has the registrations)
    pub async fn init_from_storage(&self, app_handle: &AppHandle) -> Result<()> {
        let registrations = self.list_registrations()?;

        println!(
            "🔄 [SINK_MANAGER] Loading {} event registrations from database",
            registrations.len()
        );

        for reg in &registrations {
            println!(
                "🔄 [SINK_MANAGER] Found registration: {} (type: {})",
                reg.event_id, reg.r#type
            );
        }

        for registration in &registrations {
            if let EventConfig::Daemon(sink) = &registration.config {
                let result = async {
                    self.ensure_sink_started("daemon", app_handle, sink).await?;
                    sink.on_register(app_handle, registration, self.db.clone())
                        .await
                }
                .await;

                if let Err(err) = result {
                    tracing::error!(
                        event_id = %registration.event_id,
                        error = %err,
                        "Failed to restore daemon event"
                    );
                }
            }
        }

        // Group registrations by sink type to start each sink once
        let mut sink_types = std::collections::HashSet::new();
        for registration in &registrations {
            // Extract sink type from config
            let sink_type = match &registration.config {
                EventConfig::Discord(_) => "discord",
                EventConfig::Email(_) => "email",
                EventConfig::Http(_) => "http",
                EventConfig::Rss(_) => "rss",
                EventConfig::Slack(_) => "slack",
                EventConfig::Telegram(_) => "telegram",
                EventConfig::WebWatcher(_) => "web_watcher",
                EventConfig::File(_) => "file",
                EventConfig::Webhook(_) => "webhook",
                EventConfig::GitHub(_) => "github",
                EventConfig::Mqtt(_) => "mqtt",
                EventConfig::Notion(_) => "notion",
                EventConfig::GeoLocation(_) => "geolocation",
                EventConfig::Cron(_) => "cron",
                _ => continue,
            };

            sink_types.insert(sink_type);
        }

        // Start each unique sink type without blocking the main initialization path
        println!(
            "📋 [SINK_MANAGER] Unique sink types to start: {:?}",
            sink_types
        );

        for sink_type in sink_types {
            let sink_type = sink_type.to_string();
            let manager = self.clone();
            let app_handle = app_handle.clone();

            flow_like_types::tokio::spawn(async move {
                tracing::info!("⚙️ Starting {} sink during initialization", sink_type);

                let start_result = match sink_type.as_str() {
                    "cron" => {
                        let cron_sink = CronSink {
                            schedule: CronSchedule::Expression {
                                expression: "0 0 * * *".to_string(),
                            },
                            last_fired: None,
                            timezone: None,
                            sink_execution: None,
                        };
                        manager
                            .ensure_sink_started("cron", &app_handle, &cron_sink)
                            .await
                    }
                    "http" | "api" => {
                        let http_sink = super::http::HttpSink {
                            path: String::new(),
                            method: String::new(),
                            auth_token: None,
                            sink_execution: None,
                        };
                        manager
                            .ensure_sink_started("http", &app_handle, &http_sink)
                            .await
                    }
                    "discord" => {
                        let discord_sink = super::discord::DiscordSink {
                            token: String::new(),
                            bot_name: None,
                            bot_description: None,
                            intents: None,
                            channel_whitelist: None,
                            channel_blacklist: None,
                            respond_to_mentions: true,
                            respond_to_dms: true,
                            command_prefix: "!".to_string(),
                        };
                        manager
                            .ensure_sink_started("discord", &app_handle, &discord_sink)
                            .await
                    }
                    "telegram" => {
                        let telegram_sink = super::telegram::TelegramSink {
                            bot_token: String::new(),
                            bot_name: None,
                            bot_description: None,
                            chat_whitelist: None,
                            chat_blacklist: None,
                            respond_to_mentions: true,
                            respond_to_private: true,
                            command_prefix: "/".to_string(),
                        };
                        manager
                            .ensure_sink_started("telegram", &app_handle, &telegram_sink)
                            .await
                    }
                    "slack" => {
                        let slack_sink = super::slack::SlackSink {
                            bot_token: String::new(),
                            app_token: None,
                            channel_id: None,
                            team_id: None,
                            last_event_ts: None,
                        };
                        manager
                            .ensure_sink_started("slack", &app_handle, &slack_sink)
                            .await
                    }
                    "email" => {
                        let email_sink = super::email::EmailSink {
                            imap_server: String::new(),
                            imap_port: 993,
                            username: String::new(),
                            password: String::new(),
                            folder: None,
                            use_tls: true,
                            last_seen_uid: None,
                        };
                        manager
                            .ensure_sink_started("email", &app_handle, &email_sink)
                            .await
                    }
                    "rss" => {
                        let rss_sink = super::rss::RSSSink {
                            feed_url: String::new(),
                            poll_interval: 300,
                            headers: None,
                            filter_keywords: None,
                        };
                        manager
                            .ensure_sink_started("rss", &app_handle, &rss_sink)
                            .await
                    }
                    "deeplink" => {
                        let deeplink_sink = super::deeplink::DeeplinkSink {
                            route: String::new(),
                        };
                        manager
                            .ensure_sink_started("deeplink", &app_handle, &deeplink_sink)
                            .await
                    }
                    _ => {
                        tracing::debug!(
                            "Sink type {} will be started on first registration",
                            sink_type
                        );
                        Ok(())
                    }
                };

                if let Err(err) = start_result {
                    tracing::error!("❌ Failed to start {} sink: {}", sink_type, err);
                }
            });
        }

        tracing::info!(
            "✅ Sink initialization complete. {} event registrations ready.",
            registrations.len()
        );
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event_sink::daemon::DaemonRestartPolicy;

    #[test]
    fn supports_daemon_sink_registration() {
        assert!(EventSinkManager::supports_sink_registration("daemon"));
    }

    #[test]
    fn parses_legacy_daemon_registration_config() {
        let config =
            RegistrationStorage::parse_config_json(r#"{"restart_policy":"always"}"#, "daemon")
                .unwrap();

        match config {
            EventConfig::Daemon(sink) => {
                assert_eq!(sink.restart_policy, DaemonRestartPolicy::Always);
            }
            other => panic!("expected daemon config, got {other:?}"),
        }
    }

    #[test]
    fn parses_tagged_daemon_registration_config() {
        let config = RegistrationStorage::parse_config_json(
            r#"{"sink_type":"daemon","restart_policy":"never"}"#,
            "daemon",
        )
        .unwrap();

        match config {
            EventConfig::Daemon(sink) => {
                assert_eq!(sink.restart_policy, DaemonRestartPolicy::Never);
            }
            other => panic!("expected daemon config, got {other:?}"),
        }
    }

    #[test]
    fn parses_flow_daemon_event_config() {
        let db_path = std::env::temp_dir().join(format!(
            "flow-like-daemon-test-{}.sqlite",
            flow_like_types::create_id()
        ));
        let manager = EventSinkManager::new(db_path.to_str().unwrap()).unwrap();

        let config = manager
            .parse_event_config("daemon", br#"{"restart_policy":"always"}"#)
            .unwrap();

        match config {
            EventConfig::Daemon(sink) => {
                assert_eq!(sink.restart_policy, DaemonRestartPolicy::Always);
            }
            other => panic!("expected daemon config, got {other:?}"),
        }

        drop(manager);
        let _ = std::fs::remove_file(db_path);
    }
}
