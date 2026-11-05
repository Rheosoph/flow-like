//! LanceDB tables at a location the host chooses (the bot's workspace): facts, segment
//! summaries, the history index and the compaction state. Tables appear on first write.

mod facts;
mod history;
mod lance;
mod segments;
mod state;

use std::collections::BTreeSet;
use std::path::Path;
use std::sync::Arc;

use flow_like_storage::databases::vector::lancedb::connect_lance;
use flow_like_storage::lancedb::Connection;
use flow_like_storage::lancedb::table::WriteOptions;
use flow_like_types::{Result, anyhow};

pub use facts::{FactFilter, FactQuery, FactStore, ScoredFact};
pub use history::{HistoryHit, HistoryIndex, HistoryQuery, projection};
pub(crate) use lance::embed_or_wait;
pub use segments::{ScoredSegment, SegmentStore};
pub use state::{CompactionState, Lease, StateConflict, StateStore};

use crate::llm::Embedder;

const ROOM_PREFIX: &str = "memory_room_";

pub struct MemoryStores {
    pub facts: FactStore,
    pub segments: SegmentStore,
    pub history: HistoryIndex,
    pub state: StateStore,
    /// Owner-direct facts available for recall in a room. Room writes use `facts` only.
    pub owner_facts: Option<FactStore>,
    connection: Connection,
    write_options: Option<WriteOptions>,
    room_scope: Option<String>,
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
            connection,
            write_options,
            room_scope: None,
        }
    }

    /// A room keeps its own history positions, summaries, facts and compaction lease. The
    /// owner's existing facts remain available for recall without becoming writable here.
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
            owner_facts: Some(
                FactStore::open(connection.clone(), embedder, write_options.clone()).await,
            ),
            connection,
            write_options,
            room_scope: Some(scope.to_owned()),
        }
    }

    pub(crate) fn room_scope(&self) -> Option<&str> {
        self.room_scope.as_deref()
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
    /// versions older than Lance's 7-day retention. A failing embedder only postpones the
    /// embedding. Hosts run it through `MemoryEngine::optimize`, under the lease.
    pub(crate) async fn optimize(&self) -> Result<()> {
        for (table, embedded) in [
            ("facts", self.facts.reembed().await),
            ("summaries", self.segments.reembed().await),
        ] {
            if let Err(error) = embedded {
                tracing::warn!(
                    "Embedding the memory's {table} failed; BM25 finds them meanwhile: {error:#}"
                );
            }
        }
        self.facts.optimize().await?;
        self.segments.optimize().await?;
        self.history.optimize().await
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
        Ok(())
    }

    /// Drops every table: the bot is gone, or its memory is reset.
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
            SegmentStore::TABLE,
            HistoryIndex::TABLE,
            StateStore::TABLE,
        ]
        .contains(&table))
    .then_some(scope)
}

#[cfg(test)]
mod tests {
    use super::room_scope_of;

    #[test]
    fn room_tables_require_the_complete_reserved_name() {
        let scope = "a".repeat(64);
        for table in ["facts", "segments", "history", "state"] {
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
