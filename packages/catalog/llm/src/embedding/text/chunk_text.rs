use crate::generative::embedding::{CachedEmbeddingModel, CachedEmbeddingModelObject};
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic, NodeScores},
    pin::{PinOptions, ValueType},
    variable::VariableType,
};
use flow_like_types::{anyhow, async_trait, bail, json::json, tokio};

#[crate::register_node]
#[derive(Default)]
pub struct ChunkText {}

impl ChunkText {
    pub fn new() -> Self {
        ChunkText {}
    }
}

#[async_trait]
impl NodeLogic for ChunkText {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "chunk_text",
            "Chunk Text",
            "Splits long text into sized/overlapping chunks using the cached embedding model's splitter",
            "AI/Preprocessing",
        );
        node.set_flowscript_name("ai.processing", "chunkText");

        node.set_scores(
            NodeScores::new()
                .set_privacy(9)
                .set_security(9)
                .set_performance(8)
                .set_governance(9)
                .set_reliability(8)
                .set_cost(10)
                .build(),
        );

        node.set_long_running(true);
        node.add_icon("/flow/icons/bot-invoke.svg");

        node.add_input_pin(
            "exec_in",
            "Input",
            "Execution trigger",
            VariableType::Execution,
        );

        node.add_input_pin(
            "text",
            "Text",
            "Source string that needs chunking",
            VariableType::String,
        );

        node.add_input_pin(
            "model",
            "Model",
            "Cached embedding Bit providing the tokenizer/splitter",
            VariableType::Struct,
        )
        .set_schema::<CachedEmbeddingModel>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin(
            "capacity",
            "Capacity",
            "Max characters/tokens in each chunk",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(512)));

        node.add_input_pin(
            "overlap",
            "Overlap",
            "How many characters/tokens overlap between consecutive chunks",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(20)));

        node.add_input_pin(
            "markdown",
            "Markdown",
            "Use a Markdown-aware splitter (true) or the plain splitter",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(true)));

        node.add_output_pin(
            "exec_out",
            "Output",
            "Fires once chunking completes",
            VariableType::Execution,
        );

        node.add_output_pin(
            "chunks",
            "Chunks",
            "Array of chunked text segments",
            VariableType::String,
        )
        .set_value_type(ValueType::Array);

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let text: String = context.evaluate_pin("text").await?;
        let model: CachedEmbeddingModel = context.evaluate_pin("model").await?;
        let capacity: i64 = context.evaluate_pin("capacity").await?;
        let overlap: i64 = context.evaluate_pin("overlap").await?;
        let markdown: bool = context.evaluate_pin("markdown").await?;

        let cached_model = context.get_cache(&model.cache_key).await;
        if cached_model.is_none() {
            bail!("Model not found in cache");
        }

        let cached_model = cached_model.unwrap();
        let embedding_model = cached_model
            .as_any()
            .downcast_ref::<CachedEmbeddingModelObject>()
            .ok_or(anyhow!("Failed to Downcast Model"))?;

        let (text_splitter, md_splitter) =
            if let Some(text_model) = embedding_model.text_model.clone() {
                text_model
                    .get_splitter(Some(capacity as usize), Some(overlap as usize))
                    .await?
            } else if let Some(image_model) = embedding_model.image_model.clone() {
                image_model
                    .get_splitter(Some(capacity as usize), Some(overlap as usize))
                    .await?
            } else {
                return Err(anyhow!("No model found"));
            };

        let chunks = tokio::task::spawn_blocking(move || {
            if markdown {
                md_splitter.chunks(&text)
            } else {
                text_splitter.chunks(&text)
            }
        })
        .await
        .map_err(|e| anyhow!("Blocking task failed: {}", e))??;

        context.set_pin_value("chunks", json!(chunks)).await?;
        context.activate_exec_pin("exec_out").await?;

        Ok(())
    }
}
