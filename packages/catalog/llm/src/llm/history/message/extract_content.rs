use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic, NodeScores},
    variable::VariableType,
};
use flow_like_model_provider::history::{Content, HistoryMessage, MessageContent};
use flow_like_types::{async_trait, json::json};

#[crate::register_node]
#[derive(Default)]
pub struct ExtractContentNode {}

impl ExtractContentNode {
    pub fn new() -> Self {
        ExtractContentNode {}
    }
}

#[async_trait]
impl NodeLogic for ExtractContentNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "ai_generative_message_extract_content",
            "Extract Content",
            "Extracts text content from a chat message, flattening multi-part payloads",
            "AI/Generative/History/Message",
        );
        node.add_icon("/flow/icons/message.svg");
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
            "message",
            "Message",
            "Message whose text content will be extracted",
            VariableType::Struct,
        )
        .set_schema::<HistoryMessage>();

        node.add_output_pin(
            "content",
            "Content",
            "Concatenated text content",
            VariableType::String,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let message: HistoryMessage = context.evaluate_pin("message").await?;
        let content = match message.content {
            MessageContent::String(text) => text,
            MessageContent::Contents(contents) => contents
                .iter()
                .filter_map(|item| match item {
                    Content::Text { text, .. } => Some(text.as_str()),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join("\n"),
        };

        context
            .set_pin_value("content", json!(content.trim()))
            .await?;

        Ok(())
    }
}
