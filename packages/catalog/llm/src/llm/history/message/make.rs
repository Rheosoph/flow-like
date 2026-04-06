/// # Make Message Node
/// Create a new Message object with either image or image Message Content
/// Set the message type via Role input.
/// In case of a Tool Message, the associated Tool Call Id has to be provided as well
use flow_like::flow::{
    board::Board,
    execution::context::ExecutionContext,
    node::{Node, NodeLogic, NodeScores},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_model_provider::history::{
    Content, ContentType, HistoryMessage, ImageUrl, MessageContent, Role,
};
use flow_like_types::{Value, async_trait, json::json};
use std::sync::Arc;
#[crate::register_node]
#[derive(Default)]
pub struct MakeHistoryMessageNode {}

impl MakeHistoryMessageNode {
    pub fn new() -> Self {
        MakeHistoryMessageNode {}
    }

    fn add_role_pin(node: &mut Node) {
        node.add_input_pin("role", "Role", "Author role", VariableType::String)
            .set_options(
                PinOptions::new()
                    .set_valid_values(vec![
                        "Assistant".to_string(),
                        "System".to_string(),
                        "User".to_string(),
                        "Tool".to_string(),
                    ])
                    .build(),
            )
            .set_default_value(Some(json!("User")));
    }

    fn add_type_pin(node: &mut Node) {
        node.add_input_pin("type", "Type", "Message content type", VariableType::String)
            .set_options(
                PinOptions::new()
                    .set_valid_values(vec!["Text".to_string(), "Image".to_string()])
                    .build(),
            )
            .set_default_value(Some(json!("Text")));
    }

    fn parse_role(role: &str) -> Role {
        match role {
            "Assistant" => Role::Assistant,
            "System" => Role::System,
            "Tool" => Role::Tool,
            _ => Role::User,
        }
    }

    async fn read_tool_call_id(
        role: &Role,
        context: &mut ExecutionContext,
    ) -> flow_like_types::Result<Option<String>> {
        if matches!(role, Role::Tool) {
            context.evaluate_pin("tool_call_id").await.map(Some)
        } else {
            Ok(None)
        }
    }

    async fn build_content(
        message_type: &str,
        context: &mut ExecutionContext,
    ) -> flow_like_types::Result<MessageContent> {
        match message_type {
            "Image" => {
                let image_pin: String = context.evaluate_pin("image").await?;
                Ok(MessageContent::Contents(vec![Content::Image {
                    content_type: ContentType::ImageUrl,
                    image_url: ImageUrl {
                        url: image_pin,
                        detail: None,
                    },
                }]))
            }
            _ => {
                let text_pin: String = context.evaluate_pin("text").await?;
                Ok(MessageContent::Contents(vec![Content::Text {
                    content_type: ContentType::Text,
                    text: text_pin,
                }]))
            }
        }
    }
}

#[async_trait]
impl NodeLogic for MakeHistoryMessageNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "ai_generative_make_history_message",
            "Make Message",
            "Creates a chat message with text or image content and optional tool metadata",
            "AI/Generative/History/Message",
        );
        node.add_icon("/flow/icons/message.svg");
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
        Self::add_role_pin(&mut node);
        Self::add_type_pin(&mut node);

        node.add_output_pin(
            "message",
            "Message",
            "Newly constructed chat message",
            VariableType::Struct,
        )
        .set_schema::<HistoryMessage>();

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let role_input: String = context.evaluate_pin("role").await?;
        let message_type: String = context.evaluate_pin("type").await?;
        let role = Self::parse_role(&role_input);
        let tool_call_id = Self::read_tool_call_id(&role, context).await?;
        let content = Self::build_content(&message_type, context).await?;

        let message = HistoryMessage {
            content,
            role,
            name: None,
            tool_call_id,
            tool_calls: None,
            annotations: None,
        };

        context.set_pin_value("message", json!(message)).await?;
        Ok(())
    }

    async fn on_update(&self, node: &mut Node, _board: &Board) {
        let type_pin: String = node
            .get_pin_by_name("type")
            .and_then(|pin| pin.default_value.clone())
            .and_then(|bytes| flow_like_types::json::from_slice::<Value>(&bytes).ok())
            .and_then(|json| json.as_str().map(ToOwned::to_owned))
            .unwrap_or_default();

        let role_pin: String = node
            .get_pin_by_name("role")
            .and_then(|pin| pin.default_value.clone())
            .and_then(|bytes| flow_like_types::json::from_slice::<Value>(&bytes).ok())
            .and_then(|json| json.as_str().map(ToOwned::to_owned))
            .unwrap_or_default();

        // sync role pin <-> tool call pin
        match (role_pin.as_str(), node.get_pin_by_name("tool_call_id")) {
            ("Tool", None) => {
                node.add_input_pin(
                    "tool_call_id",
                    "Tool Call Id",
                    "Tool Call Identifier",
                    VariableType::String,
                );
            }
            ("Tool", Some(_)) => {}
            (_, Some(pin)) => {
                node.pins.remove(&pin.id.clone());
            }
            _ => {}
        }

        let text_pin = node.get_pin_by_name("text");
        let image_pin = node.get_pin_by_name("image");

        if (type_pin == *"Text") && text_pin.is_some() {
            return;
        }

        if (type_pin == *"Image") && image_pin.is_some() {
            return;
        }

        let mut removal = vec![];

        if type_pin == "Text" {
            if let Some(image_pin) = image_pin {
                removal.push(image_pin.id.clone());
            }

            for id in removal {
                node.pins.remove(&id);
            }

            node.add_input_pin("text", "Text", "Text Content", VariableType::String);
            return;
        }

        if let Some(text_pin) = text_pin {
            removal.push(text_pin.id.clone());
        }

        for id in removal {
            node.pins.remove(&id);
        }

        node.add_input_pin("image", "Image", "Image Content", VariableType::String);
    }
}
