//! The segment table: every summary of every level with its range, searchable by vector and
//! BM25. The `parent` column is authoritative: a summary without one is active.

use std::collections::HashMap;
use std::sync::Arc;

use chrono_tz::Tz;
use flow_like_storage::arrow_schema::Schema;
use flow_like_storage::lancedb::Connection;
use flow_like_storage::lancedb::table::WriteOptions;
use flow_like_types::Result;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::lance::{
    Probe, Table, embed_or_wait, from_millis, in_list, int_field, millis, quote, search_pool,
    text_field,
};
use crate::llm::Embedder;
use crate::model::{Seq, SeqSpan};
use crate::retrieval::{RRF_K, TimeRange, rrf};
use crate::summary::SegmentSummary;

const COLUMNS: [&str; 13] = [
    "id",
    "level",
    "seq_from",
    "seq_to",
    "t_from",
    "t_to",
    "title",
    "text",
    "body",
    "parent",
    "version",
    "created_at",
    "embedding_model",
];

/// The table's columns; the vector column is added with the first vector.
fn schema() -> Schema {
    Schema::new(vec![
        text_field("id", false),
        int_field("level", false),
        int_field("seq_from", false),
        int_field("seq_to", false),
        int_field("t_from", false),
        int_field("t_to", false),
        text_field("title", false),
        text_field("text", false),
        text_field("body", false),
        text_field("parent", true),
        int_field("version", false),
        int_field("created_at", false),
        text_field("embedding_model", false),
    ])
}

fn seq_column(seq: Seq) -> i64 {
    i64::try_from(seq).unwrap_or(i64::MAX)
}

/// Summaries whose time overlaps `range`.
fn overlaps(range: TimeRange) -> String {
    format!(
        "t_from < {} AND t_to >= {}",
        millis(range.to),
        millis(range.from)
    )
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SegmentRow {
    id: String,
    level: i64,
    seq_from: i64,
    seq_to: i64,
    t_from: i64,
    t_to: i64,
    title: String,
    text: String,
    body: String,
    parent: Option<String>,
    version: i64,
    created_at: i64,
    embedding_model: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    vector: Option<Vec<f32>>,
}

impl SegmentRow {
    fn new(summary: &SegmentSummary, model: &str, vector: Option<Vec<f32>>) -> Result<Self> {
        Ok(Self {
            id: summary.id.clone(),
            level: i64::from(summary.level),
            seq_from: seq_column(summary.span.from),
            seq_to: seq_column(summary.span.to),
            t_from: millis(summary.t_from),
            t_to: millis(summary.t_to),
            title: summary.body.title.clone(),
            text: summary.render(Tz::UTC),
            body: serde_json::to_string(summary)?,
            parent: summary.parent.clone(),
            version: i64::from(summary.version),
            created_at: millis(summary.created_at),
            embedding_model: if vector.is_some() {
                model.to_string()
            } else {
                String::new()
            },
            vector,
        })
    }

    fn into_summary(self) -> Result<SegmentSummary> {
        let mut summary: SegmentSummary = serde_json::from_str(&self.body)?;
        summary.parent = self.parent;
        summary.t_from = from_millis(self.t_from);
        summary.t_to = from_millis(self.t_to);
        Ok(summary)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ScoredSegment {
    pub summary: SegmentSummary,
    pub score: f64,
}

pub struct SegmentStore {
    table: Table,
    embedder: Arc<dyn Embedder>,
}

impl SegmentStore {
    pub const TABLE: &'static str = "segments";

    pub(crate) async fn open(
        connection: Connection,
        embedder: Arc<dyn Embedder>,
        write_options: Option<WriteOptions>,
    ) -> Self {
        Self::open_named(connection, embedder, write_options, Self::TABLE.to_string()).await
    }

    pub(crate) async fn open_named(
        connection: Connection,
        embedder: Arc<dyn Embedder>,
        write_options: Option<WriteOptions>,
        name: String,
    ) -> Self {
        Self {
            table: Table::open(connection, name, Some("text"), write_options).await,
            embedder,
        }
    }

    /// Writes summaries (replacing by id): a retried compaction overwrites its own segments.
    /// When the embedder fails they are stored without vectors.
    pub async fn put(&self, summaries: &[SegmentSummary]) -> Result<()> {
        if summaries.is_empty() {
            return Ok(());
        }
        let texts: Vec<String> = summaries
            .iter()
            .map(|summary| summary.render(Tz::UTC))
            .collect();
        let vectors = embed_or_wait(self.embedder.as_ref(), &texts).await;
        let model = self.embedder.model_id().to_string();
        let rows = summaries
            .iter()
            .zip(vectors)
            .map(|(summary, vector)| SegmentRow::new(summary, &model, vector))
            .collect::<Result<Vec<_>>>()?;
        let _writes = self.table.write_lock().await;
        self.table.upsert_vectored(schema(), &rows).await
    }

    async fn read(&self, filter: &str) -> Result<Vec<SegmentSummary>> {
        let rows: Vec<SegmentRow> = self.table.all_rows(filter, &COLUMNS).await?;
        let mut summaries = rows
            .into_iter()
            .map(SegmentRow::into_summary)
            .collect::<Result<Vec<_>>>()?;
        summaries.sort_by_key(|summary| (summary.span.from, summary.level, summary.span.to));
        Ok(summaries)
    }

    pub async fn get(&self, id: &str) -> Result<Option<SegmentSummary>> {
        Ok(self
            .read(&format!("id = {}", quote(id)))
            .await?
            .into_iter()
            .next())
    }

    /// Every summary of every level, oldest range first: the jump list.
    pub async fn all(&self) -> Result<Vec<SegmentSummary>> {
        self.read("1 = 1").await
    }

    /// The summaries the prompt shows: those not merged into a higher level.
    pub async fn active(&self) -> Result<Vec<SegmentSummary>> {
        self.read("parent IS NULL").await
    }

    /// A merged summary's children in order.
    pub async fn children(&self, id: &str) -> Result<Vec<SegmentSummary>> {
        self.read(&format!("parent = {}", quote(id))).await
    }

    /// Every summary whose range contains `seq`, lowest level first.
    pub async fn covering(&self, seq: Seq) -> Result<Vec<SegmentSummary>> {
        let seq = seq_column(seq);
        let mut summaries = self
            .read(&format!("seq_from <= {seq} AND seq_to >= {seq}"))
            .await?;
        summaries.sort_by_key(|summary| summary.level);
        Ok(summaries)
    }

    /// The summary with exactly this range, lowest level first.
    pub async fn with_span(&self, span: SeqSpan) -> Result<Option<SegmentSummary>> {
        let mut summaries = self
            .read(&format!(
                "seq_from = {} AND seq_to = {}",
                seq_column(span.from),
                seq_column(span.to)
            ))
            .await?;
        summaries.sort_by_key(|summary| summary.level);
        Ok(summaries.into_iter().next())
    }

    pub async fn set_parent(&self, ids: &[String], parent: Option<&str>) -> Result<()> {
        if ids.is_empty() || !self.table.exists().await? {
            return Ok(());
        }
        let _writes = self.table.write_lock().await;
        self.table
            .update(
                &in_list("id", ids.iter().map(String::as_str)),
                HashMap::from([("parent".to_string(), json!(parent))]),
            )
            .await
    }

    pub async fn delete(&self, ids: &[String]) -> Result<()> {
        if ids.is_empty() {
            return Ok(());
        }
        let _writes = self.table.write_lock().await;
        self.table
            .delete(&in_list("id", ids.iter().map(String::as_str)))
            .await
    }

    /// Hybrid search over every level's text, fused by RRF. `time_range` filters; `time_hint`
    /// only ranks summaries overlapping it higher.
    pub async fn search(
        &self,
        text: &str,
        time_range: Option<TimeRange>,
        time_hint: Option<TimeRange>,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<ScoredSegment>> {
        if text.trim().is_empty() || !self.table.exists().await? {
            return Ok(Vec::new());
        }
        let filter = time_range.map_or_else(|| "1 = 1".to_string(), overlaps);
        let vector = self
            .table
            .query_vector(self.embedder.as_ref(), text)
            .await?;
        let probe = Probe {
            text,
            vector: vector.as_deref(),
            model: self.embedder.model_id(),
            columns: &COLUMNS,
            pool: search_pool(limit),
        };
        let mut rows: HashMap<String, Value> = HashMap::new();
        let mut lists = self.table.rankings(&probe, &filter, &mut rows).await?;
        if let Some(range) = time_hint {
            let within = format!("({filter}) AND {}", overlaps(range));
            lists.extend(self.table.rankings(&probe, &within, &mut rows).await?);
        }
        let mut found = Vec::new();
        for (id, score) in rrf(&lists, RRF_K).into_iter().skip(offset).take(limit) {
            if let Some(row) = rows.remove(&id) {
                found.push(ScoredSegment {
                    summary: serde_json::from_value::<SegmentRow>(row)?.into_summary()?,
                    score,
                });
            }
        }
        Ok(found)
    }

    /// Gives every summary without a vector from the current embedder one; see
    /// [`crate::store::FactStore::reembed`].
    pub async fn reembed(&self) -> Result<usize> {
        self.table.reembed(self.embedder.as_ref(), &COLUMNS).await
    }

    pub(crate) async fn optimize(&self) -> Result<()> {
        self.table.optimize().await
    }

    pub(crate) async fn prune(&self) -> Result<()> {
        self.table.prune().await
    }

    pub(crate) async fn refresh(&self) -> Result<()> {
        self.table.refresh().await
    }

    pub(crate) async fn drop_all(&self) -> Result<()> {
        self.table.drop_table().await
    }
}
