#[cfg(feature = "execute")]
use ahash::AHashSet;
#[cfg(not(feature = "execute"))]
use flow_like::flow::execution::context::ExecutionContext;
#[cfg(feature = "execute")]
use flow_like::flow::execution::{
    LogLevel, context::ExecutionContext, egress, internal_node::InternalNode, log::LogMessage,
};

use flow_like::flow::{
    node::{Node, NodeLogic},
    pin::{PinOptions, PinType},
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

#[cfg(feature = "execute")]
use tokio_tungstenite::tungstenite::{
    Message,
    client::IntoClientRequest,
    http::{HeaderName, HeaderValue},
};

use super::{WebSocketConfig, WebSocketSession};

#[cfg(feature = "execute")]
use super::{CachedWebSocketConnection, CachedWebSocketSink};

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
        node.set_flowscript_name("websocket", "connect");
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
        let handler = referenced_fns.first().cloned();
        if let Some(handler) = &handler {
            let handler_node = handler.node.lock().await;
            context.log_message(
                &format!(
                    "WebSocket on_message handler registered: {} [{}]",
                    handler_node.name, handler_node.id
                ),
                LogLevel::Debug,
            );
            if referenced_fns.len() > 1 {
                context.log_message(
                    "WebSocket Connect uses only the first referenced on_message handler",
                    LogLevel::Warn,
                );
            }
        } else {
            context.log_message(
                "WebSocket Connect has no referenced on_message handler; incoming messages will be ignored",
                LogLevel::Debug,
            );
        }

        let url = effective_websocket_url(&config);
        let mut request = url
            .as_str()
            .into_client_request()
            .map_err(|e| flow_like_types::anyhow!("Failed to build WS request: {}", e))?;

        if let Some(headers) = &config.headers {
            for (key, value) in headers {
                let header_name = HeaderName::from_bytes(key.as_bytes()).map_err(|e| {
                    flow_like_types::anyhow!("Invalid WS header name '{}': {}", key, e)
                })?;

                if is_reserved_websocket_header(&header_name) {
                    context.log_message(
                        &format!("Ignoring reserved WebSocket header '{}'", key),
                        LogLevel::Warn,
                    );
                    continue;
                }

                let header_value = HeaderValue::from_str(value).map_err(|e| {
                    flow_like_types::anyhow!("Invalid WS header value for '{}': {}", key, e)
                })?;

                request.headers_mut().insert(header_name, header_value);
            }
        }

        // Resolve and connect ourselves so the egress policy vets the address
        // the socket actually goes to; the request keeps the hostname for
        // Host / SNI.
        let uri = request.uri().clone();
        let host = uri
            .host()
            .ok_or_else(|| flow_like_types::anyhow!("WebSocket URL has no host"))?
            .to_string();
        let port = uri.port_u16().unwrap_or(match uri.scheme_str() {
            Some("wss") => 443,
            _ => 80,
        });
        let addrs = match egress::resolve_socket_addrs(context.execution_environment(), &host, port)
            .await
        {
            Ok(addrs) => addrs,
            Err(e) => {
                context.log_message(
                    &format!("WebSocket connection refused: {}", e),
                    LogLevel::Error,
                );
                return Ok(());
            }
        };
        let connector = if config.tls.secure {
            let tls_config = crate::web::tls::client_config(&config.tls)?
                .ok_or_else(|| flow_like_types::anyhow!("TLS client configuration is required"))?;
            Some(tokio_tungstenite::Connector::Rustls(Arc::new(tls_config)))
        } else {
            None
        };
        let connect_result = match tokio::net::TcpStream::connect(addrs.as_slice()).await {
            Ok(tcp) => {
                tokio_tungstenite::client_async_tls_with_config(request, tcp, None, connector).await
            }
            Err(e) => Err(tokio_tungstenite::tungstenite::Error::Io(e)),
        };

        let (ws_stream, _response) = match connect_result {
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
            sink: CachedWebSocketSink::Client(Arc::new(tokio::sync::Mutex::new(sink))),
            close_notify: close_notify.clone(),
            reader_handle: tokio::sync::Mutex::new(None),
        };
        let cacheable: Arc<dyn Cacheable> = Arc::new(cached);
        context.set_cache(&ref_id, cacheable).await;

        let session = WebSocketSession {
            ref_id: ref_id.clone(),
            url,
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

        spawn_message_reader(context, stream, handler, &ref_id, close_notify.clone()).await?;

        let timeout = config.timeout_seconds;
        let cancellation_token = context.get_cancellation_token();
        let mut cancelled = false;
        if timeout > 0 {
            tokio::select! {
                _ = close_notify.notified() => {}
                _ = tokio::time::sleep(std::time::Duration::from_secs(timeout)) => {
                    context.log_message("WebSocket connection timed out", LogLevel::Warn);
                    let cache = context.cache.read().await;
                    if let Some(conn) = cache.get(&ref_id)
                        && let Some(conn) = conn.as_any().downcast_ref::<CachedWebSocketConnection>() {
                            close_cached_sink(&conn.sink).await;
                        }
                }
                _ = super::super::wait_for_cancel(cancellation_token.clone()) => {
                    cancelled = true;
                    context.log_message("WebSocket connection cancelled", LogLevel::Warn);
                    close_notify.notify_waiters();
                    let cache = context.cache.read().await;
                    if let Some(conn) = cache.get(&ref_id)
                        && let Some(conn) = conn.as_any().downcast_ref::<CachedWebSocketConnection>() {
                            close_cached_sink(&conn.sink).await;
                        }
                }
            }
        } else {
            tokio::select! {
                _ = close_notify.notified() => {}
                _ = super::super::wait_for_cancel(cancellation_token.clone()) => {
                    cancelled = true;
                    context.log_message("WebSocket connection cancelled", LogLevel::Warn);
                    close_notify.notify_waiters();
                    let cache = context.cache.read().await;
                    if let Some(conn) = cache.get(&ref_id)
                        && let Some(conn) = conn.as_any().downcast_ref::<CachedWebSocketConnection>() {
                            close_cached_sink(&conn.sink).await;
                        }
                }
            }
        }

        {
            let mut cache = context.cache.write().await;
            cache.remove(&ref_id);
        }

        context.deactivate_exec_pin("on_connect").await?;
        context.activate_exec_pin("on_close").await?;

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
fn is_reserved_websocket_header(header_name: &HeaderName) -> bool {
    matches!(
        header_name.as_str(),
        "connection" | "host" | "sec-websocket-key" | "sec-websocket-version" | "upgrade"
    )
}

#[cfg(feature = "execute")]
fn effective_websocket_url(config: &WebSocketConfig) -> String {
    if config.tls.secure && config.url.starts_with("ws://") {
        format!("wss://{}", &config.url["ws://".len()..])
    } else {
        config.url.clone()
    }
}

#[cfg(feature = "execute")]
pub(crate) async fn close_cached_sink(sink: &CachedWebSocketSink) {
    use futures::SinkExt;

    match sink {
        CachedWebSocketSink::Client(sink) => {
            let mut sink = sink.lock().await;
            let _ = sink.close().await;
        }
        CachedWebSocketSink::Server(sink) => {
            let mut sink = sink.lock().await;
            let _ = sink.close().await;
        }
    }
}

#[cfg(feature = "execute")]
fn normalize_pin_key(key: &str) -> String {
    key.to_lowercase().replace('_', "")
}

#[cfg(feature = "execute")]
fn remove_payload_field(
    obj: &mut flow_like_types::json::Map<String, flow_like_types::Value>,
) -> Option<flow_like_types::Value> {
    if let Some(value) = obj.remove("payload") {
        return Some(value);
    }

    let key = obj
        .keys()
        .find(|key| normalize_pin_key(key) == "payload")
        .cloned();

    key.and_then(|key| obj.remove(&key))
}

#[cfg(feature = "execute")]
fn parse_text_message(text: &str) -> Option<flow_like_types::Value> {
    flow_like_types::json::from_str::<flow_like_types::Value>(text).ok()
}

#[cfg(feature = "execute")]
fn wrap_payload_value(value: flow_like_types::Value) -> flow_like_types::Value {
    let mut payload = flow_like_types::json::Map::new();
    payload.insert("payload".to_string(), value);
    flow_like_types::Value::Object(payload)
}

#[cfg(feature = "execute")]
async fn reset_handler_output_pins(context: &ExecutionContext) {
    let pins: Vec<_> = context
        .node
        .pins
        .iter()
        .filter(|pin| pin.pin_type == PinType::Output && pin.data_type != VariableType::Execution)
        .map(|pin| (*pin).clone())
        .collect();

    for pin in pins {
        pin.reset().await;
    }
}

#[cfg(feature = "execute")]
async fn set_named_output_pin(
    context: &ExecutionContext,
    name: &str,
    value: flow_like_types::Value,
) -> bool {
    let pins: Vec<_> = context
        .node
        .pins
        .iter()
        .filter(|pin| pin.pin_type == PinType::Output && pin.name.as_ref() == name)
        .map(|pin| (*pin).clone())
        .collect();

    let matched = !pins.is_empty();
    for pin in pins {
        pin.set_value(value.clone()).await;
    }

    matched
}

#[cfg(feature = "execute")]
async fn set_first_typed_output_pin(
    context: &ExecutionContext,
    data_type: VariableType,
    value: flow_like_types::Value,
) -> bool {
    let pin = context
        .node
        .pins
        .iter()
        .find(|pin| {
            pin.pin_type == PinType::Output
                && pin.data_type == data_type
                && pin.name.as_ref() != "payload"
        })
        .map(|pin| (*pin).clone());

    if let Some(pin) = pin {
        pin.set_value(value).await;
        return true;
    }

    false
}

#[cfg(feature = "execute")]
async fn map_object_to_output_pins(
    context: &ExecutionContext,
    remaining: &mut flow_like_types::json::Map<String, flow_like_types::Value>,
) -> bool {
    let pins: Vec<_> = context
        .node
        .pins
        .iter()
        .filter(|pin| {
            pin.pin_type == PinType::Output
                && pin.data_type != VariableType::Execution
                && pin.name.as_ref() != "payload"
        })
        .map(|pin| (pin.name.to_string(), (*pin).clone()))
        .collect();

    let mut matched = false;
    for (name, pin) in pins {
        if let Some(val) = remaining.remove(&name) {
            pin.set_value(val).await;
            matched = true;
            continue;
        }

        let normalized = normalize_pin_key(&name);
        let key = remaining
            .keys()
            .find(|key| normalize_pin_key(key) == normalized)
            .cloned();
        if let Some(key) = key
            && let Some(val) = remaining.remove(&key)
        {
            pin.set_value(val).await;
            matched = true;
        }
    }

    matched
}

#[cfg(feature = "execute")]
async fn apply_incoming_message_to_handler(
    context: &mut ExecutionContext,
    msg: &Message,
    single_string: bool,
    single_byte: bool,
    only_payload: bool,
    has_payload_pin: bool,
) -> bool {
    reset_handler_output_pins(context).await;
    let mut matched_message = false;

    match msg {
        Message::Text(text) => {
            if let Some(payload) = parse_text_message(text) {
                let mut remaining = payload.as_object().cloned();

                if !only_payload && let Some(remaining) = remaining.as_mut() {
                    matched_message |= map_object_to_output_pins(context, remaining).await;
                }

                if has_payload_pin {
                    let payload = remaining
                        .as_mut()
                        .and_then(remove_payload_field)
                        .unwrap_or_else(|| {
                            remaining
                                .take()
                                .map(flow_like_types::Value::Object)
                                .unwrap_or(payload)
                        });
                    matched_message |= set_named_output_pin(context, "payload", payload).await;
                }
            } else if single_string {
                matched_message |=
                    set_first_typed_output_pin(context, VariableType::String, json!(text.as_str()))
                        .await;
            } else if has_payload_pin {
                matched_message |= set_named_output_pin(
                    context,
                    "payload",
                    wrap_payload_value(json!(text.as_str())),
                )
                .await;
            }
        }
        Message::Binary(data) => {
            let data = data.to_vec();
            if single_byte {
                matched_message |=
                    set_first_typed_output_pin(context, VariableType::Byte, json!(data.clone()))
                        .await;
            }

            if !single_byte && has_payload_pin {
                matched_message |=
                    set_named_output_pin(context, "payload", wrap_payload_value(json!(data))).await;
            }
        }
        _ => {}
    }

    matched_message
}

#[cfg(feature = "execute")]
struct MessageHandlerContext {
    connected_nodes: Arc<
        flow_like_types::sync::DashMap<String, Arc<flow_like_types::sync::Mutex<ExecutionContext>>>,
    >,
    single_string: bool,
    single_byte: bool,
    only_payload: bool,
    has_payload_pin: bool,
}

#[cfg(feature = "execute")]
async fn create_message_handler_context(
    context: &ExecutionContext,
    reference_function: Arc<InternalNode>,
) -> MessageHandlerContext {
    use flow_like_types::sync::{DashMap, Mutex};

    let ref_node_pins = reference_function.pins.clone();
    let mut has_string_pin = false;
    let mut has_byte_pin = false;
    let mut has_payload_pin = false;
    let mut typed_pin_count: usize = 0;

    for pin in ref_node_pins.iter() {
        if pin.pin_type != PinType::Output || pin.data_type == VariableType::Execution {
            continue;
        }
        if pin.name.as_ref() == "payload" {
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

    let connected_nodes: Arc<DashMap<String, Arc<Mutex<ExecutionContext>>>> =
        Arc::new(DashMap::new());

    let mut sub_context = context.create_sub_context(&reference_function).await;
    sub_context.delegated = true;
    let sub = Arc::new(Mutex::new(sub_context));
    connected_nodes.insert(reference_function.node.lock().await.id.clone(), sub);

    MessageHandlerContext {
        connected_nodes,
        single_string: typed_pin_count == 1 && has_string_pin,
        single_byte: typed_pin_count == 1 && has_byte_pin,
        only_payload: typed_pin_count == 0 && has_payload_pin,
        has_payload_pin,
    }
}

#[cfg(feature = "execute")]
async fn spawn_message_reader(
    context: &mut ExecutionContext,
    mut stream: super::WsStream,
    handler: Option<Arc<InternalNode>>,
    ref_id: &str,
    close_notify: Arc<tokio::sync::Notify>,
) -> flow_like_types::Result<()> {
    let parent_node_id = context.node.node.lock().await.id.clone();
    let handler_context = if let Some(reference_function) = handler {
        Some(create_message_handler_context(context, reference_function).await)
    } else {
        None
    };

    let handle = tokio::spawn(async move {
        while let Some(msg_result) = stream.next().await {
            let msg = match msg_result {
                Ok(msg) => msg,
                Err(e) => {
                    if let Some(handler_context) = &handler_context {
                        for entry in handler_context.connected_nodes.iter() {
                            let (_id, ctx) = entry.pair();
                            let mut ctx = ctx.lock().await;
                            ctx.log_message(
                                &format!("WebSocket read error: {}", e),
                                LogLevel::Error,
                            );
                            if let Err(flush_err) = ctx.flush_logs().await {
                                tracing::warn!(
                                    "Failed to flush WebSocket read error log: {:?}",
                                    flush_err
                                );
                            }
                        }
                    }
                    tracing::warn!("WebSocket read error: {}", e);
                    break;
                }
            };

            match &msg {
                Message::Close(_) => break,
                Message::Ping(_) | Message::Pong(_) | Message::Frame(_) => continue,
                Message::Text(_) | Message::Binary(_) => {}
            }

            let Some(handler_context) = &handler_context else {
                continue;
            };

            let mut recursion_guard = AHashSet::new();
            recursion_guard.insert(parent_node_id.clone());

            for entry in handler_context.connected_nodes.iter() {
                let (_id, ctx) = entry.pair();
                let mut ctx = ctx.lock().await;
                let matched_message = apply_incoming_message_to_handler(
                    &mut ctx,
                    &msg,
                    handler_context.single_string,
                    handler_context.single_byte,
                    handler_context.only_payload,
                    handler_context.has_payload_pin,
                )
                .await;

                if !matched_message {
                    ctx.log_message(
                        "WebSocket incoming message did not match any output pin on the referenced handler",
                        LogLevel::Warn,
                    );
                }

                let mut log_message =
                    LogMessage::new("WebSocket on_message", LogLevel::Debug, None);
                let run =
                    InternalNode::trigger(&mut ctx, &mut Some(recursion_guard.clone()), true).await;
                if let Err(e) = &run {
                    ctx.log_message(
                        &format!("WebSocket on_message handler failed: {:?}", e),
                        LogLevel::Error,
                    );
                }
                log_message.end();
                ctx.log(log_message);
                ctx.end_trace();
                if let Err(e) = ctx.flush_logs().await {
                    tracing::warn!("Failed to flush WebSocket on_message logs: {:?}", e);
                }
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

#[cfg(all(test, feature = "execute"))]
mod tests {
    use super::*;
    use ahash::AHashMap;
    use flow_like::flow::{
        board::ExecutionStage,
        execution::{
            Run, context::ExecutionContext, internal_node::InternalNode, internal_pin::InternalPin,
        },
        node::{Node, NodeLogic},
    };
    use flow_like::{
        profile::Profile,
        state::{FlowLikeConfig, FlowLikeState},
        utils::http::HTTPClient,
    };
    use flow_like_types::{
        async_trait,
        json::json,
        sync::{Mutex, RwLock},
    };
    use futures::SinkExt;
    use std::sync::{Arc, Weak};
    use tokio::net::TcpListener;

    #[derive(Default)]
    struct NoopLogic;

    #[async_trait]
    impl NodeLogic for NoopLogic {
        fn get_node(&self) -> Node {
            Node::new("test_noop", "Test Noop", "No-op test node", "Tests")
        }

        async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
            Ok(())
        }
    }

    fn internal_node(node: Node) -> Arc<InternalNode> {
        let mut pins = AHashMap::new();
        let mut name_cache: AHashMap<String, Vec<Arc<InternalPin>>> = AHashMap::new();

        for pin in node.pins.values() {
            let internal_pin = Arc::new(InternalPin::new(pin, false));
            name_cache
                .entry(pin.name.clone())
                .or_default()
                .push(internal_pin.clone());
            pins.insert(pin.id.clone(), internal_pin);
        }

        let internal = Arc::new(InternalNode::new(
            node,
            pins,
            Arc::new(NoopLogic),
            name_cache,
        ));

        for pin in internal.pins.iter() {
            pin.init_node(Arc::downgrade(&internal));
            pin.init_connected_to(Vec::new());
            pin.init_depends_on(Vec::new());
        }

        internal
    }

    fn node_with_outputs(outputs: &[(&str, VariableType)]) -> Arc<InternalNode> {
        let mut node = Node::new("test_handler", "Test Handler", "Handler test node", "Tests");
        node.add_output_pin("exec_out", "Exec", "Execute", VariableType::Execution);
        for (name, data_type) in outputs {
            node.add_output_pin(name, name, name, data_type.clone());
        }
        internal_node(node)
    }

    async fn test_context(
        current: Arc<InternalNode>,
        nodes: Vec<Arc<InternalNode>>,
    ) -> ExecutionContext {
        let mut node_map = AHashMap::new();
        for node in nodes {
            node_map.insert(node.node_id().to_string(), node);
        }
        node_map
            .entry(current.node_id().to_string())
            .or_insert_with(|| current.clone());

        let state = Arc::new(FlowLikeState::new(
            FlowLikeConfig::new(),
            HTTPClient::new_without_refetch(),
        ));
        let variables = Arc::new(Mutex::new(AHashMap::new()));
        let cache = Arc::new(RwLock::new(AHashMap::new()));
        let run: Weak<Mutex<Run>> = Weak::new();

        ExecutionContext::new(
            Arc::new(node_map),
            &run,
            &state,
            &current,
            &variables,
            &cache,
            LogLevel::Debug,
            ExecutionStage::Dev,
            Arc::new(Profile::default()),
            None,
            Arc::new(RwLock::new(Vec::new())),
            None,
            None,
            Arc::new(AHashMap::new()),
        )
        .await
    }

    async fn output_value(context: &ExecutionContext, name: &str) -> flow_like_types::Value {
        raw_output_value(context, name).await.unwrap()
    }

    async fn raw_output_value(
        context: &ExecutionContext,
        name: &str,
    ) -> Option<flow_like_types::Value> {
        context
            .node
            .get_pin_by_name(name)
            .await
            .unwrap()
            .get_raw_value()
            .await
    }

    async fn run_local_websocket_message(
        message: Message,
        handler: Arc<InternalNode>,
    ) -> ExecutionContext {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut websocket = tokio_tungstenite::accept_async(stream).await.unwrap();
            websocket.send(message).await.unwrap();
            websocket.close(None).await.unwrap();
        });

        let mut connect_node = WebSocketConnectNode::new().get_node();
        connect_node
            .fn_refs
            .as_mut()
            .unwrap()
            .fn_refs
            .push(handler.node_id().to_string());

        let parent = internal_node(connect_node);
        let mut context = test_context(parent, vec![handler.clone()]).await;
        context
            .set_pin_value(
                "config",
                json!(WebSocketConfig {
                    url: format!("ws://{}", addr),
                    headers: None,
                    timeout_seconds: 5,
                    tls: Default::default(),
                }),
            )
            .await
            .unwrap();

        WebSocketConnectNode::new().run(&mut context).await.unwrap();
        server.await.unwrap();

        test_context(handler, Vec::new()).await
    }

    async fn run_local_websocket_tls_message(
        message: Message,
        handler: Arc<InternalNode>,
    ) -> ExecutionContext {
        let ca = crate::web::tls::create_ca_certificate("FlowLike WS Connect Test CA").unwrap();
        let leaf = crate::web::tls::create_signed_certificate(
            &ca,
            "localhost",
            vec!["localhost".to_string(), "127.0.0.1".to_string()],
            "Server",
        )
        .unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let tls_acceptor = crate::web::tls::server_acceptor(&crate::web::tls::TlsConfig {
            secure: true,
            certificate: Some(leaf),
            ..Default::default()
        })
        .unwrap()
        .unwrap();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let stream = tls_acceptor.accept(stream).await.unwrap();
            let mut websocket = tokio_tungstenite::accept_async(stream).await.unwrap();
            websocket.send(message).await.unwrap();
            websocket.close(None).await.unwrap();
        });

        let mut connect_node = WebSocketConnectNode::new().get_node();
        connect_node
            .fn_refs
            .as_mut()
            .unwrap()
            .fn_refs
            .push(handler.node_id().to_string());

        let parent = internal_node(connect_node);
        let mut context = test_context(parent, vec![handler.clone()]).await;
        context
            .set_pin_value(
                "config",
                json!(WebSocketConfig {
                    url: format!("ws://{}", addr),
                    headers: None,
                    timeout_seconds: 5,
                    tls: crate::web::tls::TlsConfig {
                        secure: true,
                        ca_certificate_pem: Some(ca.certificate_pem),
                        ..Default::default()
                    },
                }),
            )
            .await
            .unwrap();

        WebSocketConnectNode::new().run(&mut context).await.unwrap();
        server.await.unwrap();

        test_context(handler, Vec::new()).await
    }

    #[tokio::test]
    async fn referenced_handler_context_is_delegated_to_avoid_payload_lookup_regression() {
        let parent = internal_node(WebSocketConnectNode::new().get_node());
        let handler = node_with_outputs(&[("payload", VariableType::Struct)]);
        let parent_context = test_context(parent, vec![handler.clone()]).await;

        let handler_context = create_message_handler_context(&parent_context, handler).await;
        let sub_context = handler_context
            .connected_nodes
            .iter()
            .next()
            .expect("handler context should exist")
            .value()
            .clone();
        let sub_context = sub_context.lock().await;

        assert!(
            sub_context.delegated,
            "referenced event handlers must be delegated so Generic Event does not call get_payload()"
        );
    }

    #[tokio::test]
    async fn json_payload_field_maps_to_generic_payload_pin() {
        let handler = node_with_outputs(&[("payload", VariableType::Struct)]);
        let mut context = test_context(handler, Vec::new()).await;

        let matched = apply_incoming_message_to_handler(
            &mut context,
            &Message::Text(r#"{"payload":{"ok":true},"ignored":1}"#.into()),
            false,
            false,
            true,
            true,
        )
        .await;

        assert!(matched);
        assert_eq!(output_value(&context, "payload").await, json!({"ok": true}));
    }

    #[tokio::test]
    async fn json_object_maps_named_pins_and_leftovers_to_payload() {
        let handler = node_with_outputs(&[
            ("event_type", VariableType::String),
            ("payload", VariableType::Struct),
        ]);
        let mut context = test_context(handler, Vec::new()).await;

        let matched = apply_incoming_message_to_handler(
            &mut context,
            &Message::Text(r#"{"eventType":"chat","body":"hi"}"#.into()),
            false,
            false,
            false,
            true,
        )
        .await;

        assert!(matched);
        assert_eq!(output_value(&context, "event_type").await, json!("chat"));
        assert_eq!(
            output_value(&context, "payload").await,
            json!({"body": "hi"})
        );
    }

    #[tokio::test]
    async fn non_json_text_maps_to_wrapped_payload_struct() {
        let handler = node_with_outputs(&[("payload", VariableType::Struct)]);
        let mut context = test_context(handler, Vec::new()).await;

        let matched = apply_incoming_message_to_handler(
            &mut context,
            &Message::Text("plain echo".into()),
            false,
            false,
            true,
            true,
        )
        .await;

        assert!(matched);
        assert_eq!(
            output_value(&context, "payload").await,
            json!({"payload": "plain echo"})
        );
    }

    #[tokio::test]
    async fn non_json_text_uses_single_string_pin_when_payload_is_the_only_other_output() {
        let handler = node_with_outputs(&[
            ("message", VariableType::String),
            ("payload", VariableType::Struct),
        ]);
        let mut context = test_context(handler, Vec::new()).await;

        let matched = apply_incoming_message_to_handler(
            &mut context,
            &Message::Text("plain echo".into()),
            true,
            false,
            false,
            true,
        )
        .await;

        assert!(matched);
        assert_eq!(output_value(&context, "message").await, json!("plain echo"));
        assert_eq!(
            raw_output_value(&context, "payload").await,
            None,
            "non-JSON string should prefer the single string pin over the payload struct"
        );
    }

    #[tokio::test]
    async fn json_text_does_not_use_single_string_shortcut() {
        let handler = node_with_outputs(&[
            ("message", VariableType::String),
            ("payload", VariableType::Struct),
        ]);
        let mut context = test_context(handler, Vec::new()).await;

        let matched = apply_incoming_message_to_handler(
            &mut context,
            &Message::Text(r#"{"payload":{"server":"banner"}}"#.into()),
            true,
            false,
            false,
            true,
        )
        .await;

        assert!(matched);
        assert_eq!(raw_output_value(&context, "message").await, None);
        assert_eq!(
            output_value(&context, "payload").await,
            json!({"server": "banner"})
        );
    }

    #[tokio::test]
    async fn binary_message_uses_single_byte_pin_when_payload_is_the_only_other_output() {
        let handler = node_with_outputs(&[
            ("bytes", VariableType::Byte),
            ("payload", VariableType::Struct),
        ]);
        let mut context = test_context(handler, Vec::new()).await;

        let matched = apply_incoming_message_to_handler(
            &mut context,
            &Message::Binary(vec![1_u8, 2, 3].into()),
            false,
            true,
            false,
            true,
        )
        .await;

        assert!(matched);
        assert_eq!(output_value(&context, "bytes").await, json!([1, 2, 3]));
        assert_eq!(
            raw_output_value(&context, "payload").await,
            None,
            "binary data should prefer the single byte pin over the payload struct"
        );
    }

    #[tokio::test]
    async fn binary_message_without_single_byte_pin_maps_to_wrapped_payload_struct() {
        let handler = node_with_outputs(&[("payload", VariableType::Struct)]);
        let mut context = test_context(handler, Vec::new()).await;

        let matched = apply_incoming_message_to_handler(
            &mut context,
            &Message::Binary(vec![1_u8, 2, 3].into()),
            false,
            false,
            true,
            true,
        )
        .await;

        assert!(matched);
        assert_eq!(
            output_value(&context, "payload").await,
            json!({"payload": [1, 2, 3]})
        );
    }

    #[tokio::test]
    async fn websocket_connect_e2e_wraps_non_json_text_into_payload_struct() {
        let handler = node_with_outputs(&[("payload", VariableType::Struct)]);
        let context =
            run_local_websocket_message(Message::Text("plain echo".into()), handler).await;

        assert_eq!(
            output_value(&context, "payload").await,
            json!({"payload": "plain echo"})
        );
    }

    #[tokio::test]
    async fn websocket_connect_tls_e2e_wraps_non_json_text_into_payload_struct() {
        let handler = node_with_outputs(&[("payload", VariableType::Struct)]);
        let context =
            run_local_websocket_tls_message(Message::Text("secure echo".into()), handler).await;

        assert_eq!(
            output_value(&context, "payload").await,
            json!({"payload": "secure echo"})
        );
    }
}
