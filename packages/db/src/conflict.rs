use sea_orm::{DbErr, RuntimeErr, TransactionError};

/// A transaction failure the engine asks the client to retry from scratch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DbConflict {
    /// Two transactions touched the same rows; the later committer lost.
    /// PostgreSQL/CockroachDB `40001`, Aurora DSQL `OC000`.
    Serialization,
    /// PostgreSQL detected a lock cycle and aborted one participant (`40P01`).
    Deadlock,
    /// Aurora DSQL's cached catalog is stale after concurrent DDL (`OC001`).
    SchemaChanged,
    /// The connection died or could not be obtained (SQLSTATE class `08`,
    /// `57P01`–`57P03`, `53300`, `53400`, or a driver I/O, protocol, worker or
    /// pool failure). A fresh pooled connection re-runs the transaction.
    ConnectionLost,
    /// The outcome of `COMMIT` is unknown: CockroachDB `40003`, or the
    /// connection dropped while the commit was in flight. Only an idempotent
    /// body may be re-run after this.
    AmbiguousCommit,
}

impl DbConflict {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Serialization => "serialization",
            Self::Deadlock => "deadlock",
            Self::SchemaChanged => "schema_changed",
            Self::ConnectionLost => "connection_lost",
            Self::AmbiguousCommit => "ambiguous_commit",
        }
    }

    /// Whether the previous attempt may already have committed.
    pub fn is_ambiguous(self) -> bool {
        matches!(self, Self::AmbiguousCommit)
    }
}

/// Classify a sea-orm error raised while a transaction is open (`BEGIN`, a
/// statement, or `ROLLBACK`).
pub fn classify_db_err(err: &DbErr) -> Option<DbConflict> {
    match err {
        DbErr::ConnectionAcquire(_) => Some(DbConflict::ConnectionLost),
        DbErr::Exec(RuntimeErr::SqlxError(sqlx_err))
        | DbErr::Query(RuntimeErr::SqlxError(sqlx_err))
        | DbErr::Conn(RuntimeErr::SqlxError(sqlx_err)) => classify_sqlx(sqlx_err),
        _ => None,
    }
}

/// Classify an error raised by `COMMIT` itself, where a lost connection
/// leaves the outcome unknown instead of guaranteeing a rollback.
pub fn classify_commit_err(err: &DbErr) -> Option<DbConflict> {
    match classify_db_err(err)? {
        DbConflict::ConnectionLost => Some(DbConflict::AmbiguousCommit),
        conflict => Some(conflict),
    }
}

fn classify_sqlx(err: &sea_orm::sqlx::Error) -> Option<DbConflict> {
    use sea_orm::sqlx::Error;
    match err {
        Error::Database(db_err) => {
            let code = db_err.code()?;
            classify_sqlstate(&code, db_err.message())
        }
        Error::Io(_)
        | Error::Protocol(_)
        | Error::WorkerCrashed
        | Error::PoolTimedOut
        | Error::PoolClosed => Some(DbConflict::ConnectionLost),
        _ => None,
    }
}

/// Map a SQLSTATE (plus the message, which carries DSQL's `OC` sub-code) to a
/// conflict. Constraint violations (`23xxx`), resource limits (`54000`,
/// `54011`, `53200`), syntax and privilege errors (`42xxx`) and an already
/// aborted transaction (`25P02`) are never conflicts.
pub fn classify_sqlstate(code: &str, message: &str) -> Option<DbConflict> {
    match code {
        "OC001" => Some(DbConflict::SchemaChanged),
        "OC000" => Some(DbConflict::Serialization),
        "40001" if message.contains("OC001") => Some(DbConflict::SchemaChanged),
        "40001" => Some(DbConflict::Serialization),
        "40P01" => Some(DbConflict::Deadlock),
        "40003" => Some(DbConflict::AmbiguousCommit),
        "57P01" | "57P02" | "57P03" | "53300" | "53400" => Some(DbConflict::ConnectionLost),
        "08000" | "08001" | "08003" | "08004" | "08006" | "08007" => {
            Some(DbConflict::ConnectionLost)
        }
        _ => None,
    }
}

/// Errors that may wrap a retryable database conflict.
pub trait AsDbConflict {
    fn db_conflict(&self) -> Option<DbConflict>;

    /// Stamp a conflict the retry loop established from context, such as an
    /// ambiguous `COMMIT`; error types that carry no classification keep it.
    fn with_conflict(self, _conflict: DbConflict) -> Self
    where
        Self: Sized,
    {
        self
    }
}

impl AsDbConflict for DbErr {
    fn db_conflict(&self) -> Option<DbConflict> {
        classify_db_err(self)
    }
}

impl<E: AsDbConflict> AsDbConflict for TransactionError<E> {
    fn db_conflict(&self) -> Option<DbConflict> {
        match self {
            TransactionError::Connection(err) => classify_db_err(err),
            TransactionError::Transaction(err) => err.db_conflict(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dsql_codes_and_messages_classify() {
        assert_eq!(
            classify_sqlstate("40001", "change conflicts with another transaction (OC000)"),
            Some(DbConflict::Serialization)
        );
        assert_eq!(
            classify_sqlstate(
                "40001",
                "schema has been updated by another transaction (OC001)"
            ),
            Some(DbConflict::SchemaChanged)
        );
        assert_eq!(
            classify_sqlstate("OC000", ""),
            Some(DbConflict::Serialization)
        );
        assert_eq!(
            classify_sqlstate("OC001", ""),
            Some(DbConflict::SchemaChanged)
        );
    }

    #[test]
    fn cockroach_and_postgres_codes_classify() {
        assert_eq!(
            classify_sqlstate(
                "40001",
                "restart transaction: TransactionRetryWithProtoRefreshError"
            ),
            Some(DbConflict::Serialization)
        );
        assert_eq!(
            classify_sqlstate("40P01", "deadlock detected"),
            Some(DbConflict::Deadlock)
        );
        assert_eq!(
            classify_sqlstate("40003", "result is ambiguous"),
            Some(DbConflict::AmbiguousCommit)
        );
    }

    #[test]
    fn connection_failures_classify_as_lost() {
        for code in [
            "08000", "08003", "08006", "57P01", "57P02", "57P03", "53300", "53400",
        ] {
            assert_eq!(
                classify_sqlstate(code, ""),
                Some(DbConflict::ConnectionLost),
                "{code}"
            );
        }
    }

    #[test]
    fn constraint_and_limit_violations_are_not_conflicts() {
        for code in [
            "23505", "23503", "23502", "23514", "54000", "54011", "53200", "42601", "42501",
            "42P01", "25P02",
        ] {
            assert_eq!(classify_sqlstate(code, "not retryable"), None, "{code}");
            assert_eq!(classify_sqlstate("08P01", "invalid message length"), None);
            assert_eq!(
                classify_sqlstate("08006", "connection failure"),
                Some(DbConflict::ConnectionLost)
            );
        }
    }

    #[test]
    fn driver_failures_classify_by_phase() {
        let io = sea_orm::sqlx::Error::Io(std::io::Error::other("connection reset"));
        let err = DbErr::Conn(RuntimeErr::SqlxError(std::sync::Arc::new(io)));
        assert_eq!(classify_db_err(&err), Some(DbConflict::ConnectionLost));
        assert_eq!(classify_commit_err(&err), Some(DbConflict::AmbiguousCommit));

        let pool = DbErr::ConnectionAcquire(sea_orm::ConnAcquireErr::Timeout);
        assert_eq!(classify_db_err(&pool), Some(DbConflict::ConnectionLost));

        let custom = DbErr::Custom("app not found".into());
        assert_eq!(classify_db_err(&custom), None);
        assert_eq!(classify_commit_err(&custom), None);
    }

    #[test]
    fn only_ambiguous_commit_is_ambiguous() {
        assert!(DbConflict::AmbiguousCommit.is_ambiguous());
        for conflict in [
            DbConflict::Serialization,
            DbConflict::Deadlock,
            DbConflict::SchemaChanged,
            DbConflict::ConnectionLost,
        ] {
            assert!(!conflict.is_ambiguous(), "{}", conflict.as_str());
        }
    }
}
