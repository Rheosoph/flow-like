//! What the bots need from the program that runs them (§1.12).

use async_trait::async_trait;
use serde_json::Value;
use tokio::sync::{mpsc, watch};
use tokio_util::sync::CancellationToken;

use crate::state::BotsState;

/// Whether a bot may connect. `Held` carries the wire code of the hold.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Gate {
    Undecided,
    Open,
    Held(&'static str),
}

/// How one run ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RunEnd {
    Succeeded,
    Failed,
    Cancelled,
    TimedOut,
}

/// One event a run sent to its caller (`chat_stream_partial`, `chat_stream`, `chat_out`, …).
#[derive(Clone, Debug, PartialEq)]
pub struct StreamEvent {
    pub kind: String,
    pub payload: Value,
}

impl StreamEvent {
    pub fn new(kind: impl Into<String>, payload: Value) -> Self {
        Self {
            kind: kind.into(),
            payload,
        }
    }
}

#[async_trait]
pub trait BotHost: Send + Sync + 'static {
    /// One run of `event_id` with a Chat payload; the run's events arrive on `events`.
    async fn run(
        &self,
        event_id: &str,
        payload: Value,
        events: mpsc::Sender<StreamEvent>,
        cancel: CancellationToken,
    ) -> RunEnd;
    fn gate(&self, event_id: &str) -> watch::Receiver<Gate>;
    fn load(&self) -> Option<BotsState>;
    fn save(&self, state: &BotsState) -> std::io::Result<()>;
    /// The device's wall clock, Unix seconds.
    fn now(&self) -> i64;
    /// What a flow receives in place of the token.
    fn handle(&self, event_id: &str) -> String;
    /// Telegram: an update reached the poller. True when a waiting flow node took it.
    fn observed(&self, _event_id: &str, _update: &Value) -> bool {
        false
    }
}
