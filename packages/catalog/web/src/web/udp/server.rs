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
use std::sync::Arc;

use super::{UdpServerConfig, UdpSession};

#[cfg(feature = "execute")]
use super::CachedUdpSocket;
#[cfg(feature = "execute")]
use crate::web::message_handler::{
    IncomingPayload, create_message_handler_context, trigger_message_handler,
};

#[crate::register_node]
#[derive(Default)]
pub struct UdpServerNode {}

impl UdpServerNode {
    pub fn new() -> Self {
        UdpServerNode {}
    }
}

#[async_trait]
impl NodeLogic for UdpServerNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "udp_server",
            "UDP Server",
            "Binds a UDP socket. Typed lifecycle pins describe the socket; incoming datagrams are delivered to the referenced on-message handler.",
            "Web/UDP",
        );
        node.set_flowscript_name("udp", "server");
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
            "Start the UDP server",
            VariableType::Execution,
        );
        node.add_input_pin(
            "config",
            "Config",
            "UDP server configuration",
            VariableType::Struct,
        )
        .set_schema::<UdpServerConfig>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin(
            "on_listening",
            "On Listening",
            "Fires when the socket is bound and ready",
            VariableType::Execution,
        );
        node.add_output_pin(
            "session",
            "Session",
            "UDP server socket session",
            VariableType::Struct,
        )
        .set_schema::<UdpSession>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());
        node.add_output_pin(
            "local_addr",
            "Local Addr",
            "Bound local socket address",
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
            "Fires if the socket fails to bind",
            VariableType::Execution,
        );

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("on_listening").await?;
        context.deactivate_exec_pin("on_close").await?;
        context.activate_exec_pin("exec_error").await?;

        let config: UdpServerConfig = context.evaluate_pin("config").await?;
        let referenced_fns = context.get_referenced_functions().await?;
        let handler = referenced_fns.first().cloned();
        if referenced_fns.len() > 1 {
            context.log_message(
                "UDP Server uses only the first referenced on-message handler",
                LogLevel::Warn,
            );
        }

        let addr = format!("{}:{}", config.host, config.port);
        let socket = match tokio::net::UdpSocket::bind(&addr).await {
            Ok(socket) => socket,
            Err(err) => {
                context.log_message(
                    &format!("UDP server bind failed on {}: {}", addr, err),
                    LogLevel::Error,
                );
                return Ok(());
            }
        };

        let local_addr = socket.local_addr()?.to_string();
        let ref_id = format!("udp_{}", flow_like_types::create_id());
        let close_notify = Arc::new(tokio::sync::Notify::new());
        let cached = CachedUdpSocket {
            socket: Arc::new(socket),
            close_notify: close_notify.clone(),
        };
        let socket = cached.socket.clone();
        let cacheable: Arc<dyn Cacheable> = Arc::new(cached);
        context.set_cache(&ref_id, cacheable).await;

        let session = UdpSession {
            ref_id: ref_id.clone(),
            local_addr: local_addr.clone(),
        };
        context
            .set_pin_value("session", json!(session.clone()))
            .await?;
        context
            .set_pin_value("local_addr", json!(local_addr.clone()))
            .await?;
        context.deactivate_exec_pin("exec_error").await?;
        context.activate_exec_pin("on_listening").await?;
        trigger_connected_exec(context, "on_listening", "UDP server on_listening").await;

        let handler_context = if let Some(handler) = handler {
            Some(
                create_message_handler_context(
                    context,
                    handler,
                    &[
                        "_client",
                        "session",
                        "sender_addr",
                        "target_host",
                        "target_port",
                    ],
                )
                .await,
            )
        } else {
            None
        };

        let timeout = config.timeout_seconds;
        let cancellation_token = context.get_cancellation_token();
        let mut cancelled = false;
        let mut buf = vec![0_u8; 65535];

        loop {
            let recv = if timeout > 0 {
                tokio::select! {
                    result = socket.recv_from(&mut buf) => Some(result),
                    _ = tokio::time::sleep(std::time::Duration::from_secs(timeout)) => {
                        context.log_message("UDP server timed out", LogLevel::Warn);
                        None
                    }
                    _ = super::super::wait_for_cancel(cancellation_token.clone()) => {
                        cancelled = true;
                        context.log_message("UDP server cancelled", LogLevel::Warn);
                        None
                    }
                }
            } else {
                tokio::select! {
                    result = socket.recv_from(&mut buf) => Some(result),
                    _ = super::super::wait_for_cancel(cancellation_token.clone()) => {
                        cancelled = true;
                        context.log_message("UDP server cancelled", LogLevel::Warn);
                        None
                    }
                }
            };

            let Some(recv) = recv else {
                break;
            };

            let (len, sender_addr) = match recv {
                Ok(result) => result,
                Err(err) => {
                    context.log_message(&format!("UDP recv error: {}", err), LogLevel::Error);
                    break;
                }
            };

            if let Some(handler_context) = &handler_context {
                let data = buf[..len].to_vec();
                let target_host = sender_addr.ip().to_string();
                let target_port = sender_addr.port() as i64;
                let sender_addr = sender_addr.to_string();
                let payload = match std::str::from_utf8(&data) {
                    Ok(text) => IncomingPayload::Text(text.to_string()),
                    Err(_) => IncomingPayload::Binary(data),
                };
                trigger_message_handler(
                    handler_context,
                    payload,
                    &[
                        ("session", json!(session.clone())),
                        (
                            "_client",
                            json!({
                                "session": session.clone(),
                                "sender_addr": sender_addr.clone(),
                                "target_host": target_host.clone(),
                                "target_port": target_port,
                            }),
                        ),
                        ("sender_addr", json!(sender_addr)),
                        ("target_host", json!(target_host)),
                        ("target_port", json!(target_port)),
                    ],
                    "UDP server on_message",
                )
                .await;
            }
        }

        close_notify.notify_waiters();
        {
            let mut cache = context.cache.write().await;
            cache.remove(&ref_id);
        }
        context.deactivate_exec_pin("on_listening").await?;
        context.activate_exec_pin("on_close").await?;
        trigger_connected_exec(context, "on_close", "UDP server on_close").await;

        if cancelled {
            return Err(flow_like_types::anyhow!("Execution was cancelled"));
        }

        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "UDP requires the 'execute' feature"
        ))
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

#[cfg(all(test, feature = "execute"))]
mod tests {
    use super::*;
    use crate::web::test_support::{
        free_udp_port, internal_node, node_with_outputs, output_value, test_context,
    };
    use flow_like::flow::{node::NodeLogic, variable::VariableType};
    use flow_like_types::json::json;

    #[tokio::test]
    async fn udp_server_e2e_delivers_utf8_datagram_to_payload_handler() {
        let port = free_udp_port();
        let handler = node_with_outputs(&[("payload", VariableType::Struct)]);
        let mut node = UdpServerNode::new().get_node();
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
                json!(UdpServerConfig {
                    host: "127.0.0.1".to_string(),
                    port,
                    timeout_seconds: 1,
                }),
            )
            .await
            .unwrap();

        let server = tokio::spawn(async move { UdpServerNode::new().run(&mut context).await });
        let socket = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let addr = format!("127.0.0.1:{port}");
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        socket.send_to(b"hello", &addr).await.unwrap();

        server.await.unwrap().unwrap();
        let handler_context = test_context(handler, Vec::new()).await;
        let payload = output_value(&handler_context, "payload").await.unwrap();
        assert_eq!(payload["payload"], json!("hello"));
        assert!(payload["_client"]["sender_addr"].as_str().is_some());
        assert!(payload["_client"]["target_host"].as_str().is_some());
        assert!(payload["_client"]["target_port"].as_i64().is_some());
    }
}
