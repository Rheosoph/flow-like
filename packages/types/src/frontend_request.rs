//! Pending run→frontend requests (e.g. live micro-widget queries).
//!
//! Process-global like the interaction registry: the executing run registers
//! a request and awaits the oneshot; the host (Tauri command or API route)
//! resolves it with the frontend's response. First response wins — later
//! deliveries find no entry and report `false`.

use serde_json::Value;
use std::collections::HashMap;
use std::sync::LazyLock;
use tokio::sync::{Mutex, oneshot};

static PENDING_FRONTEND_REQUESTS: LazyLock<Mutex<HashMap<String, oneshot::Sender<Value>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// Register a pending request; await the receiver (with your own timeout) and
/// call [`abandon_frontend_request`] when giving up so the entry is cleaned.
pub async fn register_frontend_request(request_id: &str) -> oneshot::Receiver<Value> {
    let (sender, receiver) = oneshot::channel();
    let mut pending = PENDING_FRONTEND_REQUESTS.lock().await;
    pending.insert(request_id.to_string(), sender);
    receiver
}

/// Deliver a response to the awaiting run. Returns `false` when the request
/// is unknown, already resolved, or timed out.
pub async fn resolve_frontend_request(request_id: &str, response: Value) -> bool {
    let sender = {
        let mut pending = PENDING_FRONTEND_REQUESTS.lock().await;
        pending.remove(request_id)
    };
    match sender {
        Some(sender) => sender.send(response).is_ok(),
        None => false,
    }
}

/// Drop a pending request (timeout/cancellation cleanup).
pub async fn abandon_frontend_request(request_id: &str) {
    let mut pending = PENDING_FRONTEND_REQUESTS.lock().await;
    pending.remove(request_id);
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn test_resolve_roundtrip() {
        let receiver = register_frontend_request("req-a").await;
        assert!(resolve_frontend_request("req-a", json!({"ok": true, "value": 42})).await);
        let value = receiver.await.unwrap();
        assert_eq!(value["value"], 42);
    }

    #[tokio::test]
    async fn test_first_response_wins() {
        let receiver = register_frontend_request("req-b").await;
        assert!(resolve_frontend_request("req-b", json!({"ok": true})).await);
        assert!(!resolve_frontend_request("req-b", json!({"ok": false})).await);
        assert!(receiver.await.unwrap()["ok"].as_bool().unwrap());
    }

    #[tokio::test]
    async fn test_unknown_and_abandoned() {
        assert!(!resolve_frontend_request("req-missing", json!({})).await);

        let receiver = register_frontend_request("req-c").await;
        abandon_frontend_request("req-c").await;
        assert!(!resolve_frontend_request("req-c", json!({})).await);
        assert!(receiver.await.is_err());
    }

    #[tokio::test]
    async fn test_dropped_receiver_reports_unresolved() {
        let receiver = register_frontend_request("req-d").await;
        drop(receiver);
        assert!(!resolve_frontend_request("req-d", json!({})).await);
    }
}
