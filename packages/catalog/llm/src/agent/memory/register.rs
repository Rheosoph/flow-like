use super::config::MemoryConfig;
use crate::generative::agent::Agent;
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic, NodeScores},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_types::{async_trait, json};

#[crate::register_node]
#[derive(Default)]
pub struct RegisterMemoryNode {}

impl RegisterMemoryNode {
    pub fn new() -> Self {
        RegisterMemoryNode {}
    }
}

#[async_trait]
impl NodeLogic for RegisterMemoryNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "agent_register_memory",
            "Register Memory",
            "Gives the agent autonomous access to persistent memory tools (_memory_search, _memory_store, _memory_compress)",
            "AI/Agents/Builder",
        );
        node.set_version(2);
        node.add_icon("/flow/icons/bot-invoke.svg");

        node.set_scores(
            NodeScores::new()
                .set_privacy(6)
                .set_security(7)
                .set_performance(7)
                .set_governance(7)
                .set_reliability(8)
                .set_cost(5)
                .build(),
        );

        node.add_input_pin(
            "agent_in",
            "Agent",
            "Agent object to register memory on",
            VariableType::Struct,
        )
        .set_schema::<Agent>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin(
            "memory_config",
            "Memory Config",
            "MemoryConfig from Create Memory Config node (bundles database + embedding model + tuning parameters)",
            VariableType::Struct,
        )
        .set_schema::<MemoryConfig>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin(
            "agent_out",
            "Agent",
            "Agent with memory tools registered",
            VariableType::Struct,
        )
        .set_schema::<Agent>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let mut agent: Agent = context.evaluate_pin("agent_in").await?;
        let memory_config: MemoryConfig = context.evaluate_pin("memory_config").await?;

        agent.set_memory(memory_config);

        context
            .set_pin_value("agent_out", json::json!(agent))
            .await?;

        Ok(())
    }
}
