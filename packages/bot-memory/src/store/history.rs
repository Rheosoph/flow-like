//! The history index: one BM25-searchable row per message projection (text, tool gists and UI
//! projections). The inbox stays the history; this only finds positions in it.

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use flow_like_storage::arrow_schema::Schema;
use flow_like_storage::lancedb::Connection;
use flow_like_storage::lancedb::table::WriteOptions;
use flow_like_types::Result;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::lance::{
    Table, from_millis, in_list, int_field, millis, quote, ranked_ids, search_pool, text_field,
};
use crate::model::{HistoryItem, Role, Seq};
use crate::retrieval::{RRF_K, TimeRange, rrf};
use crate::text::one_line;

const COLUMNS: [&str; 6] = ["id", "seq", "message_id", "role", "created_at", "text"];

fn schema() -> Schema {
    Schema::new(vec![
        text_field("id", false),
        int_field("seq", false),
        text_field("message_id", false),
        text_field("role", false),
        int_field("created_at", false),
        text_field("text", false),
    ])
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct HistoryRow {
    id: String,
    seq: i64,
    message_id: String,
    role: String,
    created_at: i64,
    text: String,
}

/// What `history_search` matches: the text, then each tool's name and gist and each UI
/// block's projection.
pub fn projection(item: &HistoryItem) -> String {
    let mut parts = Vec::new();
    if let Some(speaker) = &item.speaker {
        parts.push(format!("speaker {}", speaker.label()));
    }
    parts.push(one_line(&item.text));
    for tool in &item.tools {
        let gist = tool
            .result
            .as_ref()
            .and_then(|result| result.gist.as_deref())
            .unwrap_or_default();
        parts.push(one_line(&format!("{} {gist}", tool.name)));
    }
    parts.extend(item.ui.iter().map(|ui| one_line(&ui.projection)));
    parts.retain(|part| !part.is_empty());
    parts.join(" · ")
}

#[derive(Debug, Clone, PartialEq)]
pub struct HistoryQuery {
    pub text: String,
    pub role: Option<Role>,
    /// Only messages written within the range.
    pub time_range: Option<TimeRange>,
    /// A time the query mentions: messages written then rank higher, the others stay.
    pub time_hint: Option<TimeRange>,
    pub limit: usize,
    pub offset: usize,
}

fn written_within(range: TimeRange) -> String {
    format!(
        "created_at >= {} AND created_at < {}",
        millis(range.from),
        millis(range.to)
    )
}

fn to_hit(hit: Value, score: f64) -> Result<HistoryHit> {
    let row: HistoryRow = serde_json::from_value(hit)?;
    Ok(HistoryHit {
        seq: u64::try_from(row.seq).unwrap_or_default(),
        message_id: row.message_id,
        role: Role::parse(&row.role).unwrap_or(Role::External),
        created_at: from_millis(row.created_at),
        text: row.text,
        score,
    })
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HistoryHit {
    pub seq: Seq,
    pub message_id: String,
    pub role: Role,
    pub created_at: DateTime<Utc>,
    pub text: String,
    pub score: f64,
}

pub struct HistoryIndex {
    table: Table,
}

impl HistoryIndex {
    pub const TABLE: &'static str = "history";

    pub(crate) async fn open(connection: Connection, write_options: Option<WriteOptions>) -> Self {
        Self::open_named(connection, write_options, Self::TABLE.to_string()).await
    }

    pub(crate) async fn open_named(
        connection: Connection,
        write_options: Option<WriteOptions>,
        name: String,
    ) -> Self {
        Self {
            table: Table::open(connection, name, Some("text"), write_options).await,
        }
    }

    /// Adds or refreshes messages (an edit re-indexes); tombstones leave the index.
    pub async fn index(&self, items: &[HistoryItem]) -> Result<()> {
        let deleted: Vec<Seq> = items
            .iter()
            .filter(|item| item.deleted)
            .map(|item| item.seq)
            .collect();
        self.remove(&deleted).await?;
        let rows: Vec<HistoryRow> = items
            .iter()
            .filter(|item| !item.deleted)
            .map(|item| HistoryRow {
                id: item.seq.to_string(),
                seq: i64::try_from(item.seq).unwrap_or(i64::MAX),
                message_id: item.message_id.clone(),
                role: item.role.label().to_string(),
                created_at: millis(item.created_at),
                text: projection(item),
            })
            .collect();
        if rows.is_empty() {
            return Ok(());
        }
        self.table.create(schema()).await?;
        self.table.upsert(&rows).await
    }

    pub async fn remove(&self, seqs: &[Seq]) -> Result<()> {
        for chunk in seqs.chunks(200) {
            let ids: Vec<String> = chunk.iter().map(ToString::to_string).collect();
            self.table
                .delete(&in_list("id", ids.iter().map(String::as_str)))
                .await?;
        }
        Ok(())
    }

    pub async fn count(&self) -> Result<usize> {
        self.table.count(None).await
    }

    /// BM25 over the projections, best first. A time hint adds the ranking of the messages
    /// written then, fused by RRF, so they come first without hiding the others.
    pub async fn search(&self, query: &HistoryQuery) -> Result<Vec<HistoryHit>> {
        let mut clauses = Vec::new();
        if let Some(role) = query.role {
            clauses.push(format!("role = {}", quote(role.label())));
        }
        if let Some(range) = query.time_range {
            clauses.push(written_within(range));
        }
        let filter = (!clauses.is_empty()).then(|| clauses.join(" AND "));
        let Some(hint) = query.time_hint else {
            let hits = self
                .table
                .full_text(
                    &query.text,
                    filter.as_deref(),
                    &COLUMNS,
                    query.limit,
                    query.offset,
                )
                .await?;
            return hits
                .into_iter()
                .map(|hit| {
                    let score = hit.get("_score").and_then(Value::as_f64).unwrap_or(0.0);
                    to_hit(hit, score)
                })
                .collect();
        };
        let pool = search_pool(query.limit);
        let within = filter.as_deref().map_or_else(
            || written_within(hint),
            |filter| format!("({filter}) AND {}", written_within(hint)),
        );
        let mut rows: HashMap<String, Value> = HashMap::new();
        let mut lists = Vec::new();
        for scope in [filter.as_deref(), Some(within.as_str())] {
            let hits = self
                .table
                .full_text(&query.text, scope, &COLUMNS, pool, 0)
                .await?;
            lists.push(ranked_ids(hits, &mut rows));
        }
        rrf(&lists, RRF_K)
            .into_iter()
            .skip(query.offset)
            .take(query.limit)
            .filter_map(|(id, score)| rows.remove(&id).map(|hit| to_hit(hit, score)))
            .collect()
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
