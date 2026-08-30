use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic, NodeScores},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_model_provider::history::History;
use flow_like_types::{async_trait, json::json};

#[crate::register_node]
#[derive(Default)]
pub struct SetHistoryUserNode {}

impl SetHistoryUserNode {
    pub fn new() -> Self {
        SetHistoryUserNode {}
    }
}

#[async_trait]
impl NodeLogic for SetHistoryUserNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "ai_generative_set_history_user",
            "Set History User",
            "Updates the user identifier stored alongside the chat history",
            "AI/Generative/History",
        );
        node.set_flowscript_name("history", "setUser");
        node.set_receiver("history");
        node.add_icon("/flow/icons/history.svg");
        node.set_version(1);
        node.set_scores(
            NodeScores::new()
                .set_privacy(10)
                .set_security(10)
                .set_performance(9)
                .set_reliability(10)
                .set_governance(9)
                .set_cost(10)
                .build(),
        );

        node.add_input_pin(
            "exec_in",
            "Input",
            "Begin execution when the update should occur",
            VariableType::Execution,
        );

        node.add_input_pin(
            "history",
            "History",
            "Existing chat history to update",
            VariableType::Struct,
        )
        .set_schema::<History>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin(
            "user",
            "User",
            "User identifier or label to attach",
            VariableType::String,
        );

        node.add_output_pin(
            "exec_out",
            "Output",
            "Signals completion once the user is stored",
            VariableType::Execution,
        );

        node.add_output_pin(
            "history_out",
            "History",
            "History reflecting the new user metadata",
            VariableType::Struct,
        )
        .set_schema::<History>();

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let mut history: History = context.evaluate_pin("history").await?;
        let user: String = context.evaluate_pin("user").await?;

        history.user = Some(user);

        context.set_pin_value("history_out", json!(history)).await?;
        context.activate_exec_pin("exec_out").await?;

        Ok(())
    }
}
