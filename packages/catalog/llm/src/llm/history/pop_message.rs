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
pub struct PopHistoryMessageNode {}

impl PopHistoryMessageNode {
    pub fn new() -> Self {
        PopHistoryMessageNode {}
    }
}

#[async_trait]
impl NodeLogic for PopHistoryMessageNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "ai_generative_pop_history_message",
            "Pop Message from History",
            "Removes and returns the last message in a chat history",
            "AI/Generative/History",
        );
        node.set_flowscript_name("history", "popMessage");
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
            "Trigger when ready to pop",
            VariableType::Execution,
        );

        node.add_input_pin(
            "history",
            "History",
            "History to remove the message from",
            VariableType::Struct,
        )
        .set_schema::<History>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin(
            "exec_out",
            "Output",
            "Fires when a message was popped",
            VariableType::Execution,
        );

        node.add_output_pin(
            "history_out",
            "History",
            "History after removing the message",
            VariableType::Struct,
        )
        .set_schema::<History>();

        node.add_output_pin(
            "message",
            "Message",
            "Removed message",
            VariableType::Struct,
        )
        .set_schema::<HistoryMessage>();

        node.add_output_pin(
            "empty",
            "Empty",
            "Fires when no message was available",
            VariableType::Execution,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("empty").await?;
        let mut history: History = context.evaluate_pin("history").await?;

        if history.messages.is_empty() {
            context.activate_exec_pin("empty").await?;
            context.set_pin_value("history_out", json!(history)).await?;
            return Ok(());
        }

        if let Some(message) = history.messages.pop() {
            context.set_pin_value("message", json!(message)).await?;
            context.set_pin_value("history_out", json!(history)).await?;
            context.activate_exec_pin("exec_out").await?;
        }

        Ok(())
    }
}
