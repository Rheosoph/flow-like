/// # Register Work IQ Tools Node
/// Adds the Microsoft Work IQ remote MCP server to an Agent. The signed-in user's Work IQ token
/// is resolved when the agent connects, so no bearer is stored in the Agent pin.
use crate::generative::agent::Agent;
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic, NodeScores},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_catalog_data_support::work_iq::{WORK_IQ_PROVIDER_ID, WORK_IQ_SCOPE};
use flow_like_types::{async_trait, json::json};

/// Work IQ tools that only read. The mutation tools (`create_entity`, `update_entity`,
/// `delete_entity`, `do_action`) are left out unless writes are allowed.
#[cfg(feature = "execute")]
const READ_ONLY_TOOLS: [&str; 7] = [
    "fetch",
    "fetch_blob",
    "call_function",
    "ask",
    "list_agents",
    "get_schema",
    "search_paths",
];

#[crate::register_node]
#[derive(Default)]
pub struct RegisterWorkIqToolsNode {}

#[async_trait]
impl NodeLogic for RegisterWorkIqToolsNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "agent_register_work_iq_tools",
            "Register Work IQ Tools",
            "Gives the agent the Microsoft Work IQ tools (mail, calendar, files, people, chats, sites and Microsoft 365 Copilot) as the signed-in user. A tenant admin must enable Work IQ; tool calls are billed in Copilot Credits.",
            "AI/Agents/Builder",
        );
        node.set_flowscript_name("agent", "registerWorkIqTools");
        node.set_receiver("agent_in");
        node.add_icon("/flow/icons/copilot.svg");
        node.set_scores(
            NodeScores::new()
                .set_privacy(6)
                .set_security(8)
                .set_performance(6)
                .set_governance(8)
                .set_reliability(7)
                .set_cost(3)
                .build(),
        );

        node.add_input_pin(
            "agent_in",
            "Agent",
            "Agent object to add the Work IQ tools to",
            VariableType::Struct,
        )
        .set_schema::<Agent>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin(
            "allow_writes",
            "Allow Writes",
            "Also register the tools that create, update, delete or send Microsoft 365 data. Tenant policy must allow them too.",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(false)));

        node.add_output_pin(
            "agent_out",
            "Agent",
            "Agent object with the Work IQ tools registered",
            VariableType::Struct,
        )
        .set_schema::<Agent>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_oauth_provider(WORK_IQ_PROVIDER_ID);
        node.add_required_oauth_scopes(WORK_IQ_PROVIDER_ID, vec![WORK_IQ_SCOPE]);
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        use flow_like_catalog_data_support::work_iq::WORK_IQ_MCP_URL;

        let mut agent: Agent = context.evaluate_pin("agent_in").await?;
        let allow_writes: bool = context.evaluate_pin("allow_writes").await.unwrap_or(false);

        if !context.has_oauth_token(WORK_IQ_PROVIDER_ID) {
            return Err(flow_like_types::anyhow!(
                "Microsoft Work IQ is not authenticated or its token expired. Authorize access when prompted."
            ));
        }

        let tool_filter = (!allow_writes).then(|| {
            READ_ONLY_TOOLS
                .iter()
                .map(|tool| tool.to_string())
                .collect()
        });

        agent.add_mcp_server(super::McpServerConfig {
            uri: WORK_IQ_MCP_URL.to_string(),
            tool_filter,
            auth_header: None,
            remote_app_id: None,
            remote_event_id: None,
            oauth_provider_id: Some(WORK_IQ_PROVIDER_ID.to_string()),
            custom_headers: Default::default(),
        });

        context.set_pin_value("agent_out", json!(agent)).await?;
        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "LLM processing requires the 'execute' feature"
        ))
    }
}
