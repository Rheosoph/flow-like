use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic, NodeScores},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_catalog_core::NodeGraphConnection;
use flow_like_types::{async_trait, json::json};

/// # KG Summarize
/// Given a subgraph payload (nodes + edges), produces a natural-language summary
/// suitable for injection into an LLM context window.
#[crate::register_node]
#[derive(Default)]
pub struct KgSummarizeNode {}

impl KgSummarizeNode {
    pub fn new() -> Self {
        KgSummarizeNode {}
    }
}

#[async_trait]
impl NodeLogic for KgSummarizeNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "kg_summarize",
            "KG Summarize",
            "Converts a subgraph (nodes + edges) into a natural-language summary for LLM consumption",
            "AI/Memory/Graph",
        );
        node.add_icon("/flow/icons/bot-invoke.svg");

        node.set_scores(
            NodeScores::new()
                .set_privacy(6)
                .set_security(8)
                .set_performance(8)
                .set_governance(7)
                .set_reliability(9)
                .set_cost(1)
                .build(),
        );

        node.add_input_pin("exec_in", "Input", "Trigger", VariableType::Execution);

        node.add_input_pin(
            "graph",
            "Graph Connection",
            "Graph connection reference (for label metadata)",
            VariableType::Struct,
        )
        .set_schema::<NodeGraphConnection>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin(
            "subgraph",
            "Subgraph",
            "Subgraph payload (output from KG Retrieve, Neighbors, or Subgraph nodes)",
            VariableType::Struct,
        );

        node.add_input_pin(
            "max_tokens",
            "Max Tokens",
            "Approximate maximum token budget for the summary (controls verbosity)",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(500)));

        node.add_input_pin(
            "include_properties",
            "Include Properties",
            "Whether to include node/edge properties in the summary",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(true)));

        node.add_output_pin(
            "exec_out",
            "Done",
            "Fires when summarization completes",
            VariableType::Execution,
        );
        node.add_output_pin(
            "summary",
            "Summary",
            "Natural-language summary of the subgraph",
            VariableType::String,
        );
        node.add_output_pin(
            "node_count",
            "Node Count",
            "Number of nodes in the input subgraph",
            VariableType::Integer,
        );
        node.add_output_pin(
            "edge_count",
            "Edge Count",
            "Number of edges in the input subgraph",
            VariableType::Integer,
        );

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        use flow_like_storage::databases::graph::SubgraphResult;

        context.deactivate_exec_pin("exec_out").await?;

        let _conn: NodeGraphConnection = context.evaluate_pin("graph").await?;
        let subgraph: SubgraphResult = context.evaluate_pin("subgraph").await?;
        let _max_tokens: i64 = context.evaluate_pin("max_tokens").await.unwrap_or(500);
        let include_properties: bool = context
            .evaluate_pin("include_properties")
            .await
            .unwrap_or(true);

        let mut lines: Vec<String> = Vec::new();

        // Header
        lines.push(format!(
            "Knowledge graph context: {} nodes, {} edges.",
            subgraph.nodes.len(),
            subgraph.edges.len()
        ));

        // Group nodes by label
        let mut label_groups: std::collections::HashMap<String, Vec<&flow_like_storage::databases::graph::SubgraphNode>> =
            std::collections::HashMap::new();
        for node in &subgraph.nodes {
            label_groups
                .entry(node.label.clone())
                .or_default()
                .push(node);
        }

        for (label, group_nodes) in &label_groups {
            lines.push(format!("\n{} ({}):", label, group_nodes.len()));
            for gn in group_nodes {
                let caption = gn.caption.as_deref().unwrap_or(&gn.id);
                if include_properties && !gn.props.is_null() {
                    let props_str = if let Some(obj) = gn.props.as_object() {
                        obj.iter()
                            .map(|(k, v)| format!("{k}={v}"))
                            .collect::<Vec<_>>()
                            .join(", ")
                    } else {
                        gn.props.to_string()
                    };
                    lines.push(format!("  - {caption} [{props_str}]"));
                } else {
                    lines.push(format!("  - {caption}"));
                }
            }
        }

        // Edges
        if !subgraph.edges.is_empty() {
            lines.push("\nRelationships:".to_string());
            for edge in &subgraph.edges {
                if include_properties && !edge.props.is_null() && edge.props != json!({}) {
                    let props_str = if let Some(obj) = edge.props.as_object() {
                        obj.iter()
                            .map(|(k, v)| format!("{k}={v}"))
                            .collect::<Vec<_>>()
                            .join(", ")
                    } else {
                        edge.props.to_string()
                    };
                    lines.push(format!(
                        "  ({}) -[{} {{{}}}]-> ({})",
                        edge.source, edge.label, props_str, edge.target
                    ));
                } else {
                    lines.push(format!(
                        "  ({}) -[{}]-> ({})",
                        edge.source, edge.label, edge.target
                    ));
                }
            }
        }

        let summary = lines.join("\n");
        let node_count = subgraph.nodes.len() as i64;
        let edge_count = subgraph.edges.len() as i64;

        context.set_pin_value("summary", json!(summary)).await?;
        context
            .set_pin_value("node_count", json!(node_count))
            .await?;
        context
            .set_pin_value("edge_count", json!(edge_count))
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
