use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::ValueType,
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json};

#[crate::register_node]
#[derive(Default)]
pub struct QueryOntologyObjectsNode {}

impl QueryOntologyObjectsNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for QueryOntologyObjectsNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "ontology_query_objects",
            "Query Ontology Objects",
            "Reads a bounded object preview through a saved Data Studio ontology",
            "Data Studio/Objects",
        );
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
            "object_type",
            "Object Type",
            "Stable object type label resolved by the ontology",
            VariableType::String,
        );
        node.add_input_pin(
            "limit",
            "Limit",
            "Maximum number of objects to return (capped at 500)",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(100)));
        node.add_output_pin(
            "exec_out",
            "Loaded",
            "Objects loaded successfully",
            VariableType::Execution,
        );
        node.add_output_pin(
            "error",
            "Error",
            "The ontology or its source could not be read",
            VariableType::Execution,
        );
        node.add_output_pin(
            "error_message",
            "Error Message",
            "Details for a failed object read",
            VariableType::String,
        );
        let objects = node.add_output_pin(
            "objects",
            "Objects",
            "Typed objects from the selected ontology object type",
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

        let ontology_id: String = context.evaluate_pin("ontology_id").await?;
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
        let ontology = match lancegraph::load_overlay(&connection, &ontology_id).await {
            Ok(ontology) => ontology,
            Err(error) => {
                context
                    .set_pin_value("error_message", json!(error.to_string()))
                    .await?;
                context.activate_exec_pin("error").await?;
                return Ok(());
            }
        };
        if !ontology.bindings_enabled {
            context
                .set_pin_value(
                    "error_message",
                    json!("Object bindings are disabled for this ontology"),
                )
                .await?;
            context.activate_exec_pin("error").await?;
            return Ok(());
        }
        let objects = match lancegraph::sample_overlay(
            &connection,
            &ontology,
            &object_type,
            limit.clamp(1, 500) as usize,
        )
        .await
        {
            Ok(objects) => objects,
            Err(error) => {
                context
                    .set_pin_value("error_message", json!(error.to_string()))
                    .await?;
                context.activate_exec_pin("error").await?;
                return Ok(());
            }
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
    fn ontology_query_struct_pins_have_schemas() {
        let node = QueryOntologyObjectsNode::new().get_node();
        assert!(
            node.pins
                .values()
                .filter(|pin| pin.data_type == VariableType::Struct)
                .all(|pin| pin.schema.is_some())
        );
    }
}
