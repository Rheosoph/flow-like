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

#[cfg(feature = "execute")]
use flow_like_types::Cacheable;

use super::{TcpConfig, TcpSession};

#[cfg(feature = "execute")]
use super::CachedTcpConnection;

#[crate::register_node]
#[derive(Default)]
pub struct TcpConnectNode {}

impl TcpConnectNode {
    pub fn new() -> Self {
        TcpConnectNode {}
    }
}

#[async_trait]
impl NodeLogic for TcpConnectNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "tcp_connect",
            "TCP Connect",
            "Opens a TCP connection to a remote host. Triggers on_connect with the session, \
             then invokes the on-message handler for each incoming data chunk. Holds execution \
             until the connection closes, then triggers on_close.",
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
            "Initiate the TCP connection",
            VariableType::Execution,
        );
        node.add_input_pin(
            "config",
            "Config",
            "TCP connection configuration (host, port, optional timeout)",
            VariableType::Struct,
        )
        .set_schema::<TcpConfig>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin(
            "on_connect",
            "On Connect",
            "Fires after the connection is established",
            VariableType::Execution,
        );
        node.add_output_pin(
            "session",
            "Session",
            "TCP session reference for use with Send/Close nodes",
            VariableType::Struct,
        )
        .set_schema::<TcpSession>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin(
            "on_close",
            "On Close",
            "Fires when the TCP connection is closed",
            VariableType::Execution,
        );

        node.add_output_pin(
            "exec_error",
            "Error",
            "Fires if the connection fails to establish",
            VariableType::Execution,
        );

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("on_connect").await?;
        context.deactivate_exec_pin("on_close").await?;
        context.activate_exec_pin("exec_error").await?;

        let config: TcpConfig = context.evaluate_pin("config").await?;
        let referenced_fns = context.get_referenced_functions().await?;
        let handler = referenced_fns
            .first()
            .ok_or_else(|| flow_like_types::anyhow!("No on-message handler function referenced"))?
            .clone();

        let addr = format!("{}:{}", config.host, config.port);
        let stream = match tokio::net::TcpStream::connect(&addr).await {
            Ok(s) => s,
            Err(e) => {
                context.log_message(&format!("TCP connection failed: {}", e), LogLevel::Error);
                return Ok(());
            }
        };

        let remote_addr = stream
            .peer_addr()
            .map(|a| a.to_string())
            .unwrap_or_else(|_| addr.clone());

        let (reader, writer) = tokio::io::split(stream);

        let ref_id = format!("tcp_{}", flow_like_types::create_id());
        let close_notify = Arc::new(tokio::sync::Notify::new());

        let cached = CachedTcpConnection {
            reader: Arc::new(tokio::sync::Mutex::new(reader)),
            writer: Arc::new(tokio::sync::Mutex::new(writer)),
            close_notify: close_notify.clone(),
        };
        let cacheable: Arc<dyn Cacheable> = Arc::new(cached);
        context.set_cache(&ref_id, cacheable).await;

        let session = TcpSession {
            ref_id: ref_id.clone(),
            remote_addr,
        };

        context.set_pin_value("session", json!(session)).await?;

        context.deactivate_exec_pin("exec_error").await?;
        context.activate_exec_pin("on_connect").await?;

        let on_connect_pin = context.get_pin_by_name("on_connect").await?;
        let connected_on_connect = on_connect_pin.get_connected_nodes();
        for node in connected_on_connect {
            let mut sub = context.create_sub_context(&node).await;
            sub.delegated = true;
            let mut message = LogMessage::new("TCP on_connect", LogLevel::Debug, None);
            let _ = InternalNode::trigger(&mut sub, &mut None, true).await;
            message.end();
            sub.log(message);
            sub.end_trace();
            context.push_sub_context(&mut sub);
        }

        spawn_message_reader(context, &handler, &ref_id, close_notify.clone()).await?;

        let timeout = config.timeout_seconds;
        if timeout > 0 {
            tokio::select! {
                _ = close_notify.notified() => {}
                _ = tokio::time::sleep(std::time::Duration::from_secs(timeout)) => {
                    context.log_message("TCP connection timed out", LogLevel::Warn);
                    let cache = context.cache.read().await;
                    if let Some(conn) = cache.get(&ref_id)
                        && let Some(conn) = conn.as_any().downcast_ref::<CachedTcpConnection>() {
                            use tokio::io::AsyncWriteExt;
                            let mut writer = conn.writer.lock().await;
                            let _ = writer.shutdown().await;
                        }
                }
            }
        } else {
            close_notify.notified().await;
        }

        {
            let mut cache = context.cache.write().await;
            cache.remove(&ref_id);
        }

        context.deactivate_exec_pin("on_connect").await?;
        context.activate_exec_pin("on_close").await?;

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
async fn spawn_message_reader(
    context: &mut ExecutionContext,
    handler: &Arc<flow_like::flow::execution::internal_node::InternalNode>,
    ref_id: &str,
    close_notify: Arc<tokio::sync::Notify>,
) -> flow_like_types::Result<()> {
    use flow_like::flow::pin::PinType;
    use flow_like_types::sync::{DashMap, Mutex};
    use tokio::io::AsyncReadExt;

    let reference_function = handler;

    let ref_node_pins = reference_function.pins.clone();
    let mut has_string_pin = false;
    let mut has_byte_pin = false;
    let mut typed_pin_count: usize = 0;

    for (_, pin) in ref_node_pins.iter() {
        if pin.pin_type != PinType::Output || pin.data_type == VariableType::Execution {
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
    connected_nodes.insert(on_message_node.node_id().to_string(), sub);

    let parent_node_id = context.node.node_id().to_string();

    let reader = {
        let cache = context.cache.read().await;
        let conn = cache
            .get(ref_id)
            .ok_or_else(|| flow_like_types::anyhow!("TCP connection not in cache"))?;
        let conn = conn
            .as_any()
            .downcast_ref::<CachedTcpConnection>()
            .ok_or_else(|| flow_like_types::anyhow!("Failed to downcast TCP connection"))?;
        conn.reader.clone()
    };

    let close_notify_inner = close_notify.clone();

    tokio::spawn(async move {
        let mut buf = vec![0u8; 8192];
        let close_fut = close_notify_inner.notified();
        tokio::pin!(close_fut);

        loop {
            let read_result = tokio::select! {
                _ = &mut close_fut => break,
                result = async { reader.lock().await.read(&mut buf).await } => result,
            };

            let n = match read_result {
                Ok(0) => break,
                Ok(n) => n,
                Err(e) => {
                    tracing::warn!("TCP read error: {}", e);
                    break;
                }
            };

            let data = buf[..n].to_vec();

            let mut recursion_guard = AHashSet::new();
            recursion_guard.insert(parent_node_id.clone());

            for entry in connected_nodes.iter() {
                let (_id, ctx) = entry.pair();
                let mut ctx = ctx.lock().await;

                if single_string {
                    if let Ok(text) = std::str::from_utf8(&data) {
                        let pins: Vec<_> = ctx
                            .node
                            .pins
                            .iter()
                            .filter(|(_, p)| {
                                p.pin_type == PinType::Output
                                    && p.data_type == VariableType::String
                                    && p.name != "payload"
                            })
                            .map(|(_, p)| p.clone())
                            .collect();
                        for pin in pins {
                            pin.set_value(json!(text)).await;
                            break;
                        }
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
                        })
                        .map(|(_, p)| p.clone())
                        .collect();
                    for pin in pins {
                        pin.set_value(json!(data)).await;
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
                        pin.set_value(json!(data)).await;
                    }
                }

                let mut log_message = LogMessage::new("TCP on_message", LogLevel::Debug, None);
                let run =
                    InternalNode::trigger(&mut ctx, &mut Some(recursion_guard.clone()), true).await;
                log_message.end();
                ctx.log(log_message);
                ctx.end_trace();
                if let Err(e) = run {
                    tracing::warn!("TCP on_message handler error: {:?}", e);
                }
            }
        }

        close_notify.notify_waiters();
    });

    Ok(())
}
