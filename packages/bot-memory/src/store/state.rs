//! Where compaction stands: the last compacted position, items kept verbatim from before it,
//! the Now block, how far facts were mined and the engine working on it. A save is a
//! compare-and-swap on the revision, so writers from an older state never overwrite a newer one.

use std::collections::HashMap;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use flow_like_storage::arrow_array::{Int64Array, RecordBatch, StringArray};
use flow_like_storage::arrow_schema::Schema;
use flow_like_storage::lancedb::Connection;
use flow_like_storage::lancedb::table::WriteOptions;
use flow_like_types::{Result, create_id};
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::lance::{Table, int_field, millis, text_field};
use crate::model::{Seq, SeqSpan};
use crate::summary::NowBlock;

/// An engine's claim on the memory while it compacts, mines or follows a deletion.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Lease {
    pub holder: String,
    pub until: DateTime<Utc>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct CompactionState {
    /// Every position up to here is covered by a summary.
    #[serde(default)]
    pub compacted_through: Option<Seq>,
    /// Items at or before `compacted_through` the prompt still shows verbatim.
    #[serde(default)]
    pub retained: Vec<Seq>,
    #[serde(default)]
    pub now: Option<NowBlock>,
    /// Facts were extracted from every position up to here, except the `unmined` spans.
    #[serde(default)]
    pub mined_through: Option<Seq>,
    /// Spans whose extraction failed or whose facts must be extracted again; idle mining retries
    /// them.
    #[serde(default)]
    pub unmined: Vec<SeqSpan>,
    /// Summaries a deletion cleaned without a model; idle work writes them again.
    #[serde(default)]
    pub pending_rewrites: Vec<String>,
    /// Deletions up to this position were followed (facts, summaries and index cleaned).
    #[serde(default)]
    pub deletions_through: Option<Seq>,
    #[serde(default)]
    pub compactions: u32,
    #[serde(default)]
    pub updated_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub lease: Option<Lease>,
    /// Bumped by every save.
    #[serde(default)]
    pub revision: u64,
}

/// Another writer saved the state first.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StateConflict {
    pub expected: u64,
    pub found: u64,
}

impl std::fmt::Display for StateConflict {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "The memory state was saved by another writer meanwhile (revision {}, expected {}); plan again",
            self.found, self.expected
        )
    }
}

impl std::error::Error for StateConflict {}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct StateRow {
    id: String,
    body: String,
    revision: i64,
    /// Unique per save, so a save can tell its own write from an equal one by another writer.
    writer: String,
    updated_at: i64,
}

const ROW_ID: &str = "state";
const COLUMNS: [&str; 5] = ["id", "body", "revision", "writer", "updated_at"];

fn schema() -> Schema {
    Schema::new(vec![
        text_field("id", false),
        text_field("body", false),
        int_field("revision", false),
        text_field("writer", false),
        int_field("updated_at", false),
    ])
}

pub struct StateStore {
    table: Table,
}

impl StateStore {
    pub const TABLE: &'static str = "state";

    pub(crate) async fn open(connection: Connection, write_options: Option<WriteOptions>) -> Self {
        Self {
            table: Table::open(connection, Self::TABLE, None, write_options).await,
        }
    }

    /// The stored row, read fresh, so a save from another handle is seen.
    async fn row(&self) -> Result<Option<StateRow>> {
        self.table.refresh().await?;
        let rows: Vec<StateRow> = self
            .table
            .rows(&format!("id = '{ROW_ID}'"), &COLUMNS, 1, 0)
            .await?;
        Ok(rows.into_iter().next())
    }

    /// The saved state, or the empty state of a bot that never compacted.
    pub async fn load(&self) -> Result<CompactionState> {
        let Some(row) = self.row().await? else {
            return Ok(CompactionState::default());
        };
        let mut state: CompactionState = serde_json::from_str(&row.body)?;
        state.revision = u64::try_from(row.revision).unwrap_or_default();
        Ok(state)
    }

    /// Saves `state` if the stored revision is still `state.revision` and returns it with the
    /// next one; fails with [`StateConflict`] when another writer saved first.
    pub async fn save(&self, state: &CompactionState) -> Result<CompactionState> {
        self.ensure_row().await?;
        let saved = CompactionState {
            revision: state.revision + 1,
            ..state.clone()
        };
        let writer = create_id();
        self.table
            .update(
                &format!("id = '{ROW_ID}' AND revision = {}", state.revision),
                HashMap::from([
                    ("body".to_string(), json!(serde_json::to_string(&saved)?)),
                    (
                        "revision".to_string(),
                        json!(i64::try_from(saved.revision)?),
                    ),
                    ("writer".to_string(), json!(writer)),
                    (
                        "updated_at".to_string(),
                        json!(saved.updated_at.map_or(0, millis)),
                    ),
                ]),
            )
            .await?;
        let stored = self.row().await?;
        if stored.as_ref().is_none_or(|row| row.writer != writer) {
            return Err(StateConflict {
                expected: state.revision,
                found: stored.map_or(0, |row| u64::try_from(row.revision).unwrap_or_default()),
            }
            .into());
        }
        Ok(saved)
    }

    /// The row exists from the table's creation on, so even the first save is a swap.
    async fn ensure_row(&self) -> Result<()> {
        if self.table.exists().await? {
            return Ok(());
        }
        let empty = serde_json::to_string(&CompactionState::default())?;
        let row = RecordBatch::try_new(
            Arc::new(schema()),
            vec![
                Arc::new(StringArray::from(vec![ROW_ID])),
                Arc::new(StringArray::from(vec![empty])),
                Arc::new(Int64Array::from(vec![0])),
                Arc::new(StringArray::from(vec![""])),
                Arc::new(Int64Array::from(vec![0])),
            ],
        )?;
        self.table.create_with(row).await
    }

    pub(crate) async fn refresh(&self) -> Result<()> {
        self.table.refresh().await
    }

    pub(crate) async fn prune(&self) -> Result<()> {
        self.table.prune().await
    }

    pub(crate) async fn drop_all(&self) -> Result<()> {
        self.table.drop_table().await
    }
}
