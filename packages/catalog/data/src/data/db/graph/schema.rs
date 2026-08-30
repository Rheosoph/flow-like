use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_catalog_core::NodeGraphConnection;
use flow_like_types::async_trait;
#[cfg(feature = "execute")]
use flow_like_types::json::json;

/// # Graph Schema
/// Retrieves the schema (labels and properties) of a graph overlay.
#[crate::register_node]
#[derive(Default)]
pub struct GraphSchemaNode {}

impl GraphSchemaNode {
    pub fn new() -> Self {
        GraphSchemaNode {}
    }
}

#[async_trait]
impl NodeLogic for GraphSchemaNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "graph_schema",
            "Graph Schema",
            "Retrieves the schema (labels and properties) of a graph overlay",
            "Data/Database/Graph/Meta",
        );
        node.set_flowscript_name("db.graph", "schema");
        node.set_receiver("graph");
        node.add_icon("/flow/icons/database.svg");

        node.add_input_pin("exec_in", "Input", "", VariableType::Execution);
        node.add_input_pin(
            "graph",
            "Graph Connection",
            "Graph connection reference",
            VariableType::Struct,
        )
        .set_schema::<NodeGraphConnection>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin(
            "exec_out",
            "Done",
            "Schema retrieved",
            VariableType::Execution,
        );
        node.add_output_pin(
            "schema",
            "Schema",
            "Graph schema with labels and properties",
            VariableType::Struct,
        )
        .set_schema::<flow_like_storage_contracts::graph::GraphSchemaResult>();

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        use flow_like_storage::databases::graph::GraphStore;

        context.deactivate_exec_pin("exec_out").await?;

        let conn: NodeGraphConnection = context.evaluate_pin("graph").await?;
        let store = super::load_graph_store(context, &conn.cache_key).await?;
        let schema = store.schema().await?;

        context.set_pin_value("schema", json!(schema)).await?;
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
