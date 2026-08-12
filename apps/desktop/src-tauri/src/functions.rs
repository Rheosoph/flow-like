use serde::Serialize;

pub mod a2ui;
pub mod ai;
pub mod app;
pub mod bit;
pub mod developer;
pub mod device_id;
pub mod download;
pub mod event_sink_commands;
pub mod feedback;
pub mod file;
pub mod flow;
pub mod interaction;
pub mod notifications;
pub mod permissions;
pub mod recording;
pub mod registry;
pub mod settings;
pub mod statistics;
pub mod storage_management;
pub mod system;
pub mod telemetry;
pub mod tmp;

#[derive(Debug, Serialize)]
pub struct TauriFunctionError {
    error: String,
}

impl TauriFunctionError {
    pub fn new(error: &str) -> Self {
        Self {
            error: error.to_string(),
        }
    }
}

impl std::fmt::Display for TauriFunctionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.error)
    }
}

impl std::error::Error for TauriFunctionError {}

// impl From<flow_like::flow_like_storage::async_duckdb::Error> for TauriFunctionError {
//     fn from(error: flow_like::flow_like_storage::async_duckdb::Error) -> Self {
//         Self {
//             error: error.to_string(),
//         }
//     }
// }

impl From<flow_like_types::Error> for TauriFunctionError {
    fn from(error: flow_like_types::Error) -> Self {
        Self {
            error: error.to_string(),
        }
    }
}

impl From<serde_json::Error> for TauriFunctionError {
    fn from(error: serde_json::Error) -> Self {
        Self {
            error: error.to_string(),
        }
    }
}
