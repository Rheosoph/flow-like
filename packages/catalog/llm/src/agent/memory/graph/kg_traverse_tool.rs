use crate::generative::agent::Agent;
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic, NodeScores},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_catalog_core::NodeGraphConnection;
use flow_like_model_provider::history::{
    HistoryFunction, HistoryFunctionParameters, HistoryJSONSchemaDefine, HistoryJSONSchemaType,
    Tool, ToolType,
};
use flow_like_types::{async_trait, json};
use std::collections::HashMap;

/// # KG Traverse Tool
/// Registers a graph traversal tool that an agent can call mid-conversation.
/// The tool lets the LLM query the knowledge graph via Cypher during a chat turn.
#[crate::register_node]
#[derive(Default)]
pub struct KgTraverseToolNode {}

impl KgTraverseToolNode {
    pub fn new() -> Self {
        KgTraverseToolNode {}
    }
}

#[async_trait]
impl NodeLogic for KgTraverseToolNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "kg_traverse_tool",
            "Register KG Traverse Tool",
            "Registers a knowledge graph traversal tool on the agent so it can query the graph mid-conversation",
            "AI/Agents/Builder",
        );
        node.add_icon("/flow/icons/bot-invoke.svg");

        node.set_scores(
            NodeScores::new()
                .set_privacy(6)
                .set_security(6)
                .set_performance(8)
                .set_governance(6)
                .set_reliability(7)
                .set_cost(2)
                .build(),
        );

        node.add_input_pin(
            "agent_in",
            "Agent",
            "Agent to register the KG tool on",
            VariableType::Struct,
        )
        .set_schema::<Agent>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin(
            "graph",
            "Graph Connection",
            "Graph connection from Open Graph Overlay node",
            VariableType::Struct,
        )
        .set_schema::<NodeGraphConnection>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin(
            "tool_name",
            "Tool Name",
            "Name for the registered tool (shown to the LLM)",
            VariableType::String,
        )
        .set_default_value(Some(json::json!("query_knowledge_graph")));

        node.add_input_pin(
            "tool_description",
            "Tool Description",
            "Description of the tool for the LLM",
            VariableType::String,
        )
        .set_default_value(Some(json::json!(
            "Query the knowledge graph using Cypher. Returns matching nodes and relationships."
        )));

        node.add_output_pin(
            "agent_out",
            "Agent",
            "Agent with the KG traverse tool registered",
            VariableType::Struct,
        )
        .set_schema::<Agent>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let mut agent: Agent = context.evaluate_pin("agent_in").await?;
        let _conn: NodeGraphConnection = context.evaluate_pin("graph").await?;
        let tool_name: String = context
            .evaluate_pin("tool_name")
            .await
            .unwrap_or_else(|_| "query_knowledge_graph".to_string());
        let tool_description: String = context.evaluate_pin("tool_description").await.unwrap_or_else(|_| {
            "Query the knowledge graph using Cypher. Returns matching nodes and relationships."
                .to_string()
        });

        // Build the tool schema: the LLM gets a `query` string parameter
        let mut properties: HashMap<String, Box<HistoryJSONSchemaDefine>> = HashMap::new();
        properties.insert(
            "query".to_string(),
            Box::new(HistoryJSONSchemaDefine {
                schema_type: Some(HistoryJSONSchemaType::String),
                description: Some(
                    "Cypher query to execute against the knowledge graph".to_string(),
                ),
                enum_values: None,
                properties: None,
                required: None,
                items: None,
            }),
        );

        let tool = Tool {
            tool_type: ToolType::Function,
            function: HistoryFunction {
                name: tool_name,
                description: Some(tool_description),
                parameters: HistoryFunctionParameters {
                    schema_type: HistoryJSONSchemaType::Object,
                    properties: Some(properties),
                    required: Some(vec!["query".to_string()]),
                },
            },
        };

        agent.add_tool(tool);

        context
            .set_pin_value("agent_out", json::json!(agent))
            .await?;

        Ok(())
    }
}
