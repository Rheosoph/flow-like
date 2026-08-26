use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic, NodeScores},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_model_provider::history::{History, HistoryMessage};
use flow_like_types::{async_trait, json::json};

#[crate::register_node]
#[derive(Default)]
pub struct PushHistoryMessageNode {}

impl PushHistoryMessageNode {
    pub fn new() -> Self {
        PushHistoryMessageNode {}
    }
}

#[async_trait]
impl NodeLogic for PushHistoryMessageNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "ai_generative_add_history_message",
            "Push Message",
            "Appends a chat message to the end of a history",
            "AI/Generative/History",
        );
        node.set_flowscript_name("history", "addMessage");
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
            "Start execution when ready to append",
            VariableType::Execution,
        );

        node.add_input_pin(
            "history",
            "History",
            "Chat history to append to",
            VariableType::Struct,
        )
        .set_schema::<History>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin(
            "message",
            "Message",
            "Message that should be appended",
            VariableType::Struct,
        )
        .set_schema::<HistoryMessage>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin(
            "exec_out",
            "Output",
            "Signals completion after appending",
            VariableType::Execution,
        );

        node.add_output_pin(
            "history_out",
            "History",
            "History including the new message",
            VariableType::Struct,
        )
        .set_schema::<History>();

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let mut history: History = context.evaluate_pin("history").await?;
        let message: HistoryMessage = context.evaluate_pin("message").await?;

        history.messages.push(message);

        context.set_pin_value("history_out", json!(history)).await?;
        context.activate_exec_pin("exec_out").await?;

        Ok(())
    }
}
