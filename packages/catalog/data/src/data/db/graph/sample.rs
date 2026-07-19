use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_catalog_core::{DEFAULT_GRAPH_SAMPLE_SIZE, NodeGraphConnection};
use flow_like_types::{async_trait, json::json};

/// # Graph Sample
/// Samples objects of a given label from a graph overlay, useful for previewing
/// what an object type looks like without writing a query.
#[crate::register_node]
#[derive(Default)]
pub struct GraphSampleNode {}

impl GraphSampleNode {
    pub fn new() -> Self {
        GraphSampleNode {}
    }
}

#[async_trait]
impl NodeLogic for GraphSampleNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "graph_sample",
            "Graph Sample",
            "Samples objects of a given label from a graph overlay for previewing",
            "Data/Database/Graph/Query",
        );
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
        node.add_input_pin(
            "label",
            "Label",
            "Object type (node label) to sample from",
            VariableType::String,
        );
        node.add_input_pin(
            "count",
            "Count",
            "Number of objects to sample (capped at 500)",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(DEFAULT_GRAPH_SAMPLE_SIZE)));

        node.add_output_pin(
            "exec_out",
            "Done",
            "Sample retrieved",
            VariableType::Execution,
        );
        node.add_output_pin("error", "Error", "Sampling failed", VariableType::Execution);
        node.add_output_pin(
            "error_message",
            "Error Message",
            "Error details",
            VariableType::String,
        );
        node.add_output_pin("rows", "Objects", "Sampled objects", VariableType::Struct)
            .set_value_type(flow_like::flow::pin::ValueType::Array);

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        use flow_like_storage::databases::graph::GraphStore;

        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("error").await?;

        let conn: NodeGraphConnection = context.evaluate_pin("graph").await?;
        let label: String = context.evaluate_pin("label").await?;
        let count: i64 = context
            .evaluate_pin("count")
            .await
            .unwrap_or(DEFAULT_GRAPH_SAMPLE_SIZE as i64);
        let count = (count.max(1) as usize).min(500);

        let store = super::load_graph_store(context, &conn.cache_key).await?;

        match store.sample(&label, count).await {
            Ok(rows) => {
                context.set_pin_value("rows", json!(rows)).await?;
                context.activate_exec_pin("exec_out").await?;
            }
            Err(e) => {
                context
                    .set_pin_value("error_message", json!(e.to_string()))
                    .await?;
                context.activate_exec_pin("error").await?;
            }
        }

        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "Node execution is not enabled. Rebuild with the 'execute' feature flag."
        ))
    }
}
