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

#[cfg(feature = "execute")]
use futures::StreamExt;

use super::{WebSocketConfig, WebSocketSession};

#[cfg(feature = "execute")]
use super::CachedWebSocketConnection;

#[crate::register_node]
#[derive(Default)]
pub struct WebSocketConnectNode {}

impl WebSocketConnectNode {
    pub fn new() -> Self {
        WebSocketConnectNode {}
    }
}

#[async_trait]
impl NodeLogic for WebSocketConnectNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "websocket_connect",
            "WebSocket Connect",
            "Opens a WebSocket connection. Immediately triggers on_connect with the session, \
             then invokes on_message for each incoming message. Holds execution until the \
             connection closes, then triggers on_close.",
            "Web/WebSocket",
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
            "Initiate the WebSocket connection",
            VariableType::Execution,
        );
        node.add_input_pin(
            "config",
            "Config",
            "WebSocket connection configuration (URL, optional headers, optional timeout)",
            VariableType::Struct,
        )
        .set_schema::<WebSocketConfig>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());
        node.add_output_pin(
            "on_connect",
            "On Connect",
            "Fires immediately after the connection is established",
            VariableType::Execution,
        );
        node.add_output_pin(
            "session",
            "Session",
            "WebSocket session reference for use with Send/Close nodes",
            VariableType::Struct,
        )
        .set_schema::<WebSocketSession>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin(
            "on_close",
            "On Close",
            "Fires when the WebSocket connection is closed (by server, timeout, or error)",
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

        let config: WebSocketConfig = context.evaluate_pin("config").await?;
        let referenced_fns = context.get_referenced_functions().await?;
        let handler = referenced_fns
            .first()
            .ok_or_else(|| flow_like_types::anyhow!("No on-message handler function referenced"))?
            .clone();

        let mut request = tokio_tungstenite::tungstenite::http::Request::builder().uri(&config.url);

        if let Some(headers) = &config.headers {
            for (key, value) in headers {
                request = request.header(key.as_str(), value.as_str());
            }
        }

        let request = request
            .body(())
            .map_err(|e| flow_like_types::anyhow!("Failed to build WS request: {}", e))?;

        let (ws_stream, _response) = match tokio_tungstenite::connect_async(request).await {
            Ok(result) => result,
            Err(e) => {
                context.log_message(
                    &format!("WebSocket connection failed: {}", e),
                    LogLevel::Error,
                );
                return Ok(());
            }
        };

        let (sink, stream) = futures::StreamExt::split(ws_stream);

        let ref_id = format!("ws_{}", flow_like_types::create_id());
        let close_notify = Arc::new(tokio::sync::Notify::new());

        let cached = CachedWebSocketConnection {
            sink: Arc::new(tokio::sync::Mutex::new(sink)),
            close_notify: close_notify.clone(),
            reader_handle: tokio::sync::Mutex::new(None),
        };
        let cacheable: Arc<dyn Cacheable> = Arc::new(cached);
        context.set_cache(&ref_id, cacheable).await;

        let session = WebSocketSession {
            ref_id: ref_id.clone(),
            url: config.url.clone(),
        };

        context.set_pin_value("session", json!(session)).await?;

        context.deactivate_exec_pin("exec_error").await?;
        context.activate_exec_pin("on_connect").await?;

        let on_connect_pin = context.get_pin_by_name("on_connect").await?;
        let connected_on_connect = on_connect_pin.get_connected_nodes();
        for node in connected_on_connect {
            let mut sub = context.create_sub_context(&node).await;
            sub.delegated = true;
            let mut message = LogMessage::new("WebSocket on_connect", LogLevel::Debug, None);
            let _ = InternalNode::trigger(&mut sub, &mut None, true).await;
            message.end();
            sub.log(message);
            sub.end_trace();
            context.push_sub_context(&mut sub);
        }

        spawn_message_reader(context, stream, &handler, &ref_id, close_notify.clone()).await?;

        let timeout = config.timeout_seconds;
        if timeout > 0 {
            tokio::select! {
                _ = close_notify.notified() => {}
                _ = tokio::time::sleep(std::time::Duration::from_secs(timeout)) => {
                    context.log_message("WebSocket connection timed out", LogLevel::Warn);
                    let cache = context.cache.read().await;
                    if let Some(conn) = cache.get(&ref_id)
                        && let Some(conn) = conn.as_any().downcast_ref::<CachedWebSocketConnection>() {
                            let mut sink = conn.sink.lock().await;
                            use futures::SinkExt;
                            let _ = sink.close().await;
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
            "WebSocket requires the 'execute' feature"
        ))
    }
}

#[cfg(feature = "execute")]
async fn spawn_message_reader(
    context: &mut ExecutionContext,
    mut stream: super::WsStream,
    handler: &Arc<InternalNode>,
    ref_id: &str,
    close_notify: Arc<tokio::sync::Notify>,
) -> flow_like_types::Result<()> {
    use flow_like::flow::pin::PinType;
    use flow_like_types::sync::{DashMap, Mutex};
    use tokio_tungstenite::tungstenite::Message;

    let reference_function = handler;

    let ref_node_pins = reference_function.pins.clone();
    let mut has_string_pin = false;
    let mut has_byte_pin = false;
    let mut has_payload_pin = false;
    let mut typed_pin_count: usize = 0;

    for (_, pin) in ref_node_pins.iter() {
        if pin.pin_type != PinType::Output || pin.data_type == VariableType::Execution {
            continue;
        }
        if pin.name == "payload" {
            has_payload_pin = true;
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
    let only_payload = typed_pin_count == 0 && has_payload_pin;

    let connected_nodes: Arc<DashMap<String, Arc<Mutex<ExecutionContext>>>> =
        Arc::new(DashMap::new());

    let on_message_node = reference_function.clone();
    let sub = Arc::new(Mutex::new(
        context.create_sub_context(&on_message_node).await,
    ));
    connected_nodes.insert(on_message_node.node.lock().await.id.clone(), sub);

    let parent_node_id = context.node.node.lock().await.id.clone();

    let handle = tokio::spawn(async move {
        while let Some(msg_result) = stream.next().await {
            let msg = match msg_result {
                Ok(msg) => msg,
                Err(e) => {
                    tracing::warn!("WebSocket read error: {}", e);
                    break;
                }
            };

            match &msg {
                Message::Close(_) => break,
                Message::Ping(_) | Message::Pong(_) | Message::Frame(_) => continue,
                Message::Text(_) | Message::Binary(_) => {}
            }

            let mut recursion_guard = AHashSet::new();
            recursion_guard.insert(parent_node_id.clone());

            for entry in connected_nodes.iter() {
                let (_id, ctx) = entry.pair();
                let mut ctx = ctx.lock().await;

                match &msg {
                    Message::Text(text) => {
                        if single_string {
                            let pins = &ctx.node.pins;
                            for (_, pin) in pins.iter() {
                                if pin.pin_type == PinType::Output
                                    && pin.data_type == VariableType::String
                                    && pin.name != "payload"
                                {
                                    pin.set_value(json!(text.as_str())).await;
                                    break;
                                }
                            }
                        } else if only_payload {
                            if let Ok(parsed) =
                                flow_like_types::json::from_str::<flow_like_types::Value>(text)
                            {
                                let payload_pins: Vec<_> = ctx
                                    .node
                                    .pins
                                    .iter()
                                    .filter(|(_, p)| {
                                        p.pin_type == PinType::Output && p.name == "payload"
                                    })
                                    .map(|(_, p)| p.clone())
                                    .collect();
                                for pin in payload_pins {
                                    pin.set_value(parsed.clone()).await;
                                }
                            } else {
                                let payload_pins: Vec<_> = ctx
                                    .node
                                    .pins
                                    .iter()
                                    .filter(|(_, p)| {
                                        p.pin_type == PinType::Output && p.name == "payload"
                                    })
                                    .map(|(_, p)| p.clone())
                                    .collect();
                                for pin in payload_pins {
                                    pin.set_value(json!(text.as_str())).await;
                                }
                            }
                        } else if let Ok(parsed) =
                            flow_like_types::json::from_str::<flow_like_types::Value>(text)
                        {
                            if let Some(obj) = parsed.as_object() {
                                let mut remaining = obj.clone();
                                let pins: Vec<_> = ctx
                                    .node
                                    .pins
                                    .iter()
                                    .filter(|(_, p)| {
                                        p.pin_type == PinType::Output
                                            && p.data_type != VariableType::Execution
                                            && p.name != "payload"
                                    })
                                    .map(|(_, p)| (p.name.clone(), p.clone()))
                                    .collect();

                                for (name, pin) in &pins {
                                    if let Some(val) = remaining.remove(name) {
                                        pin.set_value(val).await;
                                    } else {
                                        let normalized = name.to_lowercase().replace('_', "");
                                        let key = remaining
                                            .keys()
                                            .find(|k| {
                                                k.to_lowercase().replace('_', "") == normalized
                                            })
                                            .cloned();
                                        if let Some(k) = key
                                            && let Some(val) = remaining.remove(&k)
                                        {
                                            pin.set_value(val).await;
                                        }
                                    }
                                }
                                let payload_pins: Vec<_> = ctx
                                    .node
                                    .pins
                                    .iter()
                                    .filter(|(_, p)| {
                                        p.pin_type == PinType::Output && p.name == "payload"
                                    })
                                    .map(|(_, p)| p.clone())
                                    .collect();
                                for pin in payload_pins {
                                    pin.set_value(json!(remaining)).await;
                                }
                            } else {
                                let payload_pins: Vec<_> = ctx
                                    .node
                                    .pins
                                    .iter()
                                    .filter(|(_, p)| {
                                        p.pin_type == PinType::Output && p.name == "payload"
                                    })
                                    .map(|(_, p)| p.clone())
                                    .collect();
                                for pin in payload_pins {
                                    pin.set_value(parsed.clone()).await;
                                }
                            }
                        } else {
                            let payload_pins: Vec<_> = ctx
                                .node
                                .pins
                                .iter()
                                .filter(|(_, p)| {
                                    p.pin_type == PinType::Output && p.name == "payload"
                                })
                                .map(|(_, p)| p.clone())
                                .collect();
                            for pin in payload_pins {
                                pin.set_value(json!(text.as_str())).await;
                            }
                        }
                    }
                    Message::Binary(data) => {
                        if single_byte {
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
                                pin.set_value(json!(data.to_vec())).await;
                                break;
                            }
                        } else {
                            let payload_pins: Vec<_> = ctx
                                .node
                                .pins
                                .iter()
                                .filter(|(_, p)| {
                                    p.pin_type == PinType::Output && p.name == "payload"
                                })
                                .map(|(_, p)| p.clone())
                                .collect();
                            for pin in payload_pins {
                                pin.set_value(json!(data.to_vec())).await;
                            }
                        }
                    }
                    _ => continue,
                }

                let mut log_message =
                    LogMessage::new("WebSocket on_message", LogLevel::Debug, None);
                let run =
                    InternalNode::trigger(&mut ctx, &mut Some(recursion_guard.clone()), true).await;
                log_message.end();
                ctx.log(log_message);
                ctx.end_trace();
                if let Err(e) = run {
                    tracing::warn!("WebSocket on_message handler error: {:?}", e);
                }
            }
        }

        close_notify.notify_waiters();
    });

    let cache = context.cache.read().await;
    if let Some(conn) = cache.get(ref_id)
        && let Some(conn) = conn.as_any().downcast_ref::<CachedWebSocketConnection>()
    {
        let mut guard = conn.reader_handle.lock().await;
        *guard = Some(handle);
    }

    Ok(())
}
