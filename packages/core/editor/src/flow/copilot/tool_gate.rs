//! Holds a bot turn's tool calls until its host knows the turn answers at all. A host that starts
//! the reply alongside its participation decision passes the gate in [`super::bot_run::BotRunContext`];
//! no tool runs, and no later round starts, before the decision lets the turn act.

use std::sync::Arc;

use async_trait::async_trait;
use flow_like_types::tokio::sync::watch;
use serde_json::Value;

use super::platform::PlatformToolBridge;

const HELD_BACK: &str = "{\"error\":\"turn_not_answering\",\"retryable\":false,\"message\":\"This turn stopped before the tool ran; nothing was run.\"}";

/// The host's side of a [`ToolGate`]. Dropping it without a decision keeps the gate shut.
#[derive(Debug)]
pub struct ToolGateOpener(watch::Sender<Option<bool>>);

impl ToolGateOpener {
    /// The turn answers: held and later tool calls run.
    pub fn open(&self) {
        self.0.send_replace(Some(true));
    }

    /// The turn does not answer: held and later tool calls are refused and the run stops.
    pub fn shut(&self) {
        self.0.send_replace(Some(false));
    }
}

#[derive(Clone, Debug)]
pub struct ToolGate(watch::Receiver<Option<bool>>);

/// A gate that holds every tool call until its opener decides.
pub fn tool_gate() -> (ToolGateOpener, ToolGate) {
    let (sender, receiver) = watch::channel(None);
    (ToolGateOpener(sender), ToolGate(receiver))
}

impl ToolGate {
    /// Waits for the host's decision: true when the turn may act.
    pub async fn passes(&self) -> bool {
        let mut receiver = self.0.clone();
        match receiver.wait_for(Option::is_some).await {
            Ok(decision) => decision.unwrap_or(false),
            Err(_) => false,
        }
    }

    /// True once the host decided the turn does not answer, or dropped the opener undecided.
    pub fn is_shut(&self) -> bool {
        let decision = *self.0.borrow();
        match decision {
            Some(answers) => !answers,
            None => self.0.has_changed().is_err(),
        }
    }
}

impl PartialEq for ToolGate {
    fn eq(&self, other: &Self) -> bool {
        self.0.same_channel(&other.0)
    }
}

impl Eq for ToolGate {}

/// Applies a [`ToolGate`] to every tool call of a turn.
pub(super) struct GatedBridge {
    gate: ToolGate,
    inner: Arc<dyn PlatformToolBridge>,
}

impl GatedBridge {
    pub(super) fn new(gate: ToolGate, inner: Arc<dyn PlatformToolBridge>) -> Self {
        Self { gate, inner }
    }
}

#[async_trait]
impl PlatformToolBridge for GatedBridge {
    async fn call(&self, tool_name: &str, arguments: Value) -> String {
        if !self.gate.passes().await {
            return HELD_BACK.to_string();
        }
        self.inner.call(tool_name, arguments).await
    }

    fn supports_tool(&self, tool_name: &str) -> bool {
        self.inner.supports_tool(tool_name)
    }

    async fn drain_steering(&self) -> Vec<String> {
        self.inner.drain_steering().await
    }

    fn watches_room(&self) -> bool {
        self.inner.watches_room()
    }

    async fn drain_room_updates(&self) -> Vec<String> {
        self.inner.drain_room_updates().await
    }

    async fn is_cancelled(&self) -> bool {
        self.gate.is_shut() || self.inner.is_cancelled().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[derive(Default)]
    struct Counting(AtomicUsize);

    #[async_trait]
    impl PlatformToolBridge for Counting {
        async fn call(&self, _tool_name: &str, _arguments: Value) -> String {
            self.0.fetch_add(1, Ordering::SeqCst);
            "ran".into()
        }
    }

    #[tokio::test]
    async fn a_held_call_runs_once_the_turn_answers() {
        let (opener, gate) = tool_gate();
        let inner = Arc::new(Counting::default());
        let bridge = GatedBridge::new(gate, inner.clone());
        let call = tokio::spawn(async move {
            bridge.call("web_search", Value::Null).await
        });
        tokio::task::yield_now().await;
        assert_eq!(inner.0.load(Ordering::SeqCst), 0);
        opener.open();
        assert_eq!(call.await.unwrap(), "ran");
        assert_eq!(inner.0.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn a_turn_that_stays_quiet_runs_no_tool_and_stops() {
        let (opener, gate) = tool_gate();
        let inner = Arc::new(Counting::default());
        let bridge = GatedBridge::new(gate, inner.clone());
        assert!(!bridge.is_cancelled().await);
        opener.shut();
        assert_eq!(bridge.call("web_search", Value::Null).await, HELD_BACK);
        assert!(bridge.is_cancelled().await);
        assert_eq!(inner.0.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn a_dropped_opener_keeps_the_gate_shut() {
        let (opener, gate) = tool_gate();
        let inner = Arc::new(Counting::default());
        let bridge = GatedBridge::new(gate, inner.clone());
        drop(opener);
        assert_eq!(bridge.call("web_search", Value::Null).await, HELD_BACK);
        assert!(bridge.is_cancelled().await);
        assert_eq!(inner.0.load(Ordering::SeqCst), 0);
    }
}
