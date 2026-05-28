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
use std::sync::{
    Arc,
    atomic::{AtomicU32, Ordering},
};

use super::{TcpListenConfig, TcpSession};

#[cfg(feature = "execute")]
use super::CachedTcpConnection;
#[cfg(feature = "execute")]
use crate::web::message_handler::{
    IncomingPayload, MessageHandlerContext, create_message_handler_context, trigger_message_handler,
};

#[crate::register_node]
#[derive(Default)]
pub struct TcpServerNode {}

impl TcpServerNode {
    pub fn new() -> Self {
        TcpServerNode {}
    }
}

#[async_trait]
impl NodeLogic for TcpServerNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "tcp_server",
            "TCP Server",
            "Binds a TCP server. Typed lifecycle events are exposed as pins; incoming data chunks are delivered to the referenced on-message handler.",
            "Web/TCP",
        );
        node.add_icon("/flow/icons/web.svg");
        node.set_long_running(true);
        node.set_can_reference_fns(true);
        node.scores = Some(
            flow_like::flow::node::NodeScores::new()
                .set_privacy(7)
                .set_security(6)
                .set_performance(8)
                .set_governance(5)
                .set_reliability(7)
                .set_cost(10)
                .build(),
        );

        node.add_input_pin(
            "exec_in",
            "Execute",
            "Start the TCP server",
            VariableType::Execution,
        );
        node.add_input_pin(
            "config",
            "Config",
            "TCP server configuration",
            VariableType::Struct,
        )
        .set_schema::<TcpListenConfig>()
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
            "Fires for each accepted TCP client",
            VariableType::Execution,
        );
        node.add_output_pin(
            "session",
            "Session",
            "Accepted TCP client session",
            VariableType::Struct,
        )
        .set_schema::<TcpSession>()
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
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("on_listening").await?;
        context.deactivate_exec_pin("on_connect").await?;
        context.deactivate_exec_pin("on_close").await?;
        context.activate_exec_pin("exec_error").await?;

        let config: TcpListenConfig = context.evaluate_pin("config").await?;
        let referenced_fns = context.get_referenced_functions().await?;
        let handler = referenced_fns.first().cloned();
        if referenced_fns.len() > 1 {
            context.log_message(
                "TCP Server uses only the first referenced on-message handler",
                LogLevel::Warn,
            );
        }

        let addr = format!("{}:{}", config.host, config.port);
        let listener = match tokio::net::TcpListener::bind(&addr).await {
            Ok(listener) => listener,
            Err(err) => {
                context.log_message(
                    &format!("TCP server bind failed on {}: {}", addr, err),
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
                    &format!("TCP server TLS configuration failed: {}", err),
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
        trigger_connected_exec(context, "on_listening", "TCP server on_listening").await;

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
                        context.log_message("TCP server timed out", LogLevel::Warn);
                        None
                    }
                    _ = super::super::wait_for_cancel(cancellation_token.clone()) => {
                        cancelled = true;
                        context.log_message("TCP server cancelled", LogLevel::Warn);
                        None
                    }
                }
            } else {
                tokio::select! {
                    result = listener.accept() => Some(result),
                    _ = super::super::wait_for_cancel(cancellation_token.clone()) => {
                        cancelled = true;
                        context.log_message("TCP server cancelled", LogLevel::Warn);
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
                    context.log_message(&format!("TCP accept error: {}", err), LogLevel::Error);
                    continue;
                }
            };

            if config.max_connections > 0
                && active_connections.load(Ordering::Relaxed) >= config.max_connections
            {
                context.log_message(
                    "TCP server rejected connection because max_connections was reached",
                    LogLevel::Warn,
                );
                continue;
            }

            let remote_addr = remote_addr.to_string();
            let (reader, writer) = if let Some(acceptor) = &tls_acceptor {
                match acceptor.accept(stream).await {
                    Ok(stream) => crate::web::tls::boxed_split(stream),
                    Err(err) => {
                        context.log_message(
                            &format!("TCP TLS handshake failed: {}", err),
                            LogLevel::Error,
                        );
                        continue;
                    }
                }
            } else {
                crate::web::tls::boxed_split(stream)
            };
            active_connections.fetch_add(1, Ordering::Relaxed);
            let ref_id = format!("tcp_{}", flow_like_types::create_id());
            let close_notify = Arc::new(tokio::sync::Notify::new());
            let cached = CachedTcpConnection {
                reader: Arc::new(tokio::sync::Mutex::new(reader)),
                writer: Arc::new(tokio::sync::Mutex::new(writer)),
                close_notify: close_notify.clone(),
            };
            let cacheable: Arc<dyn Cacheable> = Arc::new(cached);
            context.set_cache(&ref_id, cacheable).await;
            accepted_refs.push(ref_id.clone());

            let session = TcpSession {
                ref_id: ref_id.clone(),
                remote_addr: remote_addr.clone(),
            };
            context
                .set_pin_value("session", json!(session.clone()))
                .await?;
            context
                .set_pin_value("remote_addr", json!(remote_addr.clone()))
                .await?;
            context.activate_exec_pin("on_connect").await?;
            trigger_connected_exec(context, "on_connect", "TCP server on_connect").await;

            spawn_tcp_reader(
                context,
                handler_context.clone(),
                ref_id,
                close_notify,
                session,
                remote_addr,
                active_connections.clone(),
            )
            .await?;
        }

        shutdown_tcp_connections(context, &accepted_refs).await;
        context.deactivate_exec_pin("on_listening").await?;
        context.deactivate_exec_pin("on_connect").await?;
        context.activate_exec_pin("on_close").await?;
        trigger_connected_exec(context, "on_close", "TCP server on_close").await;

        if cancelled {
            return Err(flow_like_types::anyhow!("Execution was cancelled"));
        }

        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "TCP requires the 'execute' feature"
        ))
    }
}

#[cfg(feature = "execute")]
async fn shutdown_tcp_connections(context: &ExecutionContext, ref_ids: &[String]) {
    futures::future::join_all(
        ref_ids
            .iter()
            .map(|ref_id| shutdown_tcp_connection(context, ref_id)),
    )
    .await;
}

#[cfg(feature = "execute")]
async fn shutdown_tcp_connection(context: &ExecutionContext, ref_id: &str) {
    use tokio::io::AsyncWriteExt;

    let cache_entry = {
        let cache = context.cache.read().await;
        cache.get(ref_id).cloned()
    };

    let Some(entry) = cache_entry else {
        return;
    };
    let Some(conn) = entry.as_any().downcast_ref::<CachedTcpConnection>() else {
        return;
    };

    conn.close_notify.notify_waiters();
    let writer = conn.writer.clone();
    let shutdown_result = tokio::time::timeout(std::time::Duration::from_millis(500), async move {
        let mut writer = writer.lock().await;
        writer.shutdown().await
    })
    .await;
    if let Ok(Err(err)) = shutdown_result {
        tracing::warn!("TCP server connection shutdown error: {}", err);
    }

    let mut cache = context.cache.write().await;
    cache.remove(ref_id);
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
async fn spawn_tcp_reader(
    context: &ExecutionContext,
    handler_context: Option<MessageHandlerContext>,
    ref_id: String,
    close_notify: Arc<tokio::sync::Notify>,
    session: TcpSession,
    remote_addr: String,
    active_connections: Arc<AtomicU32>,
) -> flow_like_types::Result<()> {
    use tokio::io::AsyncReadExt;

    let reader = {
        let cache = context.cache.read().await;
        let conn = cache
            .get(&ref_id)
            .ok_or_else(|| flow_like_types::anyhow!("TCP connection not in cache"))?;
        let conn = conn
            .as_any()
            .downcast_ref::<CachedTcpConnection>()
            .ok_or_else(|| flow_like_types::anyhow!("Failed to downcast TCP connection"))?;
        conn.reader.clone()
    };
    let cache = context.cache.clone();

    tokio::spawn(async move {
        let mut buf = vec![0_u8; 8192];
        loop {
            let read_result = tokio::select! {
                result = async { reader.lock().await.read(&mut buf).await } => result,
                _ = close_notify.notified() => break,
            };

            let len = match read_result {
                Ok(0) => break,
                Ok(len) => len,
                Err(err) => {
                    tracing::warn!("TCP server read error: {}", err);
                    break;
                }
            };

            let data = buf[..len].to_vec();
            if let Some(handler_context) = &handler_context {
                let payload = match std::str::from_utf8(&data) {
                    Ok(text) => IncomingPayload::Text(text.to_string()),
                    Err(_) => IncomingPayload::Binary(data),
                };
                trigger_message_handler(
                    handler_context,
                    payload,
                    &[
                        ("session", json!(session.clone())),
                        ("_client", json!(session.clone())),
                        ("remote_addr", json!(remote_addr.clone())),
                    ],
                    "TCP server on_message",
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
    });

    Ok(())
}

#[cfg(all(test, feature = "execute"))]
mod tests {
    use super::*;
    use crate::web::test_support::{
        free_tcp_port, internal_node, node_with_outputs, output_value, test_context,
    };
    use flow_like::flow::{node::NodeLogic, variable::VariableType};
    use flow_like_types::json::json;
    use std::time::Duration;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[tokio::test]
    async fn tcp_server_e2e_delivers_utf8_chunk_to_payload_handler() {
        let port = free_tcp_port();
        let handler = node_with_outputs(&[("payload", VariableType::Struct)]);
        let mut node = TcpServerNode::new().get_node();
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
                json!(TcpListenConfig {
                    host: "127.0.0.1".to_string(),
                    port,
                    timeout_seconds: 1,
                    max_connections: 8,
                    tls: Default::default(),
                }),
            )
            .await
            .unwrap();

        let server = tokio::spawn(async move { TcpServerNode::new().run(&mut context).await });
        let addr = format!("127.0.0.1:{port}");
        let mut stream = None;
        for _ in 0..20 {
            match tokio::net::TcpStream::connect(&addr).await {
                Ok(s) => {
                    stream = Some(s);
                    break;
                }
                Err(_) => tokio::time::sleep(std::time::Duration::from_millis(50)).await,
            }
        }
        let mut stream = stream.expect("tcp server should accept loopback connection");
        stream.write_all(b"hello").await.unwrap();
        stream.shutdown().await.unwrap();

        server.await.unwrap().unwrap();
        let handler_context = test_context(handler, Vec::new()).await;
        let payload = output_value(&handler_context, "payload").await.unwrap();
        assert_eq!(payload["payload"], json!("hello"));
        assert!(
            payload["_client"]["ref_id"]
                .as_str()
                .is_some_and(|value| value.starts_with("tcp_"))
        );
    }

    #[tokio::test]
    async fn tcp_server_e2e_closes_open_clients_when_server_stops() {
        let port = free_tcp_port();
        let parent = internal_node(TcpServerNode::new().get_node());
        let mut context = test_context(parent, Vec::new()).await;
        context
            .set_pin_value(
                "config",
                json!(TcpListenConfig {
                    host: "127.0.0.1".to_string(),
                    port,
                    timeout_seconds: 1,
                    max_connections: 8,
                    tls: Default::default(),
                }),
            )
            .await
            .unwrap();

        let server = tokio::spawn(async move { TcpServerNode::new().run(&mut context).await });
        let addr = format!("127.0.0.1:{port}");
        let mut stream = None;
        for _ in 0..20 {
            match tokio::net::TcpStream::connect(&addr).await {
                Ok(s) => {
                    stream = Some(s);
                    break;
                }
                Err(_) => tokio::time::sleep(Duration::from_millis(50)).await,
            }
        }
        let mut stream = stream.expect("tcp server should accept loopback connection");

        tokio::time::timeout(Duration::from_secs(3), server)
            .await
            .expect("server should stop on timeout")
            .unwrap()
            .unwrap();

        let mut buf = [0_u8; 1];
        let read = tokio::time::timeout(Duration::from_secs(1), stream.read(&mut buf))
            .await
            .expect("client should observe tcp server-side shutdown");
        assert!(
            read.is_err() || read.unwrap() == 0,
            "tcp client should reach EOF or error after server shutdown"
        );
    }

    #[tokio::test]
    async fn tcp_server_tls_e2e_delivers_utf8_chunk_to_payload_handler() {
        let ca = crate::web::tls::create_ca_certificate("FlowLike TCP Test CA").unwrap();
        let leaf = crate::web::tls::create_signed_certificate(
            &ca,
            "localhost",
            vec!["localhost".to_string()],
            "Server",
        )
        .unwrap();
        let port = free_tcp_port();
        let handler = node_with_outputs(&[("payload", VariableType::Struct)]);
        let mut node = TcpServerNode::new().get_node();
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
                json!(TcpListenConfig {
                    host: "127.0.0.1".to_string(),
                    port,
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

        let server = tokio::spawn(async move { TcpServerNode::new().run(&mut context).await });
        let addr = format!("127.0.0.1:{port}");
        let mut tcp_stream = None;
        for _ in 0..20 {
            match tokio::net::TcpStream::connect(&addr).await {
                Ok(s) => {
                    tcp_stream = Some(s);
                    break;
                }
                Err(_) => tokio::time::sleep(Duration::from_millis(50)).await,
            }
        }
        let tcp_stream = tcp_stream.expect("tcp tls server should accept loopback connection");
        let client_tls = crate::web::tls::TlsConfig {
            secure: true,
            ca_certificate_pem: Some(ca.certificate_pem),
            server_name: Some("localhost".to_string()),
            ..Default::default()
        };
        let connector = crate::web::tls::client_connector(&client_tls)
            .unwrap()
            .unwrap();
        let server_name = crate::web::tls::tls_server_name(&client_tls, "localhost").unwrap();
        let mut stream = connector.connect(server_name, tcp_stream).await.unwrap();
        stream.write_all(b"hello tls").await.unwrap();
        stream.shutdown().await.unwrap();

        server.await.unwrap().unwrap();
        let handler_context = test_context(handler, Vec::new()).await;
        let payload = output_value(&handler_context, "payload").await.unwrap();
        assert_eq!(payload["payload"], json!("hello tls"));
        assert!(
            payload["_client"]["ref_id"]
                .as_str()
                .is_some_and(|value| value.starts_with("tcp_"))
        );
    }
}
