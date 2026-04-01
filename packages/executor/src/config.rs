use serde::{Deserialize, Serialize};
use std::time::Duration;

/// Configuration for the executor
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutorConfig {
    /// Interval for batching events before sending to callback (milliseconds)
    #[serde(default = "default_batch_interval_ms")]
    pub batch_interval_ms: u64,
    /// Maximum events to batch before forcing a send
    #[serde(default = "default_max_batch_size")]
    pub max_batch_size: usize,
    /// Timeout for callback HTTP requests (milliseconds)
    #[serde(default = "default_callback_timeout_ms")]
    pub callback_timeout_ms: u64,
    /// Number of retries for failed callback requests
    #[serde(default = "default_callback_retries")]
    pub callback_retries: u32,
    /// Execution timeout (seconds)
    #[serde(default = "default_execution_timeout_secs")]
    pub execution_timeout_secs: u64,
}

fn default_batch_interval_ms() -> u64 {
    1000
}
fn default_max_batch_size() -> usize {
    100
}
fn default_callback_timeout_ms() -> u64 {
    5000
}
fn default_callback_retries() -> u32 {
    3
}
fn default_execution_timeout_secs() -> u64 {
    3600
}

impl Default for ExecutorConfig {
    fn default() -> Self {
        Self {
            batch_interval_ms: default_batch_interval_ms(),
            max_batch_size: default_max_batch_size(),
            callback_timeout_ms: default_callback_timeout_ms(),
            callback_retries: default_callback_retries(),
            execution_timeout_secs: default_execution_timeout_secs(),
        }
    }
}

impl ExecutorConfig {
    pub fn from_env() -> Self {
        Self {
            batch_interval_ms: std::env::var("EXECUTOR_BATCH_INTERVAL_MS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or_else(default_batch_interval_ms),
            max_batch_size: std::env::var("EXECUTOR_MAX_BATCH_SIZE")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or_else(default_max_batch_size),
            callback_timeout_ms: std::env::var("EXECUTOR_CALLBACK_TIMEOUT_MS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or_else(default_callback_timeout_ms),
            callback_retries: std::env::var("EXECUTOR_CALLBACK_RETRIES")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or_else(default_callback_retries),
            execution_timeout_secs: std::env::var("EXECUTOR_TIMEOUT_SECS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or_else(default_execution_timeout_secs),
        }
    }

    pub fn batch_interval(&self) -> Duration {
        Duration::from_millis(self.batch_interval_ms)
    }

    pub fn callback_timeout(&self) -> Duration {
        Duration::from_millis(self.callback_timeout_ms)
    }

    pub fn execution_timeout(&self) -> Duration {
        Duration::from_secs(self.execution_timeout_secs)
    }
}
