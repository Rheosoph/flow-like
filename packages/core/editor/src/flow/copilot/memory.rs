//! Semantic memory for the global assistant and for bots, backed by LanceDB.
//!
//! Every [`MemoryScope`] gets its OWN memory table: FlowPilot one per profile
//! (`assistant-memory/<profile_id>`), every bot one of its own (`bot-memory/<bot_id>`), so memories
//! of a work profile, of FlowPilot and of each bot never mix. The embedding model is user-selected
//! (a bit id), resolved through the same `EmbeddingFactory` the rest of the app uses. All of this is
//! reachable without a flow `ExecutionContext`, so it plugs directly into the context-free
//! `PlatformCopilot`.

#[cfg(feature = "flow-runtime")]
use std::cmp::Reverse;
#[cfg(feature = "flow-runtime")]
use std::collections::hash_map::DefaultHasher;
#[cfg(feature = "flow-runtime")]
use std::hash::{Hash, Hasher};
use std::sync::Arc;
#[cfg(feature = "flow-runtime")]
use std::time::{SystemTime, UNIX_EPOCH};

#[cfg(feature = "flow-runtime")]
use flow_like_model_provider::embedding::EmbeddingModelLogic;
#[cfg(feature = "flow-runtime")]
use flow_like_storage::databases::vector::{VectorStore, lancedb::LanceDBVectorStore};
#[cfg(any(feature = "flow-runtime", test))]
use flow_like_storage::object_store::path::Path;
#[cfg(feature = "flow-runtime")]
use flow_like_types::json::json;
#[cfg(feature = "flow-runtime")]
use flow_like_types::tokio::sync::RwLock;
use flow_like_types::{Result, bail};
#[cfg(feature = "flow-runtime")]
use flow_like_types::{anyhow, create_id};

use crate::bit::Bit;
use crate::models::llm::ModelUsageContext;
use crate::state::FlowLikeState;

#[cfg(feature = "flow-runtime")]
const MAX_RECALLED_MEMORY_ITEM_CHARS: usize = 500;
#[cfg(feature = "flow-runtime")]
const MAX_RECALLED_MEMORY_PROMPT_CHARS: usize = 2_400;

#[cfg(any(feature = "flow-runtime", test))]
fn bounded_memory_text(value: &str, max_chars: usize) -> String {
    let value = value.trim();
    if value.chars().count() <= max_chars {
        return value.to_string();
    }
    let mut bounded: String = value.chars().take(max_chars.saturating_sub(1)).collect();
    bounded.push('…');
    bounded
}

/// Whose memory a store holds. FlowPilot keeps one store per profile and every bot keeps its own,
/// so two bots, or a bot and FlowPilot, never read or write each other's memories.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MemoryScope {
    /// FlowPilot (Flux) in one profile.
    Profile(String),
    /// One bot, in whatever profile it runs.
    Bot(String),
}

impl MemoryScope {
    /// `[users/<owner>/]assistant-memory/<profile_id>` or `[users/<owner>/]bot-memory/<bot_id>`.
    /// `owner` scopes the table to a user on multi-tenant deployments, matching the per-user
    /// credential prefix; the desktop (single user) passes `None`. Each id is one encoded path
    /// segment, so no id reaches another scope's directory.
    #[cfg(any(feature = "flow-runtime", test))]
    fn directory(&self, owner: Option<&str>) -> Path {
        let (kind, id) = match self {
            Self::Profile(profile_id) => ("assistant-memory", profile_id),
            Self::Bot(bot_id) => ("bot-memory", bot_id),
        };
        let base = match owner.map(str::trim).filter(|owner| !owner.is_empty()) {
            Some(owner) => Path::from("users").join(owner).join(kind),
            None => Path::from(kind),
        };
        base.join(id.as_str())
    }
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

/// Semantic memory store for one [`MemoryScope`]. Cheap to hold across the chat loop
/// (async-locked store).
#[cfg(feature = "flow-runtime")]
pub struct AssistantMemory {
    store: RwLock<LanceDBVectorStore>,
    embedding: Arc<dyn EmbeddingModelLogic>,
    embedding_model_id: String,
    scope: MemoryScope,
    read_only: bool,
}

#[cfg(not(feature = "flow-runtime"))]
pub struct AssistantMemory {
    scope: MemoryScope,
    read_only: bool,
}

impl AssistantMemory {
    /// Whose memory this is; a run only accepts a store of its own scope.
    pub fn scope(&self) -> &MemoryScope {
        &self.scope
    }

    /// Recall existing owner guidance without saving unattributed statements from a room.
    pub fn read_only(mut self) -> Self {
        self.read_only = true;
        self
    }

    pub fn is_read_only(&self) -> bool {
        self.read_only
    }

    fn ensure_writable(&self) -> Result<()> {
        if self.read_only {
            bail!(
                "Legacy memory is read-only in shared rooms. Existing memories can be recalled, but this turn cannot save to this store."
            );
        }
        Ok(())
    }
}

#[cfg(feature = "flow-runtime")]
impl AssistantMemory {
    /// Open the LanceDB store of `scope` (no embedding model). Lazily created on first write.
    /// See [`MemoryScope::directory`] for the layout and how `owner` scopes it.
    async fn open_store(
        state: &Arc<FlowLikeState>,
        owner: Option<&str>,
        scope: &MemoryScope,
    ) -> Result<LanceDBVectorStore> {
        let dir = scope.directory(owner);
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

    /// Open (or lazily create on first write) the memory table of `scope`, embedding with the
    /// given bit. `access_token` and `usage_context` let a cloud host route a remote-capable Bit
    /// through the authenticated embedding proxy. See [`MemoryScope::directory`] for how `owner`
    /// scopes the table.
    pub async fn open(
        state: Arc<FlowLikeState>,
        owner: Option<&str>,
        scope: &MemoryScope,
        embedding_bit: &Bit,
        access_token: Option<String>,
        usage_context: Option<ModelUsageContext>,
    ) -> Result<Self> {
        let embedding = state
            .embedding_factory
            .build_text_routed(embedding_bit, state.clone(), access_token, usage_context)
            .await?;
        let store = Self::open_store(&state, owner, scope).await?;

        Ok(Self {
            store: RwLock::new(store),
            embedding,
            embedding_model_id: embedding_bit.id.clone(),
            scope: scope.clone(),
            read_only: false,
        })
    }

    /// Report how many observations a scope has stored and which embedding model produced them.
    pub async fn status(
        state: Arc<FlowLikeState>,
        owner: Option<&str>,
        scope: &MemoryScope,
    ) -> Result<MemoryStatus> {
        let store = Self::open_store(&state, owner, scope).await?;
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

    /// Delete all memories of a scope by DROPPING the table (data and schema). Used when the
    /// embedding model changes: a new model usually has a different vector dimension, so the table
    /// must be recreated with the new schema on the next insert — purging rows would keep the old
    /// schema and permanently break `_memory_store`.
    pub async fn clear(
        state: Arc<FlowLikeState>,
        owner: Option<&str>,
        scope: &MemoryScope,
    ) -> Result<()> {
        let mut store = Self::open_store(&state, owner, scope).await?;
        store.drop_table().await
    }

    /// List a scope's stored observations, newest first, for display and management in the UI.
    /// Reads the table directly (no embedding model needed), mirroring `status`/`clear`. Returns an
    /// empty list for a fresh scope whose table does not exist yet.
    pub async fn list(
        state: Arc<FlowLikeState>,
        owner: Option<&str>,
        scope: &MemoryScope,
    ) -> Result<Vec<MemoryEntry>> {
        let store = Self::open_store(&state, owner, scope).await?;
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
        scope: &MemoryScope,
        id: &str,
    ) -> Result<()> {
        let store = Self::open_store(&state, owner, scope).await?;
        let escaped = id.replace('\'', "''");
        store.delete(&format!("id = '{escaped}'")).await
    }

    /// Embed `content` and append it as a memory observation. Returns the total observation count.
    pub async fn store(&self, role: &str, content: &str) -> Result<usize> {
        self.ensure_writable()?;
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
        if self.is_read_only() {
            sections.push_str("\n\n## MEMORY\nYou can recall existing owner guidance with `_memory_search`. This legacy memory is read-only in this shared room. Follow relevant guidance when answering within your configured role, and keep unrelated private information out of shared answers. You cannot save memories to this store in this turn; never claim that you did.");
            return sections;
        }
        sections.push_str(match self.scope {
            MemoryScope::Profile(_) => {
                "\n\n## MEMORY\nYou have persistent, profile-scoped memory. Use `_memory_search` to recall facts and `_memory_store` to save important user facts, preferences, and decisions. Store salient facts immediately rather than only saying you will remember them."
            }
            MemoryScope::Bot(_) => {
                "\n\n## MEMORY\nYou have your own persistent memory, separate from every other bot and from Flux. Use `_memory_search` to recall facts and `_memory_store` to save important facts, preferences, and decisions of your owner. Store salient facts immediately rather than only saying you will remember them."
            }
        });
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

#[cfg(not(feature = "flow-runtime"))]
impl AssistantMemory {
    fn unavailable<T>() -> Result<T> {
        bail!("Assistant memory requires the `flow-runtime` feature")
    }

    pub async fn open(
        _state: Arc<FlowLikeState>,
        _owner: Option<&str>,
        _scope: &MemoryScope,
        _embedding_bit: &Bit,
        _access_token: Option<String>,
        _usage_context: Option<ModelUsageContext>,
    ) -> Result<Self> {
        Self::unavailable()
    }

    pub async fn status(
        _state: Arc<FlowLikeState>,
        _owner: Option<&str>,
        _scope: &MemoryScope,
    ) -> Result<MemoryStatus> {
        Self::unavailable()
    }

    pub async fn clear(
        _state: Arc<FlowLikeState>,
        _owner: Option<&str>,
        _scope: &MemoryScope,
    ) -> Result<()> {
        Self::unavailable()
    }

    pub async fn list(
        _state: Arc<FlowLikeState>,
        _owner: Option<&str>,
        _scope: &MemoryScope,
    ) -> Result<Vec<MemoryEntry>> {
        Self::unavailable()
    }

    pub async fn delete_entry(
        _state: Arc<FlowLikeState>,
        _owner: Option<&str>,
        _scope: &MemoryScope,
        _id: &str,
    ) -> Result<()> {
        Self::unavailable()
    }

    pub async fn store(&self, _role: &str, _content: &str) -> Result<usize> {
        self.ensure_writable()?;
        Self::unavailable()
    }

    pub async fn prompt_sections(&self, _user_prompt: &str) -> String {
        String::new()
    }

    pub async fn search(&self, _query: &str, _top_k: usize) -> Result<Vec<String>> {
        Self::unavailable()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(not(feature = "flow-runtime"))]
    #[tokio::test]
    async fn read_only_memory_refuses_tool_writes_before_opening_storage() {
        let memory = AssistantMemory {
            scope: MemoryScope::Bot("bot_a".into()),
            read_only: false,
        }
        .read_only();
        assert!(memory.is_read_only());
        assert_eq!(memory.scope(), &MemoryScope::Bot("bot_a".into()));
        let result = super::super::platform::run_memory_tool(
            super::super::tool_spec::MEMORY_STORE_TOOL,
            &serde_json::json!({ "role": "user", "content": "A room member's claim" }),
            Some(&memory),
        )
        .await;
        let result: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert_eq!(result["status"], "error");
        assert!(
            result["error"]
                .as_str()
                .unwrap()
                .contains("read-only in shared rooms")
        );
        assert!(!result["error"].as_str().unwrap().contains("flow-runtime"));
    }

    #[test]
    fn recalled_memory_text_is_unicode_safe_and_bounded() {
        assert_eq!(bounded_memory_text("  short fact  ", 20), "short fact");
        let bounded = bounded_memory_text(&"🙂".repeat(20), 8);
        assert_eq!(bounded.chars().count(), 8);
        assert!(bounded.ends_with('…'));
    }

    #[test]
    fn flowpilot_and_every_bot_keep_separate_memory_stores() {
        let flowpilot = MemoryScope::Profile("profile-1".into());
        let first = MemoryScope::Bot("bot_a".into());
        let second = MemoryScope::Bot("bot_b".into());
        let same_id_as_profile = MemoryScope::Bot("profile-1".into());

        for owner in [None, Some("user-1")] {
            let directories: Vec<String> = [&flowpilot, &first, &second, &same_id_as_profile]
                .iter()
                .map(|scope| scope.directory(owner).to_string())
                .collect();
            for (index, directory) in directories.iter().enumerate() {
                for other in &directories[index + 1..] {
                    assert_ne!(directory, other);
                    assert!(!directory.starts_with(&format!("{other}/")));
                    assert!(!other.starts_with(&format!("{directory}/")));
                }
            }
        }
        assert_eq!(
            flowpilot.directory(None).to_string(),
            "assistant-memory/profile-1"
        );
        assert_eq!(first.directory(None).to_string(), "bot-memory/bot_a");
        assert_eq!(
            first.directory(Some("user-1")).to_string(),
            "users/user-1/bot-memory/bot_a"
        );
        assert_eq!(
            MemoryScope::Bot("bot_a".into()).directory(Some("user-2")),
            Path::from("users/user-2/bot-memory/bot_a")
        );
    }

    #[test]
    fn a_bot_id_cannot_reach_another_memory_store() {
        for hostile in [
            "../assistant-memory/profile-1",
            "..",
            "bot_a/../../assistant-memory/profile-1",
        ] {
            let directory = MemoryScope::Bot(hostile.into()).directory(None).to_string();
            assert!(directory.starts_with("bot-memory/"), "{directory}");
            assert_eq!(directory.matches('/').count(), 1, "{directory}");
        }
    }
}
