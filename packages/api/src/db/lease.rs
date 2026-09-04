//! Committed lease rows standing in for a cross-replica mutex.
//!
//! A `MutationLock` row is claimed by a short retried transaction that only
//! succeeds while nobody holds it or the previous holder's lease has expired.
//! That is the same on PostgreSQL, CockroachDB and Aurora DSQL, where a write
//! intent never blocks and a transaction cannot stay open across storage work.
//! The holder keeps the row alive with a heartbeat and hands it back on
//! release; expiry covers a holder that crashed.

use crate::db::AsDbConflict;
use crate::error::ApiError;
use crate::state::State;
use flow_like_types::tokio::{self, sync::OwnedMutexGuard, task::JoinHandle};
use sea_orm::{ConnectionTrait, DatabaseConnection, DbErr, Statement};
use std::sync::Arc;
use std::time::{Duration, Instant};

pub const LEASE_TTL: Duration = Duration::from_secs(30);
pub const LEASE_HEARTBEAT: Duration = Duration::from_secs(10);
pub const LEASE_WAIT_BUDGET: Duration = Duration::from_secs(15);
const LEASE_RETRY_MIN_MS: u64 = 50;
const LEASE_RETRY_MAX_MS: u64 = 200;

pub(crate) const ENSURE_LOCK_ROW_SQL: &str =
    r#"INSERT INTO "MutationLock" ("id") VALUES ($1) ON CONFLICT ("id") DO NOTHING"#;
pub(crate) const CLAIM_LEASE_SQL: &str = r#"UPDATE "MutationLock" SET "owner" = $2, "expiresAt" = now() + interval '30 seconds', "updatedAt" = now() WHERE "id" = $1 AND ("owner" IS NULL OR "expiresAt" IS NULL OR "expiresAt" < now() OR "owner" = $2)"#;
pub(crate) const EXTEND_LEASE_SQL: &str = r#"UPDATE "MutationLock" SET "expiresAt" = now() + interval '30 seconds', "updatedAt" = now() WHERE "id" = $1 AND "owner" = $2"#;
pub(crate) const RELEASE_LEASE_SQL: &str = r#"UPDATE "MutationLock" SET "owner" = NULL, "expiresAt" = NULL, "updatedAt" = now() WHERE "id" = $1 AND "owner" = $2"#;
pub(crate) const TOUCH_LOCK_ROW_SQL: &str =
    r#"UPDATE "MutationLock" SET "updatedAt" = now() WHERE "id" = $1"#;

fn statement<C: ConnectionTrait>(
    connection: &C,
    sql: &str,
    values: impl IntoIterator<Item = sea_orm::Value>,
) -> Statement {
    Statement::from_sql_and_values(connection.get_database_backend(), sql, values)
}

/// Create the lock row if it does not exist yet. Run inside a retried
/// transaction: on an OCC engine two first-time inserts of the same id fail
/// at commit instead of turning into a no-op.
pub(crate) async fn ensure_lock_row<C: ConnectionTrait>(
    connection: &C,
    lock_id: i64,
) -> Result<(), DbErr> {
    connection
        .execute_raw(statement(connection, ENSURE_LOCK_ROW_SQL, [lock_id.into()]))
        .await?;
    Ok(())
}

/// Write the lock row inside a short transaction so concurrent transactions
/// on the same key serialize: the write blocks on PostgreSQL and makes the
/// later committer lose (and retry) on CockroachDB and DSQL. Only for bodies
/// that do nothing but database work; a lease is the tool for anything longer.
pub(crate) async fn touch_lock_row<C: ConnectionTrait>(
    connection: &C,
    lock_id: i64,
) -> Result<(), DbErr> {
    ensure_lock_row(connection, lock_id).await?;
    let result = connection
        .execute_raw(statement(connection, TOUCH_LOCK_ROW_SQL, [lock_id.into()]))
        .await?;
    if result.rows_affected() != 1 {
        return Err(DbErr::RecordNotFound(format!(
            "mutation lock row {lock_id} disappeared before it was written"
        )));
    }
    Ok(())
}

pub(crate) fn new_owner_id() -> String {
    format!("{}:{}", std::process::id(), uuid::Uuid::new_v4())
}

fn retry_delay() -> Duration {
    Duration::from_millis(rand::random_range(LEASE_RETRY_MIN_MS..=LEASE_RETRY_MAX_MS))
}

fn lease_busy(lock_id: i64) -> ApiError {
    ApiError::conflict(format!(
        "BOARD_LOCKED: another writer holds mutation lock {lock_id}; retry shortly"
    ))
}

async fn try_claim(state: &State, lock_id: i64, owner: &str) -> Result<bool, ApiError> {
    let owner = owner.to_owned();
    state
        .transaction(move |txn| {
            let owner = owner.clone();
            Box::pin(async move {
                ensure_lock_row(txn, lock_id).await?;
                let result = txn
                    .execute_raw(statement(
                        txn,
                        CLAIM_LEASE_SQL,
                        [lock_id.into(), owner.into()],
                    ))
                    .await?;
                Ok::<bool, ApiError>(result.rows_affected() == 1)
            })
        })
        .await
}

async fn claim_with_wait(state: &State, lock_id: i64, owner: &str) -> Result<(), ApiError> {
    let deadline = Instant::now() + LEASE_WAIT_BUDGET;
    loop {
        match try_claim(state, lock_id, owner).await {
            Ok(true) => return Ok(()),
            Ok(false) => {}
            Err(err) if err.db_conflict().is_some() => {
                tracing::debug!(lock_id, %err, "lease claim lost a commit race; waiting");
            }
            Err(err) => return Err(err),
        }
        if Instant::now() >= deadline {
            return Err(lease_busy(lock_id));
        }
        tokio::time::sleep(retry_delay()).await;
    }
}

async fn release_rows(db: &DatabaseConnection, owner: &str, lock_ids: &[i64]) {
    for &lock_id in lock_ids {
        let result = db
            .execute_raw(statement(
                db,
                RELEASE_LEASE_SQL,
                [lock_id.into(), owner.to_owned().into()],
            ))
            .await;
        match result {
            Ok(result) if result.rows_affected() == 1 => {}
            Ok(_) => tracing::warn!(
                lock_id,
                owner,
                "mutation lease was no longer ours at release; it expired or was reclaimed"
            ),
            Err(err) => tracing::warn!(
                lock_id,
                owner,
                %err,
                "mutation lease release failed; the lease expires on its own"
            ),
        }
    }
}

struct LeaseShared {
    db: DatabaseConnection,
    owner: String,
    lock_ids: parking_lot::Mutex<Vec<i64>>,
}

impl LeaseShared {
    fn take_lock_ids(&self) -> Vec<i64> {
        std::mem::take(&mut *self.lock_ids.lock())
    }
}

async fn heartbeat(shared: Arc<LeaseShared>) {
    loop {
        tokio::time::sleep(LEASE_HEARTBEAT).await;
        let lock_ids = shared.lock_ids.lock().clone();
        for lock_id in lock_ids {
            let result = shared
                .db
                .execute_raw(statement(
                    &shared.db,
                    EXTEND_LEASE_SQL,
                    [lock_id.into(), shared.owner.clone().into()],
                ))
                .await;
            match result {
                Ok(result) if result.rows_affected() == 1 => {}
                Ok(_) => tracing::warn!(
                    lock_id,
                    owner = %shared.owner,
                    "mutation lease expired while still held; another writer may have taken it"
                ),
                Err(err) => tracing::debug!(
                    lock_id,
                    owner = %shared.owner,
                    %err,
                    "mutation lease heartbeat failed; retrying on the next tick"
                ),
            }
        }
    }
}

/// One or more claimed `MutationLock` rows plus the process-local mutexes that
/// pair with them.
///
/// Dropping the lease hands the rows back in a detached task; [`Self::release`]
/// waits for that, so a same-process successor does not spin on the row while
/// the release is still in flight. The local mutexes outlive the row release
/// in both cases.
pub(crate) struct MutationLease {
    shared: Arc<LeaseShared>,
    locals: Vec<OwnedMutexGuard<()>>,
    heartbeat: Option<JoinHandle<()>>,
}

impl MutationLease {
    /// Wait up to [`LEASE_WAIT_BUDGET`] for `lock_id`; `local` is the already
    /// held process-local mutex for the same key.
    pub(crate) async fn claim(
        state: &State,
        lock_id: i64,
        local: OwnedMutexGuard<()>,
    ) -> Result<Self, ApiError> {
        let owner = new_owner_id();
        claim_with_wait(state, lock_id, &owner).await?;
        let shared = Arc::new(LeaseShared {
            db: state.db.clone(),
            owner,
            lock_ids: parking_lot::Mutex::new(vec![lock_id]),
        });
        let heartbeat = tokio::spawn(heartbeat(shared.clone()));
        Ok(Self {
            shared,
            locals: vec![local],
            heartbeat: Some(heartbeat),
        })
    }

    /// Claim another row under the same owner and heartbeat.
    pub(crate) async fn claim_additional(
        &mut self,
        state: &State,
        lock_id: i64,
        local: OwnedMutexGuard<()>,
    ) -> Result<(), ApiError> {
        claim_with_wait(state, lock_id, &self.shared.owner).await?;
        self.shared.lock_ids.lock().push(lock_id);
        self.locals.push(local);
        Ok(())
    }

    pub(crate) fn owner(&self) -> &str {
        &self.shared.owner
    }

    pub(crate) async fn release(mut self) {
        self.stop_heartbeat();
        let lock_ids = self.shared.take_lock_ids();
        release_rows(&self.shared.db, &self.shared.owner, &lock_ids).await;
    }

    fn stop_heartbeat(&mut self) {
        if let Some(heartbeat) = self.heartbeat.take() {
            heartbeat.abort();
        }
    }
}

impl Drop for MutationLease {
    fn drop(&mut self) {
        self.stop_heartbeat();
        let lock_ids = self.shared.take_lock_ids();
        if lock_ids.is_empty() {
            return;
        }
        let shared = self.shared.clone();
        let locals = std::mem::take(&mut self.locals);
        match tokio::runtime::Handle::try_current() {
            Ok(handle) => {
                handle.spawn(async move {
                    release_rows(&shared.db, &shared.owner, &lock_ids).await;
                    drop(locals);
                });
            }
            Err(_) => tracing::warn!(
                owner = %shared.owner,
                ?lock_ids,
                "mutation lease dropped outside a runtime; it expires on its own"
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lease_sql_is_portable_row_writes() {
        for sql in [
            ENSURE_LOCK_ROW_SQL,
            CLAIM_LEASE_SQL,
            EXTEND_LEASE_SQL,
            RELEASE_LEASE_SQL,
            TOUCH_LOCK_ROW_SQL,
        ] {
            assert!(!sql.contains("pg_advisory"), "{sql}");
            assert!(!sql.contains("FOR UPDATE"), "{sql}");
        }
        assert!(ENSURE_LOCK_ROW_SQL.contains("ON CONFLICT"));
        assert!(CLAIM_LEASE_SQL.starts_with("UPDATE"));
        assert!(TOUCH_LOCK_ROW_SQL.starts_with("UPDATE"));
    }

    #[test]
    fn lease_ttl_matches_the_interval_literal() {
        let literal = format!("interval '{} seconds'", LEASE_TTL.as_secs());
        assert!(CLAIM_LEASE_SQL.contains(&literal));
        assert!(EXTEND_LEASE_SQL.contains(&literal));
        assert!(LEASE_HEARTBEAT * 2 < LEASE_TTL);
        assert!(LEASE_WAIT_BUDGET < LEASE_TTL);
    }

    #[test]
    fn claim_only_takes_free_expired_or_own_rows() {
        assert!(CLAIM_LEASE_SQL.contains(r#""owner" IS NULL"#));
        assert!(CLAIM_LEASE_SQL.contains(r#""expiresAt" < now()"#));
        assert!(CLAIM_LEASE_SQL.contains(r#""owner" = $2"#));
        assert!(EXTEND_LEASE_SQL.ends_with(r#""owner" = $2"#));
        assert!(RELEASE_LEASE_SQL.ends_with(r#""owner" = $2"#));
    }

    #[test]
    fn owner_ids_are_unique_per_claim() {
        let a = new_owner_id();
        let b = new_owner_id();
        assert_ne!(a, b);
        let pid = std::process::id().to_string();
        assert!(a.starts_with(&format!("{pid}:")));
    }

    #[test]
    fn retry_delay_is_jittered_within_bounds() {
        for _ in 0..64 {
            let delay = retry_delay().as_millis() as u64;
            assert!((LEASE_RETRY_MIN_MS..=LEASE_RETRY_MAX_MS).contains(&delay));
        }
    }
}
