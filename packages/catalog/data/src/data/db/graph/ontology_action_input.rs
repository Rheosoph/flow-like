use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::ValueType,
    variable::VariableType,
};
use flow_like_types::{Value, async_trait, json::json};

/// Reads the typed object(s) and parameters an ontology action was invoked
/// with, from inside its implementation board. The board stays a plain event;
/// this node exposes the governed action payload as typed pins.
#[crate::register_node]
#[derive(Default)]
pub struct OntologyActionInputNode {}

impl OntologyActionInputNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for OntologyActionInputNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "ontology_action_input",
            "Ontology Action Input",
            "Reads the typed objects and parameters the ontology action was invoked with",
            "Data Studio/Actions",
        );
        node.set_version(2);
        node.add_icon("/flow/icons/database.svg");
        // The action invoke delivers its payload to the event's start node, and
        // `get_payload` only resolves for that node. This node reads that payload,
        // so it must be the entry point — a start node with no inbound exec, like
        // the other event nodes. A downstream placement can never see the payload.
        node.set_start(true);
        node.add_input_pin(
            "ontology_id",
            "Ontology",
            "Saved ontology identifier (types the outputs from the action contract)",
            VariableType::String,
        );
        node.add_input_pin(
            "action_id",
            "Action",
            "Saved ontology action identifier (types the outputs from the action contract)",
            VariableType::String,
        );
        node.add_output_pin(
            "exec_out",
            "Loaded",
            "Action input read successfully",
            VariableType::Execution,
        );
        let object = node.add_output_pin(
            "object",
            "Object",
            "The first (or only) object the action was invoked with",
            VariableType::Struct,
        );
        object.schema = Some(
            json!({
                "type": "object",
                "additionalProperties": true
            })
            .to_string(),
        );
        let objects = node.add_output_pin(
            "objects",
            "Objects",
            "Every object the action was invoked with",
            VariableType::Struct,
        );
        objects.set_value_type(ValueType::Array);
        objects.schema = Some(
            json!({
                "type": "object",
                "additionalProperties": true
            })
            .to_string(),
        );
        let parameters = node.add_output_pin(
            "parameters",
            "Parameters",
            "Typed parameters the action was invoked with",
            VariableType::Struct,
        );
        parameters.schema = Some(
            json!({
                "type": "object",
                "additionalProperties": true
            })
            .to_string(),
        );
        node.add_output_pin(
            "object_type",
            "Object Type",
            "Object type the action targets",
            VariableType::String,
        );
        node.add_output_pin(
            "object_ids",
            "Object IDs",
            "Identifiers of the targeted objects",
            VariableType::String,
        )
        .set_value_type(ValueType::Array);
        node.add_output_pin(
            "idempotency_key",
            "Idempotency Key",
            "Client-supplied retry key, if any",
            VariableType::String,
        );
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let payload = context
            .get_payload()
            .await?
            .payload
            .clone()
            .unwrap_or(Value::Null);
        let ontology = payload.get("_ontology");

        context
            .set_pin_value(
                "object",
                payload.get("object").cloned().unwrap_or(Value::Null),
            )
            .await?;
        context
            .set_pin_value(
                "objects",
                payload.get("objects").cloned().unwrap_or_else(|| json!([])),
            )
            .await?;
        context
            .set_pin_value(
                "parameters",
                payload
                    .get("parameters")
                    .cloned()
                    .unwrap_or_else(|| json!({})),
            )
            .await?;
        context
            .set_pin_value(
                "object_type",
                ontology
                    .and_then(|value| value.get("object_type"))
                    .cloned()
                    .unwrap_or_else(|| json!("")),
            )
            .await?;
        context
            .set_pin_value(
                "object_ids",
                ontology
                    .and_then(|value| value.get("object_ids"))
                    .cloned()
                    .unwrap_or_else(|| json!([])),
            )
            .await?;
        context
            .set_pin_value(
                "idempotency_key",
                ontology
                    .and_then(|value| value.get("idempotency_key"))
                    .cloned()
                    .unwrap_or(Value::Null),
            )
            .await?;

        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn action_input_struct_outputs_have_schemas() {
        let node = OntologyActionInputNode::new().get_node();
        assert!(
            node.pins
                .values()
                .filter(|pin| pin.data_type == VariableType::Struct)
                .all(|pin| pin.schema.is_some())
        );
    }

    // `get_payload` only resolves for the event's start node, so the action
    // input must be that entry point — a start node with no inbound exec pin.
    #[test]
    fn action_input_is_a_start_node_without_inbound_exec() {
        let node = OntologyActionInputNode::new().get_node();
        assert_eq!(node.start, Some(true), "action input must be a start node");
        assert!(
            !node.pins.values().any(|pin| pin.name == "exec_in"),
            "an inbound exec pin routes this node downstream, where get_payload fails"
        );
    }
}
