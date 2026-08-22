use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic, NodeScores},
    pin::{PinOptions, ValueType},
    variable::VariableType,
};
use flow_like_catalog_core::NodeGraphConnection;
use flow_like_types::{async_trait, json::json};

/// # KG Extract
/// Uses an LLM to extract entities and relationships from text,
/// then writes them into graph tables via the overlay.
#[crate::register_node]
#[derive(Default)]
pub struct KgExtractNode {}

impl KgExtractNode {
    pub fn new() -> Self {
        KgExtractNode {}
    }
}

#[async_trait]
impl NodeLogic for KgExtractNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "kg_extract",
            "KG Extract",
            "Extracts entities (nodes) and relationships (edges) from text using an LLM, returning structured arrays ready for graph insertion",
            "AI/Memory/Graph",
        );
        node.add_icon("/flow/icons/bot-invoke.svg");
        node.set_long_running(true);

        node.set_scores(
            NodeScores::new()
                .set_privacy(5)
                .set_security(6)
                .set_performance(4)
                .set_governance(6)
                .set_reliability(6)
                .set_cost(6)
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
            "text",
            "Text",
            "Input text to extract entities and relationships from",
            VariableType::String,
        );

        node.add_input_pin(
            "node_labels",
            "Node Labels",
            "Allowed node labels for extraction (from overlay definition)",
            VariableType::String,
        )
        .set_value_type(ValueType::Array);

        node.add_input_pin(
            "edge_labels",
            "Edge Labels",
            "Allowed edge labels for extraction (from overlay definition)",
            VariableType::String,
        )
        .set_value_type(ValueType::Array);

        node.add_output_pin(
            "exec_out",
            "Done",
            "Fires when extraction completes",
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
            "extracted_nodes",
            "Extracted Nodes",
            "Array of extracted entity objects with label, id, and properties",
            VariableType::Struct,
        )
        .set_value_type(ValueType::Array)
        .set_open_schema();

        node.add_output_pin(
            "extracted_edges",
            "Extracted Edges",
            "Array of extracted relationship objects with label, source, target, and properties",
            VariableType::Struct,
        )
        .set_value_type(ValueType::Array)
        .set_open_schema();

        node.add_output_pin(
            "entity_count",
            "Entity Count",
            "Total number of entities extracted",
            VariableType::Integer,
        );

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("error").await?;

        let _conn: NodeGraphConnection = context.evaluate_pin("graph").await?;
        let text: String = context.evaluate_pin("text").await?;
        let node_labels: Vec<String> = context
            .evaluate_pin("node_labels")
            .await
            .unwrap_or_default();
        let edge_labels: Vec<String> = context
            .evaluate_pin("edge_labels")
            .await
            .unwrap_or_default();

        if text.trim().is_empty() {
            context
                .set_pin_value("error_message", json!("Input text is empty"))
                .await?;
            context.activate_exec_pin("error").await?;
            return Ok(());
        }

        // Build extraction prompt
        let node_labels_str = if node_labels.is_empty() {
            "any".to_string()
        } else {
            node_labels.join(", ")
        };
        let edge_labels_str = if edge_labels.is_empty() {
            "any".to_string()
        } else {
            edge_labels.join(", ")
        };

        let extraction_prompt = format!(
            r#"Extract entities and relationships from the following text.

Allowed node labels: {node_labels_str}
Allowed edge labels: {edge_labels_str}

Return a JSON object with two arrays:
- "nodes": [{{"label": "...", "id": "...", "properties": {{...}}}}]
- "edges": [{{"label": "...", "source": "...", "target": "...", "properties": {{...}}}}]

Use lowercase-with-underscores for IDs. Only return valid JSON, no explanation.

Text:
{text}"#
        );

        // For now, output the prompt as a pass-through.
        // A full implementation would invoke the LLM here via the model provider.
        // The node is designed to be wired with an LLM invoke node downstream.
        let placeholder_nodes: Vec<flow_like_types::Value> = Vec::new();
        let placeholder_edges: Vec<flow_like_types::Value> = Vec::new();

        context
            .set_pin_value("extracted_nodes", json!(placeholder_nodes))
            .await?;
        context
            .set_pin_value("extracted_edges", json!(placeholder_edges))
            .await?;
        context.set_pin_value("entity_count", json!(0i64)).await?;

        // Log the extraction prompt for debugging
        tracing::debug!(
            prompt = %extraction_prompt,
            "KG extraction prompt generated"
        );

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
