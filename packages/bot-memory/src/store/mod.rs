//! LanceDB tables at a location the host chooses (the bot's workspace): facts, segment
//! summaries, the history index and the compaction state. Tables appear on first write.

mod facts;
mod history;
mod lance;
mod segments;
mod state;

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

pub struct MemoryStores {
    pub facts: FactStore,
    pub segments: SegmentStore,
    pub history: HistoryIndex,
    pub state: StateStore,
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
            state: StateStore::open(connection, write_options).await,
        }
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
        self.state.refresh().await
    }

    /// Drops every table: the bot is gone, or its memory is reset.
    pub async fn drop_all(&self) -> Result<()> {
        self.facts.drop_all().await?;
        self.segments.drop_all().await?;
        self.history.drop_all().await?;
        self.state.drop_all().await
    }
}
