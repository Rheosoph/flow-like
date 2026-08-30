use crate::generative::agent::Agent;
/// # Register Function Tools Node
/// Adds referenced Flow-Like functions as tool references to an Agent object.
/// Function references are stored and converted to tools at execution time to keep data slim.
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic, NodeScores},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_types::{async_trait, json};

#[crate::register_node]
#[derive(Default)]
pub struct RegisterFunctionToolsNode {}

impl RegisterFunctionToolsNode {
    pub fn new() -> Self {
        RegisterFunctionToolsNode {}
    }
}

#[async_trait]
impl NodeLogic for RegisterFunctionToolsNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "agent_register_function_tools",
            "Register Function Tools",
            "Adds referenced Flow-Like functions as callable tool references to an Agent",
            "AI/Agents/Builder",
        );
        node.set_flowscript_name("agent", "registerFunctionTools");
        node.set_receiver("agent_in");
        node.set_version(2);
        node.add_icon("/flow/icons/bot-invoke.svg");
        node.set_can_reference_fns(true);

        node.set_scores(
            NodeScores::new()
                .set_privacy(5)
                .set_security(5)
                .set_performance(8)
                .set_governance(6)
                .set_reliability(7)
                .set_cost(2)
                .build(),
        );

        node.add_input_pin(
            "agent_in",
            "Agent",
            "Agent object to add function references to",
            VariableType::Struct,
        )
        .set_schema::<Agent>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin(
            "agent_out",
            "Agent",
            "Agent object with registered function tool references",
            VariableType::Struct,
        )
        .set_schema::<Agent>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let mut agent: Agent = context.evaluate_pin("agent_in").await?;

        let referenced_functions = context.get_referenced_functions().await?;

        for referenced_node in referenced_functions {
            let node_guard = referenced_node.node.lock().await;
            let node_id = node_guard.id.clone();
            let node_name = node_guard.name.clone();
            drop(node_guard);

            // Store only the reference, not the full tool definition
            agent.add_function_ref(node_id, node_name);
        }

        context
            .set_pin_value("agent_out", json::json!(agent))
            .await?;

        Ok(())
    }
}
