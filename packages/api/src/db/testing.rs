//! Chaos injection for retried transactions, so the DSQL failure modes can be
//! exercised against a plain PostgreSQL container.
//!
//! Two faults are available, each switched on by an environment variable and
//! also settable programmatically through [`Chaos::new`]:
//!
//! * `FLOW_LIKE_DB_CHAOS_ROW_LIMIT=N` — before the body's result is committed,
//!   the rows inserted, updated and deleted by the transaction so far are
//!   summed from `pg_stat_xact_user_tables` (PostgreSQL only; the view is
//!   transaction-local, so the count is exact) and the attempt fails with a
//!   synthesized SQLSTATE `54000` when they exceed `N`. This is the error
//!   Aurora DSQL raises past its 3,000-row cap, and it is never retried.
//! * `FLOW_LIKE_DB_CHAOS_CONFLICT_EVERY=N` — every Nth transaction's first
//!   attempt runs its body and then fails with a synthesized `40001 (OC000)`
//!   in place of the commit, which the retry loop treats like a lost commit
//!   race. A body that performs a side effect before commit performs it twice
//!   under this fault, which is exactly what a test should catch.
//!
//! The wrapper composes with [`retry_transaction`] rather than hooking into
//! it: the fault is injected from inside the body closure, on the same open
//! transaction, right where the engine would raise the real error.

use flow_like_db::{AsDbConflict, DbDialect, RetryPolicy, retry_transaction};
use sea_orm::sqlx::error::{DatabaseError, ErrorKind};
use sea_orm::{
    ConnectionTrait, DatabaseBackend, DatabaseConnection, DatabaseTransaction, DbErr,
    IsolationLevel, RuntimeErr, Statement,
};
use std::borrow::Cow;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};

pub const ROW_LIMIT_ENV: &str = "FLOW_LIKE_DB_CHAOS_ROW_LIMIT";
pub const CONFLICT_EVERY_ENV: &str = "FLOW_LIKE_DB_CHAOS_CONFLICT_EVERY";

pub const ROW_LIMIT_SQLSTATE: &str = "54000";
pub const CONFLICT_SQLSTATE: &str = "40001";
pub const CONFLICT_MESSAGE: &str = "change conflicts with another transaction (OC000)";

const MUTATED_ROWS_SQL: &str = "SELECT COALESCE(SUM(n_tup_ins + n_tup_upd + n_tup_del), 0)::BIGINT AS rows FROM pg_stat_xact_user_tables";

/// Fault configuration plus counters of what it injected.
#[derive(Debug, Default)]
pub struct Chaos {
    row_limit: Option<u64>,
    conflict_every: Option<u64>,
    counters: Arc<Counters>,
}

#[derive(Debug, Default)]
struct Counters {
    transactions: AtomicU64,
    injected_conflicts: AtomicU64,
    row_limit_failures: AtomicU64,
}

impl Chaos {
    pub fn new(row_limit: Option<u64>, conflict_every: Option<u64>) -> Self {
        Self {
            row_limit,
            conflict_every: conflict_every.filter(|every| *every > 0),
            ..Self::default()
        }
    }

    pub fn from_env() -> Self {
        Self::new(env_u64(ROW_LIMIT_ENV), env_u64(CONFLICT_EVERY_ENV))
    }

    /// The process-wide instance read from the environment once.
    pub fn global() -> &'static Chaos {
        static GLOBAL: OnceLock<Chaos> = OnceLock::new();
        GLOBAL.get_or_init(Chaos::from_env)
    }

    pub fn is_active(&self) -> bool {
        self.row_limit.is_some() || self.conflict_every.is_some()
    }

    pub fn row_limit(&self) -> Option<u64> {
        self.row_limit
    }

    pub fn conflict_every(&self) -> Option<u64> {
        self.conflict_every
    }

    /// Transactions started through this instance, counting each call once
    /// regardless of how many attempts it took.
    pub fn transactions(&self) -> u64 {
        self.counters.transactions.load(Ordering::Relaxed)
    }

    pub fn injected_conflicts(&self) -> u64 {
        self.counters.injected_conflicts.load(Ordering::Relaxed)
    }

    pub fn row_limit_failures(&self) -> u64 {
        self.counters.row_limit_failures.load(Ordering::Relaxed)
    }

    /// [`retry_transaction`] with the configured faults injected after the
    /// body and before commit.
    pub async fn transaction<F, T, E>(
        &self,
        db: &DatabaseConnection,
        dialect: DbDialect,
        isolation: Option<IsolationLevel>,
        policy: &RetryPolicy,
        body: F,
    ) -> Result<T, E>
    where
        F: for<'c> Fn(
                &'c DatabaseTransaction,
            ) -> Pin<Box<dyn Future<Output = Result<T, E>> + Send + 'c>>
            + Send
            + Sync,
        T: Send,
        E: From<DbErr> + AsDbConflict + std::fmt::Display + std::fmt::Debug + Send,
    {
        let sequence = self.counters.transactions.fetch_add(1, Ordering::Relaxed) + 1;
        let conflict_on_first_attempt = self
            .conflict_every
            .is_some_and(|every| sequence.is_multiple_of(every));
        let row_limit = self.row_limit;
        let counters = self.counters.clone();
        let attempts = Arc::new(AtomicU64::new(0));
        retry_transaction(db, dialect, isolation, policy, move |txn| {
            let attempt = attempts.fetch_add(1, Ordering::Relaxed) + 1;
            let counters = counters.clone();
            let inner = body(txn);
            Box::pin(async move {
                let value = inner.await?;
                if let Some(limit) = row_limit {
                    let rows = mutated_rows(txn).await?;
                    if rows > limit {
                        counters.row_limit_failures.fetch_add(1, Ordering::Relaxed);
                        return Err(E::from(row_limit_error(rows, limit)));
                    }
                }
                if conflict_on_first_attempt && attempt == 1 {
                    counters.injected_conflicts.fetch_add(1, Ordering::Relaxed);
                    return Err(E::from(conflict_error()));
                }
                Ok(value)
            })
        })
        .await
    }

    /// [`Self::transaction`] against a [`crate::state::State`], mirroring
    /// [`crate::state::State::transaction`].
    pub async fn state_transaction<F, T, E>(
        &self,
        state: &crate::state::State,
        body: F,
    ) -> Result<T, E>
    where
        F: for<'c> Fn(
                &'c DatabaseTransaction,
            ) -> Pin<Box<dyn Future<Output = Result<T, E>> + Send + 'c>>
            + Send
            + Sync,
        T: Send,
        E: From<DbErr> + AsDbConflict + std::fmt::Display + std::fmt::Debug + Send,
    {
        self.transaction(
            &state.db,
            state.db_dialect,
            None,
            &RetryPolicy::default(),
            body,
        )
        .await
    }
}

fn env_u64(name: &str) -> Option<u64> {
    std::env::var(name).ok()?.trim().parse().ok()
}

/// Rows inserted, updated and deleted so far by the transaction `conn` is
/// running in. PostgreSQL only.
pub async fn mutated_rows<C: ConnectionTrait>(conn: &C) -> Result<u64, DbErr> {
    let row = conn
        .query_one_raw(Statement::from_string(
            DatabaseBackend::Postgres,
            MUTATED_ROWS_SQL,
        ))
        .await?
        .ok_or_else(|| DbErr::Custom("pg_stat_xact_user_tables returned no row".into()))?;
    let rows: i64 = row.try_get("", "rows")?;
    Ok(rows.max(0) as u64)
}

/// The SQLSTATE carried by a driver error, if `err` wraps one.
pub fn sqlstate(err: &DbErr) -> Option<String> {
    match err {
        DbErr::Exec(RuntimeErr::SqlxError(sqlx_err))
        | DbErr::Query(RuntimeErr::SqlxError(sqlx_err))
        | DbErr::Conn(RuntimeErr::SqlxError(sqlx_err)) => match sqlx_err.as_ref() {
            sea_orm::sqlx::Error::Database(db_err) => db_err.code().map(|code| code.into_owned()),
            _ => None,
        },
        _ => None,
    }
}

pub fn row_limit_error(rows: u64, limit: u64) -> DbErr {
    synthesized_error(
        ROW_LIMIT_SQLSTATE,
        format!("transaction row limit exceeded: {rows} rows mutated, limit {limit}"),
    )
}

pub fn conflict_error() -> DbErr {
    synthesized_error(CONFLICT_SQLSTATE, CONFLICT_MESSAGE)
}

/// A `DbErr` shaped like one the PostgreSQL driver raises, so the conflict
/// classifier and every `sqlstate` check see the given code.
pub fn synthesized_error(code: &str, message: impl Into<String>) -> DbErr {
    let error = ChaosDatabaseError {
        code: code.to_owned(),
        message: message.into(),
    };
    DbErr::Exec(RuntimeErr::SqlxError(Arc::new(
        sea_orm::sqlx::Error::Database(Box::new(error)),
    )))
}

#[derive(Debug)]
struct ChaosDatabaseError {
    code: String,
    message: String,
}

impl std::fmt::Display for ChaosDatabaseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} (chaos, SQLSTATE {})", self.message, self.code)
    }
}

impl std::error::Error for ChaosDatabaseError {}

impl DatabaseError for ChaosDatabaseError {
    fn message(&self) -> &str {
        &self.message
    }

    fn code(&self) -> Option<Cow<'_, str>> {
        Some(Cow::Borrowed(&self.code))
    }

    fn as_error(&self) -> &(dyn std::error::Error + Send + Sync + 'static) {
        self
    }

    fn as_error_mut(&mut self) -> &mut (dyn std::error::Error + Send + Sync + 'static) {
        self
    }

    fn into_error(self: Box<Self>) -> Box<dyn std::error::Error + Send + Sync + 'static> {
        self
    }

    fn kind(&self) -> ErrorKind {
        ErrorKind::Other
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use flow_like_db::{DbConflict, classify_commit_err, classify_db_err};

    #[test]
    fn synthesized_conflict_classifies_like_the_real_one() {
        let err = conflict_error();
        assert_eq!(sqlstate(&err).as_deref(), Some(CONFLICT_SQLSTATE));
        assert_eq!(classify_db_err(&err), Some(DbConflict::Serialization));
        assert_eq!(classify_commit_err(&err), Some(DbConflict::Serialization));
        assert_eq!(err.db_conflict(), Some(DbConflict::Serialization));
    }

    #[test]
    fn row_limit_error_is_never_a_conflict() {
        let err = row_limit_error(3_001, 3_000);
        assert_eq!(sqlstate(&err).as_deref(), Some(ROW_LIMIT_SQLSTATE));
        assert_eq!(classify_db_err(&err), None);
        assert!(err.to_string().contains("transaction row limit exceeded"));
    }

    #[test]
    fn conflict_every_zero_is_off() {
        let chaos = Chaos::new(None, Some(0));
        assert!(!chaos.is_active());
        assert!(Chaos::new(Some(10), None).is_active());
        assert_eq!(Chaos::new(None, Some(3)).conflict_every(), Some(3));
    }

    #[test]
    fn plain_errors_carry_no_sqlstate() {
        assert_eq!(sqlstate(&DbErr::Custom("x".into())), None);
    }

    #[allow(dead_code)]
    fn chaos_transaction_future_is_send(chaos: &Chaos, db: &DatabaseConnection) {
        fn assert_send<T: Send>(_: &T) {}
        let future = chaos.transaction::<_, (), DbErr>(
            db,
            DbDialect::Postgres,
            None,
            &RetryPolicy::NONE,
            |_txn| Box::pin(async { Ok(()) }),
        );
        assert_send(&future);
    }
}
