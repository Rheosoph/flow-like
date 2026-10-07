use super::*;
use flow_like_catalog_embedding::CachedEmbeddingModel;
use flow_like_model_provider::embedding::interface::{
    EmbeddingBatch, EmbeddingOptions, EmbeddingPurpose,
};
#[cfg(feature = "execute")]
use {
    super::prepare_embedding_media::{
        EmbeddingMediaKind, decode_embedding_media, embedding_media_kind,
    },
    flow_like_model_provider::embedding::interface::{
        EmbeddingDescriptor, EmbeddingError, EmbeddingModality, EmbeddingRequest,
    },
};

fn media_embedding_node(video: bool) -> Node {
    let (name, label, method) = if video {
        ("embed_video", "Embed Video", "embedVideo")
    } else {
        ("embed_audio", "Embed Audio", "embedAudio")
    };
    let mut node = Node::new(
        name,
        label,
        "Embeds a file using its extension to select text, image, audio, or video input",
        "AI/Embedding",
    );
    node.set_flowscript_name("ai.embedding", method);
    node.set_version(1);
    node.set_long_running(true);
    node.add_icon("/flow/icons/bot-invoke.svg");
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
    add_exec_pins(&mut node);
    add_flow_path_input(
        &mut node,
        "source",
        "Source",
        "A media or UTF-8 text file; its extension selects the embedding modality",
    );
    node.add_input_pin(
        "model",
        "Model",
        "A loaded embedding model that supports the detected file modality",
        VariableType::Struct,
    )
    .set_schema::<CachedEmbeddingModel>()
    .set_options(PinOptions::new().set_enforce_schema(true).build());
    node.add_input_pin(
        "purpose",
        "Purpose",
        "The embedding task supported by the model",
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
    if video {
        node.add_input_pin(
            "max_frames",
            "Max Frames",
            "Maximum uniformly sampled frames when the source is a video",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(16)));
        node.add_input_pin(
            "include_audio",
            "Include Audio",
            "Include a video's audio track; missing audio returns an error",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(false)));
    }
    node.add_output_pin(
        "vector",
        "Vector",
        "The file embedding",
        VariableType::Float,
    )
    .set_value_type(ValueType::Array);
    node.add_output_pin(
        "result",
        "Result",
        "The vector with its embedding space, model revision, and usage",
        VariableType::Struct,
    )
    .set_schema::<EmbeddingBatch>()
    .set_options(PinOptions::new().set_enforce_schema(true).build());
    node
}

#[cfg(feature = "execute")]
fn validate_media_request(
    descriptor: &EmbeddingDescriptor,
    kind: EmbeddingMediaKind,
    purpose: EmbeddingPurpose,
    options: &EmbeddingOptions,
) -> flow_like_types::Result<()> {
    descriptor.validate(&EmbeddingRequest {
        items: vec![],
        purpose,
        options: options.clone(),
    })?;
    let required = match kind {
        EmbeddingMediaKind::Text => vec![EmbeddingModality::Text],
        EmbeddingMediaKind::Image => vec![EmbeddingModality::Image],
        EmbeddingMediaKind::Audio => vec![EmbeddingModality::Audio],
        EmbeddingMediaKind::Video {
            include_audio: true,
            ..
        } => vec![EmbeddingModality::Audio, EmbeddingModality::Video],
        EmbeddingMediaKind::Video { .. } => vec![EmbeddingModality::Video],
    };
    for modality in &required {
        if !descriptor.modalities.contains(modality) {
            return Err(EmbeddingError::UnsupportedModality(*modality).into());
        }
    }
    if required.len() > 1
        && !descriptor.joint_combinations.iter().any(|allowed| {
            let mut allowed = allowed.clone();
            allowed.sort();
            allowed.dedup();
            allowed == required
        })
    {
        return Err(EmbeddingError::UnsupportedCombination(required).into());
    }
    Ok(())
}

#[cfg(feature = "execute")]
async fn run_media_embedding(
    context: &mut ExecutionContext,
    video: bool,
) -> flow_like_types::Result<()> {
    context.deactivate_exec_pin("exec_out").await?;
    let handle: CachedEmbeddingModel = context.evaluate_pin("model").await?;
    let model = handle.resolve_model(context).await?;
    let source: FlowPath = context.evaluate_pin("source").await?;
    let purpose: EmbeddingPurpose = context.evaluate_pin("purpose").await?;
    let options: EmbeddingOptions = context.evaluate_pin("options").await?;
    let mut kind = embedding_media_kind(&source.object_path(), 16, false)?;
    if video && matches!(kind, EmbeddingMediaKind::Video { .. }) {
        let max_frames: i64 = context.evaluate_pin("max_frames").await?;
        if !(1..=1024).contains(&max_frames) {
            return Err(flow_like_types::anyhow!(
                "Max Frames must be between 1 and 1024"
            ));
        }
        kind = EmbeddingMediaKind::Video {
            max_frames: max_frames as usize,
            include_audio: context.evaluate_pin("include_audio").await?,
        };
    }
    validate_media_request(model.descriptor(), kind, purpose, &options)?;
    let (store, path) = flow_path_object(context, &source).await?;
    let input = decode_embedding_media(store.as_ref(), &path, kind)
        .await?
        .into_input()?;
    let dimensions = options
        .dimensions
        .unwrap_or(model.descriptor().space.dimensions);
    let request = EmbeddingRequest {
        items: vec![input],
        purpose,
        options,
    };
    model.descriptor().validate(&request)?;
    let result = model.embed(request).await?;
    if result.embeddings.len() != 1
        || dimensions == 0
        || result.space.dimensions != dimensions
        || result.embeddings[0].len() != result.space.dimensions
        || result.embeddings[0].iter().any(|value| !value.is_finite())
    {
        return Err(flow_like_types::anyhow!(
            "Model must return one finite vector with its declared dimensions for the file"
        ));
    }
    context
        .set_pin_value("vector", json!(result.embeddings[0]))
        .await?;
    context.set_pin_value("result", json!(result)).await?;
    context.activate_exec_pin("exec_out").await?;
    Ok(())
}

#[cfg(not(feature = "execute"))]
async fn run_media_embedding(
    _context: &mut ExecutionContext,
    _video: bool,
) -> flow_like_types::Result<()> {
    Err(execute_feature_error())
}

#[crate::register_node]
#[derive(Default)]
pub struct EmbedAudioNode;

impl EmbedAudioNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for EmbedAudioNode {
    fn get_node(&self) -> Node {
        media_embedding_node(false)
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        run_media_embedding(context, false).await
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct EmbedVideoNode;

impl EmbedVideoNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for EmbedVideoNode {
    fn get_node(&self) -> Node {
        media_embedding_node(true)
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        run_media_embedding(context, true).await
    }
}
