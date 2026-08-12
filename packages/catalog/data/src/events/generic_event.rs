use std::sync::Arc;

use flow_like::flow::{
    execution::{context::ExecutionContext, internal_pin::InternalPin},
    node::{Node, NodeLogic},
    pin::PinType,
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json};
pub mod push_generic_result;

fn normalize_key(key: &str) -> String {
    key.to_lowercase().replace('_', "")
}

fn find_matching_key(
    obj: &flow_like_types::json::Map<String, flow_like_types::Value>,
    pin_name: &str,
) -> Option<String> {
    let normalized_pin = normalize_key(pin_name);

    obj.keys()
        .find(|key| normalize_key(key) == normalized_pin)
        .cloned()
}

fn collect_pins(context: &ExecutionContext) -> (Vec<Arc<InternalPin>>, Vec<String>) {
    let mut exec_pins = Vec::new();
    let mut output_pins = Vec::new();

    for pin in context.node.pins.values() {
        if pin.pin_type == PinType::Output {
            if pin.data_type == VariableType::Execution {
                exec_pins.push(pin.clone());
            } else if pin.name != "payload" {
                output_pins.push(pin.name.clone());
            }
        }
    }

    (exec_pins, output_pins)
}

async fn try_match_and_set_pin(
    context: &mut ExecutionContext,
    obj: &flow_like_types::json::Map<String, flow_like_types::Value>,
    pin_name: &str,
) -> flow_like_types::Result<Option<String>> {
    if let Some(value) = obj.get(pin_name) {
        context.set_pin_value(pin_name, value.clone()).await?;
        return Ok(Some(pin_name.to_string()));
    }

    if let Some(key) = find_matching_key(obj, pin_name)
        && let Some(value) = obj.get(&key)
    {
        context.set_pin_value(pin_name, value.clone()).await?;
        return Ok(Some(key));
    }

    Ok(None)
}

async fn map_payload_to_pins(
    context: &mut ExecutionContext,
    obj: &mut flow_like_types::json::Map<String, flow_like_types::Value>,
    output_pins: &[String],
) -> flow_like_types::Result<()> {
    let mut matched_keys = Vec::new();

    for pin_name in output_pins {
        if let Some(key) = try_match_and_set_pin(context, obj, pin_name).await? {
            matched_keys.push(key);
        }
    }

    for key in matched_keys {
        obj.remove(&key);
    }

    Ok(())
}

async fn activate_all_exec_pins(
    context: &ExecutionContext,
    exec_pins: Vec<Arc<InternalPin>>,
) -> flow_like_types::Result<()> {
    for exec_pin in exec_pins {
        context.activate_exec_pin_ref(&exec_pin).await?;
    }
    Ok(())
}

async fn process_payload(
    context: &mut ExecutionContext,
    output_pins: &[String],
) -> flow_like_types::Result<()> {
    let payload_data = context.get_payload().await?;
    let mut payload = payload_data
        .payload
        .clone()
        .unwrap_or_else(|| flow_like_types::Value::Object(flow_like_types::json::Map::new()));

    if let Some(obj) = payload.as_object_mut() {
        map_payload_to_pins(context, obj, output_pins).await?;
        context.set_pin_value("payload", json!(obj)).await?;
    } else {
        context.set_pin_value("payload", payload).await?;
    }

    Ok(())
}

#[crate::register_node]
#[derive(Default)]
pub struct GenericEventNode {}

impl GenericEventNode {
    pub fn new() -> Self {
        GenericEventNode {}
    }
}

#[async_trait]
impl NodeLogic for GenericEventNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "events_generic",
            "Generic Event",
            "A generic event without input or output",
            "Events",
        );
        node.add_icon("/flow/icons/event.svg");
        node.set_start(true);
        node.set_can_be_referenced_by_fns(true);

        node.add_output_pin(
            "exec_out",
            "Exec Out",
            "Starting an event",
            VariableType::Execution,
        );

        node.add_output_pin(
            "payload",
            "Payload",
            "The payload of the event",
            VariableType::Struct,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let (exec_pins, output_pins) = collect_pins(context);

        if context.delegated {
            return activate_all_exec_pins(context, exec_pins).await;
        }

        process_payload(context, &output_pins).await?;
        activate_all_exec_pins(context, exec_pins).await?;

        Ok(())
    }
}
