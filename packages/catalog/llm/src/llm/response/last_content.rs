use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic, NodeScores},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_model_provider::{history::Content, response::Response};
use flow_like_types::{async_trait, json::json};

#[crate::register_node]
#[derive(Default)]
pub struct LastContentNode {}

impl LastContentNode {
    pub fn new() -> Self {
        LastContentNode {}
    }
}

#[async_trait]
impl NodeLogic for LastContentNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "ai_generative_llm_response_last_content",
            "Last Content",
            "Extracts the content string from the last assistant message in a response",
            "AI/Generative/Response",
        );
        node.set_flowscript_name("ai.response", "lastContent");
        node.set_receiver("response");
        node.add_icon("/flow/icons/history.svg");
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
            "response",
            "Response",
            "LLM response to extract from",
            VariableType::Struct,
        )
        .set_schema::<Response>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin(
            "content",
            "Content",
            "Content string from the last message",
            VariableType::String,
        );

        node.add_output_pin(
            "success",
            "Success",
            "Whether content was successfully extracted",
            VariableType::Boolean,
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

        node.add_output_pin(
            "reasoning",
            "Reasoning",
            "Displayable reasoning returned by the model",
            VariableType::String,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let response: Response = context.evaluate_pin("response").await?;
        let message = response.last_message();
        let content = message
            .and_then(|message| message.content.clone())
            .unwrap_or_default();
        let parts = message
            .map(|message| message.ordered_content_parts())
            .unwrap_or_default();
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
        let reasoning = message
            .and_then(|message| message.reasoning.clone())
            .unwrap_or_default();
        let success = !parts.is_empty()
            || !reasoning.is_empty()
            || message.is_some_and(|message| !message.tool_calls.is_empty());
        context.set_pin_value("content", json!(content)).await?;
        context.set_pin_value("success", json!(success)).await?;
        context.set_pin_value("parts", json!(parts)).await?;
        context.set_pin_value("images", json!(images)).await?;
        context.set_pin_value("audio", json!(audio)).await?;
        context.set_pin_value("videos", json!(videos)).await?;
        context.set_pin_value("documents", json!(documents)).await?;
        context.set_pin_value("reasoning", json!(reasoning)).await?;

        Ok(())
    }
}
