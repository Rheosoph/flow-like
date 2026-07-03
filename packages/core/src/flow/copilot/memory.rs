//! Profile-scoped semantic memory for the global assistant, backed by LanceDB.
//!
//! Each profile gets its OWN memory table (`assistant-memory/<profile_id>`), so memories in a work
//! profile never mix with those in another profile. The embedding model is user-selected (a bit id),
//! resolved through the same `EmbeddingFactory` the rest of the app uses. All of this is reachable
//! without a flow `ExecutionContext`, so it plugs directly into the context-free `PlatformCopilot`.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use flow_like_model_provider::embedding::EmbeddingModelLogic;
use flow_like_storage::databases::vector::{VectorStore, lancedb::LanceDBVectorStore};
use flow_like_storage::object_store::path::Path;
use flow_like_types::json::json;
use flow_like_types::tokio::sync::RwLock;
use flow_like_types::{Result, anyhow, bail, create_id};

use crate::bit::Bit;
use crate::state::FlowLikeState;

/// Snapshot of a profile's memory table: how many observations it holds and which embedding model
/// they were written with (so the UI can warn before switching to an incompatible model).
#[derive(Debug, Clone, flow_like_types::json::Serialize)]
pub struct MemoryStatus {
    pub count: usize,
    pub embedding_model_id: Option<String>,
}

/// Semantic memory store for one profile. Cheap to hold across the chat loop (async-locked store).
pub struct AssistantMemory {
    store: RwLock<LanceDBVectorStore>,
    embedding: Arc<dyn EmbeddingModelLogic>,
    embedding_model_id: String,
}

impl AssistantMemory {
    /// Open the per-profile LanceDB store (no embedding model). Lazily created on first write.
    async fn open_store(state: &Arc<FlowLikeState>, profile_id: &str) -> Result<LanceDBVectorStore> {
        let dir = Path::from("assistant-memory").child(profile_id);
        let builder = {
            let config = state.config.read().await;
            let build = config
                .callbacks
                .build_user_database
                .clone()
                .ok_or_else(|| anyhow!("No user database builder registered"))?;
            build(dir)
        };
        let connection = state.with_lance_session(builder).execute().await?;
        let mut store = LanceDBVectorStore::from_connection(connection, "memory".to_string()).await;
        if let Some(opts) = state.config.read().await.callbacks.lance_write_options.clone() {
            store.set_write_options(opts);
        }
        Ok(store)
    }

    /// Open (or lazily create on first write) the memory table for `profile_id`, embedding with the
    /// given bit. The table lives under the user store at `assistant-memory/<profile_id>`.
    pub async fn open(
        state: Arc<FlowLikeState>,
        profile_id: &str,
        embedding_bit: &Bit,
    ) -> Result<Self> {
        let embedding = state
            .embedding_factory
            .lock()
            .await
            .build_text(embedding_bit, state.clone())
            .await?;
        let store = Self::open_store(&state, profile_id).await?;

        Ok(Self {
            store: RwLock::new(store),
            embedding,
            embedding_model_id: embedding_bit.id.clone(),
        })
    }

    /// Report how many observations a profile has stored and which embedding model produced them.
    pub async fn status(state: Arc<FlowLikeState>, profile_id: &str) -> Result<MemoryStatus> {
        let store = Self::open_store(&state, profile_id).await?;
        let count = store.count(None).await.unwrap_or(0);
        let embedding_model_id = if count > 0 {
            store
                .filter("1=1", Some(vec!["embedding_model".to_string()]), 1, 0)
                .await
                .ok()
                .and_then(|rows| rows.into_iter().next())
                .and_then(|row| {
                    row.get("embedding_model")
                        .and_then(|value| value.as_str())
                        .map(str::to_string)
                })
        } else {
            None
        };
        Ok(MemoryStatus {
            count,
            embedding_model_id,
        })
    }

    /// Delete all memories for a profile by DROPPING the table (data and schema). Used when the
    /// embedding model changes: a new model usually has a different vector dimension, so the table
    /// must be recreated with the new schema on the next insert — purging rows would keep the old
    /// schema and permanently break `_memory_store`.
    pub async fn clear(state: Arc<FlowLikeState>, profile_id: &str) -> Result<()> {
        let mut store = Self::open_store(&state, profile_id).await?;
        store.drop_table().await
    }

    /// Embed `content` and append it as a memory observation. Returns the total observation count.
    pub async fn store(&self, role: &str, content: &str) -> Result<usize> {
        let content = content.trim();
        if content.is_empty() {
            return Ok(0);
        }

        let embeddings = self
            .embedding
            .text_embed_document(&vec![content.to_string()])
            .await?;
        if embeddings.is_empty() {
            bail!("Embedding returned no vectors");
        }

        let mut hasher = DefaultHasher::new();
        content.to_lowercase().hash(&mut hasher);
        let content_hash = format!("{:x}", hasher.finish());
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as i64;

        let record = json!({
            "id": create_id(),
            "content": content,
            "content_hash": content_hash,
            "role": role,
            "embedding_model": self.embedding_model_id,
            "vector": embeddings[0],
            "timestamp": now,
        });

        let mut store = self.store.write().await;
        store.insert(vec![record]).await?;
        let count = store.count(None).await.unwrap_or(0);
        Ok(count)
    }

    /// Embed `query` and return up to `top_k` relevant memory contents (most similar first). Returns
    /// an empty list rather than erroring when the table does not exist yet (fresh profile).
    pub async fn search(&self, query: &str, top_k: usize) -> Result<Vec<String>> {
        let query = query.trim();
        if query.is_empty() || top_k == 0 {
            return Ok(Vec::new());
        }

        let embeddings = self.embedding.text_embed_query(&vec![query.to_string()]).await?;
        if embeddings.is_empty() {
            return Ok(Vec::new());
        }
        let vector: Vec<f64> = embeddings[0].iter().map(|value| *value as f64).collect();

        let store = self.store.read().await;
        let results = match store
            .vector_search(
                vector,
                None,
                Some(vec!["content".to_string(), "role".to_string()]),
                top_k,
                0,
            )
            .await
        {
            Ok(results) => results,
            // No table yet / empty memory — treat as "nothing recalled".
            Err(_) => return Ok(Vec::new()),
        };

        Ok(results
            .iter()
            .filter_map(|record| {
                record
                    .get("content")
                    .and_then(|value| value.as_str())
                    .map(str::to_string)
            })
            .collect())
    }
}
