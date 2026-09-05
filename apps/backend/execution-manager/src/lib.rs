//! Trusted supervision and egress enforcement, separate from tenant execution.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{sync::Arc, time::Duration};
use tokio::sync::{Mutex, mpsc};

pub mod config;
pub mod docker;
pub mod gateway;
pub mod kubernetes;
pub mod server;
pub use config::CommonConfig;
pub use flow_like_types_contracts::dispatch::DispatchPayload as Dispatch;

pub const MAX_INPUT: usize = 8 * 1024 * 1024;
pub const MAX_EVENT: usize = 1024 * 1024;
pub const MAX_OUTPUT: usize = 64 * 1024 * 1024;
pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{0}")]
    Invalid(String),
    #[error("No ready execution capacity")]
    NoCapacity,
    #[error("Execution supervision unavailable")]
    Unavailable,
    #[error("Execution cancelled or already assigned")]
    Cancelled,
    #[error("Execution interrupted: {0}")]
    Internal(String),
}

impl Error {
    pub fn internal(error: impl std::fmt::Display) -> Self {
        Self::Internal(error.to_string())
    }
    pub fn invalid(error: impl std::fmt::Display) -> Self {
        Self::Invalid(error.to_string())
    }
}

macro_rules! internal_error {
    ($($ty:ty),+ $(,)?) => {$(impl From<$ty> for Error {
        fn from(error: $ty) -> Self { Self::internal(error) }
    })+};
}
internal_error!(
    std::io::Error,
    serde_json::Error,
    reqwest::Error,
    redis::RedisError,
    rusqlite::Error,
    hyper::Error,
    axum::http::Error,
    tokio::task::JoinError,
    tokio::time::error::Elapsed,
    url::ParseError
);

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum Mode {
    #[serde(rename = "callback")]
    Callback,
    #[serde(rename = "callback-queued")]
    CallbackQueued,
    #[serde(rename = "stream")]
    Stream,
}
impl Mode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Callback => "callback",
            Self::CallbackQueued => "callback-queued",
            Self::Stream => "stream",
        }
    }
}

/// A slow or disconnected viewer cannot retain execution capacity indefinitely.
/// Once delivery stalls, close that stream and let the accepted run continue.
#[derive(Clone, Default)]
pub struct EventSink(Arc<Mutex<Option<mpsc::Sender<Value>>>>);
impl EventSink {
    pub fn new(sender: mpsc::Sender<Value>) -> Self {
        Self(Arc::new(Mutex::new(Some(sender))))
    }
    pub async fn send(&self, event: Value) {
        let mut sender = self.0.lock().await;
        if let Some(channel) = sender.as_ref()
            && !matches!(
                tokio::time::timeout(Duration::from_secs(1), channel.send(event)).await,
                Ok(Ok(()))
            )
        {
            *sender = None;
        }
    }
}

#[async_trait]
pub trait Reservation: Send {
    async fn execute(
        self: Box<Self>,
        payload: Dispatch,
        mode: Mode,
        events: EventSink,
    ) -> Result<Value>;
}

#[async_trait]
pub trait Backend: Send + Sync {
    fn ready(&self) -> bool;
    fn metrics(&self) -> String;
    async fn prepare(self: Arc<Self>) -> Result<()>;
    async fn reserve(&self, payload: &Dispatch) -> Result<Box<dyn Reservation>>;
    async fn cancel(&self, run_id: &str) -> Result<Value>;
    async fn shutdown(&self) -> Result<()>;
}
