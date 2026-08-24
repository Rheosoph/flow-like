use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic, NodeScores},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_model_provider::history::{History, HistoryMessage, Role};
use flow_like_types::{async_trait, json::json};

#[crate::register_node]
#[derive(Default)]
pub struct SetSystemPromptMessageNode {}

impl SetSystemPromptMessageNode {
    pub fn new() -> Self {
        SetSystemPromptMessageNode {}
    }
}

#[async_trait]
impl NodeLogic for SetSystemPromptMessageNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "ai_generative_set_system_prompt_message",
            "Set System Message",
            "Creates or replaces the system prompt within a chat history before invoking an LLM",
            "AI/Generative/History",
        );
        node.set_flowscript_name("history", "setSystemPrompt");
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
            "Begin execution when ready to update the history",
            VariableType::Execution,
        );

        node.add_input_pin(
            "history",
            "History",
            "Existing chat history to modify",
            VariableType::Struct,
        )
        .set_schema::<History>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin(
            "message",
            "Message",
            "System-level prompt text",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_output_pin(
            "exec_out",
            "Output",
            "Signals completion once the history is updated",
            VariableType::Execution,
        );

        node.add_output_pin(
            "history_out",
            "History",
            "History including the new system prompt",
            VariableType::Struct,
        )
        .set_schema::<History>();

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let mut history: History = context.evaluate_pin("history").await?;
        let new_message: String = context.evaluate_pin("message").await?;

        let new_message = HistoryMessage::from_string(Role::System, &new_message);

        history
            .messages
            .retain(|message| message.role != Role::System);
        history.messages.insert(0, new_message);

        context.set_pin_value("history_out", json!(history)).await?;
        context.activate_exec_pin("exec_out").await?;

        Ok(())
    }
}
