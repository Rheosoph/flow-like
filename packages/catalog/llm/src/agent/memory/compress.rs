use super::config::MemoryConfig;
use crate::generative::embedding::CachedEmbeddingModelObject;
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic, NodeScores},
    pin::{PinOptions, ValueType},
    variable::VariableType,
};
use flow_like_model_provider::response::{LLMUsageStats, Usage};
use flow_like_storage::databases::vector::VectorStore;
use flow_like_types::{
    Value, async_trait, bail,
    json::{self, json},
};
use std::time::{SystemTime, UNIX_EPOCH};

#[cfg(feature = "execute")]
use rig::completion::Completion;
#[cfg(feature = "execute")]
use rig::message::AssistantContent;
#[cfg(feature = "execute")]
use std::time::Instant;

#[crate::register_node]
#[derive(Default)]
pub struct CompressMemoryNode {}

impl CompressMemoryNode {
    pub fn new() -> Self {
        CompressMemoryNode {}
    }
}

#[async_trait]
impl NodeLogic for CompressMemoryNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "memory_compress",
            "Compress Memory",
            "Compresses old memory observations into a summary using an LLM, then replaces them in the store. Runs the embedding model to store the summary vector.",
            "AI/Memory",
        );
        node.set_version(1);
        node.add_icon("/flow/icons/bot-invoke.svg");
        node.set_long_running(true);

        node.set_scores(
            NodeScores::new()
                .set_privacy(5)
                .set_security(7)
                .set_performance(5)
                .set_governance(6)
                .set_reliability(7)
                .set_cost(3)
                .build(),
        );

        node.add_input_pin("exec_in", "Input", "Trigger", VariableType::Execution);

        node.add_input_pin(
            "memory_config",
            "Memory Config",
            "MemoryConfig from Create Memory Config node",
            VariableType::Struct,
        )
        .set_schema::<MemoryConfig>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin(
            "observations",
            "Observations",
            "Array of memory records to compress (typically older observations from Search Memory)",
            VariableType::Struct,
        )
        .set_value_type(ValueType::Array);

        node.add_input_pin(
            "model",
            "Compression Model",
            "LLM model Bit for generating the summary",
            VariableType::Struct,
        )
        .set_schema::<flow_like::bit::Bit>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin(
            "exec_out",
            "Done",
            "Fires when compression completes",
            VariableType::Execution,
        );

        node.add_output_pin(
            "summary_text",
            "Summary",
            "The compressed summary text",
            VariableType::String,
        );

        node.add_output_pin(
            "compressed_count",
            "Compressed Count",
            "Number of observations that were compressed",
            VariableType::Integer,
        );

        node.add_output_pin(
            "stats",
            "Stats",
            "Token usage and model statistics from the compaction LLM call",
            VariableType::Struct,
        )
        .set_schema::<LLMUsageStats>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let config: MemoryConfig = context.evaluate_pin("memory_config").await?;
        let observations: Vec<Value> = context.evaluate_pin("observations").await?;
        let model_bit: flow_like::bit::Bit = context.evaluate_pin("model").await?;

        if observations.is_empty() {
            context.set_pin_value("summary_text", json!("")).await?;
            context.set_pin_value("compressed_count", json!(0)).await?;
            context
                .set_pin_value("stats", json!(LLMUsageStats::default()))
                .await?;
            context.activate_exec_pin("exec_out").await?;
            return Ok(());
        }

        // Build the text block from observations
        let mut obs_text = String::new();
        let mut obs_ids: Vec<String> = Vec::new();
        for obs in &observations {
            if let Some(content) = obs.get("content").and_then(|c| c.as_str()) {
                let role = obs
                    .get("role")
                    .and_then(|r| r.as_str())
                    .unwrap_or("observation");
                obs_text.push_str(&format!("[{}] {}\n", role, content));
            }
            if let Some(id) = obs.get("id").and_then(|i| i.as_str()) {
                obs_ids.push(id.to_string());
            }
        }

        let prompt = format!(
            "Compress the following conversation/observation history into a concise summary. \
             Preserve key facts, decisions, user preferences, and context. \
             Drop redundant details. Output only the summary text, nothing else.\n\n{}",
            obs_text
        );

        // Use the provided LLM to generate summary
        let history = None;
        let agent_builder = model_bit.agent(context, &history).await?;
        let summary_agent = agent_builder
            .preamble("You are a memory compressor. Be concise but preserve key facts, decisions, user preferences, and context.")
            .build();

        let start = Instant::now();
            let response = summary_agent
                .completion(prompt, Vec::<rig::completion::Message>::new())
            .await
            .map_err(|e| flow_like_types::anyhow!("Failed to create compression request: {}", e))?
            .send()
            .await
            .map_err(|e| flow_like_types::anyhow!("Compression LLM call failed: {}", e))?;
        let duration_ms = start.elapsed().as_millis() as u64;

        let model_name = model_bit.meta.get("en").map(|m| m.name.clone());
        let mut stats = LLMUsageStats {
            usage: Usage::from_rig(response.usage),
            model: model_name,
            duration_ms: Some(duration_ms),
            iterations: None,
            calls: Vec::new(),
        };
        stats.set_duration_ms(duration_ms);

        let mut summary = String::new();
        for content in response.choice {
            if let AssistantContent::Text(t) = content {
                summary.push_str(&t.text);
            }
        }
        let summary = summary.trim().to_string();

        if summary.is_empty() {
            bail!("LLM returned empty summary");
        }

        // Embed the summary
        let cached_model = context
            .get_cache(&config.embedding_model.cache_key)
            .await
            .ok_or_else(|| flow_like_types::anyhow!("Embedding model not found in cache"))?;
        let embedding_obj = cached_model
            .as_any()
            .downcast_ref::<CachedEmbeddingModelObject>()
            .ok_or_else(|| flow_like_types::anyhow!("Failed to downcast embedding model"))?;

        let embeddings = if let Some(model) = &embedding_obj.text_model {
            model.text_embed_document(&vec![summary.clone()]).await?
        } else {
            bail!("No text embedding model available");
        };

        if embeddings.is_empty() {
            bail!("Embedding returned empty vector for summary");
        }

        let cached_db = config.database.load(context).await?;

        // Delete old observations
        if !obs_ids.is_empty() {
            let ids_filter = obs_ids
                .iter()
                .map(|id| format!("'{}'", id.replace('\'', "''")))
                .collect::<Vec<_>>()
                .join(", ");
            let filter = format!("id IN ({})", ids_filter);
            let db = cached_db.db.read().await;
            db.delete(&filter).await?;
            drop(db);
        }

        // Insert summary record
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as i64;
        let summary_record = json!({
            "id": uuid::Uuid::new_v4().to_string(),
            "content": summary,
            "role": "summary",
            "vector": embeddings[0],
            "timestamp": now,
        });

        let mut db = cached_db.db.write().await;
        db.insert(vec![summary_record]).await?;
        drop(db);

        let compressed_count = observations.len();
        context
            .set_pin_value("summary_text", json::json!(summary))
            .await?;
        context
            .set_pin_value("compressed_count", json!(compressed_count as i64))
            .await?;
        context.set_pin_value("stats", json!(stats)).await?;
        context.activate_exec_pin("exec_out").await?;

        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "This node requires the 'execute' feature"
        ))
    }
}
