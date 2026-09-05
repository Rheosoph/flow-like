use anyhow::Result;
use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};

use super::manager::DbConnection;
use super::{EventRegistration, EventSink};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeeplinkSink {
    #[serde(alias = "path")]
    pub route: String,
}

impl DeeplinkSink {
    fn init_tables(db: &DbConnection) -> Result<()> {
        let conn = db.lock().unwrap();

        conn.execute(
            "CREATE TABLE IF NOT EXISTS deeplink_routes (
                event_id TEXT PRIMARY KEY,
                app_id TEXT NOT NULL,
                route TEXT NOT NULL,
                created_at INTEGER NOT NULL,
                UNIQUE(app_id, route)
            )",
            [],
        )?;

        Self::migrate_legacy_path_column(&conn)?;

        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_deeplink_app_route ON deeplink_routes(app_id, route)",
            [],
        )?;

        Ok(())
    }

    fn migrate_legacy_path_column(conn: &Connection) -> Result<()> {
        let mut stmt = conn.prepare("PRAGMA table_info(deeplink_routes)")?;
        let column_names = stmt
            .query_map([], |row| row.get::<_, String>(1))?
            .collect::<Result<Vec<_>, _>>()?;

        let has_route = column_names.iter().any(|name| name == "route");
        let has_path = column_names.iter().any(|name| name == "path");

        if !has_route && has_path {
            tracing::info!("Migrating legacy deeplink_routes path column to route column");
            conn.execute(
                "ALTER TABLE deeplink_routes RENAME COLUMN path TO route",
                [],
            )?;
            conn.execute("DROP INDEX IF EXISTS idx_deeplink_app_path", [])?;
        }

        Ok(())
    }

    fn add_route(
        db: &DbConnection,
        registration: &EventRegistration,
        config: &DeeplinkSink,
    ) -> Result<()> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_secs() as i64;

        let conn = db.lock().unwrap();

        let existing = conn
            .query_row(
                "SELECT event_id FROM deeplink_routes WHERE app_id = ? AND route = ?",
                params![&registration.app_id, &config.route],
                |row| row.get::<_, String>(0),
            )
            .ok();

        if let Some(existing_event_id) = existing {
            if existing_event_id != registration.event_id {
                anyhow::bail!(
                    "Deeplink route '{}' for app '{}' is already registered to event '{}'",
                    config.route,
                    registration.app_id,
                    existing_event_id
                );
            } else {
                tracing::info!(
                    "Deeplink route already registered for event '{}', updating timestamp",
                    registration.event_id
                );
            }
        }

        conn.execute(
            "INSERT OR REPLACE INTO deeplink_routes (event_id, app_id, route, created_at)
             VALUES (?, ?, ?, ?)",
            params![
                &registration.event_id,
                &registration.app_id,
                &config.route,
                now
            ],
        )?;

        Ok(())
    }

    fn remove_route(db: &DbConnection, event_id: &str) -> Result<()> {
        let conn = db.lock().unwrap();
        conn.execute(
            "DELETE FROM deeplink_routes WHERE event_id = ?",
            params![event_id],
        )?;
        Ok(())
    }

    pub fn handle_trigger(
        app_handle: &AppHandle,
        app_id: &str,
        route: &str,
        query_params: serde_json::Value,
    ) -> Result<bool> {
        use crate::state::TauriEventSinkManagerState;

        let manager_state = app_handle
            .try_state::<TauriEventSinkManagerState>()
            .ok_or_else(|| anyhow::anyhow!("EventSinkManager state not available"))?;

        let manager = manager_state.0.clone();
        let db = match manager.try_lock() {
            Ok(manager_guard) => manager_guard.db(),
            Err(_) => {
                tracing::warn!(
                    "EventSinkManager busy while resolving deeplink route for app '{}'",
                    app_id
                );
                return Ok(false);
            }
        };

        let conn = db.lock().unwrap();

        let event_id: String = conn
            .query_row(
                "SELECT event_id FROM deeplink_routes WHERE app_id = ? AND route = ?",
                params![app_id, route],
                |row| row.get(0),
            )
            .map_err(|_| {
                anyhow::anyhow!(
                    "No deeplink route found for app_id='{}' route='{}'",
                    app_id,
                    route
                )
            })?;

        drop(conn);

        let parameter_count = query_params.as_object().map_or(0, serde_json::Map::len);
        tracing::info!(
            event_id = %event_id,
            app_id = %app_id,
            route = %route,
            parameter_count,
            "Triggering deeplink event"
        );

        let payload = match query_params {
            serde_json::Value::Null => Some(serde_json::Value::Object(serde_json::Map::new())),
            serde_json::Value::Object(obj) => Some(serde_json::Value::Object(obj)),
            other => Some(other),
        };

        let manager_guard = match manager.try_lock() {
            Ok(manager_guard) => manager_guard,
            Err(_) => {
                tracing::warn!(
                    "EventSinkManager busy while firing deeplink event '{}'",
                    event_id
                );
                return Ok(false);
            }
        };

        manager_guard.fire_event(app_handle, &event_id, payload, None)
    }
}

#[async_trait::async_trait]
impl EventSink for DeeplinkSink {
    async fn start(&self, _app_handle: &AppHandle, db: DbConnection) -> Result<()> {
        Self::init_tables(&db)?;
        tracing::info!("🔗 Deeplink event sink initialized");
        Ok(())
    }

    async fn stop(&self, _app_handle: &AppHandle, _db: DbConnection) -> Result<()> {
        tracing::info!("🔗 Deeplink event sink stopped");
        Ok(())
    }

    async fn on_register(
        &self,
        _app_handle: &AppHandle,
        registration: &EventRegistration,
        db: DbConnection,
    ) -> Result<()> {
        tracing::info!(
            "Registering deeplink route for event '{}': route '{}'",
            registration.event_id,
            self.route
        );

        Self::add_route(&db, registration, self)?;

        tracing::info!(
            "✅ Deeplink route registered: flow-like://trigger/{}/{}",
            registration.app_id,
            self.route
        );

        Ok(())
    }

    async fn on_unregister(
        &self,
        _app_handle: &AppHandle,
        registration: &EventRegistration,
        db: DbConnection,
    ) -> Result<()> {
        tracing::info!(
            "Unregistering deeplink route for event '{}'",
            registration.event_id
        );

        Self::remove_route(&db, &registration.event_id)?;
        Ok(())
    }
}
