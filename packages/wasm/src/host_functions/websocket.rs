//! WebSocket client connections owned by one package instance in one run.
//!
//! Guests hold opaque references. Background I/O keeps making progress between
//! node calls, and run cleanup cancels every task even if a guest retains a handle.

use flow_like::flow::execution::{egress, ExecutionEnvironment};
use futures::{SinkExt, StreamExt};
use parking_lot::Mutex;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::TcpStream;
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;
use tokio_tungstenite::tungstenite::{
    client::IntoClientRequest, protocol::WebSocketConfig, Message,
};
use tokio_tungstenite::WebSocketStream;

const MAX_RESOURCES: usize = 128;
const QUEUE_CAPACITY: usize = 32;
const MAX_MESSAGE_BYTES: usize = 1024 * 1024;
const IO_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, Default)]
struct OwnedTask(Mutex<Option<JoinHandle<()>>>);

impl OwnedTask {
    fn abort(&self) {
        if let Some(task) = self.0.lock().as_ref() {
            task.abort();
        }
    }

    async fn shutdown(&self) {
        let task = self.0.lock().take();
        if let Some(task) = task {
            task.abort();
            let _ = task.await;
        }
    }
}

impl Drop for OwnedTask {
    fn drop(&mut self) {
        if let Some(task) = self.0.get_mut().take() {
            task.abort();
        }
    }
}

#[derive(Debug)]
struct SendCommand {
    message: Message,
    result: oneshot::Sender<bool>,
}

#[derive(Debug)]
struct WsConnection {
    outgoing: mpsc::Sender<SendCommand>,
    incoming: tokio::sync::Mutex<mpsc::Receiver<Message>>,
    task: OwnedTask,
}

#[derive(Debug, Default)]
struct ResourceState {
    closed: bool,
    resources: HashMap<String, Arc<WsConnection>>,
    legacy_handles: HashMap<i32, String>,
}

/// This registry must never be reused by another package or another run.
#[derive(Debug, Default)]
pub struct WebSocketResources {
    state: Mutex<ResourceState>,
}

impl WebSocketResources {
    /// Cancel immediately. `shutdown` also waits until the sockets are released.
    pub fn cancel(&self) {
        let mut state = self.state.lock();
        state.closed = true;
        for resource in state.resources.values() {
            resource.task.abort();
        }
    }

    /// Permanently invalidate references and join every resource's I/O task.
    pub async fn shutdown(&self) {
        let resources = {
            let mut state = self.state.lock();
            state.closed = true;
            state.legacy_handles.clear();
            std::mem::take(&mut state.resources)
        };
        for resource in resources.values() {
            resource.task.abort();
        }
        for resource in resources.values() {
            resource.task.shutdown().await;
        }
    }

    fn get(&self, id: &str) -> Option<Arc<WsConnection>> {
        let state = self.state.lock();
        if state.closed {
            return None;
        }
        state.resources.get(id).cloned()
    }

    fn insert_connection<S>(&self, socket: WebSocketStream<S>) -> Option<String>
    where
        S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
    {
        let mut state = self.state.lock();
        if state.closed || state.resources.len() >= MAX_RESOURCES {
            return None;
        }
        let id = new_reference("ws")?;
        let (outgoing, mut commands) = mpsc::channel::<SendCommand>(QUEUE_CAPACITY);
        let (messages, incoming) = mpsc::channel(QUEUE_CAPACITY);
        let task = tokio::spawn(async move {
            let (mut sink, mut stream) = socket.split();
            loop {
                tokio::select! {
                    command = commands.recv() => {
                        let Some(command) = command else { break };
                        let sent = matches!(tokio::time::timeout(IO_TIMEOUT, sink.send(command.message)).await, Ok(Ok(())));
                        let _ = command.result.send(sent);
                        if !sent { break; }
                    }
                    message = stream.next() => {
                        let Some(Ok(message)) = message else { break };
                        let closed = matches!(message, Message::Close(_));
                        let flush = matches!(message, Message::Ping(_) | Message::Close(_));
                        // A slow guest may fill its queue. Close this connection
                        // instead of growing memory or blocking other sockets.
                        if messages.try_send(message).is_err() { break; }
                        if flush && !matches!(tokio::time::timeout(IO_TIMEOUT, sink.flush()).await, Ok(Ok(()))) {
                            break;
                        }
                        if closed { break; }
                    }
                }
            }
        });
        state.resources.insert(
            id.clone(),
            Arc::new(WsConnection {
                outgoing,
                incoming: tokio::sync::Mutex::new(incoming),
                task: OwnedTask(Mutex::new(Some(task))),
            }),
        );
        Some(id)
    }

    /// Resolve through the execution egress policy and connect to those addresses.
    pub async fn connect(
        &self,
        environment: ExecutionEnvironment,
        allowed_hosts: Option<&[String]>,
        url: &str,
        headers_json: &str,
    ) -> Option<String> {
        if self.state.lock().closed {
            return None;
        }
        let mut request = url.into_client_request().ok()?;
        let uri = request.uri().clone();
        let scheme = uri.scheme_str()?;
        if scheme != "ws" && scheme != "wss" {
            return None;
        }
        let host = uri.host()?.trim_matches(['[', ']']);
        if !host_allowed(allowed_hosts, host) {
            return None;
        }
        let headers: HashMap<String, String> = serde_json::from_str(headers_json).ok()?;
        for (name, value) in headers {
            let name = tokio_tungstenite::tungstenite::http::HeaderName::try_from(name).ok()?;
            // Keep routing and the WebSocket handshake under host control.
            if matches!(name.as_str(), "host" | "connection" | "upgrade")
                || name.as_str().starts_with("sec-websocket-")
            {
                return None;
            }
            let value = tokio_tungstenite::tungstenite::http::HeaderValue::try_from(value).ok()?;
            request.headers_mut().insert(name, value);
        }
        let port = uri
            .port_u16()
            .unwrap_or(if scheme == "wss" { 443 } else { 80 });
        let socket = tokio::time::timeout(IO_TIMEOUT, async {
            let addresses = egress::resolve_socket_addrs(environment, host, port)
                .await
                .ok()?;
            let tcp = TcpStream::connect(addresses.as_slice()).await.ok()?;
            let (socket, _) = tokio_tungstenite::client_async_tls_with_config(
                request,
                tcp,
                Some(socket_config()),
                None,
            )
            .await
            .ok()?;
            Some(socket)
        })
        .await
        .ok()??;
        self.insert_connection(socket)
    }

    pub async fn send(&self, connection_id: &str, bytes: Vec<u8>, binary: bool) -> bool {
        if bytes.len() > MAX_MESSAGE_BYTES {
            return false;
        }
        let Some(connection) = self.get(connection_id) else {
            return false;
        };
        let message = if binary {
            Message::Binary(bytes.into())
        } else {
            let Ok(text) = String::from_utf8(bytes) else {
                return false;
            };
            Message::Text(text.into())
        };
        let (result_tx, result_rx) = oneshot::channel();
        tokio::time::timeout(IO_TIMEOUT, async {
            connection
                .outgoing
                .send(SendCommand {
                    message,
                    result: result_tx,
                })
                .await
                .ok()?;
            result_rx.await.ok()
        })
        .await
        .ok()
        .flatten()
        .unwrap_or(false)
    }

    pub async fn receive(&self, connection_id: &str, timeout_ms: u32) -> Option<String> {
        let connection = self.get(connection_id)?;
        let message = tokio::time::timeout(Duration::from_millis(timeout_ms as u64), async {
            connection.incoming.lock().await.recv().await
        })
        .await
        .ok()??;
        let (kind, data) = match message {
            Message::Text(text) => ("text", text.to_string()),
            Message::Binary(bytes) => (
                "binary",
                base64::Engine::encode(&base64::engine::general_purpose::STANDARD, bytes),
            ),
            Message::Close(frame) => (
                "close",
                frame.map(|f| f.reason.to_string()).unwrap_or_default(),
            ),
            Message::Ping(bytes) => ("ping", String::from_utf8_lossy(&bytes).into_owned()),
            Message::Pong(bytes) => ("pong", String::from_utf8_lossy(&bytes).into_owned()),
            _ => return None,
        };
        Some(serde_json::json!({ "type": kind, "data": data }).to_string())
    }

    pub async fn close(&self, id: &str) -> bool {
        let removed = {
            let mut state = self.state.lock();
            if state.closed {
                return false;
            }
            let Some(resource) = state.resources.remove(id) else {
                return false;
            };
            state.legacy_handles.retain(|_, value| value != id);
            resource
        };
        removed.task.shutdown().await;
        true
    }

    /// Compatibility for the original core-module ABI's numeric handles.
    pub fn legacy_handle(&self, reference: &str) -> Option<i32> {
        let mut state = self.state.lock();
        if state.closed || !state.resources.contains_key(reference) {
            return None;
        }
        if let Some((handle, _)) = state
            .legacy_handles
            .iter()
            .find(|(_, value)| *value == reference)
        {
            return Some(*handle);
        }
        loop {
            let mut bytes = [0; 4];
            getrandom::fill(&mut bytes).ok()?;
            let handle = i32::from_ne_bytes(bytes) & i32::MAX;
            if handle != 0 && !state.legacy_handles.contains_key(&handle) {
                state.legacy_handles.insert(handle, reference.to_owned());
                return Some(handle);
            }
        }
    }

    pub fn legacy_reference(&self, handle: i32) -> Option<String> {
        let state = self.state.lock();
        if state.closed {
            return None;
        }
        state.legacy_handles.get(&handle).cloned()
    }
}

impl Drop for WebSocketResources {
    fn drop(&mut self) {
        self.cancel();
    }
}

fn new_reference(prefix: &str) -> Option<String> {
    let mut bytes = [0; 16];
    getrandom::fill(&mut bytes).ok()?;
    Some(format!("{prefix}_{:032x}", u128::from_ne_bytes(bytes)))
}

fn socket_config() -> WebSocketConfig {
    WebSocketConfig::default()
        .max_message_size(Some(MAX_MESSAGE_BYTES))
        .max_frame_size(Some(MAX_MESSAGE_BYTES))
}

fn host_allowed(allowed_hosts: Option<&[String]>, host: &str) -> bool {
    allowed_hosts.is_none_or(|hosts| {
        hosts
            .iter()
            .any(|allowed| allowed.eq_ignore_ascii_case(host))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::net::TcpListener;

    async fn connected_pair(registry: &WebSocketResources) -> (String, WebSocketStream<TcpStream>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("ws://{}", listener.local_addr().unwrap());
        tokio::time::timeout(Duration::from_secs(2), async {
            let (connection, peer) = tokio::join!(
                registry.connect(ExecutionEnvironment::Local, None, &url, "{}"),
                async {
                    let (stream, _) = listener.accept().await.unwrap();
                    tokio_tungstenite::accept_async(stream).await.unwrap()
                }
            );
            (connection.expect("client connection"), peer)
        })
        .await
        .expect("local WebSocket handshake")
    }

    #[tokio::test]
    async fn connection_survives_individual_calls_then_shutdown() {
        let registry = WebSocketResources::default();
        let (connection, mut peer) = connected_pair(&registry).await;
        assert!(
            registry
                .send(&connection, b"from another node".to_vec(), false)
                .await
        );
        assert_eq!(
            peer.next().await.unwrap().unwrap().into_text().unwrap(),
            "from another node"
        );
        peer.send(Message::Text("reply".into())).await.unwrap();
        let reply: serde_json::Value =
            serde_json::from_str(&registry.receive(&connection, 1_000).await.unwrap()).unwrap();
        assert_eq!(reply["data"], "reply");
        registry.shutdown().await;
        assert!(!registry.send(&connection, vec![1], true).await);
        assert!(registry
            .connect(ExecutionEnvironment::Local, None, "ws://127.0.0.1:1", "{}")
            .await
            .is_none());
        let closed = tokio::time::timeout(Duration::from_secs(1), peer.next())
            .await
            .expect("peer socket closed");
        assert!(!matches!(
            closed,
            Some(Ok(Message::Text(_) | Message::Binary(_)))
        ));
    }

    #[tokio::test]
    async fn pending_receive_does_not_block_send_or_another_connection() {
        let registry = Arc::new(WebSocketResources::default());
        let (connection, mut peer) = connected_pair(&registry).await;
        let receiving_registry = registry.clone();
        let receiving_connection = connection.clone();
        let receive = tokio::spawn(async move {
            receiving_registry
                .receive(&receiving_connection, 5_000)
                .await
        });
        tokio::task::yield_now().await;
        let (other_connection, mut other_peer) = connected_pair(&registry).await;
        assert!(
            registry
                .send(&other_connection, b"independent".to_vec(), false)
                .await
        );
        assert_eq!(
            other_peer
                .next()
                .await
                .unwrap()
                .unwrap()
                .into_text()
                .unwrap(),
            "independent"
        );
        assert!(tokio::time::timeout(
            Duration::from_secs(1),
            registry.send(&connection, b"request".to_vec(), false)
        )
        .await
        .unwrap());
        assert_eq!(
            peer.next().await.unwrap().unwrap().into_text().unwrap(),
            "request"
        );
        peer.send(Message::Text("response".into())).await.unwrap();
        assert!(receive.await.unwrap().unwrap().contains("response"));
        registry.shutdown().await;
    }

    #[tokio::test]
    async fn references_cannot_cross_registries_and_close_invalidates_handles() {
        let first = WebSocketResources::default();
        let second = WebSocketResources::default();
        let (connection, mut peer) = connected_pair(&first).await;
        let legacy = first.legacy_handle(&connection).unwrap();
        assert!(!second.send(&connection, vec![1], true).await);
        assert!(second.legacy_reference(legacy).is_none());
        assert!(first.close(&connection).await);
        assert!(!first.send(&connection, vec![1], true).await);
        assert!(first.legacy_reference(legacy).is_none());
        tokio::time::timeout(Duration::from_secs(1), peer.next())
            .await
            .expect("peer socket closed");
    }

    #[tokio::test]
    async fn cancel_and_drop_stop_connections_even_with_leaked_references() {
        let registry = WebSocketResources::default();
        let (connection, mut peer) = connected_pair(&registry).await;
        let retained_connection = registry.get(&connection).unwrap();
        registry.cancel();
        assert!(registry.get(&connection).is_none());
        registry.shutdown().await;
        tokio::time::timeout(Duration::from_secs(1), peer.next())
            .await
            .expect("cancelled peer closed");
        drop(retained_connection);

        let registry = WebSocketResources::default();
        let (connection, mut peer) = connected_pair(&registry).await;
        let retained_connection = registry.get(&connection).unwrap();
        drop(registry);
        tokio::time::timeout(Duration::from_secs(1), peer.next())
            .await
            .expect("dropped registry releases connection");
        drop(retained_connection);
    }

    #[tokio::test]
    async fn connect_rejects_disallowed_hosts_and_reserved_headers() {
        let registry = WebSocketResources::default();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("ws://{}", listener.local_addr().unwrap());
        assert!(registry
            .connect(ExecutionEnvironment::Server, None, &url, "{}")
            .await
            .is_none());
        assert!(registry
            .connect(ExecutionEnvironment::Local, Some(&[]), &url, "{}")
            .await
            .is_none());
        assert!(registry
            .connect(
                ExecutionEnvironment::Local,
                None,
                &url,
                r#"{"Host":"another.example"}"#
            )
            .await
            .is_none());
        assert!(registry
            .connect(ExecutionEnvironment::Local, None, &url, "invalid json")
            .await
            .is_none());
        let allowlist = vec!["127.0.0.1".to_owned()];
        let (connection, mut peer) = tokio::time::timeout(Duration::from_secs(2), async {
            tokio::join!(
                registry.connect(ExecutionEnvironment::Local, Some(&allowlist), &url, "{}"),
                async {
                    let (stream, _) = listener.accept().await.unwrap();
                    tokio_tungstenite::accept_async(stream).await.unwrap()
                }
            )
        })
        .await
        .expect("allowlisted handshake");
        assert!(
            registry
                .close(&connection.expect("allowlisted connection"))
                .await
        );
        tokio::time::timeout(Duration::from_secs(1), peer.next())
            .await
            .expect("closed client released socket");
        registry.shutdown().await;
    }

    #[test]
    fn network_policy_checks_host_allowlist() {
        let allowlist = vec!["example.com".to_string()];
        assert!(host_allowed(Some(&allowlist), "EXAMPLE.com"));
        assert!(!host_allowed(Some(&allowlist), "other.example.com"));
        assert!(!host_allowed(Some(&[]), "example.com"));
    }
}
