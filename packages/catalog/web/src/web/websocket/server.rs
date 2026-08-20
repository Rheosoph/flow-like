#[cfg(not(feature = "execute"))]
use flow_like::flow::execution::context::ExecutionContext;
#[cfg(feature = "execute")]
use flow_like::flow::execution::{
    LogLevel, context::ExecutionContext, internal_node::InternalNode, log::LogMessage,
};
use flow_like::flow::{
    node::{Node, NodeLogic},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_types::async_trait;
#[cfg(feature = "execute")]
use flow_like_types::{Cacheable, json::json};

#[cfg(feature = "execute")]
use futures::{SinkExt, StreamExt};
#[cfg(feature = "execute")]
use std::sync::{
    Arc,
    atomic::{AtomicU32, Ordering},
};

use super::{WebSocketServerConfig, WebSocketSession};

#[cfg(feature = "execute")]
use super::{CachedWebSocketConnection, CachedWebSocketSink};
#[cfg(feature = "execute")]
use crate::web::message_handler::{
    IncomingPayload, MessageHandlerContext, create_message_handler_context, trigger_message_handler,
};

#[crate::register_node]
#[derive(Default)]
pub struct WebSocketServerNode {}

impl WebSocketServerNode {
    pub fn new() -> Self {
        WebSocketServerNode {}
    }
}

#[async_trait]
impl NodeLogic for WebSocketServerNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "websocket_server",
            "WebSocket Server",
            "Binds a WebSocket server. Typed lifecycle events are exposed as pins; incoming messages are delivered to the referenced on-message handler.",
            "Web/WebSocket",
        );
        node.add_icon("/flow/icons/web.svg");
        node.set_long_running(true);
        node.set_can_reference_fns(true);
        node.scores = Some(
            flow_like::flow::node::NodeScores::new()
                .set_privacy(8)
                .set_security(7)
                .set_performance(7)
                .set_governance(6)
                .set_reliability(7)
                .set_cost(9)
                .build(),
        );

        node.add_input_pin(
            "exec_in",
            "Execute",
            "Start the WebSocket server",
            VariableType::Execution,
        );
        node.add_input_pin(
            "config",
            "Config",
            "WebSocket server configuration",
            VariableType::Struct,
        )
        .set_schema::<WebSocketServerConfig>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin(
            "on_listening",
            "On Listening",
            "Fires when the server is bound and ready",
            VariableType::Execution,
        );
        node.add_output_pin(
            "local_addr",
            "Local Addr",
            "Bound local socket address",
            VariableType::String,
        );
        node.add_output_pin(
            "on_connect",
            "On Connect",
            "Fires for each accepted WebSocket client",
            VariableType::Execution,
        );
        node.add_output_pin(
            "session",
            "Session",
            "Accepted WebSocket client session",
            VariableType::Struct,
        )
        .set_schema::<WebSocketSession>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());
        node.add_output_pin(
            "remote_addr",
            "Remote Addr",
            "Remote client socket address",
            VariableType::String,
        );
        node.add_output_pin(
            "on_close",
            "On Close",
            "Fires when the server stops",
            VariableType::Execution,
        );
        node.add_output_pin(
            "exec_error",
            "Error",
            "Fires if the server fails to bind",
            VariableType::Execution,
        );

        node
    }

    #[cfg(feature = "execute")]
    #[allow(clippy::result_large_err)] // Err type is tungstenite's ErrorResponse, fixed by the accept_hdr_async callback contract
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("on_listening").await?;
        context.deactivate_exec_pin("on_connect").await?;
        context.deactivate_exec_pin("on_close").await?;
        context.activate_exec_pin("exec_error").await?;

        let config: WebSocketServerConfig = context.evaluate_pin("config").await?;
        let referenced_fns = context.get_referenced_functions().await?;
        let handler = referenced_fns.first().cloned();
        if referenced_fns.len() > 1 {
            context.log_message(
                "WebSocket Server uses only the first referenced on-message handler",
                LogLevel::Warn,
            );
        }

        let addr = format!("{}:{}", config.host, config.port);
        let listener = match tokio::net::TcpListener::bind(&addr).await {
            Ok(listener) => listener,
            Err(err) => {
                context.log_message(
                    &format!("WebSocket server bind failed on {}: {}", addr, err),
                    LogLevel::Error,
                );
                return Ok(());
            }
        };

        let local_addr = listener.local_addr()?.to_string();
        let tls_acceptor = match crate::web::tls::server_acceptor(&config.tls) {
            Ok(acceptor) => acceptor,
            Err(err) => {
                context.log_message(
                    &format!("WebSocket server TLS configuration failed: {}", err),
                    LogLevel::Error,
                );
                return Ok(());
            }
        };
        context
            .set_pin_value("local_addr", json!(local_addr.clone()))
            .await?;
        context.deactivate_exec_pin("exec_error").await?;
        context.activate_exec_pin("on_listening").await?;
        trigger_connected_exec(context, "on_listening", "WebSocket server on_listening").await;

        let handler_context = if let Some(handler) = handler {
            Some(
                create_message_handler_context(
                    context,
                    handler,
                    &["_client", "session", "remote_addr"],
                )
                .await,
            )
        } else {
            None
        };

        let timeout = config.timeout_seconds;
        let cancellation_token = context.get_cancellation_token();
        let mut cancelled = false;
        let active_connections = Arc::new(AtomicU32::new(0));
        let mut accepted_refs = Vec::new();

        loop {
            let accept = if timeout > 0 {
                tokio::select! {
                    result = listener.accept() => Some(result),
                    _ = tokio::time::sleep(std::time::Duration::from_secs(timeout)) => {
                        context.log_message("WebSocket server timed out", LogLevel::Warn);
                        None
                    }
                    _ = super::super::wait_for_cancel(cancellation_token.clone()) => {
                        cancelled = true;
                        context.log_message("WebSocket server cancelled", LogLevel::Warn);
                        None
                    }
                }
            } else {
                tokio::select! {
                    result = listener.accept() => Some(result),
                    _ = super::super::wait_for_cancel(cancellation_token.clone()) => {
                        cancelled = true;
                        context.log_message("WebSocket server cancelled", LogLevel::Warn);
                        None
                    }
                }
            };

            let Some(accept) = accept else {
                break;
            };

            let (stream, remote_addr) = match accept {
                Ok(pair) => pair,
                Err(err) => {
                    context.log_message(
                        &format!("WebSocket server accept error: {}", err),
                        LogLevel::Error,
                    );
                    continue;
                }
            };

            if config.max_connections > 0
                && active_connections.load(Ordering::Relaxed) >= config.max_connections
            {
                context.log_message(
                    "WebSocket server rejected connection because max_connections was reached",
                    LogLevel::Warn,
                );
                continue;
            }

            let remote_addr = remote_addr.to_string();
            let expected_path = config.path.as_deref().and_then(normalize_path);
            let stream: crate::web::tls::BoxedIo = if let Some(acceptor) = &tls_acceptor {
                match acceptor.accept(stream).await {
                    Ok(stream) => Box::new(stream),
                    Err(err) => {
                        context.log_message(
                            &format!("WebSocket TLS handshake failed: {}", err),
                            LogLevel::Error,
                        );
                        continue;
                    }
                }
            } else {
                Box::new(stream)
            };
            let ws_stream = match tokio_tungstenite::accept_hdr_async(
                stream,
                move |request: &tokio_tungstenite::tungstenite::handshake::server::Request,
                      response| {
                    if let Some(expected_path) = &expected_path
                        && request.uri().path() != expected_path
                    {
                        let mut response =
                            tokio_tungstenite::tungstenite::handshake::server::ErrorResponse::new(
                                Some(format!("Expected WebSocket path {}", expected_path)),
                            );
                        *response.status_mut() =
                            tokio_tungstenite::tungstenite::http::StatusCode::NOT_FOUND;
                        return Err(response);
                    }

                    Ok(response)
                },
            )
            .await
            {
                Ok(stream) => stream,
                Err(err) => {
                    context.log_message(
                        &format!("WebSocket handshake failed: {}", err),
                        LogLevel::Error,
                    );
                    continue;
                }
            };

            let (sink, stream) = ws_stream.split();
            active_connections.fetch_add(1, Ordering::Relaxed);
            let ref_id = format!("ws_{}", flow_like_types::create_id());
            let close_notify = Arc::new(tokio::sync::Notify::new());
            let scheme = if config.tls.secure { "wss" } else { "ws" };
            let url = match &config.path {
                Some(path) if path.starts_with('/') => format!("{scheme}://{}{}", local_addr, path),
                Some(path) if !path.is_empty() => format!("{scheme}://{}/{}", local_addr, path),
                _ => format!("{scheme}://{}", local_addr),
            };
            let session = WebSocketSession {
                ref_id: ref_id.clone(),
                url,
            };

            let cached = CachedWebSocketConnection {
                sink: CachedWebSocketSink::Server(Arc::new(tokio::sync::Mutex::new(sink))),
                close_notify: close_notify.clone(),
                reader_handle: tokio::sync::Mutex::new(None),
            };
            let cacheable: Arc<dyn Cacheable> = Arc::new(cached);
            context.set_cache(&ref_id, cacheable).await;
            accepted_refs.push(ref_id.clone());

            context
                .set_pin_value("session", json!(session.clone()))
                .await?;
            context
                .set_pin_value("remote_addr", json!(remote_addr.clone()))
                .await?;
            context.activate_exec_pin("on_connect").await?;
            trigger_connected_exec(context, "on_connect", "WebSocket server on_connect").await;

            let handle = spawn_server_message_reader(
                context,
                stream,
                handler_context.clone(),
                ref_id.clone(),
                close_notify.clone(),
                session,
                remote_addr,
                active_connections.clone(),
            )
            .await;

            let cache = context.cache.read().await;
            if let Some(conn) = cache.get(&ref_id)
                && let Some(conn) = conn.as_any().downcast_ref::<CachedWebSocketConnection>()
            {
                let mut guard = conn.reader_handle.lock().await;
                *guard = Some(handle);
            }
        }

        shutdown_server_connections(context, &accepted_refs).await;
        context.deactivate_exec_pin("on_listening").await?;
        context.deactivate_exec_pin("on_connect").await?;
        context.activate_exec_pin("on_close").await?;
        trigger_connected_exec(context, "on_close", "WebSocket server on_close").await;

        if cancelled {
            return Err(flow_like_types::anyhow!("Execution was cancelled"));
        }

        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "WebSocket requires the 'execute' feature"
        ))
    }
}

#[cfg(feature = "execute")]
fn normalize_path(path: &str) -> Option<String> {
    if path.is_empty() {
        return None;
    }

    if path.starts_with('/') {
        Some(path.to_string())
    } else {
        Some(format!("/{path}"))
    }
}

#[cfg(feature = "execute")]
async fn trigger_connected_exec(context: &mut ExecutionContext, pin_name: &str, log_name: &str) {
    let Ok(pin) = context.get_pin_by_name(pin_name).await else {
        return;
    };

    for node in pin.get_connected_nodes() {
        let mut sub = context.create_sub_context(&node).await;
        sub.delegated = true;
        let mut message = LogMessage::new(log_name, LogLevel::Debug, None);
        let _ = InternalNode::trigger(&mut sub, &mut None, true).await;
        message.end();
        sub.log(message);
        sub.end_trace();
        context.push_sub_context(&mut sub);
    }
}

#[cfg(feature = "execute")]
async fn shutdown_server_connections(context: &ExecutionContext, ref_ids: &[String]) {
    futures::future::join_all(
        ref_ids
            .iter()
            .map(|ref_id| shutdown_server_connection(context, ref_id)),
    )
    .await;
}

#[cfg(feature = "execute")]
async fn shutdown_server_connection(context: &ExecutionContext, ref_id: &str) {
    let cache_entry = {
        let cache = context.cache.read().await;
        cache.get(ref_id).cloned()
    };

    let Some(entry) = cache_entry else {
        return;
    };
    let Some(conn) = entry.as_any().downcast_ref::<CachedWebSocketConnection>() else {
        return;
    };

    let sink = conn.sink.clone();
    let close_notify = conn.close_notify.clone();
    close_notify.notify_waiters();

    let close_result = tokio::time::timeout(std::time::Duration::from_millis(500), async {
        match sink {
            CachedWebSocketSink::Client(sink) => {
                let mut sink = sink.lock().await;
                sink.close().await
            }
            CachedWebSocketSink::Server(sink) => {
                let mut sink = sink.lock().await;
                sink.close().await
            }
        }
    })
    .await;
    if let Ok(Err(err)) = close_result {
        tracing::warn!("WebSocket server connection close error: {}", err);
    }

    let handle = conn.reader_handle.lock().await.take();
    if let Some(handle) = handle {
        await_or_abort(handle).await;
    }

    let mut cache = context.cache.write().await;
    cache.remove(ref_id);
}

#[cfg(feature = "execute")]
async fn await_or_abort(mut handle: tokio::task::JoinHandle<()>) {
    tokio::select! {
        _ = &mut handle => {}
        _ = tokio::time::sleep(std::time::Duration::from_millis(500)) => {
            handle.abort();
            let _ = handle.await;
        }
    }
}

#[cfg(feature = "execute")]
#[allow(clippy::too_many_arguments)]
async fn spawn_server_message_reader(
    context: &ExecutionContext,
    mut stream: super::ServerWsStream,
    handler_context: Option<MessageHandlerContext>,
    ref_id: String,
    close_notify: Arc<tokio::sync::Notify>,
    session: WebSocketSession,
    remote_addr: String,
    active_connections: Arc<AtomicU32>,
) -> tokio::task::JoinHandle<()> {
    let cache = context.cache.clone();

    tokio::spawn(async move {
        loop {
            let msg_result = tokio::select! {
                msg = stream.next() => msg,
                _ = close_notify.notified() => break,
            };
            let Some(msg_result) = msg_result else {
                break;
            };
            let msg = match msg_result {
                Ok(msg) => msg,
                Err(err) => {
                    tracing::warn!("WebSocket server read error: {}", err);
                    break;
                }
            };

            let payload = match msg {
                tokio_tungstenite::tungstenite::Message::Text(text) => {
                    IncomingPayload::Text(text.to_string())
                }
                tokio_tungstenite::tungstenite::Message::Binary(data) => {
                    IncomingPayload::Binary(data.to_vec())
                }
                tokio_tungstenite::tungstenite::Message::Close(_) => break,
                tokio_tungstenite::tungstenite::Message::Ping(_)
                | tokio_tungstenite::tungstenite::Message::Pong(_)
                | tokio_tungstenite::tungstenite::Message::Frame(_) => continue,
            };

            if let Some(handler_context) = &handler_context {
                trigger_message_handler(
                    handler_context,
                    payload,
                    &[
                        ("session", json!(session.clone())),
                        ("_client", json!(session.clone())),
                        ("remote_addr", json!(remote_addr.clone())),
                    ],
                    "WebSocket server on_message",
                )
                .await;
            }
        }

        active_connections.fetch_sub(1, Ordering::Relaxed);
        {
            let mut cache = cache.write().await;
            cache.remove(&ref_id);
        }
        close_notify.notify_waiters();
    })
}

#[cfg(all(test, feature = "execute"))]
mod tests {
    use super::*;
    use crate::web::test_support::{
        free_tcp_port, internal_node, node_with_outputs, output_value, test_context,
    };
    use flow_like::flow::{node::NodeLogic, variable::VariableType};
    use flow_like_types::json::json;
    use futures::{SinkExt, StreamExt};
    use std::time::Duration;

    #[tokio::test]
    async fn websocket_server_e2e_delivers_non_json_text_to_payload_handler() {
        let port = free_tcp_port();
        let handler = node_with_outputs(&[("payload", VariableType::Struct)]);
        let mut node = WebSocketServerNode::new().get_node();
        node.fn_refs
            .as_mut()
            .unwrap()
            .fn_refs
            .push(handler.node_id().to_string());
        let parent = internal_node(node);
        let mut context = test_context(parent, vec![handler.clone()]).await;
        context
            .set_pin_value(
                "config",
                json!(WebSocketServerConfig {
                    host: "127.0.0.1".to_string(),
                    port,
                    path: None,
                    timeout_seconds: 1,
                    max_connections: 8,
                    tls: Default::default(),
                }),
            )
            .await
            .unwrap();

        let server =
            tokio::spawn(async move { WebSocketServerNode::new().run(&mut context).await });
        let url = format!("ws://127.0.0.1:{port}");
        let mut client = None;
        for _ in 0..20 {
            match tokio_tungstenite::connect_async(&url).await {
                Ok((stream, _)) => {
                    client = Some(stream);
                    break;
                }
                Err(_) => tokio::time::sleep(std::time::Duration::from_millis(50)).await,
            }
        }
        let mut client = client.expect("websocket server should accept loopback connection");
        client
            .send(tokio_tungstenite::tungstenite::Message::Text(
                "hello".into(),
            ))
            .await
            .unwrap();
        let _ = client.close(None).await;

        server.await.unwrap().unwrap();
        let handler_context = test_context(handler, Vec::new()).await;
        let payload = output_value(&handler_context, "payload").await.unwrap();
        assert_eq!(payload["payload"], json!("hello"));
        assert!(
            payload["_client"]["ref_id"]
                .as_str()
                .is_some_and(|value| value.starts_with("ws_"))
        );
    }

    #[tokio::test]
    async fn websocket_server_e2e_closes_open_clients_when_server_stops() {
        let port = free_tcp_port();
        let parent = internal_node(WebSocketServerNode::new().get_node());
        let mut context = test_context(parent, Vec::new()).await;
        context
            .set_pin_value(
                "config",
                json!(WebSocketServerConfig {
                    host: "127.0.0.1".to_string(),
                    port,
                    path: None,
                    timeout_seconds: 1,
                    max_connections: 8,
                    tls: Default::default(),
                }),
            )
            .await
            .unwrap();

        let server =
            tokio::spawn(async move { WebSocketServerNode::new().run(&mut context).await });
        let url = format!("ws://127.0.0.1:{port}");
        let mut client = None;
        for _ in 0..20 {
            match tokio_tungstenite::connect_async(&url).await {
                Ok((stream, _)) => {
                    client = Some(stream);
                    break;
                }
                Err(_) => tokio::time::sleep(Duration::from_millis(50)).await,
            }
        }
        let mut client = client.expect("websocket server should accept loopback connection");

        tokio::time::timeout(Duration::from_secs(3), server)
            .await
            .expect("server should stop on timeout")
            .unwrap()
            .unwrap();

        let next = tokio::time::timeout(Duration::from_secs(1), client.next())
            .await
            .expect("client should observe server-side shutdown");
        assert!(
            matches!(
                next,
                None | Some(Ok(tokio_tungstenite::tungstenite::Message::Close(_))) | Some(Err(_))
            ),
            "unexpected websocket message after shutdown: {next:?}"
        );
    }

    #[tokio::test]
    async fn websocket_server_tls_e2e_delivers_text_to_payload_handler() {
        let ca = crate::web::tls::create_ca_certificate("FlowLike WSS Test CA").unwrap();
        let leaf = crate::web::tls::create_signed_certificate(
            &ca,
            "localhost",
            vec!["localhost".to_string(), "127.0.0.1".to_string()],
            "Server",
        )
        .unwrap();
        let port = free_tcp_port();
        let handler = node_with_outputs(&[("payload", VariableType::Struct)]);
        let mut node = WebSocketServerNode::new().get_node();
        node.fn_refs
            .as_mut()
            .unwrap()
            .fn_refs
            .push(handler.node_id().to_string());
        let parent = internal_node(node);
        let mut context = test_context(parent, vec![handler.clone()]).await;
        context
            .set_pin_value(
                "config",
                json!(WebSocketServerConfig {
                    host: "127.0.0.1".to_string(),
                    port,
                    path: None,
                    timeout_seconds: 1,
                    max_connections: 8,
                    tls: crate::web::tls::TlsConfig {
                        secure: true,
                        certificate: Some(leaf),
                        ..Default::default()
                    },
                }),
            )
            .await
            .unwrap();

        let server =
            tokio::spawn(async move { WebSocketServerNode::new().run(&mut context).await });
        let url = format!("wss://127.0.0.1:{port}");
        let client_tls = crate::web::tls::TlsConfig {
            secure: true,
            ca_certificate_pem: Some(ca.certificate_pem),
            ..Default::default()
        };
        let client_config = std::sync::Arc::new(
            crate::web::tls::client_config(&client_tls)
                .unwrap()
                .unwrap(),
        );
        let mut client = None;
        for _ in 0..20 {
            match tokio_tungstenite::connect_async_tls_with_config(
                url.as_str(),
                None,
                false,
                Some(tokio_tungstenite::Connector::Rustls(client_config.clone())),
            )
            .await
            {
                Ok((stream, _)) => {
                    client = Some(stream);
                    break;
                }
                Err(_) => tokio::time::sleep(Duration::from_millis(50)).await,
            }
        }
        let mut client = client.expect("wss server should accept loopback connection");
        client
            .send(tokio_tungstenite::tungstenite::Message::Text(
                "hello wss".into(),
            ))
            .await
            .unwrap();
        let _ = client.close(None).await;

        server.await.unwrap().unwrap();
        let handler_context = test_context(handler, Vec::new()).await;
        let payload = output_value(&handler_context, "payload").await.unwrap();
        assert_eq!(payload["payload"], json!("hello wss"));
        assert!(
            payload["_client"]["ref_id"]
                .as_str()
                .is_some_and(|value| value.starts_with("ws_"))
        );
    }
}
