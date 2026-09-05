//! Redis job queue consumer for async execution dispatch.
//!
//! Atomic claims retain accepted work until settlement. A bounded notification
//! list wakes idle consumers without placing execution payloads in pub/sub.
//!
//! Version 3 records publication time using the Redis clock. A ready job older
//! than `EXECUTION_QUEUE_MAX_WAIT_SECONDS` (default 300) is retained in the dead
//! queue before execution. Explicit non-admission retries keep that original
//! age. Drain/reconcile version 2 queues before switching producers and workers;
//! missing publication metadata cannot establish a safe credential lifetime.
//!
//! ## Usage
//!
//! ```rust,ignore
//! use flow_like_api::execution::queue::{QueueWorker, QueueConfig};
//!
//! let config = QueueConfig::from_env();
//! let worker = QueueWorker::new(config).await?;
//!
//! // Run the worker loop (blocks until shutdown)
//! worker.run().await?;
//! ```
//!
//! ## Configuration
//!
//! ```bash
//! REDIS_URL=redis://localhost:6379
//! REDIS_EXECUTION_QUEUE=exec:jobs:v3
//! QUEUE_WORKER_CONCURRENCY=10
//! QUEUE_POLL_TIMEOUT_SECS=30
//! ```

pub use flow_like_types::OAuthTokenInput;

/// Queue configuration
#[derive(Clone, Debug)]
pub struct QueueConfig {
    /// Redis connection URL
    pub redis_url: String,
    /// Queue name to poll
    pub queue_name: String,
    /// Maximum concurrent job executions
    pub concurrency: usize,
    /// Maximum idle notification wait before checking retained deliveries.
    pub poll_timeout_secs: u64,
}

impl Default for QueueConfig {
    fn default() -> Self {
        Self::from_env()
    }
}

impl QueueConfig {
    pub fn from_env() -> Self {
        Self {
            redis_url: std::env::var("REDIS_URL")
                .unwrap_or_else(|_| "redis://localhost:6379".into()),
            queue_name: std::env::var("REDIS_EXECUTION_QUEUE")
                .unwrap_or_else(|_| "exec:jobs:v3".into()),
            concurrency: std::env::var("QUEUE_WORKER_CONCURRENCY")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(10),
            poll_timeout_secs: std::env::var("QUEUE_POLL_TIMEOUT_SECS")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(30),
        }
    }
}

/// Job payload from the queue, shared with every dispatch transport.
pub type QueuedJob = flow_like_types::dispatch::DispatchPayload;

/// Only a trusted dispatcher may report `NotAdmitted`, after the execution
/// backend explicitly confirms that it did not start the execution. Transport
/// errors, timeouts and execution failures must remain errors for reconciliation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QueueDisposition {
    Completed,
    NotAdmitted { retry_after: std::time::Duration },
}

/// Queue worker errors
#[derive(Debug, thiserror::Error)]
pub enum QueueError {
    #[error("Redis error: {0}")]
    Redis(String),
    #[error("Serialization error: {0}")]
    Serialization(String),
    #[error("Execution error: {0}")]
    Execution(String),
}

/// Startup, terminal acknowledgement and cleanup are outside the workflow's
/// execution budget. Share this allowance with credential checkout and bridges.
pub(crate) fn supervision_grace_seconds() -> Result<u64, QueueError> {
    fn bounded(name: &str, value: String, maximum: u64) -> Result<u64, QueueError> {
        value
            .parse::<u64>()
            .ok()
            .filter(|value| (1..=maximum).contains(value))
            .ok_or_else(|| QueueError::Execution(format!("{name} must be 1..{maximum}")))
    }
    Ok(bounded(
        "EXECUTION_STARTUP_GRACE_SECONDS",
        std::env::var("EXECUTION_STARTUP_GRACE_SECONDS")
            .or_else(|_| std::env::var("SANDBOX_STARTUP_TIMEOUT_SECONDS"))
            .unwrap_or_else(|_| "120".into()),
        600,
    )? + bounded(
        "EXECUTION_TERMINAL_GRACE_SECONDS",
        std::env::var("EXECUTION_TERMINAL_GRACE_SECONDS").unwrap_or_else(|_| "60".into()),
        300,
    )? + bounded(
        "EXECUTION_CLEANUP_TIMEOUT_SECONDS",
        std::env::var("EXECUTION_CLEANUP_TIMEOUT_SECONDS").unwrap_or_else(|_| "30".into()),
        300,
    )?)
}

/// Admission includes failed and in-flight deliveries, so a full dead-letter
/// queue cannot silently discard accepted jobs to make room for more work.
#[cfg(feature = "redis")]
pub(crate) const ENQUEUE_SCRIPT: &str = r#"
local count = redis.call('LLEN', KEYS[1]) + redis.call('HLEN', KEYS[2]) + redis.call('LLEN', KEYS[3])
if count >= tonumber(ARGV[2]) then return 0 end
local clock = redis.call('TIME')
local now = tonumber(clock[1]) * 1000 + math.floor(tonumber(clock[2]) / 1000)
redis.call('HSETNX', KEYS[5], redis.sha1hex(ARGV[1]), now)
redis.call('LPUSH', KEYS[1], ARGV[1])
redis.call('LPUSH', KEYS[4], 'ready')
redis.call('LTRIM', KEYS[4], 0, 0)
return 1
"#;

#[cfg(feature = "redis")]
mod worker {
    use super::*;
    use redis::{
        AsyncCommands, Client, Script,
        aio::{ConnectionManager, ConnectionManagerConfig},
    };
    use std::{sync::Arc, time::Duration};
    use tokio::sync::Semaphore;

    // A claim moves bytes atomically into pending storage before execution.
    // Expired claims are quarantined. They require reconciliation with the
    // execution manager before an operator can authorize another attempt.
    const CLAIM: &str = r#"
local clock = redis.call('TIME')
local now = tonumber(clock[1]) * 1000 + math.floor(tonumber(clock[2]) / 1000)
local expired = redis.call('ZRANGEBYSCORE', KEYS[3], '-inf', now, 'LIMIT', 0, 100)
for _, id in ipairs(expired) do
    local payload = redis.call('HGET', KEYS[2], id)
    if payload then
        redis.call('LPUSH', KEYS[4], cjson.encode({delivery_id=id, reason='delivery_expired_requires_reconciliation', payload=payload, failed_at=now}))
        redis.call('HDEL', KEYS[6], redis.sha1hex(payload))
    end
    redis.call('HDEL', KEYS[2], id)
    redis.call('ZREM', KEYS[3], id)
end
for skipped = 1, 100 do
    local payload = redis.call('RPOP', KEYS[1])
    if not payload then return nil end
    if redis.call('LLEN', KEYS[1]) > 0 then
        redis.call('LPUSH', KEYS[5], 'ready')
        redis.call('LTRIM', KEYS[5], 0, 0)
    end
    -- The digest only indexes trusted queue bookkeeping, never authorization.
    local identity = redis.sha1hex(payload)
    local published = redis.call('HGET', KEYS[6], identity)
    local age = published and (now - tonumber(published)) or nil
    if not age or age < 0 or age >= tonumber(ARGV[3]) then
        local reason = age and 'queue_wait_expired_before_execution' or 'queue_publication_time_missing'
        redis.call('LPUSH', KEYS[4], cjson.encode({reason=reason, payload=payload, failed_at=now}))
        redis.call('HDEL', KEYS[6], identity)
    else
        redis.call('HSET', KEYS[2], ARGV[1], payload)
        redis.call('ZADD', KEYS[3], now + tonumber(ARGV[2]), ARGV[1])
        return payload
    end
end
return nil
"#;
    const COMPLETE: &str = r#"
local payload = redis.call('HGET', KEYS[1], ARGV[1])
if not payload then return 0 end
if ARGV[2] ~= '' then
    redis.call('LPUSH', KEYS[3], cjson.encode({delivery_id=ARGV[1], reason=ARGV[2], payload=payload}))
end
redis.call('HDEL', KEYS[1], ARGV[1])
redis.call('ZREM', KEYS[2], ARGV[1])
redis.call('HDEL', KEYS[4], redis.sha1hex(payload))
return 1
"#;

    // A positive non-admission acknowledgement is the only automatic retry.
    // Fence against expired ownership even when a competing worker has not yet
    // moved the delivery to dead storage. Retain the original serialized bytes.
    const REQUEUE: &str = r#"
local payload = redis.call('HGET', KEYS[1], ARGV[1])
if not payload then return 0 end
local clock = redis.call('TIME')
local now = tonumber(clock[1]) * 1000 + math.floor(tonumber(clock[2]) / 1000)
local deadline = redis.call('ZSCORE', KEYS[2], ARGV[1])
if not deadline or tonumber(deadline) <= now then return 0 end
redis.call('LPUSH', KEYS[3], payload)
redis.call('HDEL', KEYS[1], ARGV[1])
redis.call('ZREM', KEYS[2], ARGV[1])
redis.call('LPUSH', KEYS[4], 'ready')
redis.call('LTRIM', KEYS[4], 0, 0)
return 1
"#;

    pub struct QueueWorker {
        config: QueueConfig,
        connection: ConnectionManager,
        notifications: ConnectionManager,
        semaphore: Arc<Semaphore>,
        queue_capacity: Arc<Semaphore>,
        delivery_timeout: Duration,
        notification_wait: Duration,
        maximum_queue_wait: Duration,
    }

    impl QueueWorker {
        pub async fn new(config: QueueConfig) -> Result<Self, QueueError> {
            let admission = Arc::new(Semaphore::new(config.concurrency));
            Self::with_admission(config, admission).await
        }

        pub async fn with_admission(
            config: QueueConfig,
            semaphore: Arc<Semaphore>,
        ) -> Result<Self, QueueError> {
            if config.concurrency == 0 || config.queue_name.trim().is_empty() {
                return Err(QueueError::Execution(
                    "queue concurrency and queue name must be nonzero/nonempty".into(),
                ));
            }
            let timeout = std::env::var("EXECUTION_TIMEOUT_SECONDS")
                .or_else(|_| std::env::var("EXECUTOR_TIMEOUT_SECS"))
                .unwrap_or_else(|_| "3600".into())
                .parse::<u64>()
                .ok()
                .filter(|value| (1..=86400).contains(value))
                .ok_or_else(|| {
                    QueueError::Execution("EXECUTION_TIMEOUT_SECONDS must be 1..86400".into())
                })?;
            let maximum_queue_wait = std::env::var("EXECUTION_QUEUE_MAX_WAIT_SECONDS")
                .unwrap_or_else(|_| "300".into())
                .parse::<u64>()
                .ok()
                .filter(|value| (1..=86400).contains(value))
                .ok_or_else(|| {
                    QueueError::Execution(
                        "EXECUTION_QUEUE_MAX_WAIT_SECONDS must be 1..86400".into(),
                    )
                })?;
            let client = Client::open(config.redis_url.as_str())
                .map_err(|error| QueueError::Redis(error.to_string()))?;
            // The blocking list is only a wakeup hint. Periodic claims still
            // recover pending deadlines after reconnects or missed notifications.
            let notification_wait = Duration::from_secs(config.poll_timeout_secs.clamp(1, 30));
            let connection_config = ConnectionManagerConfig::new()
                .set_connection_timeout(Duration::from_secs(5))
                .set_response_timeout(Duration::from_secs(10));
            let (connection, notifications) =
                tokio::time::timeout(Duration::from_secs(10), async {
                    tokio::try_join!(
                        client.get_connection_manager_with_config(connection_config.clone()),
                        client.get_connection_manager_with_config(
                            connection_config
                                .set_response_timeout(notification_wait + Duration::from_secs(5))
                        )
                    )
                })
                .await
                .map_err(|_| QueueError::Redis("queue startup connection timed out".into()))?
                .map_err(|error| QueueError::Redis(error.to_string()))?;
            let queue_capacity = Arc::new(Semaphore::new(config.concurrency));
            Ok(Self {
                config,
                connection,
                notifications,
                semaphore,
                queue_capacity,
                delivery_timeout: Duration::from_secs(timeout + supervision_grace_seconds()? + 120),
                notification_wait,
                maximum_queue_wait: Duration::from_secs(maximum_queue_wait),
            })
        }

        /// Leave time for a trusted terminal-status lookup before handler and
        /// retained-delivery deadlines expire. Bridges must use this budget.
        pub fn execution_request_timeout(&self) -> Duration {
            self.delivery_timeout
                .saturating_sub(Duration::from_secs(90))
        }

        /// Compatibility entry point for handlers that cannot prove non-admission.
        pub async fn run<F, Fut>(&self, handler: F) -> Result<(), QueueError>
        where
            F: Fn(QueuedJob) -> Fut + Send + Sync + Clone + 'static,
            Fut: std::future::Future<Output = Result<(), String>> + Send + 'static,
        {
            self.run_with_outcomes(move |job| {
                let future = handler(job);
                async move { future.await.map(|()| QueueDisposition::Completed) }
            })
            .await
        }

        /// Closing the shared semaphore stops claims. In-flight tasks retain
        /// their permits until the handler and delivery acknowledgement finish.
        pub async fn run_with_outcomes<F, Fut>(&self, handler: F) -> Result<(), QueueError>
        where
            F: Fn(QueuedJob) -> Fut + Send + Sync + Clone + 'static,
            Fut: std::future::Future<Output = Result<QueueDisposition, String>> + Send + 'static,
        {
            let ready = self.config.queue_name.clone();
            let pending = format!("{ready}:pending");
            let deadlines = format!("{ready}:deadlines");
            let dead = format!("{ready}:dead");
            let notify = format!("{ready}:notify");
            let published = format!("{ready}:published");
            let mut conn = self.connection.clone();
            loop {
                let Ok(queue_permit) = self.queue_capacity.clone().acquire_owned().await else {
                    return Ok(());
                };
                let Ok(permit) = self.semaphore.clone().acquire_owned().await else {
                    return Ok(());
                };
                let delivery_id = flow_like_types::create_id();
                let payload: Option<String> = Script::new(CLAIM)
                    .key(&ready)
                    .key(&pending)
                    .key(&deadlines)
                    .key(&dead)
                    .key(&notify)
                    .key(&published)
                    .arg(&delivery_id)
                    .arg(self.delivery_timeout.as_millis() as u64)
                    .arg(self.maximum_queue_wait.as_millis() as u64)
                    .invoke_async(&mut conn)
                    .await
                    .map_err(|error| QueueError::Redis(error.to_string()))?;
                let Some(payload) = payload else {
                    drop(permit);
                    drop(queue_permit);
                    let mut notifications = self.notifications.clone();
                    let mut command = redis::cmd("BLPOP");
                    command.arg(&notify).arg(self.notification_wait.as_secs());
                    let wait = command.query_async::<Option<(String, String)>>(&mut notifications);
                    tokio::pin!(wait);
                    loop {
                        tokio::select! {
                            result = &mut wait => {
                                result.map_err(|error| QueueError::Redis(error.to_string()))?;
                                break;
                            }
                            _ = tokio::time::sleep(Duration::from_millis(100)) => {
                                if self.semaphore.is_closed() { return Ok(()); }
                            }
                        }
                    }
                    continue;
                };
                let handler = handler.clone();
                let mut conn = self.connection.clone();
                let (ready, pending, deadlines, dead, notify, published) = (
                    ready.clone(),
                    pending.clone(),
                    deadlines.clone(),
                    dead.clone(),
                    notify.clone(),
                    published.clone(),
                );
                let timeout = self
                    .delivery_timeout
                    .saturating_sub(Duration::from_secs(60));
                tokio::spawn(async move {
                    let _permit = permit;
                    let _queue_permit = queue_permit;
                    let outcome = match serde_json::from_str::<QueuedJob>(&payload) {
                        Ok(job) => match tokio::time::timeout(timeout, handler(job)).await {
                            Ok(Ok(outcome)) => Ok(outcome),
                            Ok(Err(_)) => Err("execution_failed_requires_reconciliation"),
                            Err(_) => Err("execution_timeout_requires_reconciliation"),
                        },
                        Err(_) => Err("invalid_dispatch_payload"),
                    };
                    if let Ok(QueueDisposition::NotAdmitted { retry_after }) = outcome {
                        // Keep the permit while backing off so a full fleet cannot
                        // repeatedly claim and reject at Redis round-trip speed.
                        let jitter = delivery_id
                            .bytes()
                            .fold(0u64, |sum, byte| sum + byte as u64)
                            % 100;
                        tokio::time::sleep(
                            retry_after.clamp(Duration::from_millis(100), Duration::from_secs(5))
                                + Duration::from_millis(jitter),
                        )
                        .await;
                        let result = Script::new(REQUEUE)
                            .key(pending)
                            .key(deadlines)
                            .key(ready)
                            .key(notify)
                            .arg(&delivery_id)
                            .invoke_async::<i32>(&mut conn)
                            .await;
                        match result {
                            Ok(1) => {
                                tracing::debug!(%delivery_id, "Execution not admitted; delivery returned to ready queue")
                            }
                            _ => {
                                tracing::error!(%delivery_id, "Non-admitted delivery retained for reconciliation after requeue failure")
                            }
                        }
                        return;
                    }
                    let reason = outcome.err().unwrap_or("");
                    let result = Script::new(COMPLETE)
                        .key(pending)
                        .key(deadlines)
                        .key(dead)
                        .key(published)
                        .arg(&delivery_id)
                        .arg(reason)
                        .invoke_async::<i32>(&mut conn)
                        .await;
                    match result {
                        Ok(1) if reason.is_empty() => {
                            tracing::info!(%delivery_id, "Execution acknowledged")
                        }
                        Ok(1) => tracing::warn!(%delivery_id, %reason, "Delivery quarantined"),
                        _ => {
                            tracing::error!(%delivery_id, "Delivery acknowledgement failed; retained for reconciliation")
                        }
                    }
                });
            }
        }

        /// Destructive single-message polling cannot acknowledge ownership.
        /// Consumers must use `run`, which retains delivery bytes until completion.
        pub async fn poll_one(&self) -> Result<Option<QueuedJob>, QueueError> {
            Err(QueueError::Execution(
                "poll_one is unsupported; use the acknowledged worker loop".into(),
            ))
        }

        pub async fn queue_length(&self) -> Result<usize, QueueError> {
            let mut conn = self.connection.clone();
            conn.llen(&self.config.queue_name)
                .await
                .map_err(|error| QueueError::Redis(error.to_string()))
        }
    }
}

#[cfg(feature = "redis")]
pub use worker::QueueWorker;

#[cfg(not(feature = "redis"))]
pub struct QueueWorker;

#[cfg(not(feature = "redis"))]
impl QueueWorker {
    pub async fn new(_config: QueueConfig) -> Result<Self, QueueError> {
        Err(QueueError::Redis("Redis feature not enabled".into()))
    }
}

#[cfg(all(test, feature = "redis"))]
mod integration_tests {
    use super::*;
    use redis::AsyncCommands;
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    use std::time::Duration;
    use tokio::sync::Semaphore;

    #[tokio::test]
    #[ignore = "requires a disposable Redis server in REDIS_TEST_URL"]
    async fn worker_wakes_retries_only_non_admission_and_drains() {
        let redis_url = std::env::var("REDIS_TEST_URL").expect("REDIS_TEST_URL is required");
        let ready = format!("exec:test-worker:{}", flow_like_types::create_id());
        let pending = format!("{ready}:pending");
        let deadlines = format!("{ready}:deadlines");
        let dead = format!("{ready}:dead");
        let notify = format!("{ready}:notify");
        let published = format!("{ready}:published");
        let mut connection = redis::Client::open(redis_url.clone())
            .unwrap()
            .get_multiplexed_async_connection()
            .await
            .unwrap();
        let capacity = Arc::new(Semaphore::new(4));
        let worker = QueueWorker::with_admission(
            QueueConfig {
                redis_url,
                queue_name: ready.clone(),
                concurrency: 1,
                poll_timeout_secs: 30,
            },
            capacity.clone(),
        )
        .await
        .unwrap();
        let attempts = Arc::new(AtomicUsize::new(0));
        let active = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));
        let (started, mut starts) = tokio::sync::mpsc::unbounded_channel();
        let (handler_attempts, handler_active, handler_peak) =
            (attempts.clone(), active.clone(), peak.clone());
        let task = tokio::spawn(async move {
            worker
                .run_with_outcomes(move |job| {
                    let (attempts, active, peak, started) = (
                        handler_attempts.clone(),
                        handler_active.clone(),
                        handler_peak.clone(),
                        started.clone(),
                    );
                    async move {
                        started.send(job.job_id.clone()).unwrap();
                        let active_count = active.fetch_add(1, Ordering::SeqCst) + 1;
                        peak.fetch_max(active_count, Ordering::SeqCst);
                        tokio::time::sleep(Duration::from_millis(20)).await;
                        active.fetch_sub(1, Ordering::SeqCst);
                        if job.job_id == "ambiguous" {
                            return Err("transport disconnected".into());
                        }
                        if attempts.fetch_add(1, Ordering::SeqCst) == 0 {
                            Ok(QueueDisposition::NotAdmitted {
                                retry_after: Duration::from_millis(100),
                            })
                        } else {
                            Ok(QueueDisposition::Completed)
                        }
                    }
                })
                .await
        });
        // Enter the idle wait. A 30-second fallback must not delay publication.
        tokio::time::sleep(Duration::from_millis(100)).await;
        for job_id in ["retry", "ambiguous", "complete"] {
            let payload = serde_json::json!({
                "job_id": job_id, "run_id": job_id, "app_id":"app", "board_id":"board",
                "node_id":"node", "user_id":"user", "credentials":{},
                "executor_jwt":"not-used-by-test-handler", "callback_url":"https://callback.example"
            })
            .to_string();
            let accepted: i32 = redis::Script::new(ENQUEUE_SCRIPT)
                .key(&ready)
                .key(&pending)
                .key(&dead)
                .key(&notify)
                .key(&published)
                .arg(payload)
                .arg(10)
                .invoke_async(&mut connection)
                .await
                .unwrap();
            assert_eq!(accepted, 1);
        }
        let first = tokio::time::timeout(Duration::from_secs(1), starts.recv())
            .await
            .unwrap();
        assert_eq!(first.as_deref(), Some("retry"));
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let pending_count: usize = connection.hlen(&pending).await.unwrap();
                let ready_count: usize = connection.llen(&ready).await.unwrap();
                if attempts.load(Ordering::SeqCst) == 3 && pending_count == 0 && ready_count == 0 {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .unwrap();
        assert_eq!(
            peak.load(Ordering::SeqCst),
            1,
            "queue concurrency is independent of shared HTTP capacity"
        );
        let failed: Vec<String> = connection.lrange(&dead, 0, -1).await.unwrap();
        assert_eq!(failed.len(), 1);
        let failed: serde_json::Value = serde_json::from_str(&failed[0]).unwrap();
        assert_eq!(failed["reason"], "execution_failed_requires_reconciliation");
        let payload: serde_json::Value =
            serde_json::from_str(failed["payload"].as_str().unwrap()).unwrap();
        assert_eq!(payload["job_id"], "ambiguous");
        assert_eq!(connection.hlen::<_, usize>(&published).await.unwrap(), 0);
        capacity.close();
        tokio::time::timeout(Duration::from_secs(1), task)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert_eq!(capacity.available_permits(), 4);
        let _: usize = connection
            .del(&[ready, pending, deadlines, dead, notify, published])
            .await
            .unwrap();
    }
}
