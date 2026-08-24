use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::ValueType,
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json};

#[crate::register_node]
#[derive(Default)]
pub struct OntologyActionRequestNode {}

#[cfg(feature = "execute")]
async fn fail(context: &mut ExecutionContext, error: impl ToString) -> flow_like_types::Result<()> {
    context
        .set_pin_value("error_message", json!(error.to_string()))
        .await?;
    context.activate_exec_pin("error").await?;
    Ok(())
}

#[async_trait]
impl NodeLogic for OntologyActionRequestNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "ontology_action_request",
            "Prepare Ontology Action",
            "Builds a validated, typed action request from a Data Studio action binding",
            "Data Studio/Actions",
        );
        node.set_flowscript_name("ontology", "prepareAction");
        node.set_version(1);
        node.add_icon("/flow/icons/database.svg");
        node.add_input_pin("exec_in", "Input", "", VariableType::Execution);
        node.add_input_pin(
            "ontology_id",
            "Ontology",
            "Saved ontology identifier",
            VariableType::String,
        );
        node.add_input_pin(
            "action_id",
            "Action",
            "Saved ontology action identifier",
            VariableType::String,
        );
        let objects = node.add_input_pin(
            "objects",
            "Objects",
            "Objects selected for the action",
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
        let parameters = node.add_input_pin(
            "parameters",
            "Parameters",
            "Typed parameters supplied to the action",
            VariableType::Struct,
        );
        parameters.set_default_value(Some(json!({})));
        parameters.schema = Some(
            json!({
                "type": "object",
                "additionalProperties": true
            })
            .to_string(),
        );
        node.add_output_pin(
            "exec_out",
            "Prepared",
            "Action request prepared successfully",
            VariableType::Execution,
        );
        node.add_output_pin(
            "error",
            "Error",
            "The binding is missing, disabled, or invalid",
            VariableType::Execution,
        );
        node.add_output_pin(
            "error_message",
            "Error Message",
            "Details for a failed action request",
            VariableType::String,
        );
        let action_request = node.add_output_pin(
            "action_request",
            "Action Request",
            "Validated action binding, objects, and parameters",
            VariableType::Struct,
        );
        action_request.schema = Some(
            json!({
                "type": "object",
                "required": ["ontology_id", "action_id", "object_type", "object_refs", "parameters"],
                "properties": {
                    "ontology_id": { "type": "string" },
                    "action_id": { "type": "string" },
                    "object_type": { "type": "string" },
                    "object_refs": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "required": ["object_type", "id"],
                            "properties": {
                                "object_type": { "type": "string" },
                                "id": {}
                            }
                        }
                    },
                    "parameters": { "type": "object" }
                }
            })
            .to_string(),
        );
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        use flow_like_storage::databases::graph::lancegraph;

        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("error").await?;
        let ontology_id: String = context.evaluate_pin("ontology_id").await?;
        let action_id: String = context.evaluate_pin("action_id").await?;
        let objects: flow_like_types::Value = context.evaluate_pin("objects").await?;
        let parameters: flow_like_types::Value = context
            .evaluate_pin("parameters")
            .await
            .unwrap_or_else(|_| json!({}));
        // Generated bindings may expand a flat schema into one pin per property.
        let parameters = super::merge_parameter_pins(context, parameters).await;
        let execution = context
            .execution_cache
            .clone()
            .ok_or(flow_like_types::anyhow!("No execution cache found"))?;
        let app_id = execution.app_id.clone();
        let database = if let Some(credentials) = &context.credentials {
            credentials.to_db(&app_id).await?
        } else {
            let path = execution.get_storage(false)?.child("db");
            context
                .app_state
                .config
                .read()
                .await
                .callbacks
                .build_project_database
                .clone()
                .ok_or(flow_like_types::anyhow!("No database builder found"))?(path)
        };
        let connection = context
            .app_state
            .with_lance_session(database)
            .execute()
            .await?;
        let ontology = match lancegraph::load_overlay(&connection, &ontology_id).await {
            Ok(ontology) => ontology,
            Err(error) => return fail(context, error).await,
        };
        let action = ontology
            .actions
            .iter()
            .find(|action| action.id == action_id && action.enabled);
        let Some(action) = action else {
            return fail(
                context,
                format!("Enabled action '{}' was not found", action_id),
            )
            .await;
        };
        let Some(object_mapping) = ontology.nodes.iter().find(|object| {
            object.id.as_deref() == Some(action.object_type.as_str())
                || object.api_name.as_deref() == Some(action.object_type.as_str())
                || object.label == action.object_type
        }) else {
            return fail(context, "The action object type is no longer mapped").await;
        };
        let identity_column =
            lancegraph::effective_node_id_column(&ontology, &object_mapping.label)
                .unwrap_or_else(|| object_mapping.id_column.clone());
        let Some(objects) = objects.as_array() else {
            return fail(context, "Action objects must be an array").await;
        };
        if objects.is_empty() || (!action.allow_bulk && objects.len() != 1) || objects.len() > 100 {
            return fail(
                context,
                if action.allow_bulk {
                    "Bulk actions require between 1 and 100 objects"
                } else {
                    "This action requires exactly one object"
                },
            )
            .await;
        }
        if !parameters.is_object() {
            return fail(context, "Action parameters must be an object").await;
        }
        if let Some(schema) = &action.parameter_schema
            && let Err(error) =
                flow_like_catalog_core::validate_ontology_action_parameters(schema, &parameters)
        {
            return fail(
                context,
                format!("Action parameters do not match the saved contract: {error}"),
            )
            .await;
        }

        let mut seen_ids = std::collections::HashSet::with_capacity(objects.len());
        let mut object_refs = Vec::with_capacity(objects.len());
        for object in objects {
            let Some(object) = object.as_object() else {
                return fail(context, "Each selected action object must be an object").await;
            };
            let Some(id) = object.get(&identity_column) else {
                return fail(
                    context,
                    format!(
                        "Selected objects must include identity property '{}'",
                        identity_column
                    ),
                )
                .await;
            };
            let key = match id {
                flow_like_types::Value::String(value) => value.clone(),
                flow_like_types::Value::Number(value) => value.to_string(),
                flow_like_types::Value::Bool(value) => value.to_string(),
                _ => return fail(context, "Object identities must be scalar values").await,
            };
            if key.is_empty() || !seen_ids.insert(key) {
                return fail(
                    context,
                    "Duplicate or empty object identities are not allowed",
                )
                .await;
            }
            object_refs.push(json!({
                "object_type": action.object_type,
                "id": id,
            }));
        }
        context
            .set_pin_value(
                "action_request",
                json!({
                    "ontology_id": ontology_id,
                    "action_id": action.id,
                    "object_type": action.object_type,
                    "object_refs": object_refs,
                    "parameters": parameters,
                }),
            )
            .await?;
        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "Node execution is not enabled. Rebuild with the 'execute' feature flag."
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ontology_action_struct_pins_have_schemas() {
        let node = OntologyActionRequestNode::default().get_node();
        assert!(
            node.pins
                .values()
                .filter(|pin| pin.data_type == VariableType::Struct)
                .all(|pin| pin.schema.is_some())
        );
        let request_schema = node
            .pins
            .values()
            .find(|pin| pin.name == "action_request")
            .and_then(|pin| pin.schema.as_deref())
            .unwrap();
        assert!(request_schema.contains("object_refs"));
        assert!(!request_schema.contains("board_id"));
        assert!(!request_schema.contains("event_id"));
    }
}
