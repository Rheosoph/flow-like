use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::ValueType,
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json};

#[cfg(feature = "execute")]
use crate::remote_util::{ensure_remote_ontology_exposed, open_remote_project_database};

#[crate::register_node]
#[derive(Default)]
pub struct QueryRemoteOntologyObjectsNode {}

impl QueryRemoteOntologyObjectsNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[cfg(feature = "execute")]
async fn fail(context: &mut ExecutionContext, error: impl ToString) -> flow_like_types::Result<()> {
    context
        .set_pin_value("error_message", json!(error.to_string()))
        .await?;
    context.activate_exec_pin("error").await?;
    Ok(())
}

#[async_trait]
impl NodeLogic for QueryRemoteOntologyObjectsNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "ontology_query_remote_objects",
            "Query Remote Ontology Objects",
            "Reads a bounded object preview through an installed ontology contract from a connected project",
            "Data Studio/Remote Objects",
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
            "object_type",
            "Object Type",
            "Stable object type identifier resolved through the installed contract",
            VariableType::String,
        );
        node.add_input_pin(
            "limit",
            "Limit",
            "Maximum number of remote objects to return (capped at 500)",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(100)));
        node.add_output_pin(
            "exec_out",
            "Loaded",
            "Remote objects loaded successfully",
            VariableType::Execution,
        );
        node.add_output_pin(
            "error",
            "Error",
            "The installed contract or its connected project could not be read",
            VariableType::Execution,
        );
        node.add_output_pin(
            "error_message",
            "Error Message",
            "Details for a failed remote object read",
            VariableType::String,
        );
        let objects = node.add_output_pin(
            "objects",
            "Objects",
            "Typed objects from the installed remote ontology object type",
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
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        use flow_like_storage::databases::graph::lancegraph;

        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("error").await?;

        let binding_id: String = context.evaluate_pin("binding_id").await?;
        let object_type: String = context.evaluate_pin("object_type").await?;
        let limit: i64 = context.evaluate_pin("limit").await.unwrap_or(100);
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

        let object = import.contract.nodes.iter().find(|object| {
            object.id.as_deref() == Some(object_type.as_str())
                || object.api_name.as_deref() == Some(object_type.as_str())
                || object.label == object_type
        });
        let Some(object) = object else {
            return fail(
                context,
                format!(
                    "Object type '{}' is not part of the installed ontology contract",
                    object_type
                ),
            )
            .await;
        };

        if let Err(error) = ensure_remote_ontology_exposed(
            context,
            &import.target_app_id,
            &import.remote_ontology_id,
        )
        .await
        {
            return fail(context, error).await;
        }

        let remote_connection = match open_remote_project_database(
            context,
            &import.target_app_id,
            &object.table,
            false,
        )
        .await
        {
            Ok(connection) => connection,
            Err(error) => return fail(context, error).await,
        };
        let objects = match lancegraph::sample_overlay_object(
            &remote_connection,
            object,
            limit.clamp(1, 500) as usize,
        )
        .await
        {
            Ok(objects) => objects,
            Err(error) => return fail(context, error).await,
        };

        context.set_pin_value("objects", json!(objects)).await?;
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
    fn remote_ontology_query_struct_output_has_schema() {
        let node = QueryRemoteOntologyObjectsNode::new().get_node();
        assert!(
            node.pins
                .values()
                .filter(|pin| pin.data_type == VariableType::Struct)
                .all(|pin| pin.schema.is_some())
        );
    }
}
