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
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

pub const LEASE_TTL: Duration = Duration::from_secs(30);
pub const LEASE_HEARTBEAT: Duration = Duration::from_secs(10);
pub const LEASE_WAIT_BUDGET: Duration = Duration::from_secs(15);
const LEASE_RETRY_BASE_MS: u64 = 50;
const LEASE_RETRY_MAX_MS: u64 = 1_600;
/// Consecutive failed heartbeat ticks after which the lease is treated as lost. Two ticks span
/// [`LEASE_HEARTBEAT`] * 2, so a third failure could only be observed past [`LEASE_TTL`].
const LEASE_MAX_HEARTBEAT_ERRORS: u32 = 2;

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

/// Longest wait before claim attempt `attempt + 1`.
fn retry_delay_ceiling_ms(attempt: u32) -> u64 {
    (LEASE_RETRY_BASE_MS << attempt.min(6)).min(LEASE_RETRY_MAX_MS)
}

/// Half-jitter exponential backoff. A flat 50-200 ms retry lets a handful of waiters spend the
/// whole connection pool on failing claims, which starves the *holder's* heartbeat and expires
/// the very lease they are waiting for.
fn retry_delay(attempt: u32) -> Duration {
    let ceiling = retry_delay_ceiling_ms(attempt);
    Duration::from_millis(rand::random_range(ceiling.div_ceil(2)..=ceiling))
}

fn lease_busy(lock_id: i64) -> ApiError {
    ApiError::locked(
        "BOARD_LOCKED",
        format!("Another writer holds mutation lock {lock_id}; retry shortly."),
    )
}

fn lease_lost(lock_id: i64) -> ApiError {
    ApiError::locked(
        "BOARD_LOCKED",
        format!(
            "The mutation lease for lock {lock_id} expired before this write; nothing was written. Retry shortly."
        ),
    )
}

async fn try_claim(
    state: &State,
    lock_id: i64,
    owner: &str,
    ensure_row: bool,
) -> Result<bool, ApiError> {
    let owner = owner.to_owned();
    state
        .transaction(move |txn| {
            let owner = owner.clone();
            Box::pin(async move {
                if ensure_row {
                    ensure_lock_row(txn, lock_id).await?;
                }
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
    let mut attempt = 0u32;
    loop {
        // The row only has to be inserted once: after the first attempt it either exists or the
        // insert lost a commit race to another waiter that created it.
        match try_claim(state, lock_id, owner, attempt == 0).await {
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
        tokio::time::sleep(retry_delay(attempt)).await;
        attempt = attempt.saturating_add(1);
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
    /// Cleared by the heartbeat the moment it can no longer prove the rows are ours. Writers test
    /// it right before a canonical write, so a lapsed lease fails the request instead of letting
    /// two replicas overwrite each other's full-object PUT.
    held: AtomicBool,
}

impl LeaseShared {
    fn take_lock_ids(&self) -> Vec<i64> {
        std::mem::take(&mut *self.lock_ids.lock())
    }

    fn is_held(&self) -> bool {
        self.held.load(Ordering::Acquire)
    }

    fn mark_lost(&self) {
        self.held.store(false, Ordering::Release);
    }

    fn lost_lock_id(&self) -> i64 {
        self.lock_ids.lock().first().copied().unwrap_or_default()
    }
}

async fn heartbeat(shared: Arc<LeaseShared>) {
    let mut consecutive_errors = 0u32;
    loop {
        tokio::time::sleep(LEASE_HEARTBEAT).await;
        let lock_ids = shared.lock_ids.lock().clone();
        let mut errored = false;
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
                Ok(_) => {
                    tracing::warn!(
                        lock_id,
                        owner = %shared.owner,
                        "mutation lease expired while still held; another writer may have taken it"
                    );
                    shared.mark_lost();
                    return;
                }
                Err(err) => {
                    errored = true;
                    tracing::debug!(
                        lock_id,
                        owner = %shared.owner,
                        %err,
                        "mutation lease heartbeat failed; retrying on the next tick"
                    );
                }
            }
        }
        if !errored {
            consecutive_errors = 0;
            continue;
        }
        consecutive_errors += 1;
        if consecutive_errors >= LEASE_MAX_HEARTBEAT_ERRORS {
            tracing::warn!(
                owner = %shared.owner,
                consecutive_errors,
                "mutation lease heartbeat failed repeatedly; treating the lease as lost"
            );
            shared.mark_lost();
            return;
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
            held: AtomicBool::new(true),
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

    /// `Err` once the heartbeat has proven the rows are no longer ours. Call immediately before
    /// each canonical write; the window between this check and the write is bounded by the
    /// remaining [`LEASE_TTL`], which is what the lease can guarantee at all.
    pub(crate) fn ensure_held(&self) -> Result<(), ApiError> {
        if self.shared.is_held() {
            return Ok(());
        }
        Err(lease_lost(self.shared.lost_lock_id()))
    }

    pub(crate) async fn release(mut self) {
        self.stop_heartbeat();
        let lock_ids = self.shared.take_lock_ids();
        release_rows(&self.shared.db, &self.shared.owner, &lock_ids).await;
    }

    #[cfg(test)]
    fn for_test(shared: Arc<LeaseShared>) -> Self {
        Self {
            shared,
            locals: Vec::new(),
            heartbeat: None,
        }
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
    fn retry_delay_is_jittered_within_its_attempt_bounds() {
        for attempt in 0..16u32 {
            let ceiling = retry_delay_ceiling_ms(attempt);
            for _ in 0..64 {
                let delay = retry_delay(attempt).as_millis() as u64;
                assert!(delay >= ceiling.div_ceil(2), "{attempt}: {delay}");
                assert!(delay <= ceiling, "{attempt}: {delay}");
            }
        }
    }

    /// H6: a flat retry interval lets waiters spend the whole connection pool on failing claims
    /// and starve the holder's heartbeat, expiring the lease they are queueing for.
    #[test]
    fn retry_delay_backs_off_exponentially_and_caps() {
        let mut previous = 0;
        for attempt in 0..5u32 {
            let ceiling = retry_delay_ceiling_ms(attempt);
            assert!(
                ceiling > previous,
                "attempt {attempt} did not back off: {ceiling}"
            );
            previous = ceiling;
        }
        assert_eq!(retry_delay_ceiling_ms(6), LEASE_RETRY_MAX_MS);
        assert_eq!(retry_delay_ceiling_ms(30), LEASE_RETRY_MAX_MS);
    }

    /// Waiting out the whole budget, with jitter always landing on its shortest delay, must stay
    /// far under the ~300 transactions the old flat 50-200 ms loop issued per waiter against a
    /// 10-connection pool.
    #[test]
    fn claim_backoff_bounds_transactions_per_wait_budget() {
        let mut elapsed = Duration::ZERO;
        let mut attempts = 0u32;
        while elapsed < LEASE_WAIT_BUDGET {
            elapsed += Duration::from_millis(retry_delay_ceiling_ms(attempts).div_ceil(2));
            attempts += 1;
        }
        assert!(attempts <= 24, "{attempts} claim transactions per waiter");
    }

    #[test]
    fn a_lost_lease_is_reported_as_locked_not_conflict() {
        for error in [lease_busy(7), lease_lost(7)] {
            assert_eq!(error.status(), axum::http::StatusCode::LOCKED);
            assert_eq!(error.public_code(), "BOARD_LOCKED");
        }
    }

    fn shared_for_test() -> Arc<LeaseShared> {
        Arc::new(LeaseShared {
            db: DatabaseConnection::default(),
            owner: new_owner_id(),
            lock_ids: parking_lot::Mutex::new(vec![42]),
            held: AtomicBool::new(true),
        })
    }

    /// H6: heartbeat used to only `warn!` when the row was no longer ours, so the writer went on
    /// to a full-object PUT that a second replica was already making.
    #[test]
    fn ensure_held_fails_once_the_heartbeat_marked_the_lease_lost() {
        let shared = shared_for_test();
        let lease = MutationLease::for_test(shared.clone());
        assert!(lease.ensure_held().is_ok());
        shared.mark_lost();
        let error = lease
            .ensure_held()
            .expect_err("a lost lease must fail the write");
        assert_eq!(error.status(), axum::http::StatusCode::LOCKED);
        assert_eq!(error.public_code(), "BOARD_LOCKED");
        assert!(error.public_message().unwrap_or_default().contains("42"));
    }

    #[test]
    fn heartbeat_error_budget_stays_inside_the_ttl() {
        assert!(LEASE_HEARTBEAT * LEASE_MAX_HEARTBEAT_ERRORS < LEASE_TTL);
    }
}
