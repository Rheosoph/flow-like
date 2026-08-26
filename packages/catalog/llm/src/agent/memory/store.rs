use super::config::MemoryConfig;
#[cfg(feature = "execute")]
use crate::generative::embedding::CachedEmbeddingModelObject;
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic, NodeScores},
    pin::PinOptions,
    variable::VariableType,
};
#[cfg(feature = "execute")]
use flow_like_storage::databases::vector::VectorStore;
#[cfg(feature = "execute")]
use flow_like_types::bail;
use flow_like_types::{async_trait, json::json};
#[cfg(feature = "execute")]
use std::collections::hash_map::DefaultHasher;
#[cfg(feature = "execute")]
use std::hash::{Hash, Hasher};
#[cfg(feature = "execute")]
use std::time::{SystemTime, UNIX_EPOCH};

#[crate::register_node]
#[derive(Default)]
pub struct StoreMemoryNode {}

impl StoreMemoryNode {
    pub fn new() -> Self {
        StoreMemoryNode {}
    }
}

#[async_trait]
impl NodeLogic for StoreMemoryNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "memory_store",
            "Store Memory",
            "Embeds text and stores it as a memory observation in the configured LanceDB table",
            "AI/Memory",
        );
        node.set_flowscript_name("ai.memory", "store");
        node.set_receiver("memory_config");
        node.set_version(1);
        node.add_icon("/flow/icons/bot-invoke.svg");
        node.set_long_running(true);

        node.set_scores(
            NodeScores::new()
                .set_privacy(6)
                .set_security(7)
                .set_performance(7)
                .set_governance(7)
                .set_reliability(8)
                .set_cost(5)
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
            "content",
            "Content",
            "Text content to store as a memory observation",
            VariableType::String,
        );

        node.add_input_pin(
            "role",
            "Role",
            "Role of the message author",
            VariableType::String,
        )
        .set_options(
            PinOptions::new()
                .set_valid_values(vec![
                    "user".into(),
                    "assistant".into(),
                    "system".into(),
                    "observation".into(),
                    "summary".into(),
                ])
                .build(),
        )
        .set_default_value(Some(json!("observation")));

        node.add_output_pin(
            "exec_out",
            "Done",
            "Fires when memory is stored",
            VariableType::Execution,
        );

        node.add_output_pin(
            "observation_count",
            "Observation Count",
            "Total number of observations in the memory table after this insert",
            VariableType::Integer,
        );

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let config: MemoryConfig = context.evaluate_pin("memory_config").await?;
        let content: String = context.evaluate_pin("content").await?;
        let role: String = context
            .evaluate_pin("role")
            .await
            .unwrap_or_else(|_| "observation".to_string());

        if content.trim().is_empty() {
            context.activate_exec_pin("exec_out").await?;
            context.set_pin_value("observation_count", json!(0)).await?;
            return Ok(());
        }

        let cached_model = context
            .get_cache(&config.embedding_model.cache_key)
            .await
            .ok_or_else(|| flow_like_types::anyhow!("Embedding model not found in cache"))?;
        let embedding_obj = cached_model
            .as_any()
            .downcast_ref::<CachedEmbeddingModelObject>()
            .ok_or_else(|| flow_like_types::anyhow!("Failed to downcast embedding model"))?;

        let embeddings = if let Some(model) = &embedding_obj.text_model {
            model.text_embed_document(&vec![content.clone()]).await?
        } else {
            bail!("No text embedding model available");
        };

        if embeddings.is_empty() {
            bail!("Embedding returned empty vector");
        }

        let mut hasher = DefaultHasher::new();
        content.to_lowercase().hash(&mut hasher);
        let content_hash = format!("{:x}", hasher.finish());

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as i64;
        let record = json!({
            "id": uuid::Uuid::new_v4().to_string(),
            "content": content,
            "content_hash": content_hash,
            "role": role,
            "vector": embeddings[0],
            "timestamp": now,
        });

        let cached_db = config.database.load(context).await?;
        cached_db.insert_from(context, vec![record]).await?;

        cached_db.ensure_flushed().await?;

        let count = {
            let db = cached_db.db.read().await;
            db.count(None).await.unwrap_or(0)
        };

        context
            .set_pin_value("observation_count", json!(count as i64))
            .await?;
        context.activate_exec_pin("exec_out").await?;

        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "Memory storage requires the 'execute' feature"
        ))
    }
}
