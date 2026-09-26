//! Logging host functions
//!
//! Provides structured logging from WASM modules.

use super::LogEntry;
use serde_json::Value;

/// Guest log levels share the run log's numbering, which is also the scale of
/// the `log_level` threshold sent to guests: Debug = 0 through Fatal = 4.
pub use flow_like::flow::execution::LogLevel;

impl LogEntry {
    pub fn new(level: LogLevel, message: String) -> Self {
        Self {
            level: level as u8,
            message,
            data: None,
        }
    }

    pub fn with_data(level: LogLevel, message: String, data: Value) -> Self {
        Self {
            level: level as u8,
            message,
            data: Some(data),
        }
    }

    pub fn level(&self) -> LogLevel {
        LogLevel::from_u8(self.level.min(LogLevel::Fatal as u8))
    }
}
