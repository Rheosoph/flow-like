use crate::conflict::{AsDbConflict, DbConflict, classify_commit_err, classify_db_err};
use crate::dialect::DbDialect;
use sea_orm::{DatabaseConnection, DatabaseTransaction, DbErr, IsolationLevel, TransactionTrait};
use std::future::Future;
use std::pin::Pin;
use std::time::{Duration, Instant};

/// Exponential backoff with full jitter for conflict retries.
///
/// Aurora DSQL and CockroachDB report a lost commit race as an error rather
/// than blocking, so contention on a hot row (an `updatedAt` bump on a parent,
/// a counter) is expected to retry several times under load.
#[derive(Debug, Clone, Copy)]
pub struct RetryPolicy {
    pub max_attempts: u32,
    pub base_delay: Duration,
    pub max_delay: Duration,
    /// Wall-clock budget for all attempts including their backoff, so a hot
    /// row cannot pin a request for the whole attempt count.
    pub max_total: Duration,
    /// Whether the body may run again when the previous attempt's commit
    /// outcome is unknown ([`DbConflict::AmbiguousCommit`]).
    pub idempotent: bool,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_attempts: 8,
            base_delay: Duration::from_millis(5),
            max_delay: Duration::from_millis(250),
            max_total: Duration::from_secs(2),
            idempotent: false,
        }
    }
}

impl RetryPolicy {
    pub const NONE: Self = Self {
        max_attempts: 1,
        base_delay: Duration::ZERO,
        max_delay: Duration::ZERO,
        max_total: Duration::ZERO,
        idempotent: false,
    };

    /// A default policy whose body tolerates a repeated commit.
    pub const fn idempotent() -> Self {
        Self {
            max_attempts: 8,
            base_delay: Duration::from_millis(5),
            max_delay: Duration::from_millis(250),
            max_total: Duration::from_secs(2),
            idempotent: true,
        }
    }

    fn delay_for(&self, attempt: u32) -> Duration {
        let exponent = attempt.saturating_sub(1).min(16);
        let ceiling = self
            .base_delay
            .saturating_mul(1u32 << exponent)
            .min(self.max_delay);
        if ceiling.is_zero() {
            return ceiling;
        }
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.subsec_nanos() as u64)
            .unwrap_or(0);
        Duration::from_nanos(nanos % ceiling.as_nanos().max(1) as u64)
    }

    /// The backoff before the next attempt, or `None` when `attempt` (which
    /// just failed with `conflict`) was the last one this policy allows.
    fn next_delay(
        &self,
        attempt: u32,
        elapsed: Duration,
        conflict: Option<DbConflict>,
    ) -> Option<Duration> {
        let conflict = conflict?;
        if conflict.is_ambiguous() && !self.idempotent {
            return None;
        }
        if attempt >= self.max_attempts.max(1) {
            return None;
        }
        let delay = self.delay_for(attempt);
        if elapsed.saturating_add(delay) >= self.max_total {
            return None;
        }
        Some(delay)
    }
}

/// The body of a retried transaction.
///
/// It runs from scratch on every attempt, so it must be idempotent up to
/// commit: no side effects outside the database, and nothing awaited that is
/// not this transaction — a slow external call inside the body extends the
/// window in which the commit can lose a race.
pub type TransactionBody<'f, T, E> = dyn for<'c> Fn(&'c DatabaseTransaction) -> Pin<Box<dyn Future<Output = Result<T, E>> + Send + 'c>>
    + Send
    + Sync
    + 'f;

/// Run `body` in a transaction, retrying it whenever the engine reports a
/// serialization conflict, a deadlock, a stale schema or a lost connection.
///
/// Each attempt is `BEGIN` → `body` → `COMMIT`; a failed body is rolled back
/// before the backoff so no transaction is ever held across the sleep. A
/// commit whose outcome is unknown is retried only under an idempotent
/// policy; otherwise the error is returned as is.
///
/// The requested isolation level is applied on engines that support it and
/// silently dropped on DSQL, which only offers snapshot isolation.
pub async fn retry_transaction<F, T, E>(
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
    let isolation = dialect.effective_isolation(isolation);
    let started = Instant::now();
    let mut attempt = 0u32;
    loop {
        attempt += 1;
        let txn = match db.begin_with_config(isolation, None).await {
            Ok(txn) => txn,
            Err(err) => {
                let conflict = classify_db_err(&err);
                match policy.next_delay(attempt, started.elapsed(), conflict) {
                    Some(delay) => {
                        log_retry(dialect, conflict, attempt, policy, delay, "begin");
                        sleep(delay).await;
                        continue;
                    }
                    None => return Err(E::from(err)),
                }
            }
        };
        let (error, conflict, phase) = match body(&txn).await {
            Ok(value) => match txn.commit().await {
                Ok(()) => return Ok(value),
                Err(err) => {
                    let conflict = classify_commit_err(&err);
                    let error = match conflict {
                        Some(kind) if kind.is_ambiguous() => E::from(err).with_conflict(kind),
                        _ => E::from(err),
                    };
                    (error, conflict, "commit")
                }
            },
            Err(err) => {
                let _ = txn.rollback().await;
                let conflict = err.db_conflict();
                (err, conflict, "body")
            }
        };
        let Some(delay) = policy.next_delay(attempt, started.elapsed(), conflict) else {
            return Err(error);
        };
        log_retry(dialect, conflict, attempt, policy, delay, phase);
        sleep(delay).await;
    }
}

fn log_retry(
    dialect: DbDialect,
    conflict: Option<DbConflict>,
    attempt: u32,
    policy: &RetryPolicy,
    delay: Duration,
    phase: &'static str,
) {
    tracing::debug!(
        %dialect,
        conflict = conflict.map(DbConflict::as_str).unwrap_or("none"),
        phase,
        attempt,
        max_attempts = policy.max_attempts,
        delay_ms = delay.as_millis() as u64,
        "transaction lost a commit race; retrying"
    );
}

async fn sleep(delay: Duration) {
    if !delay.is_zero() {
        tokio::time::sleep(delay).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn policy() -> RetryPolicy {
        RetryPolicy {
            max_attempts: 8,
            base_delay: Duration::from_millis(10),
            max_delay: Duration::from_millis(50),
            max_total: Duration::from_secs(2),
            idempotent: false,
        }
    }

    #[test]
    fn backoff_grows_and_is_capped() {
        let policy = policy();
        for attempt in 1..=8 {
            let ceiling = policy
                .base_delay
                .saturating_mul(1u32 << (attempt - 1))
                .min(policy.max_delay);
            assert!(policy.delay_for(attempt) <= ceiling, "attempt {attempt}");
        }
        assert_eq!(RetryPolicy::NONE.delay_for(1), Duration::ZERO);
    }

    #[test]
    fn retries_stop_at_the_attempt_cap() {
        let policy = policy();
        let conflict = Some(DbConflict::Serialization);
        assert!(policy.next_delay(1, Duration::ZERO, conflict).is_some());
        assert!(policy.next_delay(7, Duration::ZERO, conflict).is_some());
        assert!(policy.next_delay(8, Duration::ZERO, conflict).is_none());
        assert!(
            RetryPolicy::NONE
                .next_delay(1, Duration::ZERO, conflict)
                .is_none()
        );
    }

    #[test]
    fn retries_stop_when_the_time_budget_is_spent() {
        let policy = policy();
        let conflict = Some(DbConflict::Deadlock);
        assert!(
            policy
                .next_delay(1, Duration::from_millis(100), conflict)
                .is_some()
        );
        assert!(
            policy
                .next_delay(1, Duration::from_secs(2), conflict)
                .is_none()
        );
        assert!(
            policy
                .next_delay(1, Duration::from_secs(3), conflict)
                .is_none()
        );
    }

    #[test]
    fn non_conflicts_are_never_retried() {
        assert!(policy().next_delay(1, Duration::ZERO, None).is_none());
    }

    #[test]
    fn ambiguous_commits_need_an_idempotent_body() {
        let conflict = Some(DbConflict::AmbiguousCommit);
        assert!(policy().next_delay(1, Duration::ZERO, conflict).is_none());
        assert!(
            RetryPolicy::idempotent()
                .next_delay(1, Duration::ZERO, conflict)
                .is_some()
        );
        assert!(
            RetryPolicy::idempotent()
                .next_delay(8, Duration::ZERO, conflict)
                .is_none()
        );
        assert!(
            RetryPolicy {
                idempotent: true,
                ..policy()
            }
            .next_delay(1, Duration::ZERO, Some(DbConflict::ConnectionLost))
            .is_some()
        );
    }

    #[allow(dead_code)]
    fn retry_future_is_send(db: &DatabaseConnection) {
        fn assert_send<T: Send>(_: &T) {}
        let future = retry_transaction::<_, (), DbErr>(
            db,
            DbDialect::Postgres,
            None,
            &RetryPolicy::NONE,
            |_txn| Box::pin(async { Ok(()) }),
        );
        assert_send(&future);
    }
}
