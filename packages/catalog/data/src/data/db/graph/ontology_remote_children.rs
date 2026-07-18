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
pub struct QueryRemoteOntologyChildrenNode {}

impl QueryRemoteOntologyChildrenNode {
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
impl NodeLogic for QueryRemoteOntologyChildrenNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "ontology_query_remote_children",
            "Query Remote Ontology Children",
            "Expands a parent object's containment children through an installed ontology contract from a connected project",
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
            "Parent Object Type",
            "Stable object type identifier of the parent, resolved through the installed contract",
            VariableType::String,
        );
        node.add_input_pin(
            "node_id",
            "Parent ID",
            "Identifier of the parent object whose children should be loaded",
            VariableType::Generic,
        );
        node.add_input_pin(
            "limit",
            "Limit",
            "Maximum number of child objects to return (capped at 500)",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(100)));
        node.add_output_pin(
            "exec_out",
            "Loaded",
            "Child objects loaded successfully",
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
            "Details for a failed remote children read",
            VariableType::String,
        );
        let objects = node.add_output_pin(
            "objects",
            "Children",
            "Typed child objects reached through containment edges of the installed contract",
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
        use std::collections::HashSet;

        use flow_like_storage::databases::graph::{GraphStore, lancegraph};

        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("error").await?;

        let binding_id: String = context.evaluate_pin("binding_id").await?;
        let object_type: String = context.evaluate_pin("object_type").await?;
        let node_id: flow_like_types::Value = context.evaluate_pin("node_id").await?;
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

        let parent = import.contract.nodes.iter().find(|object| {
            object.id.as_deref() == Some(object_type.as_str())
                || object.api_name.as_deref() == Some(object_type.as_str())
                || object.label == object_type
        });
        let Some(parent) = parent else {
            return fail(
                context,
                format!(
                    "Object type '{}' is not part of the installed ontology contract",
                    object_type
                ),
            )
            .await;
        };
        let parent_label = parent.label.clone();
        let parent_table = parent.table.clone();

        if let Err(error) = ensure_remote_ontology_exposed(
            context,
            &import.target_app_id,
            &import.remote_ontology_id,
        )
        .await
        {
            return fail(context, error).await;
        }

        // A parent object with no containment children is a valid, empty result.
        let remote_connection = match open_remote_project_database(
            context,
            &import.target_app_id,
            &parent_table,
            false,
        )
        .await
        {
            Ok(connection) => connection,
            Err(error) => return fail(context, error).await,
        };
        let store = match lancegraph::LanceGraphStore::new(remote_connection, import.contract, None)
            .await
        {
            Ok(store) => store,
            Err(error) => return fail(context, error).await,
        };
        let result = match store
            .overlay_children(&parent_label, node_id, Some(limit.clamp(1, 500) as usize))
            .await
        {
            Ok(result) => result,
            Err(error) => return fail(context, error).await,
        };

        // Children are the edge targets; this excludes the seeded parent node
        // even for self-referential hierarchies (parent -> same-label child).
        let child_ids = result
            .edges
            .iter()
            .map(|edge| edge.target.as_str())
            .collect::<HashSet<_>>();
        let objects = result
            .nodes
            .iter()
            .filter(|node| child_ids.contains(node.id.as_str()))
            .map(|node| node.props.clone())
            .collect::<Vec<_>>();

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
    fn remote_ontology_children_struct_output_has_schema() {
        let node = QueryRemoteOntologyChildrenNode::new().get_node();
        assert!(
            node.pins
                .values()
                .filter(|pin| pin.data_type == VariableType::Struct)
                .all(|pin| pin.schema.is_some())
        );
    }
}
