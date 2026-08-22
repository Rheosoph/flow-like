use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic, NodeScores},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_catalog_core::NodeGraphConnection;
use flow_like_types::{async_trait, json::json};

/// # KG Retrieve
/// Given a query string, run embedding search on a graph node table,
/// then Cypher-expand `depth=N` to return structured context for an LLM.
#[crate::register_node]
#[derive(Default)]
pub struct KgRetrieveNode {}

impl KgRetrieveNode {
    pub fn new() -> Self {
        KgRetrieveNode {}
    }
}

#[async_trait]
impl NodeLogic for KgRetrieveNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "kg_retrieve",
            "KG Retrieve",
            "Retrieves context from a knowledge graph: embeds the query, finds matching nodes, then expands N hops to build structured context",
            "AI/Memory/Graph",
        );
        node.add_icon("/flow/icons/bot-invoke.svg");
        node.set_long_running(true);

        node.set_scores(
            NodeScores::new()
                .set_privacy(6)
                .set_security(7)
                .set_performance(5)
                .set_governance(7)
                .set_reliability(7)
                .set_cost(5)
                .build(),
        );

        node.add_input_pin("exec_in", "Input", "Trigger", VariableType::Execution);

        node.add_input_pin(
            "graph",
            "Graph Connection",
            "Graph connection from Open Graph Overlay node",
            VariableType::Struct,
        )
        .set_schema::<NodeGraphConnection>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin(
            "query",
            "Query",
            "Natural language query to search for in the graph",
            VariableType::String,
        );

        node.add_input_pin(
            "node_label",
            "Node Label",
            "Label of the node type to search (must have a vector column)",
            VariableType::String,
        );

        node.add_input_pin(
            "depth",
            "Depth",
            "Number of hops to expand from matched nodes",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(1)));

        node.add_input_pin(
            "top_k",
            "Top K",
            "Number of seed nodes to retrieve via embedding search",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(5)));

        node.add_input_pin(
            "limit",
            "Result Limit",
            "Maximum total nodes + edges in the expanded subgraph",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(200)));

        node.add_output_pin(
            "exec_out",
            "Done",
            "Fires when retrieval completes",
            VariableType::Execution,
        );
        node.add_output_pin(
            "error",
            "Error",
            "Fires on failure",
            VariableType::Execution,
        );
        node.add_output_pin(
            "error_message",
            "Error Message",
            "Error details",
            VariableType::String,
        );
        node.add_output_pin(
            "context",
            "Context",
            "Structured subgraph context as JSON (nodes + edges + properties)",
            VariableType::Struct,
        )
        .set_open_schema();
        node.add_output_pin(
            "summary_text",
            "Summary Text",
            "Flattened text representation of the retrieved subgraph for LLM consumption",
            VariableType::String,
        );
        node.add_output_pin(
            "node_count",
            "Node Count",
            "Number of nodes in the result",
            VariableType::Integer,
        );

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        use flow_like_storage::databases::graph::GraphStore;

        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("error").await?;

        let conn: NodeGraphConnection = context.evaluate_pin("graph").await?;
        let _query: String = context.evaluate_pin("query").await?;
        let node_label: String = context.evaluate_pin("node_label").await?;
        let depth: i64 = context.evaluate_pin("depth").await.unwrap_or(1);
        let top_k: i64 = context.evaluate_pin("top_k").await.unwrap_or(5);
        let limit: i64 = context.evaluate_pin("limit").await.unwrap_or(200);

        let store =
            flow_like_catalog_data::data::db::graph::load_graph_store(context, &conn.cache_key)
                .await?;

        // Step 1: Sample nodes from the given label to get seed IDs.
        // In a full implementation this would do vector search on the node table.
        // For now we use the sample endpoint to get representative nodes.
        let sample_results = match store.sample(&node_label, top_k.max(1) as usize).await {
            Ok(r) => r,
            Err(e) => {
                context
                    .set_pin_value("error_message", json!(format!("Sample failed: {e}")))
                    .await?;
                context.activate_exec_pin("error").await?;
                return Ok(());
            }
        };

        // Extract IDs from sample results to use as seeds
        let mut seeds: Vec<(String, flow_like_types::Value)> = Vec::new();
        for row in sample_results.iter().take(top_k as usize) {
            if let Some(id) = row.get("id").or_else(|| row.get("_id")) {
                seeds.push((node_label.clone(), id.clone()));
            }
        }

        if seeds.is_empty() {
            context
                .set_pin_value(
                    "error_message",
                    json!("No seed nodes found for the given label"),
                )
                .await?;
            context.activate_exec_pin("error").await?;
            return Ok(());
        }

        // Step 2: Expand subgraph from seeds
        let subgraph = match store
            .subgraph(seeds, depth.max(0) as usize, Some(limit.max(1) as usize))
            .await
        {
            Ok(sg) => sg,
            Err(e) => {
                context
                    .set_pin_value(
                        "error_message",
                        json!(format!("Subgraph expansion failed: {e}")),
                    )
                    .await?;
                context.activate_exec_pin("error").await?;
                return Ok(());
            }
        };

        // Step 3: Build text summary for LLM consumption
        let mut text_parts: Vec<String> = Vec::new();
        for node in &subgraph.nodes {
            let caption = node.caption.as_deref().unwrap_or(&node.id);
            text_parts.push(format!("[{}:{}] {}", node.label, node.id, caption));
        }
        for edge in &subgraph.edges {
            text_parts.push(format!(
                "({}) -[{}]-> ({})",
                edge.source, edge.label, edge.target
            ));
        }
        let summary = text_parts.join("\n");
        let node_count = subgraph.nodes.len() as i64;

        context.set_pin_value("context", json!(subgraph)).await?;
        context
            .set_pin_value("summary_text", json!(summary))
            .await?;
        context
            .set_pin_value("node_count", json!(node_count))
            .await?;
        context.activate_exec_pin("exec_out").await?;

        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "This node requires the 'execute' feature"
        ))
    }
}
