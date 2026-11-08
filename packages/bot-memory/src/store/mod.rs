//! LanceDB tables at a location the host chooses (the bot's workspace): facts, segment
//! summaries, the history index and the compaction state. Tables appear on first write.

mod expiry;
mod facts;
mod history;
mod lance;
mod segments;
mod state;

use std::collections::BTreeSet;
use std::path::Path;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use flow_like_storage::databases::vector::lancedb::connect_lance;
use flow_like_storage::lancedb::Connection;
use flow_like_storage::lancedb::table::WriteOptions;
use flow_like_types::{Result, anyhow};

pub use facts::{FactFilter, FactQuery, FactSort, FactStore, ScoredFact};
pub use history::{HistoryHit, HistoryIndex, HistoryQuery, projection};
pub(crate) use lance::embed_or_wait;
pub(crate) use segments::PreparedSegments;
pub use segments::{ScoredSegment, SegmentStore};
pub use state::{CompactionState, Lease, StateConflict, StateStore};

use crate::llm::Embedder;

const ROOM_PREFIX: &str = "memory_room_";

/// Memory-owned names inside a bot's workspace. Hosts exclude them from general database
/// tools so those tools cannot bypass conversation memory permissions.
pub fn is_memory_table(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    name.starts_with(ROOM_PREFIX)
        || [
            FactStore::TABLE,
            "facts_expiry",
            SegmentStore::TABLE,
            HistoryIndex::TABLE,
            StateStore::TABLE,
        ]
        .contains(&name.as_str())
}

pub(crate) struct SharedFacts {
    pub scope: String,
    pub facts: FactStore,
}

pub struct MemoryStores {
    pub facts: FactStore,
    pub segments: SegmentStore,
    pub history: HistoryIndex,
    pub state: StateStore,
    /// Owner-direct facts available for recall in a room. Room writes use `facts` only.
    pub owner_facts: Option<FactStore>,
    pub(crate) shared_facts: Vec<SharedFacts>,
    connection: Connection,
    write_options: Option<WriteOptions>,
    room_scope: Option<String>,
}

pub(crate) struct PreparedOptimization {
    facts: Option<lance::PreparedEmbeddings>,
    segments: Option<lance::PreparedEmbeddings>,
    failure: Option<flow_like_types::Error>,
}

impl MemoryStores {
    /// Opens the stores on a connection the host built (the bot workspace's database, with the
    /// host's Lance session and write options).
    pub async fn open(
        connection: Connection,
        embedder: Arc<dyn Embedder>,
        write_options: Option<WriteOptions>,
    ) -> Self {
        Self {
            facts: FactStore::open(connection.clone(), embedder.clone(), write_options.clone())
                .await,
            segments: SegmentStore::open(connection.clone(), embedder, write_options.clone()).await,
            history: HistoryIndex::open(connection.clone(), write_options.clone()).await,
            state: StateStore::open(connection.clone(), write_options.clone()).await,
            owner_facts: None,
            shared_facts: Vec::new(),
            connection,
            write_options,
            room_scope: None,
        }
    }

    /// A room keeps its own history positions, summaries, facts and compaction lease. Recall
    /// from other conversations requires an explicit call to [`Self::with_recall_sources`].
    pub async fn open_room(
        connection: Connection,
        embedder: Arc<dyn Embedder>,
        write_options: Option<WriteOptions>,
        key: &str,
    ) -> Self {
        let scope = blake3::hash(key.as_bytes()).to_hex().to_string();
        Self::open_room_scope(connection, embedder, write_options, &scope).await
    }

    async fn open_room_scope(
        connection: Connection,
        embedder: Arc<dyn Embedder>,
        write_options: Option<WriteOptions>,
        scope: &str,
    ) -> Self {
        let name = |table: &str| format!("{ROOM_PREFIX}{scope}_{table}");
        Self {
            facts: FactStore::open_named(
                connection.clone(),
                embedder.clone(),
                write_options.clone(),
                name(FactStore::TABLE),
            )
            .await,
            segments: SegmentStore::open_named(
                connection.clone(),
                embedder.clone(),
                write_options.clone(),
                name(SegmentStore::TABLE),
            )
            .await,
            history: HistoryIndex::open_named(
                connection.clone(),
                write_options.clone(),
                name(HistoryIndex::TABLE),
            )
            .await,
            state: StateStore::open_named(
                connection.clone(),
                write_options.clone(),
                name(StateStore::TABLE),
            )
            .await,
            owner_facts: None,
            shared_facts: Vec::new(),
            connection,
            write_options,
            room_scope: Some(scope.to_owned()),
        }
    }

    pub(crate) fn room_scope(&self) -> Option<&str> {
        self.room_scope.as_deref()
    }

    /// Attaches only the sources the host authorized for this turn. Source facts stay in their
    /// own tables, so deletion and revoked sharing do not leave copied memories elsewhere.
    pub async fn with_recall_sources(
        mut self,
        use_owner_memory: bool,
        shared_room_keys: &[String],
    ) -> Self {
        let embedder = self.facts.embedder().clone();
        self.owner_facts = if self.room_scope.is_some() && use_owner_memory {
            Some(
                FactStore::open(
                    self.connection.clone(),
                    embedder.clone(),
                    self.write_options.clone(),
                )
                .await,
            )
        } else {
            None
        };
        self.shared_facts.clear();
        let scopes: BTreeSet<String> = shared_room_keys
            .iter()
            .filter(|key| !key.is_empty())
            .map(|key| blake3::hash(key.as_bytes()).to_hex().to_string())
            .filter(|scope| self.room_scope.as_ref() != Some(scope))
            .collect();
        for scope in scopes {
            let facts = FactStore::open_named(
                self.connection.clone(),
                embedder.clone(),
                self.write_options.clone(),
                format!("{ROOM_PREFIX}{scope}_{}", FactStore::TABLE),
            )
            .await;
            self.shared_facts.push(SharedFacts { scope, facts });
        }
        self
    }

    /// Existing room stores, including a room that has only indexed history or saved state.
    /// Exact table families keep unrelated workspace tables out of memory administration.
    pub(crate) async fn rooms(&self, embedder: Arc<dyn Embedder>) -> Result<Vec<Self>> {
        let mut scopes = BTreeSet::new();
        for name in self.connection.table_names().execute().await? {
            if let Some(scope) = room_scope_of(&name) {
                scopes.insert(scope.to_owned());
            }
        }
        let mut rooms = Vec::new();
        for scope in scopes {
            let mut stores = Self::open_room_scope(
                self.connection.clone(),
                embedder.clone(),
                self.write_options.clone(),
                &scope,
            )
            .await;
            stores.owner_facts = None;
            rooms.push(stores);
        }
        Ok(rooms)
    }

    /// Opens the stores in a local directory.
    pub async fn open_local(path: &Path, embedder: Arc<dyn Embedder>) -> Result<Self> {
        let uri = path
            .to_str()
            .ok_or_else(|| anyhow!("The memory path {} is not valid UTF-8", path.display()))?;
        let connection = connect_lance(uri).execute().await?;
        Ok(Self::open(connection, embedder, None).await)
    }

    /// Idle maintenance: embeds facts and summaries that have no vector from the current
    /// embedder yet (it failed, or the model changed), folds new rows into the indexes and drops
    /// versions older than Lance's 7-day retention. Preparation runs without the memory lease.
    /// A failed embedding is reported after table maintenance, so background work retries it.
    pub(crate) async fn prepare_optimization(&self, now: DateTime<Utc>) -> PreparedOptimization {
        fn prepared(
            table: &str,
            result: Result<lance::PreparedEmbeddings>,
            failure: &mut Option<flow_like_types::Error>,
        ) -> Option<lance::PreparedEmbeddings> {
            match result {
                Ok(prepared) => Some(prepared),
                Err(error) => {
                    tracing::warn!(
                        "Embedding the memory's {table} failed; BM25 finds them meanwhile: {error:#}"
                    );
                    failure.get_or_insert(error);
                    None
                }
            }
        }
        let mut failure = None;
        let facts = prepared("facts", self.facts.prepare_reembed(now).await, &mut failure);
        let segments = prepared(
            "summaries",
            self.segments.prepare_reembed().await,
            &mut failure,
        );
        PreparedOptimization {
            facts,
            segments,
            failure,
        }
    }

    /// Commits prepared vectors and maintains indexes under a revision-validated memory lease.
    /// Every model call finished during preparation.
    pub(crate) async fn apply_optimization(&self, prepared: PreparedOptimization) -> Result<()> {
        if let Some(facts) = prepared.facts {
            self.facts.apply_reembed(facts).await?;
        }
        if let Some(segments) = prepared.segments {
            self.segments.apply_reembed(segments).await?;
        }
        self.facts.optimize().await?;
        self.segments.optimize().await?;
        self.history.optimize().await?;
        match prepared.failure {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }

    /// Drops every older version of every table, so deleted content leaves the disk now rather
    /// than after the retention window. Runs after each deletion, under the lease.
    pub(crate) async fn prune(&self) -> Result<()> {
        self.facts.prune().await?;
        self.segments.prune().await?;
        self.history.prune().await?;
        self.state.prune().await
    }

    /// Picks up what other handles on the same tables wrote.
    pub(crate) async fn refresh(&self) -> Result<()> {
        self.facts.refresh().await?;
        self.segments.refresh().await?;
        self.history.refresh().await?;
        self.state.refresh().await?;
        if let Some(owner_facts) = &self.owner_facts {
            owner_facts.refresh().await?;
        }
        for source in &self.shared_facts {
            source.facts.refresh().await?;
        }
        Ok(())
    }

    /// Clears this conversation's content, including its expiry markers. The state remains
    /// for the engine to invalidate drafts and release its reset lease.
    pub(crate) async fn clear_content(&self) -> Result<()> {
        self.facts.clear().await?;
        self.segments.clear().await?;
        self.history.clear().await
    }

    /// Drops every table when removing a memory store permanently.
    pub async fn drop_all(&self) -> Result<()> {
        self.facts.drop_all().await?;
        self.segments.drop_all().await?;
        self.history.drop_all().await?;
        self.state.drop_all().await
    }
}

fn room_scope_of(name: &str) -> Option<&str> {
    let (scope, table) = name.strip_prefix(ROOM_PREFIX)?.split_once('_')?;
    (scope.len() == 64
        && scope.bytes().all(|byte| byte.is_ascii_hexdigit())
        && [
            FactStore::TABLE,
            "facts_expiry",
            SegmentStore::TABLE,
            HistoryIndex::TABLE,
            StateStore::TABLE,
        ]
        .contains(&table))
    .then_some(scope)
}

#[cfg(test)]
mod tests {
    use super::{is_memory_table, room_scope_of};

    #[test]
    fn workspace_memory_names_are_reserved_case_insensitively() {
        for name in [
            "facts",
            "FACTS_EXPIRY",
            "SEGMENTS",
            "history",
            "state",
            "memory_room_any_facts",
        ] {
            assert!(is_memory_table(name), "{name}");
        }
        for name in ["users", "fact_sheet", "history_archive", "memory_notes"] {
            assert!(!is_memory_table(name), "{name}");
        }
    }

    #[test]
    fn room_tables_require_the_complete_reserved_name() {
        let scope = "a".repeat(64);
        for table in ["facts", "facts_expiry", "segments", "history", "state"] {
            let name = format!("memory_room_{scope}_{table}");
            assert_eq!(room_scope_of(&name), Some(scope.as_str()));
        }
        for name in [
            "facts".to_string(),
            "memory_room_short_facts".to_string(),
            format!("memory_room_{scope}_files"),
            format!("memory_room_{scope}_facts_backup"),
            format!("memory_room_{}_facts", "g".repeat(64)),
        ] {
            assert_eq!(room_scope_of(&name), None);
        }
    }
}
