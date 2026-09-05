//! Durable admission ownership shared by managers on the same local disk.
use std::{
    path::Path,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use tokio::sync::{mpsc, oneshot};

type DbResult<T> = std::result::Result<T, String>;
type Operation = Box<dyn FnOnce(&mut Connection, &str) + Send>;

#[derive(Clone)]
pub struct Registry {
    sender: mpsc::Sender<Operation>,
}

#[derive(Clone, Debug)]
pub struct Record {
    pub name: String,
    pub gateway: String,
    pub volume: String,
}

pub fn now() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
}

impl Registry {
    pub async fn open(path: String, installation: String) -> DbResult<Self> {
        let (sender, mut receiver) = mpsc::channel::<Operation>(256);
        let (ready_tx, ready_rx) = oneshot::channel();
        std::thread::Builder::new().name("execution-registry".into()).spawn(move || {
            let opened = (|| -> DbResult<Connection> {
                if path != ":memory:"
                    && let Some(parent) = Path::new(&path).parent().filter(|p| !p.as_os_str().is_empty()) {
                        use std::os::unix::fs::DirBuilderExt;
                        std::fs::DirBuilder::new().recursive(true).mode(0o700).create(parent).map_err(|_| "Cannot create execution registry directory")?;
                }
                let db = Connection::open(&path).map_err(|_| "Cannot open execution registry")?;
                if path != ":memory:" {
                    use std::os::unix::fs::PermissionsExt;
                    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).map_err(|_| "Cannot protect execution registry")?;
                }
                db.busy_timeout(Duration::from_secs(5)).map_err(|_| "Cannot configure registry busy timeout")?;
                db.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL;
                    CREATE TABLE IF NOT EXISTS managers (installation TEXT NOT NULL, owner TEXT NOT NULL, seen REAL NOT NULL, PRIMARY KEY (installation, owner));
                    CREATE TABLE IF NOT EXISTS slots (installation TEXT NOT NULL, name TEXT NOT NULL, owner TEXT NOT NULL, gateway TEXT NOT NULL, volume TEXT NOT NULL, run_id TEXT, state TEXT NOT NULL, deadline REAL NOT NULL, PRIMARY KEY (installation, name));
                    CREATE INDEX IF NOT EXISTS slots_by_run ON slots (installation, run_id);
                    CREATE TABLE IF NOT EXISTS cancellations (installation TEXT NOT NULL, run_id TEXT NOT NULL, until REAL NOT NULL, PRIMARY KEY (installation, run_id));
                    CREATE TABLE IF NOT EXISTS assignments (installation TEXT NOT NULL, run_id TEXT NOT NULL, until REAL NOT NULL, PRIMARY KEY (installation, run_id));
                    CREATE INDEX IF NOT EXISTS cancellations_by_expiry ON cancellations (installation, until);
                    CREATE INDEX IF NOT EXISTS assignments_by_expiry ON assignments (installation, until);
                    CREATE INDEX IF NOT EXISTS slots_by_deadline ON slots (installation, deadline);
                    CREATE INDEX IF NOT EXISTS slots_by_owner ON slots (installation, owner);
                    CREATE INDEX IF NOT EXISTS managers_by_seen ON managers (installation, seen);")
                    .map_err(|_| "Cannot initialize execution registry")?;
                Ok(db)
            })();
            match opened {
                Ok(mut db) => {
                    let _ = ready_tx.send(Ok(()));
                    while let Some(operation) = receiver.blocking_recv() { operation(&mut db, &installation); }
                }
                Err(error) => { let _ = ready_tx.send(Err(error)); }
            }
        }).map_err(|_| "Cannot start execution registry")?;
        ready_rx.await.map_err(|_| "Execution registry stopped")??;
        Ok(Self { sender })
    }

    async fn call<T: Send + 'static>(
        &self,
        operation: impl FnOnce(&mut Connection, &str) -> rusqlite::Result<T> + Send + 'static,
    ) -> DbResult<T> {
        let (tx, rx) = oneshot::channel();
        self.sender
            .send(Box::new(move |db, installation| {
                let _ = tx.send(
                    operation(db, installation)
                        .map_err(|_| "Execution registry operation failed".to_string()),
                );
            }))
            .await
            .map_err(|_| "Execution registry stopped")?;
        rx.await.map_err(|_| "Execution registry stopped")?
    }

    pub async fn heartbeat(&self, owner: String) -> DbResult<()> {
        self.call(move |db, installation| {
            db.execute("INSERT INTO managers VALUES (?, ?, ?) ON CONFLICT(installation,owner) DO UPDATE SET seen=excluded.seen", params![installation, owner, now()])?;
            Ok(())
        }).await
    }

    pub async fn add(&self, row: Record, owner: String, deadline: f64) -> DbResult<()> {
        self.call(move |db, installation| {
            db.execute(
                "INSERT INTO slots VALUES (?, ?, ?, ?, ?, NULL, 'creating', ?)",
                params![
                    installation,
                    row.name,
                    owner,
                    row.gateway,
                    row.volume,
                    deadline
                ],
            )?;
            Ok(())
        })
        .await
    }

    pub async fn ready(&self, name: String) -> DbResult<()> {
        self.call(move |db, installation| {
            let changed = db.execute("UPDATE slots SET state='ready' WHERE installation=? AND name=? AND state='creating'", params![installation, name])?;
            if changed != 1 { return Err(rusqlite::Error::QueryReturnedNoRows); }
            Ok(())
        }).await
    }

    pub async fn assign(&self, name: String, run_id: String, deadline: f64) -> DbResult<bool> {
        self.call(move |db, installation| {
            let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
            let time = now();
            for table in ["cancellations", "assignments"] {
                let query = format!("SELECT 1 FROM {table} WHERE installation=? AND run_id=? AND until>?");
                if tx.query_row(&query, params![installation, run_id, time], |_| Ok(())).optional()?.is_some() { return Ok(false); }
            }
            let assigned = tx.execute("UPDATE slots SET run_id=?, state='assigned', deadline=? WHERE installation=? AND name=? AND state='ready'", params![run_id, deadline, installation, name])? == 1;
            if assigned {
                tx.execute("INSERT INTO assignments VALUES (?, ?, ?) ON CONFLICT(installation,run_id) DO UPDATE SET until=excluded.until", params![installation, run_id, (deadline + 60.0).max(time + 86460.0)])?;
            }
            tx.commit()?;
            Ok(assigned)
        }).await
    }

    pub async fn cancel(&self, run_id: String, until: f64) -> DbResult<Vec<Record>> {
        self.call(move |db, installation| {
            let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
            tx.execute("INSERT INTO cancellations VALUES (?, ?, ?) ON CONFLICT(installation,run_id) DO UPDATE SET until=MAX(until,excluded.until)", params![installation, run_id, until])?;
            let records = {
                let mut statement = tx.prepare("SELECT name,gateway,volume FROM slots WHERE installation=? AND run_id=?")?;
                statement.query_map(params![installation, run_id], record)?.collect::<rusqlite::Result<Vec<_>>>()?
            };
            tx.commit()?;
            Ok(records)
        }).await
    }

    pub async fn remove(&self, name: String) -> DbResult<()> {
        self.call(move |db, installation| {
            db.execute(
                "DELETE FROM slots WHERE installation=? AND name=?",
                params![installation, name],
            )?;
            Ok(())
        })
        .await
    }

    pub async fn cleanup_failed(&self, name: String) -> DbResult<()> {
        self.call(move |db, installation| {
            db.execute(
                "UPDATE slots SET deadline=0, state='cleanup' WHERE installation=? AND name=?",
                params![installation, name],
            )?;
            Ok(())
        })
        .await
    }

    pub async fn expired(&self, owner: String) -> DbResult<Vec<Record>> {
        // Release the actor between bounded batches so admission and ownership
        // heartbeats can progress even when many 24-hour tombstones expire.
        let until = Instant::now() + Duration::from_millis(50);
        loop {
            let more = self.call(|db, installation| {
                let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
                let mut more = false;
                for table in ["cancellations", "assignments"] {
                    let query = format!("DELETE FROM {table} WHERE rowid IN (SELECT rowid FROM {table} WHERE installation=? AND until<? ORDER BY until LIMIT 4096)");
                    more |= tx.execute(&query, params![installation, now()])? == 4096;
                }
                tx.commit()?;
                Ok(more)
            }).await?;
            if !more || Instant::now() >= until {
                break;
            }
            tokio::task::yield_now().await;
        }
        self.call(move |db, installation| {
            let mut statement = db.prepare("SELECT s.name,s.gateway,s.volume FROM slots s LEFT JOIN managers m ON s.installation=m.installation AND s.owner=m.owner WHERE s.installation=? AND (s.deadline<=? OR (s.owner<>? AND (m.seen IS NULL OR m.seen<?))) ORDER BY s.deadline LIMIT 128")?;
            let time = now();
            statement.query_map(params![installation, time, owner, time - 30.0], record)?.collect()
        }).await
    }
}

fn record(row: &rusqlite::Row<'_>) -> rusqlite::Result<Record> {
    Ok(Record {
        name: row.get(0)?,
        gateway: row.get(1)?,
        volume: row.get(2)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(name: &str) -> Record {
        Record {
            name: name.into(),
            gateway: format!("{name}-gateway"),
            volume: format!("{name}-socket"),
        }
    }

    #[tokio::test]
    async fn concurrent_assignment_and_replay_survive_cleanup_and_reopen() {
        let path = std::env::temp_dir().join(format!("registry-{}.sqlite", uuid::Uuid::new_v4()));
        let path = path.to_str().unwrap().to_owned();
        let a = Registry::open(path.clone(), "test".into()).await.unwrap();
        let b = Registry::open(path.clone(), "test".into()).await.unwrap();
        for (db, name) in [(&a, "one"), (&b, "two")] {
            db.add(row(name), name.into(), now() + 100.0).await.unwrap();
            db.ready(name.into()).await.unwrap();
        }
        let (left, right) = tokio::join!(
            a.assign("one".into(), "run".into(), now() + 100.0),
            b.assign("two".into(), "run".into(), now() + 100.0)
        );
        assert_ne!(left.unwrap(), right.unwrap());
        a.remove("one".into()).await.unwrap();
        b.remove("two".into()).await.unwrap();
        let reopened = Registry::open(path.clone(), "test".into()).await.unwrap();
        reopened
            .add(row("three"), "owner".into(), now() + 100.0)
            .await
            .unwrap();
        reopened.ready("three".into()).await.unwrap();
        assert!(
            !reopened
                .assign("three".into(), "run".into(), now() + 100.0)
                .await
                .unwrap()
        );
        drop((a, b, reopened));
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(format!("{path}-wal"));
        let _ = std::fs::remove_file(format!("{path}-shm"));
    }

    #[tokio::test]
    async fn cancellation_precedes_assignment_and_retains_binding() {
        let db = Registry::open(":memory:".into(), "test".into())
            .await
            .unwrap();
        db.add(row("slot"), "owner".into(), now() + 100.0)
            .await
            .unwrap();
        db.ready("slot".into()).await.unwrap();
        assert!(
            db.cancel("cancelled".into(), now() + 100.0)
                .await
                .unwrap()
                .is_empty()
        );
        assert!(
            !db.assign("slot".into(), "cancelled".into(), now() + 100.0)
                .await
                .unwrap()
        );
        assert!(
            db.assign("slot".into(), "active".into(), now() + 100.0)
                .await
                .unwrap()
        );
        assert_eq!(
            db.cancel("active".into(), now() + 100.0).await.unwrap()[0].name,
            "slot"
        );
    }

    #[tokio::test]
    async fn healthy_other_owner_is_preserved_and_missing_owner_is_reconciled() {
        let db = Registry::open(":memory:".into(), "test".into())
            .await
            .unwrap();
        db.heartbeat("alive".into()).await.unwrap();
        db.add(row("healthy"), "alive".into(), now() + 100.0)
            .await
            .unwrap();
        db.add(row("orphan"), "dead".into(), now() + 100.0)
            .await
            .unwrap();
        let rows = db.expired("self".into()).await.unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].name, "orphan");
    }
}
