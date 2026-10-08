#[cfg(feature = "execute")]
use ahash::AHashSet;
#[cfg(feature = "execute")]
use flow_like::flow::{
    execution::{
        LogLevel, context::ExecutionContext, internal_node::InternalNode, log::LogMessage,
    },
    pin::PinType,
    variable::VariableType,
};
#[cfg(feature = "execute")]
use flow_like_types::{
    Value,
    json::{self, json},
    sync::{DashMap, Mutex},
};
#[cfg(feature = "execute")]
use std::sync::Arc;

#[cfg(feature = "execute")]
#[derive(Clone, Debug)]
pub enum IncomingPayload {
    Text(String),
    Binary(Vec<u8>),
}

#[cfg(feature = "execute")]
#[derive(Clone)]
pub struct MessageHandlerContext {
    connected_nodes: Arc<DashMap<String, Arc<Mutex<ExecutionContext>>>>,
    parent_node_id: String,
    metadata_pin_names: AHashSet<String>,
    single_string: bool,
    single_byte: bool,
    only_payload: bool,
    has_payload_pin: bool,
}

#[cfg(feature = "execute")]
pub async fn create_message_handler_context(
    context: &ExecutionContext,
    reference_function: Arc<InternalNode>,
    metadata_pin_names: &[&str],
) -> MessageHandlerContext {
    let metadata_pin_names: AHashSet<String> = metadata_pin_names
        .iter()
        .map(|name| normalize_pin_key(name))
        .collect();

    let mut has_string_pin = false;
    let mut has_byte_pin = false;
    let mut has_payload_pin = false;
    let mut typed_pin_count: usize = 0;

    for pin in reference_function.pins.iter() {
        if pin.pin_type != PinType::Output || pin.data_type == VariableType::Execution {
            continue;
        }

        if pin.name.as_ref() == "payload" {
            has_payload_pin = true;
            continue;
        }

        if metadata_pin_names.contains(&normalize_pin_key(&pin.name)) {
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
    sub_context.context_pin_overrides = Some(Default::default());
    connected_nodes.insert(
        reference_function.node.lock().await.id.clone(),
        Arc::new(Mutex::new(sub_context)),
    );

    MessageHandlerContext {
        connected_nodes,
        parent_node_id: context.node.node.lock().await.id.clone(),
        metadata_pin_names,
        single_string: typed_pin_count == 1 && has_string_pin,
        single_byte: typed_pin_count == 1 && has_byte_pin,
        only_payload: typed_pin_count == 0 && has_payload_pin,
        has_payload_pin,
    }
}

#[cfg(feature = "execute")]
pub async fn trigger_message_handler(
    handler: &MessageHandlerContext,
    payload: IncomingPayload,
    metadata: &[(&str, Value)],
    log_name: &'static str,
) {
    let mut recursion_guard = AHashSet::new();
    recursion_guard.insert(handler.parent_node_id.clone());

    for entry in handler.connected_nodes.iter() {
        let (_id, ctx) = entry.pair();
        let mut ctx = ctx.lock().await;
        let client = metadata
            .iter()
            .find(|(name, _)| normalize_pin_key(name) == "client")
            .map(|(_, value)| value.clone());

        let matched_message =
            apply_incoming_payload_to_handler(&mut ctx, &payload, handler, client).await;

        for (name, value) in metadata {
            set_named_output_pin(&mut ctx, name, value.clone()).await;
        }

        if !matched_message {
            ctx.log_message(
                "Incoming message did not match any output pin on the referenced handler",
                LogLevel::Warn,
            );
        }

        let mut log_message = LogMessage::new(log_name, LogLevel::Debug, None);
        let run = InternalNode::trigger(&mut ctx, &mut Some(recursion_guard.clone()), true).await;
        if let Err(e) = &run {
            ctx.log_message(
                &format!("{} handler failed: {:?}", log_name, e),
                LogLevel::Error,
            );
        }
        log_message.end();
        ctx.log(log_message);
        ctx.end_trace();
        if let Err(e) = ctx.flush_logs().await {
            tracing::warn!("Failed to flush {} logs: {:?}", log_name, e);
        }
        if let Err(e) = run {
            tracing::warn!("{} handler error: {:?}", log_name, e);
        }
    }
}

/// Dispatch one delivery and report failure to transports that stop on handler errors.
#[cfg(feature = "execute")]
pub async fn trigger_message_handler_checked(
    handler: &MessageHandlerContext,
    payload: IncomingPayload,
    metadata: &[(&str, Value)],
    log_name: &'static str,
) -> flow_like_types::Result<()> {
    if handler.connected_nodes.is_empty() {
        return Err(flow_like_types::anyhow!("Message handler is unavailable"));
    }
    for entry in handler.connected_nodes.iter() {
        // Serialize deliveries to this handler while keeping dependency state and traces per call.
        let template = entry.value().lock().await;
        let mut context = template.create_sub_context(&template.node).await;
        context.delegated = true;
        context.context_pin_overrides = Some(Default::default());
        if context.is_cancelled() {
            return Err(flow_like_types::anyhow!("Message handler was cancelled"));
        }
        let client = metadata
            .iter()
            .find(|(name, _)| normalize_pin_key(name) == "client")
            .map(|(_, value)| value.clone());
        let mut matched =
            apply_incoming_payload_to_handler(&mut context, &payload, handler, client).await;
        for (name, value) in metadata {
            matched |= set_named_output_pin(&mut context, name, value.clone()).await;
        }
        if !matched {
            return Err(flow_like_types::anyhow!(
                "Incoming message did not match any output pin on the referenced handler"
            ));
        }
        let pins: Vec<_> = context
            .node
            .pins
            .iter()
            .filter(|pin| {
                pin.pin_type == PinType::Output && pin.data_type != VariableType::Execution
            })
            .map(|pin| (*pin).clone())
            .collect();
        for pin in pins {
            let value = context
                .context_pin_overrides
                .as_ref()
                .and_then(|values| values.get(pin.id()))
                .cloned();
            if let Some(value) = &value {
                pin.validate_value(value.as_ref())?;
            }
            // Never reread shared pins: another listener may be invoking the same referenced node.
            context.override_pin_value_shared(
                pin.id(),
                value.unwrap_or_else(|| Arc::new(Value::Null)),
            );
        }
        let mut guard = Some(AHashSet::from_iter([handler.parent_node_id.clone()]));
        let cancellation = context.get_cancellation_token();
        let mut log_message = LogMessage::new(log_name, LogLevel::Debug, None);
        let result = tokio::select! {
            _ = crate::web::wait_for_cancel(cancellation) => Err(flow_like_types::anyhow!("Message handler was cancelled")),
            result = InternalNode::trigger(&mut context, &mut guard, true) => result.map_err(|error| flow_like_types::anyhow!("{log_name} handler failed: {error:?}")),
        };
        log_message.end();
        context.log(log_message);
        context.end_trace();
        if context.try_get_run().is_ok() {
            if let Err(error) = context.flush_logs().await {
                tracing::warn!("Failed to flush {} logs: {:?}", log_name, error);
            }
        }
        result?;
        if context.is_cancelled() {
            return Err(flow_like_types::anyhow!("Message handler was cancelled"));
        }
    }
    Ok(())
}

#[cfg(feature = "execute")]
fn normalize_pin_key(key: &str) -> String {
    key.to_lowercase().replace('_', "")
}

#[cfg(feature = "execute")]
fn remove_payload_field(obj: &mut json::Map<String, Value>) -> Option<Value> {
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
fn parse_text_message(text: &str) -> Option<Value> {
    json::from_str::<Value>(text).ok()
}

#[cfg(feature = "execute")]
fn wrap_payload_value(value: Value) -> Value {
    let mut payload = json::Map::new();
    payload.insert("payload".to_string(), value);
    Value::Object(payload)
}

#[cfg(feature = "execute")]
fn inject_client(value: &mut Value, client: Option<Value>) {
    let Some(client) = client else {
        return;
    };

    match value {
        Value::Object(obj) => {
            obj.insert("_client".to_string(), client);
        }
        other => {
            *other = json!({
                "payload": other.clone(),
                "_client": client,
            });
        }
    }
}

#[cfg(feature = "execute")]
async fn reset_handler_output_pins(context: &mut ExecutionContext) {
    let pins: Vec<_> = context
        .node
        .pins
        .iter()
        .filter(|pin| pin.pin_type == PinType::Output && pin.data_type != VariableType::Execution)
        .map(|pin| (*pin).clone())
        .collect();

    for pin in pins {
        if let Some(overrides) = &mut context.context_pin_overrides {
            overrides.remove(pin.id());
        }
        pin.reset().await;
    }
}

#[cfg(feature = "execute")]
async fn set_named_output_pin(context: &mut ExecutionContext, name: &str, value: Value) -> bool {
    let pins: Vec<_> = context
        .node
        .pins
        .iter()
        .filter(|pin| pin.pin_type == PinType::Output && pin.name.as_ref() == name)
        .map(|pin| (*pin).clone())
        .collect();

    let matched = !pins.is_empty();
    for pin in pins {
        context.override_pin_value_shared(pin.id(), Arc::new(value.clone()));
        pin.set_value(value.clone()).await;
    }

    matched
}

#[cfg(feature = "execute")]
async fn set_first_typed_output_pin(
    context: &mut ExecutionContext,
    data_type: VariableType,
    value: Value,
    metadata_pin_names: &AHashSet<String>,
) -> bool {
    let pin = context
        .node
        .pins
        .iter()
        .find(|pin| {
            pin.pin_type == PinType::Output
                && pin.data_type == data_type
                && pin.name.as_ref() != "payload"
                && !metadata_pin_names.contains(&normalize_pin_key(&pin.name))
        })
        .map(|pin| (*pin).clone());

    if let Some(pin) = pin {
        context.override_pin_value_shared(pin.id(), Arc::new(value.clone()));
        pin.set_value(value).await;
        return true;
    }

    false
}

#[cfg(feature = "execute")]
async fn map_object_to_output_pins(
    context: &mut ExecutionContext,
    remaining: &mut json::Map<String, Value>,
    metadata_pin_names: &AHashSet<String>,
) -> bool {
    let pins: Vec<_> = context
        .node
        .pins
        .iter()
        .filter(|pin| {
            pin.pin_type == PinType::Output
                && pin.data_type != VariableType::Execution
                && pin.name.as_ref() != "payload"
                && !metadata_pin_names.contains(&normalize_pin_key(&pin.name))
        })
        .map(|pin| (pin.name.to_string(), (*pin).clone()))
        .collect();

    let mut matched = false;
    for (name, pin) in pins {
        if let Some(val) = remaining.remove(&name) {
            context.override_pin_value_shared(pin.id(), Arc::new(val.clone()));
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
            context.override_pin_value_shared(pin.id(), Arc::new(val.clone()));
            pin.set_value(val).await;
            matched = true;
        }
    }

    matched
}

#[cfg(feature = "execute")]
async fn apply_incoming_payload_to_handler(
    context: &mut ExecutionContext,
    payload: &IncomingPayload,
    handler: &MessageHandlerContext,
    client: Option<Value>,
) -> bool {
    reset_handler_output_pins(context).await;
    let mut matched_message = false;

    match payload {
        IncomingPayload::Text(text) => {
            if let Some(mut parsed) = parse_text_message(text) {
                let has_client = client.is_some();
                inject_client(&mut parsed, client);
                let mut remaining = parsed.as_object().cloned();

                if !handler.only_payload
                    && let Some(remaining) = remaining.as_mut()
                {
                    matched_message |=
                        map_object_to_output_pins(context, remaining, &handler.metadata_pin_names)
                            .await;
                }

                if handler.has_payload_pin {
                    let payload_value = if has_client {
                        remaining
                            .take()
                            .map(Value::Object)
                            .unwrap_or_else(|| parsed.clone())
                    } else {
                        remaining
                            .as_mut()
                            .and_then(remove_payload_field)
                            .unwrap_or_else(|| {
                                remaining
                                    .take()
                                    .map(Value::Object)
                                    .unwrap_or_else(|| parsed.clone())
                            })
                    };
                    matched_message |=
                        set_named_output_pin(context, "payload", payload_value).await;
                }
            } else if handler.single_string {
                matched_message |= set_first_typed_output_pin(
                    context,
                    VariableType::String,
                    json!(text),
                    &handler.metadata_pin_names,
                )
                .await;
                if handler.has_payload_pin && client.is_some() {
                    let mut payload_value = Value::Object(json::Map::new());
                    inject_client(&mut payload_value, client);
                    matched_message |=
                        set_named_output_pin(context, "payload", payload_value).await;
                }
            } else if handler.has_payload_pin {
                let mut payload_value = wrap_payload_value(json!(text));
                inject_client(&mut payload_value, client);
                matched_message |= set_named_output_pin(context, "payload", payload_value).await;
            }
        }
        IncomingPayload::Binary(data) => {
            if handler.single_byte {
                matched_message |= set_first_typed_output_pin(
                    context,
                    VariableType::Byte,
                    json!(data),
                    &handler.metadata_pin_names,
                )
                .await;
                if handler.has_payload_pin && client.is_some() {
                    let mut payload_value = Value::Object(json::Map::new());
                    inject_client(&mut payload_value, client);
                    matched_message |=
                        set_named_output_pin(context, "payload", payload_value).await;
                }
            } else if handler.has_payload_pin {
                let mut payload_value = wrap_payload_value(json!(data));
                inject_client(&mut payload_value, client);
                matched_message |= set_named_output_pin(context, "payload", payload_value).await;
            }
        }
    }

    matched_message
}

#[cfg(all(test, feature = "execute"))]
mod tests {
    use super::*;
    use ahash::AHashMap;
    use flow_like::{
        flow::{
            board::ExecutionStage,
            execution::{
                Run, context::ExecutionContext, internal_node::InternalNode,
                internal_pin::InternalPin,
            },
            node::{Node, NodeLogic},
        },
        profile::Profile,
        state::{FlowLikeConfig, FlowLikeState},
        utils::http::HTTPClient,
    };
    use flow_like_types::{
        async_trait,
        json::json,
        sync::{Mutex, RwLock},
    };
    use std::sync::{Arc, Weak};

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
        internal_node_with_logic(node, Arc::new(NoopLogic))
    }

    fn internal_node_with_logic(node: Node, logic: Arc<dyn NodeLogic>) -> Arc<InternalNode> {
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

        let internal = Arc::new(InternalNode::new(node, pins, logic, name_cache));

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

    async fn test_context(current: Arc<InternalNode>) -> ExecutionContext {
        let mut node_map = AHashMap::new();
        node_map.insert(current.node_id().to_string(), current.clone());

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
            None,
        )
        .await
    }

    async fn output_value(context: &ExecutionContext, name: &str) -> Option<Value> {
        context
            .node
            .get_pin_by_name(name)
            .await
            .unwrap()
            .get_raw_value()
            .await
    }

    #[tokio::test]
    async fn non_json_text_wraps_payload_when_no_single_string_pin() {
        let handler = node_with_outputs(&[("payload", VariableType::Struct)]);
        let mut context = test_context(handler).await;
        let handler_context = MessageHandlerContext {
            connected_nodes: Arc::new(DashMap::new()),
            parent_node_id: "parent".to_string(),
            metadata_pin_names: AHashSet::new(),
            single_string: false,
            single_byte: false,
            only_payload: true,
            has_payload_pin: true,
        };

        let matched = apply_incoming_payload_to_handler(
            &mut context,
            &IncomingPayload::Text("hello".to_string()),
            &handler_context,
            None,
        )
        .await;

        assert!(matched);
        assert_eq!(
            output_value(&context, "payload").await,
            Some(json!({"payload": "hello"}))
        );
    }

    #[tokio::test]
    async fn non_json_text_prefers_single_string_pin() {
        let handler = node_with_outputs(&[
            ("message", VariableType::String),
            ("payload", VariableType::Struct),
        ]);
        let mut context = test_context(handler).await;
        let handler_context = MessageHandlerContext {
            connected_nodes: Arc::new(DashMap::new()),
            parent_node_id: "parent".to_string(),
            metadata_pin_names: AHashSet::new(),
            single_string: true,
            single_byte: false,
            only_payload: false,
            has_payload_pin: true,
        };

        let matched = apply_incoming_payload_to_handler(
            &mut context,
            &IncomingPayload::Text("hello".to_string()),
            &handler_context,
            None,
        )
        .await;

        assert!(matched);
        assert_eq!(
            output_value(&context, "message").await,
            Some(json!("hello"))
        );
        assert_eq!(output_value(&context, "payload").await, None);
    }

    #[tokio::test]
    async fn json_object_maps_named_fields_and_leftovers_to_payload() {
        let handler = node_with_outputs(&[
            ("event_type", VariableType::String),
            ("payload", VariableType::Struct),
        ]);
        let mut context = test_context(handler).await;
        let handler_context = MessageHandlerContext {
            connected_nodes: Arc::new(DashMap::new()),
            parent_node_id: "parent".to_string(),
            metadata_pin_names: AHashSet::new(),
            single_string: true,
            single_byte: false,
            only_payload: false,
            has_payload_pin: true,
        };

        let matched = apply_incoming_payload_to_handler(
            &mut context,
            &IncomingPayload::Text(r#"{"eventType":"x","body":"hello"}"#.to_string()),
            &handler_context,
            None,
        )
        .await;

        assert!(matched);
        assert_eq!(output_value(&context, "event_type").await, Some(json!("x")));
        assert_eq!(
            output_value(&context, "payload").await,
            Some(json!({"body": "hello"}))
        );
    }

    #[tokio::test]
    async fn metadata_pins_are_not_used_as_single_string_payload_targets() {
        let handler = node_with_outputs(&[
            ("topic", VariableType::String),
            ("payload", VariableType::Struct),
        ]);
        let mut context = test_context(handler).await;
        let mut metadata = AHashSet::new();
        metadata.insert("topic".to_string());
        let handler_context = MessageHandlerContext {
            connected_nodes: Arc::new(DashMap::new()),
            parent_node_id: "parent".to_string(),
            metadata_pin_names: metadata,
            single_string: false,
            single_byte: false,
            only_payload: true,
            has_payload_pin: true,
        };

        let matched = apply_incoming_payload_to_handler(
            &mut context,
            &IncomingPayload::Text("hello".to_string()),
            &handler_context,
            None,
        )
        .await;

        assert!(matched);
        assert_eq!(output_value(&context, "topic").await, None);
        assert_eq!(
            output_value(&context, "payload").await,
            Some(json!({"payload": "hello"}))
        );
    }

    #[tokio::test]
    async fn binary_prefers_single_byte_pin() {
        let handler = node_with_outputs(&[
            ("bytes", VariableType::Byte),
            ("payload", VariableType::Struct),
        ]);
        let mut context = test_context(handler).await;
        let handler_context = MessageHandlerContext {
            connected_nodes: Arc::new(DashMap::new()),
            parent_node_id: "parent".to_string(),
            metadata_pin_names: AHashSet::new(),
            single_string: false,
            single_byte: true,
            only_payload: false,
            has_payload_pin: true,
        };

        let matched = apply_incoming_payload_to_handler(
            &mut context,
            &IncomingPayload::Binary(vec![1, 2, 3]),
            &handler_context,
            None,
        )
        .await;

        assert!(matched);
        assert_eq!(
            output_value(&context, "bytes").await,
            Some(json!([1, 2, 3]))
        );
        assert_eq!(output_value(&context, "payload").await, None);
    }

    #[tokio::test]
    async fn binary_wraps_payload_without_single_byte_pin() {
        let handler = node_with_outputs(&[("payload", VariableType::Struct)]);
        let mut context = test_context(handler).await;
        let handler_context = MessageHandlerContext {
            connected_nodes: Arc::new(DashMap::new()),
            parent_node_id: "parent".to_string(),
            metadata_pin_names: AHashSet::new(),
            single_string: false,
            single_byte: false,
            only_payload: true,
            has_payload_pin: true,
        };

        let matched = apply_incoming_payload_to_handler(
            &mut context,
            &IncomingPayload::Binary(vec![1, 2, 3]),
            &handler_context,
            None,
        )
        .await;

        assert!(matched);
        assert_eq!(
            output_value(&context, "payload").await,
            Some(json!({"payload": [1, 2, 3]}))
        );
    }

    #[tokio::test]
    async fn client_metadata_is_injected_into_payload() {
        let handler = node_with_outputs(&[
            ("_client", VariableType::Struct),
            ("payload", VariableType::Struct),
        ]);
        let mut context = test_context(handler).await;
        let mut metadata = AHashSet::new();
        metadata.insert("client".to_string());
        let handler_context = MessageHandlerContext {
            connected_nodes: Arc::new(DashMap::new()),
            parent_node_id: "parent".to_string(),
            metadata_pin_names: metadata,
            single_string: false,
            single_byte: false,
            only_payload: true,
            has_payload_pin: true,
        };
        let client = json!({"ref_id": "ws_1", "url": "ws://127.0.0.1:1234"});

        let matched = apply_incoming_payload_to_handler(
            &mut context,
            &IncomingPayload::Text("hello".to_string()),
            &handler_context,
            Some(client.clone()),
        )
        .await;
        set_named_output_pin(&mut context, "_client", client.clone()).await;

        assert!(matched);
        assert_eq!(
            output_value(&context, "payload").await,
            Some(json!({"payload": "hello", "_client": client}))
        );
        assert_eq!(output_value(&context, "_client").await, Some(client));
    }

    struct CheckedProbe {
        fail: bool,
        cancel: bool,
        observed: Arc<std::sync::Mutex<Vec<Value>>>,
    }
    #[async_trait]
    impl NodeLogic for CheckedProbe {
        fn get_node(&self) -> Node {
            Node::new("checked_probe", "Probe", "Checked dispatch test", "Tests")
        }
        async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
            assert!(
                context.context_state.is_empty(),
                "Each delivery starts with fresh dependency state"
            );
            context.context_state.insert("visited".into(), json!(true));
            let value = context.evaluate_pin::<Value>("message").await?;
            self.observed.lock().unwrap().push(value);
            if self.cancel {
                context.get_cancellation_token().unwrap().cancel();
            }
            if self.fail {
                return Err(flow_like_types::anyhow!("deliberate handler failure"));
            }
            Ok(())
        }
    }
    async fn checked_probe(
        fail: bool,
        cancel: bool,
    ) -> (MessageHandlerContext, Arc<std::sync::Mutex<Vec<Value>>>) {
        let observed = Arc::new(std::sync::Mutex::new(Vec::new()));
        let mut node = Node::new("checked_handler", "Handler", "Test", "Tests");
        node.add_output_pin("message", "Message", "Text", VariableType::String);
        node.add_output_pin("exec_out", "Done", "Done", VariableType::Execution);
        let node = internal_node_with_logic(
            node,
            Arc::new(CheckedProbe {
                fail,
                cancel,
                observed: observed.clone(),
            }),
        );
        let mut parent = test_context(node.clone()).await;
        parent.set_cancellation_token(flow_like_types::tokio_util::sync::CancellationToken::new());
        let mut handler = create_message_handler_context(&parent, node, &[]).await;
        handler.parent_node_id = "listener".into();
        (handler, observed)
    }

    #[tokio::test]
    async fn checked_dispatch_propagates_failure_and_cancellation() {
        for (fail, cancel) in [(false, false), (true, false), (false, true)] {
            let (handler, observed) = checked_probe(fail, cancel).await;
            let result = trigger_message_handler_checked(
                &handler,
                IncomingPayload::Text("value".into()),
                &[],
                "test",
            )
            .await;
            assert_eq!(result.is_err(), fail || cancel);
            assert_eq!(observed.lock().unwrap().as_slice(), &[json!("value")]);
        }
    }

    #[tokio::test]
    async fn checked_dispatch_uses_fresh_values_and_state_for_each_delivery() {
        let (handler, observed) = checked_probe(false, false).await;
        for message in ["first", "second"] {
            trigger_message_handler_checked(
                &handler,
                IncomingPayload::Text(message.into()),
                &[],
                "test",
            )
            .await
            .unwrap();
        }
        assert_eq!(
            observed.lock().unwrap().as_slice(),
            &[json!("first"), json!("second")]
        );
    }
    #[tokio::test]
    async fn checked_dispatch_clears_a_previous_field_when_next_payload_omits_it() {
        let (handler, observed) = checked_probe(false, false).await;
        trigger_message_handler_checked(
            &handler,
            IncomingPayload::Text(r#"{"message":"present"}"#.into()),
            &[],
            "test",
        )
        .await
        .unwrap();
        assert!(
            trigger_message_handler_checked(
                &handler,
                IncomingPayload::Text("{}".into()),
                &[],
                "test"
            )
            .await
            .is_err()
        );
        let entry = handler.connected_nodes.iter().next().unwrap();
        let context = entry.value().lock().await;
        assert_eq!(output_value(&context, "message").await, None);
        assert_eq!(observed.lock().unwrap().as_slice(), &[json!("present")]);
    }
    #[tokio::test]
    async fn handler_pin_overrides_isolate_two_contexts_sharing_one_node() {
        let node = node_with_outputs(&[("message", VariableType::String)]);
        let parent = test_context(node.clone()).await;
        let handler = create_message_handler_context(&parent, node.clone(), &[]).await;
        let mut first = parent.create_sub_context(&node).await;
        let mut second = parent.create_sub_context(&node).await;
        assert!(
            apply_incoming_payload_to_handler(
                &mut first,
                &IncomingPayload::Text("first".into()),
                &handler,
                None
            )
            .await
        );
        assert!(
            apply_incoming_payload_to_handler(
                &mut second,
                &IncomingPayload::Text("second".into()),
                &handler,
                None
            )
            .await
        );
        assert_eq!(
            first.evaluate_pin::<String>("message").await.unwrap(),
            "first"
        );
        assert_eq!(
            second.evaluate_pin::<String>("message").await.unwrap(),
            "second"
        );
    }
}
