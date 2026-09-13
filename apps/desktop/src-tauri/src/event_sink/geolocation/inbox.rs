use super::model::{MAX_PENDING, RETENTION_MS, Transition};
use anyhow::Result;
use rusqlite::{Connection, OptionalExtension, params};

pub fn initialize(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "PRAGMA synchronous=FULL;
        CREATE TABLE IF NOT EXISTS geofence_bindings (
            event_id TEXT PRIMARY KEY, scope TEXT NOT NULL, registration_id TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS geofence_inbox (
            id TEXT PRIMARY KEY, scope TEXT NOT NULL, event_id TEXT NOT NULL,
            registration_id TEXT NOT NULL, payload TEXT NOT NULL, occurred_at INTEGER NOT NULL,
            completed INTEGER NOT NULL DEFAULT 0, attempts INTEGER NOT NULL DEFAULT 0,
            next_attempt INTEGER NOT NULL DEFAULT 0
        );",
    )?;
    Ok(())
}

pub fn bind(conn: &Connection, event_id: &str, scope: &str, registration_id: &str) -> Result<()> {
    let tx = conn.unchecked_transaction()?;
    tx.execute(
        "DELETE FROM geofence_inbox WHERE event_id=?1 AND (scope<>?2 OR registration_id<>?3)",
        params![event_id, scope, registration_id],
    )?;
    tx.execute("INSERT INTO geofence_bindings VALUES (?1,?2,?3) ON CONFLICT(event_id) DO UPDATE SET scope=excluded.scope,registration_id=excluded.registration_id", params![event_id,scope,registration_id])?;
    tx.commit()?;
    Ok(())
}

pub fn binding(conn: &Connection, event_id: &str, scope: &str) -> Result<Option<String>> {
    Ok(conn
        .query_row(
            "SELECT registration_id FROM geofence_bindings WHERE event_id=?1 AND scope=?2",
            params![event_id, scope],
            |r| r.get(0),
        )
        .optional()?)
}

pub fn remove(conn: &Connection, event_id: &str) -> Result<()> {
    let tx = conn.unchecked_transaction()?;
    tx.execute(
        "DELETE FROM geofence_bindings WHERE event_id=?1",
        [event_id],
    )?;
    tx.execute("DELETE FROM geofence_inbox WHERE event_id=?1", [event_id])?;
    tx.commit()?;
    Ok(())
}

pub fn prune(conn: &Connection, scope: Option<&str>, now: i64) -> Result<()> {
    let tx = conn.unchecked_transaction()?;
    match scope {
        Some(scope) => {
            tx.execute("DELETE FROM geofence_bindings WHERE scope<>?1", [scope])?;
            tx.execute(
                "DELETE FROM geofence_inbox WHERE scope<>?1 OR occurred_at<?2",
                params![scope, now - RETENTION_MS],
            )?;
            tx.execute("DELETE FROM geofence_inbox WHERE completed=1 AND id NOT IN (SELECT id FROM geofence_inbox WHERE completed=1 ORDER BY occurred_at DESC LIMIT 512)", [])?;
        }
        None => {
            tx.execute("DELETE FROM geofence_bindings", [])?;
            tx.execute("DELETE FROM geofence_inbox", [])?;
        }
    }
    tx.commit()?;
    Ok(())
}

/// Return true only when the native copy may be acknowledged. A full inbox keeps it native.
pub fn accept(conn: &Connection, transition: &Transition) -> Result<bool> {
    if conn
        .query_row(
            "SELECT 1 FROM geofence_inbox WHERE id=?1",
            [&transition.id],
            |_| Ok(()),
        )
        .optional()?
        .is_some()
    {
        return Ok(true);
    }
    let count: usize = conn.query_row(
        "SELECT COUNT(*) FROM geofence_inbox WHERE completed=0",
        [],
        |r| r.get(0),
    )?;
    if count >= MAX_PENDING {
        return Ok(false);
    }
    conn.execute("INSERT OR IGNORE INTO geofence_inbox (id,scope,event_id,registration_id,payload,occurred_at) VALUES (?1,?2,?3,?4,?5,?6)", params![transition.id,transition.scope,transition.event_id,transition.registration_id,serde_json::to_string(transition)?,transition.occurred_at])?;
    Ok(true)
}

pub fn pending(conn: &Connection, scope: &str, now: i64) -> Result<Vec<Transition>> {
    let mut statement = conn.prepare("SELECT payload FROM geofence_inbox WHERE scope=?1 AND completed=0 AND next_attempt<=?2 ORDER BY occurred_at LIMIT 128")?;
    let values = statement
        .query_map(params![scope, now], |row| row.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    values
        .into_iter()
        .map(|text| Ok(serde_json::from_str(&text)?))
        .collect()
}

pub fn complete(conn: &Connection, id: &str) -> Result<()> {
    conn.execute("UPDATE geofence_inbox SET completed=1 WHERE id=?1", [id])?;
    Ok(())
}

pub fn retry(conn: &Connection, id: &str, now: i64) -> Result<()> {
    // Failed acceptance is retried for at most one hour. The stable transition ID lets Events deduplicate side effects.
    conn.execute("UPDATE geofence_inbox SET attempts=attempts+1,next_attempt=?2+MIN(300000,5000*(1 << MIN(attempts,6))) WHERE id=?1 AND completed=0", params![id,now])?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn db() -> Connection {
        let c = Connection::open_in_memory().unwrap();
        initialize(&c).unwrap();
        c
    }
    fn transition(id: &str) -> Transition {
        Transition {
            id: id.into(),
            registration_id: "registration".into(),
            scope: "scope".into(),
            app_id: "app".into(),
            event_id: "event".into(),
            transition: "enter".into(),
            occurred_at: 10_000_000,
            geometry: serde_json::json!({"type":"Point","coordinates":[1,2]}),
            radius_meters: 200.0,
        }
    }
    #[test]
    fn native_ack_requires_durable_acceptance_and_replays_are_deduplicated() {
        let c = db();
        let event = transition("one");
        assert!(accept(&c, &event).unwrap());
        assert_eq!(pending(&c, "scope", event.occurred_at).unwrap().len(), 1);
        assert!(accept(&c, &event).unwrap());
        complete(&c, "one").unwrap();
        assert!(accept(&c, &event).unwrap());
        assert!(pending(&c, "scope", event.occurred_at).unwrap().is_empty());
    }
    #[test]
    fn acknowledged_native_events_survive_database_reopen() {
        let path = std::env::temp_dir().join(format!(
            "flow-like-geofence-{}-{}.sqlite",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        {
            let c = Connection::open(&path).unwrap();
            initialize(&c).unwrap();
            bind(&c, "event", "scope", "registration").unwrap();
            assert!(accept(&c, &transition("one")).unwrap());
            retry(&c, "one", 10_000_000).unwrap();
        }
        {
            let c = Connection::open(&path).unwrap();
            initialize(&c).unwrap();
            assert_eq!(
                binding(&c, "event", "scope").unwrap().as_deref(),
                Some("registration")
            );
            assert!(pending(&c, "scope", 10_000_000).unwrap().is_empty());
            assert_eq!(pending(&c, "scope", 10_005_000).unwrap()[0].id, "one");
            complete(&c, "one").unwrap();
        }
        {
            let c = Connection::open(&path).unwrap();
            assert!(accept(&c, &transition("one")).unwrap());
            assert!(pending(&c, "scope", 10_005_000).unwrap().is_empty());
        }
        std::fs::remove_file(path).unwrap();
    }
    #[test]
    fn inbox_capacity_preserves_native_copy_and_retry_survives_reload() {
        let c = db();
        for index in 0..MAX_PENDING {
            assert!(accept(&c, &transition(&index.to_string())).unwrap());
        }
        assert!(!accept(&c, &transition("overflow")).unwrap());
        retry(&c, "0", 10_000_000).unwrap();
        assert_eq!(
            pending(&c, "scope", 10_000_000).unwrap().len(),
            MAX_PENDING - 1
        );
        assert_eq!(pending(&c, "scope", 10_005_000).unwrap().len(), MAX_PENDING);
        complete(&c, "0").unwrap();
        assert!(accept(&c, &transition("overflow")).unwrap());
    }
    #[test]
    fn account_config_and_unregister_invalidate_pending_work() {
        let c = db();
        bind(&c, "event", "scope", "registration").unwrap();
        accept(&c, &transition("one")).unwrap();
        bind(&c, "event", "scope", "new").unwrap();
        assert!(pending(&c, "scope", 10_000_000).unwrap().is_empty());
        accept(&c, &transition("two")).unwrap();
        prune(&c, Some("other"), 10_000_000).unwrap();
        assert!(binding(&c, "event", "scope").unwrap().is_none());
        assert!(pending(&c, "scope", 10_000_000).unwrap().is_empty());
        bind(&c, "event", "scope", "registration").unwrap();
        accept(&c, &transition("three")).unwrap();
        remove(&c, "event").unwrap();
        assert!(pending(&c, "scope", 10_000_000).unwrap().is_empty());
        assert!(binding(&c, "event", "scope").unwrap().is_none());
    }
    #[test]
    fn expired_transitions_and_cleared_identity_are_removed() {
        let c = db();
        accept(&c, &transition("one")).unwrap();
        prune(&c, Some("scope"), 10_000_000 + RETENTION_MS + 1).unwrap();
        assert!(pending(&c, "scope", i64::MAX).unwrap().is_empty());
        bind(&c, "event", "scope", "registration").unwrap();
        accept(&c, &transition("two")).unwrap();
        prune(&c, None, 10_000_000).unwrap();
        assert!(binding(&c, "event", "scope").unwrap().is_none());
        assert!(pending(&c, "scope", i64::MAX).unwrap().is_empty());
    }
}
