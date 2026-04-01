use serde::{Deserialize, Serialize};
use std::time::Duration;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompilerConfig {
    #[serde(default = "default_callback_timeout_ms")]
    pub callback_timeout_ms: u64,
    #[serde(default = "default_callback_retries")]
    pub callback_retries: u32,
    #[serde(default = "default_compilation_timeout_secs")]
    pub compilation_timeout_secs: u64,
    #[serde(default)]
    pub max_parallel_targets: Option<usize>,
}

fn default_callback_timeout_ms() -> u64 {
    10_000
}
fn default_callback_retries() -> u32 {
    3
}
fn default_compilation_timeout_secs() -> u64 {
    600
}

impl Default for CompilerConfig {
    fn default() -> Self {
        Self {
            callback_timeout_ms: default_callback_timeout_ms(),
            callback_retries: default_callback_retries(),
            compilation_timeout_secs: default_compilation_timeout_secs(),
            max_parallel_targets: None,
        }
    }
}

impl CompilerConfig {
    pub fn from_env() -> Self {
        Self {
            callback_timeout_ms: std::env::var("COMPILER_CALLBACK_TIMEOUT_MS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or_else(default_callback_timeout_ms),
            callback_retries: std::env::var("COMPILER_CALLBACK_RETRIES")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or_else(default_callback_retries),
            compilation_timeout_secs: std::env::var("COMPILER_TIMEOUT_SECS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or_else(default_compilation_timeout_secs),
            max_parallel_targets: std::env::var("COMPILER_MAX_PARALLEL_TARGETS")
                .ok()
                .and_then(|v| v.parse().ok()),
        }
    }

    pub fn with_timeout_secs(mut self, secs: u64) -> Self {
        self.compilation_timeout_secs = secs;
        self
    }

    pub fn callback_timeout(&self) -> Duration {
        Duration::from_millis(self.callback_timeout_ms)
    }

    pub fn compilation_timeout(&self) -> Duration {
        Duration::from_secs(self.compilation_timeout_secs)
    }
}
