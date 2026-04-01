use serde::Serialize;

pub mod a2ui;
pub mod ai;
pub mod app;
pub mod bit;
pub mod developer;
pub mod download;
pub mod event_sink_commands;
pub mod feedback;
pub mod file;
pub mod flow;
pub mod interaction;
pub mod permissions;
pub mod recording;
pub mod registry;
pub mod settings;
pub mod statistics;
pub mod system;
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
