use flow_like::flow::{
    execution::{LogLevel, context::ExecutionContext},
    node::{Node, NodeLogic},
    pin::ValueType,
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json};

#[cfg(feature = "execute")]
use crate::remote_util::{
    ensure_remote_ontology_exposed, invoke_and_collect, remote_app_session, validate_path_id,
};

const DEFAULT_REMOTE_ACTION_TIMEOUT_SECS: i64 = 120;
const MAX_REMOTE_ACTION_TIMEOUT_SECS: i64 = 1800;

#[crate::register_node]
#[derive(Default)]
pub struct RemoteOntologyActionRequestNode {}

impl RemoteOntologyActionRequestNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[cfg(feature = "execute")]
async fn fail(context: &mut ExecutionContext, error: impl ToString) -> flow_like_types::Result<()> {
    let error = error.to_string();
    context.log_message(
        &format!("Remote ontology action failed: {error}"),
        LogLevel::Error,
    );
    context.set_pin_value("error_message", json!(error)).await?;
    context.activate_exec_pin("error").await?;
    Ok(())
}

#[async_trait]
impl NodeLogic for RemoteOntologyActionRequestNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "ontology_action_request_remote",
            "Invoke Remote Ontology Action",
            "Runs a governed ontology action in a connected project through an installed contract; the producer validates and executes it authoritatively",
            "Data Studio/Remote Actions",
        );
        node.set_version(1);
        node.add_icon("/flow/icons/database.svg");
        node.add_input_pin("exec_in", "Input", "", VariableType::Execution);
        node.add_input_pin(
            "binding_id",
            "Installed Ontology",
            "Local identifier of the installed remote ontology contract",
            VariableType::String,
        );
        node.add_input_pin(
            "action_id",
            "Action",
            "Action identifier resolved through the installed contract",
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
        node.add_input_pin(
            "timeout",
            "Timeout (s)",
            "Maximum seconds to wait for the remote action to finish (capped at 1800)",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(DEFAULT_REMOTE_ACTION_TIMEOUT_SECS)));
        node.add_output_pin(
            "exec_out",
            "Invoked",
            "Remote action finished successfully",
            VariableType::Execution,
        );
        node.add_output_pin(
            "error",
            "Error",
            "The installed contract, its connected project, or the action run failed",
            VariableType::Execution,
        );
        node.add_output_pin(
            "error_message",
            "Error Message",
            "Details for a failed remote action",
            VariableType::String,
        );
        let result = node.add_output_pin(
            "result",
            "Result",
            "Result payload emitted by the producer's action run",
            VariableType::Struct,
        );
        result.schema = Some(
            json!({
                "type": "object",
                "additionalProperties": true
            })
            .to_string(),
        );
        node.add_output_pin(
            "run_id",
            "Run ID",
            "Identifier of the producer-side action run",
            VariableType::String,
        );
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        use flow_like_storage::databases::graph::lancegraph;

        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("error").await?;

        let binding_id: String = context.evaluate_pin("binding_id").await?;
        let action_id: String = context.evaluate_pin("action_id").await?;
        let objects: flow_like_types::Value = context.evaluate_pin("objects").await?;
        let parameters: flow_like_types::Value = context
            .evaluate_pin("parameters")
            .await
            .unwrap_or_else(|_| json!({}));
        // Generated bindings may expand a flat schema into one pin per property.
        let parameters = super::merge_parameter_pins(context, parameters).await;
        let timeout: i64 = context
            .evaluate_pin("timeout")
            .await
            .unwrap_or(DEFAULT_REMOTE_ACTION_TIMEOUT_SECS);
        let timeout = timeout.clamp(1, MAX_REMOTE_ACTION_TIMEOUT_SECS) as u64;

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

        let import = match lancegraph::load_ontology_import(&connection, &binding_id).await {
            Ok(import) => import,
            Err(error) => return fail(context, error).await,
        };
        if !import.bindings_enabled {
            return fail(context, "The installed ontology bindings are disabled").await;
        }

        let action = import
            .contract
            .actions
            .iter()
            .find(|action| action.id == action_id && action.enabled);
        let Some(action) = action else {
            return fail(
                context,
                format!(
                    "Action '{}' is not part of the installed ontology contract or is disabled",
                    action_id
                ),
            )
            .await;
        };
        let Some(object_mapping) = import.contract.nodes.iter().find(|object| {
            object.id.as_deref() == Some(action.object_type.as_str())
                || object.api_name.as_deref() == Some(action.object_type.as_str())
                || object.label == action.object_type
        }) else {
            return fail(
                context,
                "The action object type is not part of the contract",
            )
            .await;
        };
        let identity_column = match lancegraph::effective_node_id_column_checked(
            &import.contract,
            &object_mapping.label,
        ) {
            Ok(Some(column)) => column,
            Ok(None) => {
                return fail(
                    context,
                    "The action object type has no governed identity column",
                )
                .await;
            }
            Err(error) => return fail(context, error).await,
        };

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
        // Consumer-side validation is advisory fast feedback only; the producer
        // revalidates against its current, authoritative schema at invoke.
        if let Some(schema) = &action.parameter_schema
            && let Err(error) =
                flow_like_catalog_core::validate_ontology_action_parameters(schema, &parameters)
        {
            return fail(
                context,
                format!("Action parameters do not match the installed contract: {error}"),
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

        // Revocation of the source project's exposure takes effect on the next
        // run even though the installed contract snapshot stays stable.
        if let Err(error) = ensure_remote_ontology_exposed(
            context,
            &import.target_app_id,
            &import.remote_ontology_id,
            None,
        )
        .await
        {
            return fail(context, error).await;
        }

        let ontology_id = match validate_path_id(&import.remote_ontology_id, "remote ontology") {
            Ok(value) => value,
            Err(error) => return fail(context, error).await,
        };
        let action_path_id = match validate_path_id(&action.id, "ontology action") {
            Ok(value) => value,
            Err(error) => return fail(context, error).await,
        };

        let session = match remote_app_session(context, &import.target_app_id).await {
            Ok(session) => session,
            Err(error) => return fail(context, error).await,
        };
        let url = session.url(&format!(
            "graph/{ontology_id}/actions/{action_path_id}/invoke"
        ));
        let body = json!({
            "object_refs": object_refs,
            "parameters": parameters,
        });
        let outcome = match invoke_and_collect(&session, &url, &body, timeout).await {
            Ok(outcome) => outcome,
            Err(error) => return fail(context, error).await,
        };
        if let Err(error) = outcome.ensure_ok() {
            return fail(context, error).await;
        }

        context
            .set_pin_value(
                "result",
                outcome
                    .generic_result
                    .clone()
                    .unwrap_or(flow_like_types::Value::Null),
            )
            .await?;
        context
            .set_pin_value("run_id", json!(outcome.run_id.clone().unwrap_or_default()))
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
    fn remote_action_struct_pins_have_schemas() {
        let node = RemoteOntologyActionRequestNode::new().get_node();
        assert!(
            node.pins
                .values()
                .filter(|pin| pin.data_type == VariableType::Struct)
                .all(|pin| pin.schema.is_some())
        );
    }

    #[test]
    fn remote_action_exposes_no_producer_coordinates() {
        let node = RemoteOntologyActionRequestNode::new().get_node();
        assert!(node.pins.values().all(|pin| pin.name != "board_id"
            && pin.name != "board_version"
            && pin.name != "start_node_id"
            && pin.name != "event_id"
            && pin.name != "target_app_id"
            && pin.name != "remote_ontology_id"));
    }
}
