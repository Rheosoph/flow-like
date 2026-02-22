//! Redis job queue consumer for async execution dispatch.
//!
//! This module provides a worker that polls a Redis queue for execution jobs
//! and processes them using the flow-like-executor.
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
//! REDIS_EXECUTION_QUEUE=exec:jobs
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
    /// BRPOP timeout in seconds (0 = infinite)
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
                .unwrap_or_else(|_| "exec:jobs".into()),
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

/// Job payload from the queue — type alias for the shared dispatch payload.
pub type QueuedJob = flow_like_types::dispatch::DispatchPayload;

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

#[cfg(feature = "redis")]
mod worker {
    use super::*;
    use redis::{AsyncCommands, Client};
    use std::sync::Arc;
    use tokio::sync::Semaphore;

    /// Redis queue worker
    pub struct QueueWorker {
        config: QueueConfig,
        client: Client,
        semaphore: Arc<Semaphore>,
    }

    impl QueueWorker {
        pub async fn new(config: QueueConfig) -> Result<Self, QueueError> {
            let client = Client::open(config.redis_url.as_str())
                .map_err(|e| QueueError::Redis(e.to_string()))?;

            let semaphore = Arc::new(Semaphore::new(config.concurrency));

            Ok(Self {
                config,
                client,
                semaphore,
            })
        }

        /// Run the worker loop, polling for jobs until shutdown
        pub async fn run<F, Fut>(&self, handler: F) -> Result<(), QueueError>
        where
            F: Fn(QueuedJob) -> Fut + Send + Sync + Clone + 'static,
            Fut: std::future::Future<Output = Result<(), String>> + Send + 'static,
        {
            tracing::info!(
                queue = %self.config.queue_name,
                concurrency = %self.config.concurrency,
                "Starting Redis queue worker"
            );

            loop {
                // Acquire semaphore permit before polling
                let permit: Option<tokio::sync::OwnedSemaphorePermit> =
                    self.semaphore.clone().acquire_owned().await.ok();
                if permit.is_none() {
                    continue;
                }

                let mut conn = match self.client.get_multiplexed_async_connection().await {
                    Ok(c) => c,
                    Err(e) => {
                        tracing::error!(error = %e, "Failed to connect to Redis");
                        tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
                        continue;
                    }
                };

                // BRPOP blocks until a job is available or timeout
                let result: Result<Option<(String, String)>, _> = conn
                    .brpop(
                        &self.config.queue_name,
                        self.config.poll_timeout_secs as f64,
                    )
                    .await;

                match result {
                    Ok(Some((_queue, job_json))) => {
                        let handler = handler.clone();
                        let permit = permit.unwrap();

                        // Spawn task to handle job
                        tokio::spawn(async move {
                            match serde_json::from_str::<QueuedJob>(&job_json) {
                                Ok(job) => {
                                    let job_id = job.job_id.clone();
                                    let run_id = job.run_id.clone();

                                    tracing::info!(job_id = %job_id, run_id = %run_id, "Processing job");

                                    if let Err(e) = handler(job).await {
                                        tracing::error!(
                                            job_id = %job_id,
                                            run_id = %run_id,
                                            error = %e,
                                            "Job execution failed"
                                        );
                                    } else {
                                        tracing::info!(job_id = %job_id, run_id = %run_id, "Job completed");
                                    }
                                }
                                Err(e) => {
                                    tracing::error!(error = %e, "Failed to parse job payload");
                                }
                            }
                            drop(permit);
                        });
                    }
                    Ok(None) => {
                        // Timeout, continue polling
                    }
                    Err(e) => {
                        tracing::error!(error = %e, "Redis BRPOP error");
                        tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
                    }
                }
            }
        }

        /// Poll for a single job (non-blocking with timeout)
        pub async fn poll_one(&self) -> Result<Option<QueuedJob>, QueueError> {
            let mut conn = self
                .client
                .get_multiplexed_async_connection()
                .await
                .map_err(|e| QueueError::Redis(e.to_string()))?;

            let result: Option<(String, String)> = conn
                .brpop(&self.config.queue_name, 1.0)
                .await
                .map_err(|e| QueueError::Redis(e.to_string()))?;

            match result {
                Some((_queue, job_json)) => {
                    let job: QueuedJob = serde_json::from_str(&job_json)
                        .map_err(|e| QueueError::Serialization(e.to_string()))?;
                    Ok(Some(job))
                }
                None => Ok(None),
            }
        }

        /// Get the number of jobs currently in the queue
        pub async fn queue_length(&self) -> Result<usize, QueueError> {
            let mut conn = self
                .client
                .get_multiplexed_async_connection()
                .await
                .map_err(|e| QueueError::Redis(e.to_string()))?;

            let len: usize = conn
                .llen(&self.config.queue_name)
                .await
                .map_err(|e| QueueError::Redis(e.to_string()))?;

            Ok(len)
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
