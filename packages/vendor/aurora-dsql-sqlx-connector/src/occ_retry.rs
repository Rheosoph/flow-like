// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use crate::{DsqlError, Result};
use derive_builder::Builder;
use sqlx::Acquire;
use std::time::Duration;

/// Convenience macro to hide `Box::pin` boilerplate for transaction closures.
#[macro_export]
macro_rules! txn {
    ($body:expr) => {
        Box::pin(async move { $body })
    };
}

/// OCC conflict type: Data (OC000), Schema (OC001), or Unknown (40001).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OCCType {
    Data,    // OC000
    Schema,  // OC001
    Unknown, // 40001
}

#[derive(Debug, Clone, Builder)]
#[builder(build_fn(validate = "Self::validate"))]
pub struct OCCRetryConfig {
    #[builder(default = "3")]
    max_attempts: u32,
    #[builder(default = "1")]
    base_delay_ms: u64,
    #[builder(default = "100")]
    max_delay_ms: u64,
    #[builder(default = "0.25")]
    jitter_factor: f64,
}

impl OCCRetryConfigBuilder {
    fn validate(&self) -> std::result::Result<(), String> {
        if let Some(0) = self.max_attempts {
            return Err("max_attempts must be greater than 0".into());
        }

        if let Some(attempts) = self.max_attempts {
            if attempts > 100 {
                return Err("max_attempts should not exceed 100".into());
            }
        }

        if let Some(0) = self.base_delay_ms {
            return Err("base_delay_ms must be greater than 0".into());
        }

        if let Some(max) = self.max_delay_ms {
            if max > 5000 {
                return Err("max_delay_ms exceeds 5000ms".into());
            }
        }

        if let (Some(base), Some(max)) = (self.base_delay_ms, self.max_delay_ms) {
            if max < base {
                return Err("max_delay_ms must be >= base_delay_ms".into());
            }
        }

        if let Some(jitter) = self.jitter_factor {
            if !(0.0..=1.0).contains(&jitter) {
                return Err("jitter_factor must be between 0.0 and 1.0".into());
            }
        }

        Ok(())
    }
}

impl Default for OCCRetryConfig {
    fn default() -> Self {
        OCCRetryConfigBuilder::default()
            .build()
            .expect("default builder values are valid")
    }
}

/// Detect and classify OCC errors. Returns `Some((OCCType, code))` for OCC errors, `None` otherwise.
pub fn is_occ_error(err: &sqlx::Error) -> Option<(OCCType, &'static str)> {
    if let sqlx::Error::Database(db_err) = err {
        match db_err.code().as_deref() {
            Some("OC000") => return Some((OCCType::Data, "OC000")),
            Some("OC001") => return Some((OCCType::Schema, "OC001")),
            Some("40001") => {
                // Fallback: parse message for embedded OCC type
                let message = db_err.message();
                if message.contains("(OC000)") {
                    return Some((OCCType::Data, "OC000"));
                } else if message.contains("(OC001)") {
                    return Some((OCCType::Schema, "OC001"));
                } else {
                    return Some((OCCType::Unknown, "40001"));
                }
            }
            _ => return None,
        }
    }
    None
}

pub(crate) fn calculate_backoff(config: &OCCRetryConfig, attempt: u32) -> Duration {
    let base = config.base_delay_ms as f64;
    let exponent = (attempt - 1).min(31); // Cap at 2^31 to prevent overflow
    let delay = (base * 2_f64.powi(exponent as i32)).min(config.max_delay_ms as f64);
    let jitter = delay * rand::random::<f64>() * config.jitter_factor;

    Duration::from_millis((delay + jitter) as u64)
}

/// Retry an async operation on OCC errors with exponential backoff.
///
/// The closure returns `Result<T, sqlx::Error>` directly — no need to
/// wrap errors in `DsqlError`. The helper handles error mapping internally.
///
/// Re-executes the entire closure on OCC conflict. The closure should
/// contain the full transaction (including `BEGIN`/`COMMIT`) since OCC
/// errors are detected at commit time.
///
/// **Warning:** The closure must be idempotent — it may be called multiple
/// times if OCC conflicts occur. Avoid side effects (e.g. sending emails,
/// incrementing external counters) that should not be repeated.
///
/// Returns `DsqlError::OCCRetryExhausted` with the last OCC error
/// preserved as the cause when max attempts are exceeded.
pub async fn retry_on_occ<F, Fut, T>(config: &OCCRetryConfig, f: F) -> Result<T>
where
    F: Fn() -> Fut,
    Fut: std::future::Future<Output = std::result::Result<T, sqlx::Error>>,
{
    let max_attempts = config.max_attempts;
    let mut attempt = 1;

    loop {
        match f().await {
            Ok(val) => {
                return Ok(val);
            }
            Err(e) => {
                let Some((occ_type, occ_code)) = is_occ_error(&e) else {
                    return Err(DsqlError::DatabaseError(e));
                };

                if attempt == max_attempts {
                    log::error!(
                        "OCC transaction retry exhausted, type={:?}, code={}, attempts={}",
                        occ_type,
                        occ_code,
                        max_attempts
                    );
                    return Err(DsqlError::OCCRetryExhausted {
                        attempts: max_attempts,
                        occ_type,
                        source: Box::new(DsqlError::DatabaseError(e)),
                    });
                }

                let delay = calculate_backoff(config, attempt);

                log::debug!(
                    "OCC conflict detected, type={:?}, code={}, retrying after backoff, attempt={}/{}, delay_ms={}",
                    occ_type, occ_code, attempt + 1, max_attempts, delay.as_millis()
                );

                tokio::time::sleep(delay).await;

                attempt += 1;
            }
        }
    }
}

/// Extension trait for `PgPool` and `PgConnection` providing ergonomic OCC retry methods.
///
/// Connection behavior:
/// - `PgPool`: Fresh connection per retry attempt
/// - `PgConnection`: Reuses same connection per retry
#[async_trait::async_trait]
pub trait OCCRetryExt {
    /// Execute a closure within a transaction, retrying on OCC errors.
    ///
    /// The transaction is automatically started before calling the closure and
    /// committed on success. On error, the transaction is rolled back.
    ///
    /// Pass `None` to use default configuration (max_attempts: 3, exponential backoff),
    /// or `Some(&config)` for custom retry behavior.
    ///
    /// # Idempotency Warning
    /// The closure may be called multiple times on OCC conflicts. Ensure it
    /// has no side effects that should not be repeated.
    async fn transaction_with_retry<F, T>(
        &mut self,
        config: Option<&OCCRetryConfig>,
        f: F,
    ) -> Result<T>
    where
        F: for<'a> Fn(
                &'a mut sqlx::Transaction<'_, sqlx::Postgres>,
            ) -> std::pin::Pin<
                Box<
                    dyn std::future::Future<Output = std::result::Result<T, sqlx::Error>>
                        + Send
                        + 'a,
                >,
            > + Send,
        T: Send;
}

// PgPool impl - gets fresh connection per retry
#[cfg(feature = "pool")]
#[async_trait::async_trait]
impl OCCRetryExt for sqlx::postgres::PgPool {
    async fn transaction_with_retry<F, T>(
        &mut self,
        config: Option<&OCCRetryConfig>,
        f: F,
    ) -> Result<T>
    where
        F: for<'a> Fn(
                &'a mut sqlx::Transaction<'_, sqlx::Postgres>,
            ) -> std::pin::Pin<
                Box<
                    dyn std::future::Future<Output = std::result::Result<T, sqlx::Error>>
                        + Send
                        + 'a,
                >,
            > + Send,
        T: Send,
    {
        let config = config.cloned().unwrap_or_default();
        let max_attempts = config.max_attempts;
        let mut attempt = 1;

        loop {
            // Get fresh connection from pool on each retry
            let mut tx = self.begin().await.map_err(DsqlError::DatabaseError)?;

            match f(&mut tx).await {
                Ok(val) => {
                    // Attempt commit - OCC errors occur at commit time
                    match tx.commit().await {
                        Ok(_) => {
                            return Ok(val);
                        }
                        Err(e) => {
                            log::debug!(
                                "Commit failed: error={}, attempt={}/{}, will_retry={}",
                                e,
                                attempt,
                                max_attempts,
                                attempt < max_attempts
                            );

                            let Some((occ_type, occ_code)) = is_occ_error(&e) else {
                                return Err(DsqlError::DatabaseError(e));
                            };

                            if attempt == max_attempts {
                                log::error!(
                                    "OCC transaction retry exhausted on commit, type={:?}, code={}, attempts={}",
                                    occ_type, occ_code, max_attempts
                                );
                                return Err(DsqlError::OCCRetryExhausted {
                                    attempts: max_attempts,
                                    occ_type,
                                    source: Box::new(DsqlError::DatabaseError(e)),
                                });
                            }

                            let delay = calculate_backoff(&config, attempt);

                            log::debug!(
                                "OCC conflict on commit, type={:?}, code={}, retrying after backoff, attempt={}/{}, delay_ms={}",
                                occ_type, occ_code, attempt + 1, max_attempts, delay.as_millis()
                            );

                            tokio::time::sleep(delay).await;
                            attempt += 1;
                        }
                    }
                }
                Err(e) => {
                    // Explicitly rollback before handling the error
                    if let Err(rollback_err) = tx.rollback().await {
                        log::debug!(
                            "Rollback failed: original_error={}, rollback_error={}, attempt={}/{}",
                            e,
                            rollback_err,
                            attempt,
                            max_attempts
                        );
                    }

                    // Check if this is an OCC error from the closure execution
                    let Some((occ_type, occ_code)) = is_occ_error(&e) else {
                        return Err(DsqlError::DatabaseError(e));
                    };

                    if attempt == max_attempts {
                        log::error!(
                            "OCC transaction retry exhausted during execution, type={:?}, code={}, attempts={}",
                            occ_type, occ_code, max_attempts
                        );
                        return Err(DsqlError::OCCRetryExhausted {
                            attempts: max_attempts,
                            occ_type,
                            source: Box::new(DsqlError::DatabaseError(e)),
                        });
                    }

                    let delay = calculate_backoff(&config, attempt);

                    log::debug!(
                        "OCC conflict during execution, type={:?}, code={}, retrying after backoff, attempt={}/{}, delay_ms={}",
                        occ_type, occ_code, attempt + 1, max_attempts, delay.as_millis()
                    );

                    tokio::time::sleep(delay).await;
                    attempt += 1;
                }
            }
        }
    }
}

// PgConnection impl - reuses same connection per retry
#[async_trait::async_trait]
impl OCCRetryExt for sqlx::PgConnection {
    async fn transaction_with_retry<F, T>(
        &mut self,
        config: Option<&OCCRetryConfig>,
        f: F,
    ) -> Result<T>
    where
        F: for<'a> Fn(
                &'a mut sqlx::Transaction<'_, sqlx::Postgres>,
            ) -> std::pin::Pin<
                Box<
                    dyn std::future::Future<Output = std::result::Result<T, sqlx::Error>>
                        + Send
                        + 'a,
                >,
            > + Send,
        T: Send,
    {
        let config = config.cloned().unwrap_or_default();
        let max_attempts = config.max_attempts;
        let mut attempt = 1;

        loop {
            let mut tx = self.begin().await.map_err(DsqlError::DatabaseError)?;

            // OCC errors may occur during execution or at commit; both are retried.
            match f(&mut tx).await {
                Ok(val) => {
                    // Attempt commit - OCC errors occur at commit time
                    match tx.commit().await {
                        Ok(_) => {
                            return Ok(val);
                        }
                        Err(e) => {
                            log::debug!(
                                "Commit failed: error={}, attempt={}/{}, will_retry={}",
                                e,
                                attempt,
                                max_attempts,
                                attempt < max_attempts
                            );

                            let Some((occ_type, occ_code)) = is_occ_error(&e) else {
                                return Err(DsqlError::DatabaseError(e));
                            };

                            if attempt == max_attempts {
                                log::error!(
                                    "OCC transaction retry exhausted on commit, type={:?}, code={}, attempts={}",
                                    occ_type, occ_code, max_attempts
                                );
                                return Err(DsqlError::OCCRetryExhausted {
                                    attempts: max_attempts,
                                    occ_type,
                                    source: Box::new(DsqlError::DatabaseError(e)),
                                });
                            }

                            let delay = calculate_backoff(&config, attempt);

                            log::debug!(
                                "OCC conflict on commit, type={:?}, code={}, retrying after backoff, attempt={}/{}, delay_ms={}",
                                occ_type, occ_code, attempt + 1, max_attempts, delay.as_millis()
                            );

                            tokio::time::sleep(delay).await;
                            attempt += 1;
                        }
                    }
                }
                Err(e) => {
                    // Explicitly rollback before handling the error
                    if let Err(rollback_err) = tx.rollback().await {
                        log::debug!(
                            "Rollback failed: original_error={}, rollback_error={}, attempt={}/{}",
                            e,
                            rollback_err,
                            attempt,
                            max_attempts
                        );
                    }

                    // Check if this is an OCC error from the closure execution
                    let Some((occ_type, occ_code)) = is_occ_error(&e) else {
                        return Err(DsqlError::DatabaseError(e));
                    };

                    if attempt == max_attempts {
                        log::error!(
                            "OCC transaction retry exhausted during execution, type={:?}, code={}, attempts={}",
                            occ_type, occ_code, max_attempts
                        );
                        return Err(DsqlError::OCCRetryExhausted {
                            attempts: max_attempts,
                            occ_type,
                            source: Box::new(DsqlError::DatabaseError(e)),
                        });
                    }

                    let delay = calculate_backoff(&config, attempt);

                    log::debug!(
                        "OCC conflict during execution, type={:?}, code={}, retrying after backoff, attempt={}/{}, delay_ms={}",
                        occ_type, occ_code, attempt + 1, max_attempts, delay.as_millis()
                    );

                    tokio::time::sleep(delay).await;
                    attempt += 1;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::time::Duration;

    // --- Test helpers ---

    struct MockDbError {
        code: Option<String>,
        message: String,
    }

    impl std::fmt::Display for MockDbError {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(f, "{}", self.message)
        }
    }

    impl std::fmt::Debug for MockDbError {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(f, "{}", self.message)
        }
    }

    impl std::error::Error for MockDbError {}

    impl sqlx::error::DatabaseError for MockDbError {
        fn message(&self) -> &str {
            &self.message
        }

        fn code(&self) -> Option<std::borrow::Cow<'_, str>> {
            self.code
                .as_ref()
                .map(|s| std::borrow::Cow::Borrowed(s.as_str()))
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

        fn kind(&self) -> sqlx::error::ErrorKind {
            sqlx::error::ErrorKind::Other
        }
    }

    fn make_occ_error() -> sqlx::Error {
        sqlx::Error::Database(Box::new(MockDbError {
            code: Some("OC000".to_string()),
            message: "mutation conflict".to_string(),
        }))
    }

    fn make_non_occ_error() -> sqlx::Error {
        sqlx::Error::Database(Box::new(MockDbError {
            code: Some("23505".to_string()),
            message: "unique violation".to_string(),
        }))
    }

    // --- Tests ---

    #[test]
    fn test_occ_error_detection_sqlstate() {
        // 40001 without embedded code -> Unknown
        let err = sqlx::Error::Database(Box::new(MockDbError {
            code: Some("40001".to_string()),
            message: "serialization failure".to_string(),
        }));
        assert_eq!(is_occ_error(&err), Some((OCCType::Unknown, "40001")));

        // 40001 with embedded (OC000) -> Data
        let err = sqlx::Error::Database(Box::new(MockDbError {
            code: Some("40001".to_string()),
            message: "data has been updated by another transaction, please retry: (OC000)"
                .to_string(),
        }));
        assert_eq!(is_occ_error(&err), Some((OCCType::Data, "OC000")));

        // 40001 with embedded (OC001) -> Schema
        let err = sqlx::Error::Database(Box::new(MockDbError {
            code: Some("40001".to_string()),
            message: "schema has been updated by another transaction, please retry: (OC001)"
                .to_string(),
        }));
        assert_eq!(is_occ_error(&err), Some((OCCType::Schema, "OC001")));
    }

    #[test]
    fn test_occ_error_detection_oc000() {
        let err = sqlx::Error::Database(Box::new(MockDbError {
            code: Some("OC000".to_string()),
            message: "optimistic concurrency failure".to_string(),
        }));

        assert_eq!(is_occ_error(&err), Some((OCCType::Data, "OC000")));
    }

    #[test]
    fn test_occ_error_detection_oc001() {
        let err = sqlx::Error::Database(Box::new(MockDbError {
            code: Some("OC001".to_string()),
            message: "transaction conflict".to_string(),
        }));

        assert_eq!(is_occ_error(&err), Some((OCCType::Schema, "OC001")));
    }

    #[test]
    fn test_non_occ_error() {
        let err = sqlx::Error::Database(Box::new(MockDbError {
            code: Some("23505".to_string()),
            message: "unique violation".to_string(),
        }));

        assert_eq!(is_occ_error(&err), None);
    }

    #[test]
    fn test_backoff_calculation() {
        let config = OCCRetryConfig::default();

        let delay1 = calculate_backoff(&config, 1);
        assert!(delay1 >= Duration::from_millis(1));
        assert!(delay1 <= Duration::from_millis(2));

        let delay2 = calculate_backoff(&config, 2);
        assert!(delay2 >= Duration::from_millis(2));
        assert!(delay2 <= Duration::from_millis(3));
    }

    #[test]
    fn test_backoff_max_delay() {
        let config = OCCRetryConfig::default();

        let delay = calculate_backoff(&config, 10);
        assert!(delay <= Duration::from_millis(125)); // max_delay(100ms) + 25% jitter
    }

    #[test]
    fn test_builder_defaults() {
        let config = OCCRetryConfigBuilder::default().build().unwrap();
        assert_eq!(config.max_attempts, 3);
        assert_eq!(config.base_delay_ms, 1);
        assert_eq!(config.max_delay_ms, 100);
        assert!((config.jitter_factor - 0.25).abs() < f64::EPSILON);
    }

    #[test]
    fn test_builder_custom_values() {
        let config = OCCRetryConfigBuilder::default()
            .max_attempts(5u32)
            .base_delay_ms(10u64)
            .build()
            .unwrap();
        assert_eq!(config.max_attempts, 5);
        assert_eq!(config.base_delay_ms, 10);
        assert_eq!(config.max_delay_ms, 100); // default
    }

    #[test]
    fn test_is_occ_error_non_database() {
        let err = sqlx::Error::Protocol("connection refused".into());
        assert_eq!(is_occ_error(&err), None);
    }

    #[test]
    fn test_is_occ_error_no_sqlstate_code() {
        let err = sqlx::Error::Database(Box::new(MockDbError {
            code: None,
            message: "unknown error".to_string(),
        }));
        assert_eq!(is_occ_error(&err), None);
    }

    #[test]
    fn test_occ_retry_exhausted_preserves_cause() {
        let sqlx_err = sqlx::Error::Database(Box::new(MockDbError {
            code: Some("OC000".to_string()),
            message: "OC000 conflict".to_string(),
        }));
        let err = DsqlError::OCCRetryExhausted {
            attempts: 3,
            occ_type: OCCType::Data,
            source: Box::new(DsqlError::DatabaseError(sqlx_err)),
        };
        assert!(err.to_string().contains("3 attempts"));
        assert!(err.to_string().contains("OC000"));
        // Verify source is accessible via std::error::Error
        let std_err: &dyn std::error::Error = &err;
        assert!(std_err.source().is_some());
    }

    #[tokio::test]
    async fn test_retry_on_occ_succeeds_first_try() {
        let config = OCCRetryConfig::default();
        let calls = AtomicU32::new(0);

        let result = retry_on_occ(&config, || async {
            calls.fetch_add(1, Ordering::SeqCst);
            Ok::<&str, sqlx::Error>("done")
        })
        .await;

        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "done");
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn test_retry_on_occ_retries_then_succeeds() {
        let config = OCCRetryConfigBuilder::default()
            .max_attempts(3u32)
            .base_delay_ms(1u64)
            .build()
            .unwrap();
        let calls = AtomicU32::new(0);

        let result = retry_on_occ(&config, || async {
            let attempt = calls.fetch_add(1, Ordering::SeqCst);
            if attempt < 2 {
                Err(make_occ_error())
            } else {
                Ok("recovered")
            }
        })
        .await;

        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "recovered");
        assert_eq!(calls.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn test_retry_on_occ_exhausted() {
        let config = OCCRetryConfigBuilder::default()
            .max_attempts(2u32)
            .base_delay_ms(1u64)
            .build()
            .unwrap();
        let calls = AtomicU32::new(0);

        let result: Result<()> = retry_on_occ(&config, || async {
            calls.fetch_add(1, Ordering::SeqCst);
            Err::<(), sqlx::Error>(make_occ_error())
        })
        .await;

        assert!(result.is_err());
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        match result.unwrap_err() {
            DsqlError::OCCRetryExhausted {
                attempts, occ_type, ..
            } => {
                assert_eq!(attempts, 2);
                assert_eq!(occ_type, OCCType::Data);
            }
            other => panic!("Expected OCCRetryExhausted, got: {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_retry_on_occ_non_occ_error_no_retry() {
        let config = OCCRetryConfig::default();
        let calls = AtomicU32::new(0);

        let result: Result<()> = retry_on_occ(&config, || async {
            calls.fetch_add(1, Ordering::SeqCst);
            Err::<(), sqlx::Error>(make_non_occ_error())
        })
        .await;

        assert!(result.is_err());
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn test_builder_rejects_zero_attempts() {
        let result = OCCRetryConfigBuilder::default().max_attempts(0u32).build();

        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("max_attempts"),
            "Expected max_attempts error, got: {}",
            err
        );
    }

    #[test]
    fn test_builder_rejects_inverted_delays() {
        let result = OCCRetryConfigBuilder::default()
            .base_delay_ms(5000u64)
            .max_delay_ms(100u64)
            .build();

        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("max_delay_ms"),
            "Expected max_delay_ms error, got: {}",
            err
        );
    }

    #[test]
    fn test_builder_rejects_negative_jitter() {
        let result = OCCRetryConfigBuilder::default().jitter_factor(-0.5).build();

        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("jitter_factor"),
            "Expected jitter_factor error, got: {}",
            err
        );
    }

    #[test]
    fn test_builder_rejects_excessive_jitter() {
        let result = OCCRetryConfigBuilder::default().jitter_factor(2.0).build();

        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("jitter_factor"),
            "Expected jitter_factor error, got: {}",
            err
        );
    }

    #[test]
    fn test_builder_rejects_excessive_max_attempts() {
        let result = OCCRetryConfigBuilder::default()
            .max_attempts(101u32)
            .build();

        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("max_attempts"),
            "Expected max_attempts error, got: {}",
            err
        );
    }

    #[test]
    fn test_builder_rejects_zero_base_delay() {
        let result = OCCRetryConfigBuilder::default().base_delay_ms(0u64).build();

        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("base_delay_ms"),
            "Expected base_delay_ms error, got: {}",
            err
        );
    }

    #[test]
    fn test_builder_rejects_excessive_max_delay() {
        let result = OCCRetryConfigBuilder::default()
            .max_delay_ms(5001u64)
            .build();

        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("max_delay_ms"),
            "Expected max_delay_ms error, got: {}",
            err
        );
    }

    #[test]
    fn test_builder_accepts_valid_config() {
        let result = OCCRetryConfigBuilder::default()
            .max_attempts(5u32)
            .base_delay_ms(10u64)
            .max_delay_ms(50u64)
            .jitter_factor(0.25)
            .build();

        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_retry_on_occ_handles_execution_errors() {
        // Tests that OCC errors during closure execution (not just commit) are retried
        let config = OCCRetryConfigBuilder::default()
            .max_attempts(3u32)
            .base_delay_ms(1u64)
            .build()
            .unwrap();
        let calls = AtomicU32::new(0);

        let result = retry_on_occ(&config, || async {
            let attempt = calls.fetch_add(1, Ordering::SeqCst);
            // Simulate OCC error during query execution on first two attempts
            if attempt < 2 {
                Err(make_occ_error())
            } else {
                Ok("success after retry")
            }
        })
        .await;

        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "success after retry");
        assert_eq!(calls.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn test_retry_on_occ_exhausted_on_execution_error() {
        // Tests that retry exhaustion works for OCC errors during execution
        let config = OCCRetryConfigBuilder::default()
            .max_attempts(2u32)
            .base_delay_ms(1u64)
            .build()
            .unwrap();
        let calls = AtomicU32::new(0);

        let result: Result<()> = retry_on_occ(&config, || async {
            calls.fetch_add(1, Ordering::SeqCst);
            // Always return OCC error during execution
            Err::<(), sqlx::Error>(make_occ_error())
        })
        .await;

        assert!(result.is_err());
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        match result.unwrap_err() {
            DsqlError::OCCRetryExhausted {
                attempts, occ_type, ..
            } => {
                assert_eq!(attempts, 2);
                assert_eq!(occ_type, OCCType::Data);
            }
            other => panic!("Expected OCCRetryExhausted, got: {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_retry_respects_custom_config() {
        // Tests that custom retry config is respected
        let config = OCCRetryConfigBuilder::default()
            .max_attempts(5u32)
            .base_delay_ms(1u64)
            .build()
            .unwrap();
        let calls = AtomicU32::new(0);

        let result = retry_on_occ(&config, || async {
            let attempt = calls.fetch_add(1, Ordering::SeqCst);
            if attempt < 4 {
                Err(make_occ_error())
            } else {
                Ok("recovered on attempt 5")
            }
        })
        .await;

        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "recovered on attempt 5");
        assert_eq!(calls.load(Ordering::SeqCst), 5);
    }

    #[tokio::test]
    async fn test_retry_with_different_occ_codes() {
        // Tests that all OCC error codes are recognized
        let config = OCCRetryConfigBuilder::default()
            .max_attempts(4u32)
            .base_delay_ms(1u64)
            .build()
            .unwrap();
        let calls = AtomicU32::new(0);

        let result = retry_on_occ(&config, || async {
            let attempt = calls.fetch_add(1, Ordering::SeqCst);
            match attempt {
                0 => Err(sqlx::Error::Database(Box::new(MockDbError {
                    code: Some("40001".to_string()),
                    message: "serialization failure".to_string(),
                }))),
                1 => Err(sqlx::Error::Database(Box::new(MockDbError {
                    code: Some("OC000".to_string()),
                    message: "data conflict".to_string(),
                }))),
                2 => Err(sqlx::Error::Database(Box::new(MockDbError {
                    code: Some("OC001".to_string()),
                    message: "schema conflict".to_string(),
                }))),
                _ => Ok("all occ codes handled"),
            }
        })
        .await;

        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "all occ codes handled");
        assert_eq!(calls.load(Ordering::SeqCst), 4);
    }
}
