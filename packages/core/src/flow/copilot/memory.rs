//! Profile-scoped semantic memory for the global assistant, backed by LanceDB.
//!
//! Each profile gets its OWN memory table (`assistant-memory/<profile_id>`), so memories in a work
//! profile never mix with those in another profile. The embedding model is user-selected (a bit id),
//! resolved through the same `EmbeddingFactory` the rest of the app uses. All of this is reachable
//! without a flow `ExecutionContext`, so it plugs directly into the context-free `PlatformCopilot`.

use std::cmp::Reverse;
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

const MAX_RECALLED_MEMORY_ITEM_CHARS: usize = 500;
const MAX_RECALLED_MEMORY_PROMPT_CHARS: usize = 2_400;

fn bounded_memory_text(value: &str, max_chars: usize) -> String {
    let value = value.trim();
    if value.chars().count() <= max_chars {
        return value.to_string();
    }
    let mut bounded: String = value.chars().take(max_chars.saturating_sub(1)).collect();
    bounded.push('…');
    bounded
}

/// Snapshot of a profile's memory table: how many observations it holds and which embedding model
/// they were written with (so the UI can warn before switching to an incompatible model).
#[derive(Debug, Clone, flow_like_types::json::Serialize)]
pub struct MemoryStatus {
    pub count: usize,
    pub embedding_model_id: Option<String>,
}

/// A single stored observation, surfaced to the UI so the user can review and delete what the
/// assistant remembered.
#[derive(Debug, Clone, flow_like_types::json::Serialize)]
pub struct MemoryEntry {
    pub id: String,
    pub content: String,
    pub role: String,
    pub timestamp: i64,
}

/// Semantic memory store for one profile. Cheap to hold across the chat loop (async-locked store).
pub struct AssistantMemory {
    store: RwLock<LanceDBVectorStore>,
    embedding: Arc<dyn EmbeddingModelLogic>,
    embedding_model_id: String,
}

impl AssistantMemory {
    /// Open the per-profile LanceDB store (no embedding model). Lazily created on first write.
    ///
    /// `owner` scopes the table to a user on multi-tenant deployments: `Some(sub)` stores under
    /// `users/<sub>/assistant-memory/<profile_id>` (matching the per-user credential prefix so
    /// scoped writes are allowed and users never share a namespace); `None` (desktop, single user)
    /// keeps the flat `assistant-memory/<profile_id>` layout.
    async fn open_store(
        state: &Arc<FlowLikeState>,
        owner: Option<&str>,
        profile_id: &str,
    ) -> Result<LanceDBVectorStore> {
        let dir = match owner.map(str::trim).filter(|owner| !owner.is_empty()) {
            Some(owner) => Path::from("users")
                .child(owner)
                .child("assistant-memory")
                .child(profile_id),
            None => Path::from("assistant-memory").child(profile_id),
        };
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
        if let Some(opts) = state
            .config
            .read()
            .await
            .callbacks
            .lance_write_options
            .clone()
        {
            store.set_write_options(opts);
        }
        Ok(store)
    }

    /// Open (or lazily create on first write) the memory table for `profile_id`, embedding with the
    /// given bit. See [`open_store`](Self::open_store) for how `owner` scopes the table.
    pub async fn open(
        state: Arc<FlowLikeState>,
        owner: Option<&str>,
        profile_id: &str,
        embedding_bit: &Bit,
    ) -> Result<Self> {
        let embedding = state
            .embedding_factory
            .lock()
            .await
            .build_text(embedding_bit, state.clone())
            .await?;
        let store = Self::open_store(&state, owner, profile_id).await?;

        Ok(Self {
            store: RwLock::new(store),
            embedding,
            embedding_model_id: embedding_bit.id.clone(),
        })
    }

    /// Report how many observations a profile has stored and which embedding model produced them.
    pub async fn status(
        state: Arc<FlowLikeState>,
        owner: Option<&str>,
        profile_id: &str,
    ) -> Result<MemoryStatus> {
        let store = Self::open_store(&state, owner, profile_id).await?;
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
    pub async fn clear(
        state: Arc<FlowLikeState>,
        owner: Option<&str>,
        profile_id: &str,
    ) -> Result<()> {
        let mut store = Self::open_store(&state, owner, profile_id).await?;
        store.drop_table().await
    }

    /// List a profile's stored observations, newest first, for display and management in the UI.
    /// Reads the table directly (no embedding model needed), mirroring `status`/`clear`. Returns an
    /// empty list for a fresh profile whose table does not exist yet.
    pub async fn list(
        state: Arc<FlowLikeState>,
        owner: Option<&str>,
        profile_id: &str,
    ) -> Result<Vec<MemoryEntry>> {
        let store = Self::open_store(&state, owner, profile_id).await?;
        let rows = store
            .filter(
                "1=1",
                Some(vec![
                    "id".to_string(),
                    "content".to_string(),
                    "role".to_string(),
                    "timestamp".to_string(),
                ]),
                1000,
                0,
            )
            .await
            .unwrap_or_default();

        let mut entries: Vec<MemoryEntry> = rows
            .into_iter()
            .filter_map(|row| {
                Some(MemoryEntry {
                    id: row.get("id")?.as_str()?.to_string(),
                    content: row.get("content")?.as_str()?.to_string(),
                    role: row
                        .get("role")
                        .and_then(|value| value.as_str())
                        .unwrap_or("user")
                        .to_string(),
                    timestamp: row
                        .get("timestamp")
                        .and_then(|value| value.as_i64())
                        .unwrap_or(0),
                })
            })
            .collect();
        entries.sort_by_key(|entry| Reverse(entry.timestamp));
        Ok(entries)
    }

    /// Delete a single observation by id. No-op if it no longer exists.
    pub async fn delete_entry(
        state: Arc<FlowLikeState>,
        owner: Option<&str>,
        profile_id: &str,
        id: &str,
    ) -> Result<()> {
        let store = Self::open_store(&state, owner, profile_id).await?;
        let escaped = id.replace('\'', "''");
        store.delete(&format!("id = '{escaped}'")).await
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

    /// System-prompt sections advertising memory to the model: recalled facts relevant to
    /// `user_prompt` (when any) plus the standing memory-tool instructions. Shared by every
    /// backend so recall behaves identically regardless of the selected model.
    pub async fn prompt_sections(&self, user_prompt: &str) -> String {
        let mut sections = String::new();
        if let Ok(recalled) = self.search(user_prompt, 6).await
            && !recalled.is_empty()
        {
            sections.push_str("\n\n## RELEVANT MEMORY\nFacts you remembered that may help:\n");
            let mut remaining = MAX_RECALLED_MEMORY_PROMPT_CHARS;
            for item in &recalled {
                if remaining == 0 {
                    break;
                }
                let bounded =
                    bounded_memory_text(item, MAX_RECALLED_MEMORY_ITEM_CHARS.min(remaining));
                if bounded.is_empty() {
                    continue;
                }
                remaining = remaining.saturating_sub(bounded.chars().count());
                sections.push_str(&format!("- {bounded}\n"));
            }
        }
        sections.push_str(
            "\n\n## MEMORY\nYou have persistent, profile-scoped memory. Use `_memory_search` to recall facts and `_memory_store` to save important user facts, preferences, and decisions. Store salient facts immediately rather than only saying you will remember them.",
        );
        sections
    }

    /// Embed `query` and return up to `top_k` relevant memory contents (most similar first). Returns
    /// an empty list rather than erroring when the table does not exist yet (fresh profile).
    pub async fn search(&self, query: &str, top_k: usize) -> Result<Vec<String>> {
        let query = query.trim();
        if query.is_empty() || top_k == 0 {
            return Ok(Vec::new());
        }

        let embeddings = self
            .embedding
            .text_embed_query(&vec![query.to_string()])
            .await?;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recalled_memory_text_is_unicode_safe_and_bounded() {
        assert_eq!(bounded_memory_text("  short fact  ", 20), "short fact");
        let bounded = bounded_memory_text(&"🙂".repeat(20), 8);
        assert_eq!(bounded.chars().count(), 8);
        assert!(bounded.ends_with('…'));
    }
}
