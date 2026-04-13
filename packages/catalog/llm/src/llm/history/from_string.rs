/// # History From String Node
/// Create a new History struct with string content as User Message
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic, NodeScores},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_model_provider::history::{
    Content, ContentType, History, HistoryMessage, MessageContent, Role,
};
use flow_like_types::{async_trait, json::json};

#[crate::register_node]
#[derive(Default)]
pub struct HistoryFromStringNode {}

impl HistoryFromStringNode {
    pub fn new() -> Self {
        HistoryFromStringNode {}
    }
}

#[async_trait]
impl NodeLogic for HistoryFromStringNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "ai_generative_history_from_string",
            "History From String",
            "Creates a ChatHistory Struct from String (as User Message)",
            "AI/Generative/History",
        );
        node.add_icon("/flow/icons/history.svg");
        node.set_version(1);

        // Pure helper to build a one-message History from local strings.
        node.set_scores(
            NodeScores::new()
                .set_privacy(10)
                .set_security(10)
                .set_performance(9)
                .set_governance(9)
                .set_reliability(10)
                .set_cost(10)
                .build(),
        );

        node.add_input_pin(
            "model_name",
            "Model Name",
            "Model Name",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_input_pin(
            "message",
            "Message",
            "User Message String",
            VariableType::String,
        );

        node.add_output_pin("history", "History", "ChatHistory", VariableType::Struct)
            .set_schema::<History>()
            .set_options(PinOptions::new().set_enforce_schema(true).build());

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        // fetch inputs
        let model_name: String = context.evaluate_pin("model_name").await?;
        let message_str: String = context.evaluate_pin("message").await?;

        // make history
        let history = History::new(
            model_name,
            vec![HistoryMessage {
                role: Role::User,
                content: MessageContent::Contents(vec![Content::Text {
                    content_type: ContentType::Text,
                    text: message_str,
                }]),
                name: None,
                tool_call_id: None,
                tool_calls: None,
                annotations: None,
            }],
        );

        // set outputs
        context.set_pin_value("history", json!(history)).await?;
        Ok(())
    }
}
