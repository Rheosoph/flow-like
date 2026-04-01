#[cfg(feature = "execute")]
use ahash::AHashSet;
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
use flow_like_types::json::json;

#[cfg(feature = "execute")]
use std::sync::Arc;

use super::UdpSession;

#[crate::register_node]
#[derive(Default)]
pub struct UdpReceiveNode {}

impl UdpReceiveNode {
    pub fn new() -> Self {
        UdpReceiveNode {}
    }
}

#[async_trait]
impl NodeLogic for UdpReceiveNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "udp_receive",
            "UDP Receive",
            "Listens for incoming datagrams on a bound UDP socket. Invokes the on-message \
             handler for each received datagram. Holds execution until the socket is closed \
             or the timeout expires, then fires on_close.",
            "Web/UDP",
        );
        node.add_icon("/flow/icons/web.svg");
        node.set_long_running(true);
        node.set_can_reference_fns(true);
        node.scores = Some(
            flow_like::flow::node::NodeScores::new()
                .set_privacy(7)
                .set_security(6)
                .set_performance(7)
                .set_governance(5)
                .set_reliability(6)
                .set_cost(9)
                .build(),
        );

        node.add_input_pin(
            "exec_in",
            "Execute",
            "Start listening for datagrams",
            VariableType::Execution,
        );
        node.add_input_pin(
            "session",
            "Session",
            "UDP session reference from a Bind node",
            VariableType::Struct,
        )
        .set_schema::<UdpSession>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());
        node.add_input_pin(
            "timeout_seconds",
            "Timeout (seconds)",
            "How long to listen before auto-closing (0 = indefinite)",
            VariableType::Integer,
        )
        .set_default_value(Some(flow_like_types::json::json!(0)));

        node.add_output_pin(
            "on_listening",
            "On Listening",
            "Fires once the receive loop starts",
            VariableType::Execution,
        );
        node.add_output_pin(
            "on_close",
            "On Close",
            "Fires when the receive loop ends",
            VariableType::Execution,
        );

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        use flow_like::flow::pin::PinType;
        use flow_like_types::sync::{DashMap, Mutex};

        context.deactivate_exec_pin("on_listening").await?;
        context.deactivate_exec_pin("on_close").await?;

        let session: UdpSession = context.evaluate_pin("session").await?;
        let referenced_fns = context.get_referenced_functions().await?;
        let handler = referenced_fns
            .first()
            .ok_or_else(|| flow_like_types::anyhow!("No on-message handler function referenced"))?
            .clone();
        let timeout_seconds: i64 = context.evaluate_pin("timeout_seconds").await?;

        let cached = super::get_udp_socket(context, &session.ref_id).await?;
        let socket = cached.socket.clone();
        let close_notify = cached.close_notify.clone();

        context.activate_exec_pin("on_listening").await?;

        let on_listening_pin = context.get_pin_by_name("on_listening").await?;
        let connected_on_listening = on_listening_pin.get_connected_nodes();
        for node in connected_on_listening {
            let mut sub = context.create_sub_context(&node).await;
            sub.delegated = true;
            let mut message = LogMessage::new("UDP on_listening", LogLevel::Debug, None);
            let _ = InternalNode::trigger(&mut sub, &mut None, true).await;
            message.end();
            sub.log(message);
            sub.end_trace();
            context.push_sub_context(&mut sub);
        }

        let reference_function = &handler;

        let ref_node_pins = reference_function.pins.clone();
        let mut has_string_pin = false;
        let mut has_byte_pin = false;
        let mut has_sender_addr_pin = false;
        let mut typed_pin_count: usize = 0;

        for (_, pin) in ref_node_pins.iter() {
            if pin.pin_type != PinType::Output || pin.data_type == VariableType::Execution {
                continue;
            }
            if pin.name == "sender_addr" {
                has_sender_addr_pin = true;
                continue;
            }
            if pin.name == "payload" {
                continue;
            }
            typed_pin_count += 1;
            match pin.data_type {
                VariableType::String => has_string_pin = true,
                VariableType::Byte => has_byte_pin = true,
                _ => {}
            }
        }

        let single_string = typed_pin_count == 1 && has_string_pin;
        let single_byte = typed_pin_count == 1 && has_byte_pin;

        let connected_nodes: Arc<DashMap<String, Arc<Mutex<ExecutionContext>>>> =
            Arc::new(DashMap::new());

        let on_message_node = reference_function.clone();
        let sub = Arc::new(Mutex::new(
            context.create_sub_context(&on_message_node).await,
        ));
        connected_nodes.insert(on_message_node.node.lock().await.id.clone(), sub);

        let parent_node_id = context.node.node.lock().await.id.clone();

        let reader_close_notify = close_notify.clone();
        let handle = tokio::spawn(async move {
            let mut buf = vec![0u8; 65535];
            loop {
                let recv_result = tokio::select! {
                    result = socket.recv_from(&mut buf) => result,
                    _ = reader_close_notify.notified() => break,
                };

                let (len, sender_addr) = match recv_result {
                    Ok(r) => r,
                    Err(e) => {
                        tracing::warn!("UDP recv error: {}", e);
                        break;
                    }
                };

                let data = buf[..len].to_vec();
                let sender_str = sender_addr.to_string();
                let mut recursion_guard = AHashSet::new();
                recursion_guard.insert(parent_node_id.clone());

                for entry in connected_nodes.iter() {
                    let (_id, ctx) = entry.pair();
                    let mut ctx = ctx.lock().await;

                    if has_sender_addr_pin {
                        let addr_pins: Vec<_> = ctx
                            .node
                            .pins
                            .iter()
                            .filter(|(_, p)| {
                                p.pin_type == PinType::Output && p.name == "sender_addr"
                            })
                            .map(|(_, p)| p.clone())
                            .collect();
                        for pin in addr_pins {
                            pin.set_value(json!(sender_str.as_str())).await;
                        }
                    }

                    if single_string {
                        let text = String::from_utf8_lossy(&data);
                        let pins: Vec<_> = ctx
                            .node
                            .pins
                            .iter()
                            .filter(|(_, p)| {
                                p.pin_type == PinType::Output
                                    && p.data_type == VariableType::String
                                    && p.name != "payload"
                                    && p.name != "sender_addr"
                            })
                            .map(|(_, p)| p.clone())
                            .collect();
                        for pin in pins {
                            pin.set_value(json!(text.as_ref())).await;
                            break;
                        }
                    } else if single_byte {
                        let pins: Vec<_> = ctx
                            .node
                            .pins
                            .iter()
                            .filter(|(_, p)| {
                                p.pin_type == PinType::Output
                                    && p.data_type == VariableType::Byte
                                    && p.name != "payload"
                                    && p.name != "sender_addr"
                            })
                            .map(|(_, p)| p.clone())
                            .collect();
                        for pin in pins {
                            pin.set_value(json!(data.clone())).await;
                            break;
                        }
                    } else {
                        let payload_pins: Vec<_> = ctx
                            .node
                            .pins
                            .iter()
                            .filter(|(_, p)| p.pin_type == PinType::Output && p.name == "payload")
                            .map(|(_, p)| p.clone())
                            .collect();
                        for pin in payload_pins {
                            pin.set_value(json!(data.clone())).await;
                        }
                    }

                    let mut log_message = LogMessage::new("UDP on_message", LogLevel::Debug, None);
                    let run =
                        InternalNode::trigger(&mut ctx, &mut Some(recursion_guard.clone()), true)
                            .await;
                    log_message.end();
                    ctx.log(log_message);
                    ctx.end_trace();
                    if let Err(e) = run {
                        tracing::warn!("UDP on_message handler error: {:?}", e);
                    }
                }
            }

            close_notify.notify_waiters();
        });

        let timeout = timeout_seconds as u64;
        if timeout > 0 {
            tokio::select! {
                _ = handle => {}
                _ = tokio::time::sleep(std::time::Duration::from_secs(timeout)) => {
                    context.log_message("UDP receive timed out", LogLevel::Warn);
                    cached.close_notify.notify_waiters();
                }
            }
        } else {
            let _ = handle.await;
        }

        {
            let mut cache = context.cache.write().await;
            cache.remove(&session.ref_id);
        }

        context.deactivate_exec_pin("on_listening").await?;
        context.activate_exec_pin("on_close").await?;

        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "UDP requires the 'execute' feature"
        ))
    }
}
