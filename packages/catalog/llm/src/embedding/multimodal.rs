#[cfg(feature = "execute")]
use std::collections::HashMap;
use std::sync::Arc;

use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic, NodeScores},
    pin::{PinOptions, ValueType},
    variable::VariableType,
};
use flow_like_catalog_core::NodeImage;
pub use flow_like_catalog_core::{EmbeddingAudio, EmbeddingContent, EmbeddingContentPart};
use flow_like_model_provider::embedding::interface::{
    EmbeddingBatch, EmbeddingDescriptor, EmbeddingInput, EmbeddingOptions, EmbeddingPart,
    EmbeddingPurpose, EmbeddingRequest, VideoFrame, VideoInput,
};
use flow_like_types::{Result, anyhow, async_trait, image::DynamicImage, json::json};

use super::CachedEmbeddingModel;

fn ensure(condition: bool, message: &str) -> Result<()> {
    if condition {
        Ok(())
    } else {
        Err(anyhow!(message.to_owned()))
    }
}

#[async_trait]
trait ImageResolver: Send {
    async fn resolve(&mut self, image: NodeImage) -> Result<Arc<DynamicImage>>;
}

#[cfg(feature = "execute")]
struct ContextImageResolver<'a> {
    context: &'a mut ExecutionContext,
    images: HashMap<String, Arc<DynamicImage>>,
}

#[async_trait]
#[cfg(feature = "execute")]
impl ImageResolver for ContextImageResolver<'_> {
    async fn resolve(&mut self, image: NodeImage) -> Result<Arc<DynamicImage>> {
        if let Some(cached) = self.images.get(&image.image_ref) {
            return Ok(cached.clone());
        }
        let cached = image.get_image(self.context).await?;
        let snapshot = Arc::new(cached.lock().await.clone());
        self.images.insert(image.image_ref, snapshot.clone());
        Ok(snapshot)
    }
}

async fn resolve_content(
    content: EmbeddingContent,
    resolver: &mut impl ImageResolver,
) -> Result<EmbeddingInput> {
    ensure(
        !content.parts.is_empty(),
        "An embedding item needs at least one content part",
    )?;
    let mut parts = Vec::with_capacity(content.parts.len());
    for part in content.parts {
        parts.push(match part {
            EmbeddingContentPart::Text { text } => EmbeddingPart::Text(text),
            EmbeddingContentPart::Image { image } => {
                EmbeddingPart::Image(resolver.resolve(image).await?)
            }
            EmbeddingContentPart::Audio(audio) => EmbeddingPart::Audio(audio.into_audio()?),
            EmbeddingContentPart::Video(video) => {
                ensure(
                    !video.frames.is_empty() && video.duration_ms > 0,
                    "Video needs frames and a duration",
                )?;
                let mut previous = None;
                for frame in &video.frames {
                    ensure(
                        frame.timestamp_ms <= video.duration_ms
                            && previous.is_none_or(|time| frame.timestamp_ms >= time),
                        "Video timestamps must be ordered and within its duration",
                    )?;
                    previous = Some(frame.timestamp_ms);
                }
                let audio = video.audio.map(EmbeddingAudio::into_audio).transpose()?;
                let mut frames = Vec::with_capacity(video.frames.len());
                for frame in video.frames {
                    frames.push(VideoFrame {
                        timestamp_ms: frame.timestamp_ms,
                        image: resolver.resolve(frame.image).await?,
                    });
                }
                EmbeddingPart::Video(VideoInput {
                    frames,
                    duration_ms: video.duration_ms,
                    audio,
                })
            }
        });
    }
    Ok(EmbeddingInput {
        parts,
        title: content.title,
    })
}

#[cfg(feature = "execute")]
async fn cached_model(
    context: &mut ExecutionContext,
) -> Result<Arc<dyn flow_like_model_provider::embedding::interface::EmbeddingModel>> {
    let handle: CachedEmbeddingModel = context.evaluate_pin("model").await?;
    handle.resolve_model(context).await
}

fn base_node(name: &str, label: &str, description: &str, method: &str) -> Node {
    let mut node = Node::new(name, label, description, "AI/Embedding");
    node.set_flowscript_name("ai.embedding", method);
    node.set_version(1);
    node.add_icon("/flow/icons/bot-invoke.svg");
    node.set_scores(
        NodeScores::new()
            .set_privacy(10)
            .set_security(10)
            .set_performance(10)
            .set_governance(9)
            .set_reliability(9)
            .set_cost(10)
            .build(),
    );
    node.add_input_pin(
        "exec_in",
        "Input",
        "Start execution",
        VariableType::Execution,
    );
    node.add_input_pin(
        "model",
        "Model",
        "A loaded embedding model",
        VariableType::Struct,
    )
    .set_schema::<CachedEmbeddingModel>()
    .set_options(PinOptions::new().set_enforce_schema(true).build());
    node.add_output_pin(
        "exec_out",
        "Output",
        "Execution complete",
        VariableType::Execution,
    );
    node
}

fn add_request_pins(node: &mut Node) {
    node.set_long_running(true);
    node.set_scores(
        NodeScores::new()
            .set_privacy(6)
            .set_security(6)
            .set_performance(7)
            .set_governance(6)
            .set_reliability(7)
            .set_cost(6)
            .build(),
    );
    node.add_input_pin(
        "purpose",
        "Purpose",
        "The model task, such as a search query or indexed document",
        VariableType::String,
    )
    .set_schema::<EmbeddingPurpose>()
    .set_default_value(Some(json!(EmbeddingPurpose::Document)))
    .set_options(
        PinOptions::new()
            .set_valid_values(vec![
                "query".into(),
                "document".into(),
                "similarity".into(),
                "classification".into(),
                "clustering".into(),
                "code_query".into(),
            ])
            .build(),
    );
    node.add_input_pin(
        "options",
        "Options",
        "Optional output dimensions supported by the model",
        VariableType::Struct,
    )
    .set_schema::<EmbeddingOptions>()
    .set_default_value(Some(json!(EmbeddingOptions::default())))
    .set_options(PinOptions::new().set_enforce_schema(true).build());
    node.add_output_pin(
        "result",
        "Result",
        "Vectors with their embedding space, model revision, and usage",
        VariableType::Struct,
    )
    .set_schema::<EmbeddingBatch>()
    .set_options(PinOptions::new().set_enforce_schema(true).build());
}

#[cfg(feature = "execute")]
async fn embed_contents(
    context: &mut ExecutionContext,
    contents: Vec<EmbeddingContent>,
) -> Result<EmbeddingBatch> {
    let model = cached_model(context).await?;
    let purpose: EmbeddingPurpose = context.evaluate_pin("purpose").await?;
    let options: EmbeddingOptions = context.evaluate_pin("options").await?;
    let mut resolver = ContextImageResolver {
        context,
        images: HashMap::new(),
    };
    let mut items = Vec::with_capacity(contents.len());
    for content in contents {
        items.push(resolve_content(content, &mut resolver).await?);
    }
    Ok(model
        .embed(EmbeddingRequest {
            items,
            purpose,
            options,
        })
        .await?)
}

#[crate::register_node]
#[derive(Default)]
pub struct EmbedContentNode {}

impl EmbedContentNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for EmbedContentNode {
    fn get_node(&self) -> Node {
        let mut node = base_node(
            "embed_content",
            "Embed Content",
            "Embeds ordered text, image, audio, and video parts into one vector",
            "embedContent",
        );
        add_request_pins(&mut node);
        node.add_input_pin(
            "content",
            "Content",
            "Ordered parts to encode jointly; images reference the workflow image cache",
            VariableType::Struct,
        )
        .set_schema::<EmbeddingContent>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());
        node.add_output_pin(
            "vector",
            "Vector",
            "The content embedding",
            VariableType::Float,
        )
        .set_value_type(ValueType::Array);
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> Result<()> {
        flow_like_catalog_core::run_with_execute_gate!(context, {
            context.deactivate_exec_pin("exec_out").await?;
            let content: EmbeddingContent = context.evaluate_pin("content").await?;
            let result = embed_contents(context, vec![content]).await?;
            ensure(
                result.embeddings.len() == 1,
                "Model must return one vector for one content item",
            )?;
            context
                .set_pin_value("vector", json!(result.embeddings[0]))
                .await?;
            context.set_pin_value("result", json!(result)).await?;
            context.activate_exec_pin("exec_out").await?;
            Ok(())
        })
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct EmbedContentsNode {}

impl EmbedContentsNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for EmbedContentsNode {
    fn get_node(&self) -> Node {
        let mut node = base_node(
            "embed_contents",
            "Embed Content Batch",
            "Embeds each structured item into one vector in input order",
            "embedContents",
        );
        add_request_pins(&mut node);
        node.add_input_pin(
            "contents",
            "Contents",
            "Each item contains the parts to encode jointly",
            VariableType::Struct,
        )
        .set_schema::<EmbeddingContent>()
        .set_value_type(ValueType::Array)
        .set_options(PinOptions::new().set_enforce_schema(true).build());
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> Result<()> {
        flow_like_catalog_core::run_with_execute_gate!(context, {
            context.deactivate_exec_pin("exec_out").await?;
            let contents: Vec<EmbeddingContent> = context.evaluate_pin("contents").await?;
            let result = embed_contents(context, contents).await?;
            context.set_pin_value("result", json!(result)).await?;
            context.activate_exec_pin("exec_out").await?;
            Ok(())
        })
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct EmbeddingModelInfoNode {}

impl EmbeddingModelInfoNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for EmbeddingModelInfoNode {
    fn get_node(&self) -> Node {
        let mut node = base_node(
            "embedding_model_info",
            "Embedding Model Info",
            "Reads supported modalities, joint inputs, tasks, limits, and embedding space",
            "modelInfo",
        );
        node.add_output_pin(
            "descriptor",
            "Descriptor",
            "The loaded model's effective capabilities",
            VariableType::Struct,
        )
        .set_schema::<EmbeddingDescriptor>();
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> Result<()> {
        flow_like_catalog_core::run_with_execute_gate!(context, {
            context.deactivate_exec_pin("exec_out").await?;
            let model = cached_model(context).await?;
            context
                .set_pin_value("descriptor", json!(model.descriptor()))
                .await?;
            context.activate_exec_pin("exec_out").await?;
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct Images(Vec<String>);

    #[async_trait]
    impl ImageResolver for Images {
        async fn resolve(&mut self, image: NodeImage) -> Result<Arc<DynamicImage>> {
            ensure(image.image_ref != "missing", "Image not found in cache")?;
            self.0.push(image.image_ref);
            Ok(Arc::new(DynamicImage::new_rgb8(2, 3)))
        }
    }

    #[tokio::test]
    async fn resolves_joint_parts_in_order_and_keeps_media_timing() {
        let content: EmbeddingContent = flow_like_types::json::from_value(json!({
            "title": "Field recording",
            "parts": [
                {"type": "text", "text": "Describe this scene"},
                {"type": "image", "image": {"image_ref": "photo"}},
                {"type": "audio", "samples": [0.0, 0.25, -0.25, 0.0], "sample_rate": 16000, "channels": 2},
                {"type": "video", "duration_ms": 1000, "frames": [
                    {"timestamp_ms": 0, "image": {"image_ref": "first"}},
                    {"timestamp_ms": 500, "image": {"image_ref": "second"}}
                ], "audio": {"samples": [0.0, 0.5], "sample_rate": 8000, "channels": 1}}
            ]
        })).unwrap();
        let mut resolver = Images::default();
        let resolved = resolve_content(content, &mut resolver).await.unwrap();
        assert_eq!(resolved.title.as_deref(), Some("Field recording"));
        assert_eq!(resolver.0, ["photo", "first", "second"]);
        assert!(
            matches!(&resolved.parts[0], EmbeddingPart::Text(text) if text == "Describe this scene")
        );
        assert!(matches!(&resolved.parts[1], EmbeddingPart::Image(image) if image.width() == 2));
        let EmbeddingPart::Audio(audio) = &resolved.parts[2] else {
            panic!("audio part")
        };
        assert_eq!(audio.channels, 2);
        assert_eq!(&*audio.samples, &[0.0, 0.25, -0.25, 0.0]);
        let EmbeddingPart::Video(video) = &resolved.parts[3] else {
            panic!("video part")
        };
        assert_eq!(video.frames[1].timestamp_ms, 500);
        assert_eq!(video.duration_ms, 1000);
        assert_eq!(video.audio.as_ref().unwrap().sample_rate, 8000);
    }

    #[tokio::test]
    async fn rejects_missing_images_invalid_audio_and_unordered_video() {
        for value in [
            json!({"parts": []}),
            json!({"parts": [{"type": "image", "image": {"image_ref": "missing"}}]}),
            json!({"parts": [{"type": "audio", "samples": [1.0], "sample_rate": 16000, "channels": 2}]}),
            json!({"parts": [{"type": "video", "duration_ms": 1000, "frames": [
                {"timestamp_ms": 600, "image": {"image_ref": "one"}},
                {"timestamp_ms": 400, "image": {"image_ref": "two"}}
            ]}]}),
        ] {
            let content = flow_like_types::json::from_value(value).unwrap();
            assert!(
                resolve_content(content, &mut Images::default())
                    .await
                    .is_err()
            );
        }
    }

    #[test]
    fn content_schema_exposes_all_modalities_and_node_outputs_are_typed() {
        let schema = schemars::schema_for!(EmbeddingContent);
        let schema = flow_like_types::json::to_string(&schema).unwrap();
        for modality in ["text", "image", "audio", "video"] {
            assert!(schema.contains(&format!("\"{modality}\"")));
        }
        assert!(
            flow_like_types::json::from_value::<EmbeddingContentPart>(json!({"type": "unknown"}))
                .is_err()
        );
        for node in [
            EmbedContentNode::new().get_node(),
            EmbedContentsNode::new().get_node(),
        ] {
            let result = node.pins.values().find(|pin| pin.name == "result").unwrap();
            assert_eq!(result.data_type, VariableType::Struct);
            let schema: flow_like_types::Value =
                flow_like_types::json::from_str(result.schema.as_ref().unwrap()).unwrap();
            for property in ["embeddings", "space", "provenance", "usage"] {
                assert!(schema["properties"].get(property).is_some());
            }
        }
        let node = EmbedContentNode::new().get_node();
        let vector = node.pins.values().find(|pin| pin.name == "vector").unwrap();
        assert_eq!(vector.data_type, VariableType::Float);
        assert_eq!(vector.value_type, ValueType::Array);
    }
}
