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
pub struct GetSystemPromptNode {}

impl GetSystemPromptNode {
    pub fn new() -> Self {
        GetSystemPromptNode {}
    }
}

#[async_trait]
impl NodeLogic for GetSystemPromptNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "ai_generative_get_system_prompt",
            "Get System Prompt",
            "Extracts the first system-level message from a chat history for downstream use",
            "AI/Generative/History",
        );
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
            "history",
            "History",
            "Chat history that contains the system prompt",
            VariableType::Struct,
        )
        .set_schema::<History>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin(
            "system_prompt",
            "System Prompt",
            "Extracted system-level message",
            VariableType::Struct,
        )
        .set_schema::<HistoryMessage>();

        node.add_output_pin(
            "success",
            "Found",
            "True when a system message was located",
            VariableType::Boolean,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let history: History = context.evaluate_pin("history").await?;
        let system_prompt = history.messages.iter().find_map(|message| {
            if message.role == Role::System {
                Some(message.clone())
            } else {
                None
            }
        });

        if let Some(system_prompt) = system_prompt {
            context.set_pin_value("success", json!(true)).await?;
            context
                .set_pin_value("system_prompt", json!(system_prompt))
                .await?;
            return Ok(());
        };

        context.set_pin_value("success", json!(false)).await?;
        Ok(())
    }
}
