use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic, NodeScores},
    pin::PinOptions,
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
        node.set_version(2);
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

        node.add_output_pin(
            "parts",
            "Parts",
            "Ordered text and media content parts",
            VariableType::Struct,
        )
        .set_schema::<Content>()
        .set_value_type(flow_like::flow::pin::ValueType::Array)
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        for (name, label, description) in [
            ("images", "Images", "Image URLs or data URIs"),
            ("audio", "Audio", "Audio URLs or data URIs"),
            ("videos", "Videos", "Video URLs or data URIs"),
            ("documents", "Documents", "Document URLs or data URIs"),
        ] {
            node.add_output_pin(name, label, description, VariableType::String)
                .set_value_type(flow_like::flow::pin::ValueType::Array);
        }

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let message: HistoryMessage = context.evaluate_pin("message").await?;
        let parts = match message.content {
            MessageContent::String(text) => vec![Content::Text {
                content_type: flow_like_model_provider::history::ContentType::Text,
                text,
            }],
            MessageContent::Contents(contents) => contents,
        };
        let content = parts
            .iter()
            .filter_map(|item| match item {
                Content::Text { text, .. } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n");
        let mut images = Vec::new();
        let mut audio = Vec::new();
        let mut videos = Vec::new();
        let mut documents = Vec::new();
        for part in &parts {
            match part {
                Content::Image { image_url, .. } => images.push(image_url.url.clone()),
                Content::Audio { audio_url, .. } => audio.push(audio_url.clone()),
                Content::Video { video_url, .. } => videos.push(video_url.clone()),
                Content::Document { document_url, .. } => {
                    documents.push(document_url.clone());
                }
                Content::Text { .. } => {}
            }
        }

        context
            .set_pin_value("content", json!(content.trim()))
            .await?;
        context.set_pin_value("parts", json!(parts)).await?;
        context.set_pin_value("images", json!(images)).await?;
        context.set_pin_value("audio", json!(audio)).await?;
        context.set_pin_value("videos", json!(videos)).await?;
        context.set_pin_value("documents", json!(documents)).await?;

        Ok(())
    }
}
