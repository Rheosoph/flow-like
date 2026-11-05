use anyhow::Result;
use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};
use tauri::AppHandle;

use super::manager::DbConnection;
use super::{EventRegistration, EventSink};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmailSink {
    pub imap_server: String,
    pub imap_port: u16,
    pub username: String,
    pub folder: Option<String>,
    pub use_tls: bool,
    pub last_seen_uid: Option<u32>,
}

fn create_watchers_table(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute(
        "CREATE TABLE IF NOT EXISTS email_watchers (
            event_id TEXT PRIMARY KEY,
            imap_server TEXT NOT NULL,
            imap_port INTEGER NOT NULL,
            username TEXT NOT NULL,
            password TEXT NOT NULL,
            folder TEXT NOT NULL,
            use_tls INTEGER NOT NULL,
            last_seen_uid INTEGER,
            last_checked INTEGER,
            enabled INTEGER NOT NULL DEFAULT 1,
            created_at INTEGER NOT NULL
        )",
        [],
    )?;
    Ok(())
}

/// Removes the IMAP passwords that earlier versions saved: from the watcher rows and from the
/// stored registration configs.
pub(super) fn forget_stored_passwords(conn: &Connection) -> Result<()> {
    create_watchers_table(conn)?;
    conn.execute(
        "UPDATE email_watchers SET password = '' WHERE password <> ''",
        [],
    )?;

    let stored: Vec<(String, String)> = conn
        .prepare(
            "SELECT event_id, config FROM event_registrations
             WHERE type IN ('email', 'mail') AND config LIKE '%\"password\"%'",
        )?
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
        .collect::<rusqlite::Result<_>>()?;
    for (event_id, config) in stored {
        let Ok(mut config) = serde_json::from_str::<serde_json::Value>(&config) else {
            continue;
        };
        let Some(object) = config.as_object_mut() else {
            continue;
        };
        if object.remove("password").is_some() {
            conn.execute(
                "UPDATE event_registrations SET config = ?2 WHERE event_id = ?1",
                params![event_id, serde_json::to_string(&config)?],
            )?;
        }
    }
    Ok(())
}

impl EmailSink {
    fn init_tables(db: &DbConnection) -> Result<()> {
        forget_stored_passwords(&db.lock().unwrap())
    }

    /// The `password` column stays empty: no worker reads an IMAP password yet, so none is kept.
    /// Keep it in the system keychain when one does.
    fn add_watcher(db: &DbConnection, event_id: &str, config: &EmailSink) -> Result<()> {
        let conn = db.lock().unwrap();

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_secs() as i64;

        conn.execute(
            "INSERT OR REPLACE INTO email_watchers
             (event_id, imap_server, imap_port, username, password, folder, use_tls, last_seen_uid, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                event_id,
                config.imap_server,
                config.imap_port,
                config.username,
                "",
                config.folder.as_ref().unwrap_or(&"INBOX".to_string()),
                config.use_tls as i32,
                config.last_seen_uid,
                now,
            ],
        )?;

        Ok(())
    }

    fn remove_watcher(db: &DbConnection, event_id: &str) -> Result<()> {
        let conn = db.lock().unwrap();
        conn.execute(
            "DELETE FROM email_watchers WHERE event_id = ?1",
            params![event_id],
        )?;
        Ok(())
    }
}

#[async_trait::async_trait]
impl EventSink for EmailSink {
    async fn start(&self, _app_handle: &AppHandle, db: DbConnection) -> Result<()> {
        Self::init_tables(&db)?;

        // TODO: Start email polling worker
        // Worker polls email_watchers table and:
        // 1. Connects to IMAP servers
        // 2. Checks for new messages based on last_seen_uid
        // 3. Fires events for new emails
        // 4. Updates last_seen_uid and last_checked

        tracing::info!("Email sink started - polling worker ready");
        Ok(())
    }

    async fn stop(&self, _app_handle: &AppHandle, _db: DbConnection) -> Result<()> {
        // TODO: Stop email polling worker
        tracing::info!("Email sink stopped");
        Ok(())
    }

    async fn on_register(
        &self,
        _app_handle: &AppHandle,
        registration: &EventRegistration,
        db: DbConnection,
    ) -> Result<()> {
        Self::add_watcher(&db, &registration.event_id, self)?;
        tracing::info!(
            "Registered email watcher: {}@{} -> event {}",
            self.username,
            self.imap_server,
            registration.event_id
        );
        Ok(())
    }

    async fn on_unregister(
        &self,
        _app_handle: &AppHandle,
        registration: &EventRegistration,
        db: DbConnection,
    ) -> Result<()> {
        Self::remove_watcher(&db, &registration.event_id)?;
        tracing::info!("Unregistered email watcher: {}", registration.event_id);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    fn database() -> DbConnection {
        let conn = Connection::open_in_memory().expect("a database");
        conn.execute(
            "CREATE TABLE event_registrations (
                event_id TEXT PRIMARY KEY,
                type TEXT NOT NULL,
                config TEXT NOT NULL
            )",
            [],
        )
        .expect("the registrations table");
        Arc::new(Mutex::new(conn))
    }

    fn config(imap_server: &str) -> EmailSink {
        EmailSink {
            imap_server: imap_server.into(),
            imap_port: 993,
            username: "inbox@example.test".into(),
            folder: None,
            use_tls: true,
            last_seen_uid: None,
        }
    }

    fn stored_passwords(db: &DbConnection) -> Vec<String> {
        let conn = db.lock().unwrap();
        let mut statement = conn
            .prepare("SELECT password FROM email_watchers ORDER BY event_id")
            .unwrap();
        statement
            .query_map([], |row| row.get(0))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap()
    }

    #[test]
    fn a_new_watcher_keeps_no_password_in_any_column() {
        let db = database();
        EmailSink::init_tables(&db).unwrap();
        EmailSink::add_watcher(&db, "evt_new", &config("imap.example.test")).unwrap();

        assert_eq!(stored_passwords(&db), [""]);
    }

    #[test]
    fn a_password_in_a_pasted_event_config_is_dropped_not_stored() {
        let parsed: EmailSink = serde_json::from_value(serde_json::json!({
            "imap_server": "imap.example.test",
            "imap_port": 993,
            "username": "inbox@example.test",
            "password": "hunter2-plain",
            "use_tls": true,
        }))
        .unwrap();

        assert!(!serde_json::to_string(&parsed).unwrap().contains("hunter2"));
    }

    #[test]
    fn passwords_saved_by_earlier_versions_are_erased_on_first_read() {
        let db = database();
        {
            let conn = db.lock().unwrap();
            create_watchers_table(&conn).unwrap();
            conn.execute(
                "INSERT INTO email_watchers
                 (event_id, imap_server, imap_port, username, password, folder, use_tls, created_at)
                 VALUES ('evt_old', 'imap.example.test', 993, 'inbox@example.test', 'hunter2-plain', 'INBOX', 1, 0)",
                [],
            )
            .unwrap();
            let legacy = serde_json::json!({
                "sink_type": "email",
                "imap_server": "imap.example.test",
                "imap_port": 993,
                "username": "inbox@example.test",
                "password": "hunter2-plain",
                "use_tls": true,
            });
            conn.execute(
                "INSERT INTO event_registrations (event_id, type, config) VALUES ('evt_old', 'email', ?1)",
                params![legacy.to_string()],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO event_registrations (event_id, type, config) VALUES ('evt_rss', 'rss', ?1)",
                params![r#"{"sink_type":"rss","password":"not-an-email-row"}"#],
            )
            .unwrap();
        }

        EmailSink::init_tables(&db).unwrap();
        EmailSink::init_tables(&db).unwrap();

        assert_eq!(stored_passwords(&db), [""]);
        let conn = db.lock().unwrap();
        let configs: Vec<(String, String)> = conn
            .prepare("SELECT event_id, config FROM event_registrations ORDER BY event_id")
            .unwrap()
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        assert!(!configs[0].1.contains("hunter2"));
        assert!(configs[0].1.contains("imap.example.test"));
        assert_eq!(configs[1].0, "evt_rss");
        assert!(configs[1].1.contains("not-an-email-row"));
    }
}
