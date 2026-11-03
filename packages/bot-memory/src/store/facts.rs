//! The fact table: one row per fact with its vector, a BM25 index on the text, and the columns
//! the filters need (subject, kind, validity, supersession, evidence positions).

use std::collections::HashMap;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use flow_like_storage::arrow_schema::{DataType, Field, Schema};
use flow_like_storage::lancedb::Connection;
use flow_like_storage::lancedb::table::WriteOptions;
use flow_like_types::Result;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::lance::{
    Probe, Table, embed_or_wait, from_millis, in_list, int_field, millis, quote, search_pool,
    similarity, text_field,
};
use crate::facts::{Fact, FactKind, Neighbour};
use crate::llm::{Embedder, embed_checked};
use crate::model::{FactSource, Seq};
use crate::retrieval::{RRF_K, Recency, TimeRange, recency_factor, rrf};

/// Every column but the vector.
const COLUMNS: [&str; 19] = [
    "id",
    "text",
    "kind",
    "subject",
    "valid_from",
    "valid_to",
    "recorded_at",
    "expired_at",
    "observed_at",
    "anchor_at",
    "end_at",
    "evidence",
    "evidence_seqs",
    "segment_id",
    "source",
    "confidence",
    "superseded_by",
    "content_hash",
    "embedding_model",
];

/// `end_at` of a fact that holds until further notice.
const OPEN_END: i64 = i64::MAX;

fn columns() -> &'static [&'static str] {
    &COLUMNS
}

/// The table's columns; the vector column is added with the first vector.
fn schema() -> Schema {
    Schema::new(vec![
        text_field("id", false),
        text_field("text", false),
        text_field("kind", false),
        text_field("subject", false),
        int_field("valid_from", true),
        int_field("valid_to", true),
        int_field("recorded_at", false),
        int_field("expired_at", true),
        int_field("observed_at", false),
        int_field("anchor_at", false),
        int_field("end_at", false),
        text_field("evidence", false),
        text_field("evidence_seqs", false),
        text_field("segment_id", true),
        text_field("source", false),
        Field::new("confidence", DataType::Float64, false),
        text_field("superseded_by", true),
        text_field("content_hash", false),
        text_field("embedding_model", false),
    ])
}

/// `,4812,4813,`: matched with `LIKE '%,4812,%'`.
fn seq_list(seqs: &[Seq]) -> String {
    let joined: Vec<String> = seqs.iter().map(ToString::to_string).collect();
    format!(",{},", joined.join(","))
}

/// The end of the time a fact covers: its `valid_to`, the moment of an event, or open for
/// anything else that has no end yet.
fn end_at(fact: &Fact, anchor: DateTime<Utc>) -> i64 {
    match fact.valid_to {
        Some(end) => millis(end.max(anchor)),
        None if fact.kind == FactKind::Event => millis(anchor),
        None => OPEN_END,
    }
}

/// Facts whose time overlaps `range`.
fn overlaps(range: TimeRange) -> String {
    format!(
        "anchor_at < {} AND end_at >= {}",
        millis(range.to),
        millis(range.from)
    )
}

/// Facts dated to `range`: they started, or were said, within it.
fn dated_within(range: TimeRange) -> String {
    format!(
        "anchor_at >= {} AND anchor_at < {}",
        millis(range.from),
        millis(range.to)
    )
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct FactRow {
    id: String,
    text: String,
    kind: String,
    subject: String,
    valid_from: Option<i64>,
    valid_to: Option<i64>,
    recorded_at: i64,
    expired_at: Option<i64>,
    observed_at: i64,
    anchor_at: i64,
    end_at: i64,
    evidence: String,
    evidence_seqs: String,
    segment_id: Option<String>,
    source: String,
    confidence: f64,
    superseded_by: Option<String>,
    content_hash: String,
    embedding_model: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    vector: Option<Vec<f32>>,
}

impl FactRow {
    fn new(fact: &Fact, model: &str, vector: Option<Vec<f32>>) -> Result<Self> {
        let anchor = fact.anchor();
        Ok(Self {
            id: fact.id.clone(),
            text: fact.text.clone(),
            kind: fact.kind.as_str().to_string(),
            subject: fact.subject.clone(),
            valid_from: fact.valid_from.map(millis),
            valid_to: fact.valid_to.map(millis),
            recorded_at: millis(fact.recorded_at),
            expired_at: fact.expired_at.map(millis),
            observed_at: millis(fact.observed_at()),
            anchor_at: millis(anchor),
            end_at: end_at(fact, anchor),
            evidence: serde_json::to_string(&fact.evidence)?,
            evidence_seqs: seq_list(&fact.evidence_seqs()),
            segment_id: fact.segment_id.clone(),
            source: fact.source.as_str().to_string(),
            confidence: f64::from(fact.confidence),
            superseded_by: fact.superseded_by.clone(),
            content_hash: fact.content_hash.clone(),
            embedding_model: if vector.is_some() {
                model.to_string()
            } else {
                String::new()
            },
            vector,
        })
    }

    fn into_fact(self) -> Result<Fact> {
        Ok(Fact {
            id: self.id,
            text: self.text,
            kind: FactKind::parse(&self.kind).unwrap_or(FactKind::Knowledge),
            subject: self.subject,
            valid_from: self.valid_from.map(from_millis),
            valid_to: self.valid_to.map(from_millis),
            recorded_at: from_millis(self.recorded_at),
            expired_at: self.expired_at.map(from_millis),
            evidence: serde_json::from_str(&self.evidence)?,
            segment_id: self.segment_id,
            source: FactSource::parse(&self.source).unwrap_or(FactSource::ToolUntrusted),
            confidence: self.confidence as f32,
            superseded_by: self.superseded_by,
            content_hash: self.content_hash,
        })
    }

    /// The columns a metadata update rewrites: everything but the text, its hash and vector.
    fn metadata(self) -> HashMap<String, Value> {
        HashMap::from([
            ("valid_from".to_string(), json!(self.valid_from)),
            ("valid_to".to_string(), json!(self.valid_to)),
            ("expired_at".to_string(), json!(self.expired_at)),
            ("observed_at".to_string(), json!(self.observed_at)),
            ("anchor_at".to_string(), json!(self.anchor_at)),
            ("end_at".to_string(), json!(self.end_at)),
            ("evidence".to_string(), json!(self.evidence)),
            ("evidence_seqs".to_string(), json!(self.evidence_seqs)),
            ("segment_id".to_string(), json!(self.segment_id)),
            ("source".to_string(), json!(self.source)),
            ("confidence".to_string(), json!(self.confidence)),
            ("superseded_by".to_string(), json!(self.superseded_by)),
        ])
    }
}

fn to_facts(rows: Vec<FactRow>) -> Result<Vec<Fact>> {
    rows.into_iter().map(FactRow::into_fact).collect()
}

/// Which facts a read covers.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct FactFilter {
    /// Superseded facts are hidden unless asked for (questions about how things were).
    pub include_superseded: bool,
    pub kinds: Vec<FactKind>,
    pub subject: Option<String>,
    /// Facts whose time overlaps the range: from their start (or, without one, when they were
    /// said) to their end; events last a moment, other facts until they end.
    pub time_range: Option<TimeRange>,
}

impl FactFilter {
    fn sql(&self) -> String {
        let mut clauses: Vec<String> = Vec::new();
        if !self.include_superseded {
            clauses.push("superseded_by IS NULL AND expired_at IS NULL".to_string());
        }
        if !self.kinds.is_empty() {
            clauses.push(in_list("kind", self.kinds.iter().map(|kind| kind.as_str())));
        }
        if let Some(subject) = &self.subject {
            clauses.push(format!("subject = {}", quote(subject)));
        }
        if let Some(range) = self.time_range {
            clauses.push(overlaps(range));
        }
        if clauses.is_empty() {
            "1 = 1".to_string()
        } else {
            clauses.join(" AND ")
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct FactQuery {
    pub text: String,
    pub filter: FactFilter,
    /// A time the query mentions: facts dated to it rank higher, the others (a preference held
    /// for years, say) stay eligible.
    pub time_hint: Option<TimeRange>,
    pub limit: usize,
    pub offset: usize,
    pub now: DateTime<Utc>,
    pub recency: Recency,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ScoredFact {
    pub fact: Fact,
    pub score: f64,
}

pub struct FactStore {
    table: Table,
    embedder: Arc<dyn Embedder>,
}

impl FactStore {
    pub const TABLE: &'static str = "facts";

    pub(crate) async fn open(
        connection: Connection,
        embedder: Arc<dyn Embedder>,
        write_options: Option<WriteOptions>,
    ) -> Self {
        Self {
            table: Table::open(connection, Self::TABLE, Some("text"), write_options).await,
            embedder,
        }
    }

    pub fn embedder(&self) -> &Arc<dyn Embedder> {
        &self.embedder
    }

    /// Document vectors for `texts`, checked for count and dimension.
    pub async fn embed_documents(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        embed_checked(self.embedder.as_ref(), texts).await
    }

    /// Whether every fact has a vector from the current embedder. Facts written while the
    /// embedder failed or after a model change get one from [`FactStore::reembed`]; until
    /// then BM25 finds them.
    pub async fn vectors_current(&self) -> Result<bool> {
        Ok(self.unembedded().await? == 0)
    }

    /// Facts without a vector from the current embedder.
    pub async fn unembedded(&self) -> Result<usize> {
        self.table.unembedded(self.embedder.model_id()).await
    }

    /// Inserts facts (or replaces them by id); when the embedder fails they are stored without
    /// vectors.
    pub async fn insert(&self, facts: &[Fact]) -> Result<()> {
        let texts: Vec<String> = facts.iter().map(|fact| fact.text.clone()).collect();
        if texts.is_empty() {
            return Ok(());
        }
        let vectors = embed_or_wait(self.embedder.as_ref(), &texts).await;
        self.insert_embedded(facts.iter().cloned().zip(vectors).collect())
            .await
    }

    /// Inserts (or replaces by id) facts with their vectors. A fact without one, or with one
    /// that does not fit the table (another model), waits for [`FactStore::reembed`].
    pub async fn insert_embedded(&self, facts: Vec<(Fact, Option<Vec<f32>>)>) -> Result<()> {
        let model = self.embedder.model_id().to_string();
        let rows = facts
            .into_iter()
            .map(|(fact, vector)| FactRow::new(&fact, &model, vector))
            .collect::<Result<Vec<_>>>()?;
        let _writes = self.table.write_lock().await;
        self.table.upsert_vectored(schema(), &rows).await
    }

    /// Rewrites evidence, validity, supersession, trust and confidence; the text and its vector
    /// stay.
    pub async fn update_metadata(&self, fact: &Fact) -> Result<()> {
        let row = FactRow::new(fact, self.embedder.model_id(), None)?;
        let _writes = self.table.write_lock().await;
        self.table
            .update(&format!("id = {}", quote(&fact.id)), row.metadata())
            .await
    }

    async fn read(&self, filter: &str) -> Result<Vec<Fact>> {
        to_facts(self.table.all_rows(filter, columns()).await?)
    }

    pub async fn get(&self, id: &str) -> Result<Option<Fact>> {
        Ok(self
            .read(&format!("id = {}", quote(id)))
            .await?
            .into_iter()
            .next())
    }

    /// Facts whose id starts with `prefix`, so the model can name one by a short id.
    pub async fn find_prefix(&self, prefix: &str) -> Result<Vec<Fact>> {
        let prefix: String = prefix
            .chars()
            .filter(|c| c.is_ascii_alphanumeric() || *c == '_')
            .collect();
        if prefix.len() < 4 {
            return Ok(Vec::new());
        }
        let mut facts = self.read(&format!("id LIKE '{prefix}%'")).await?;
        facts.retain(|fact| fact.id.starts_with(&prefix));
        Ok(facts)
    }

    /// Every fact the filter covers, newest anchor first.
    pub async fn all(&self, filter: &FactFilter) -> Result<Vec<Fact>> {
        let mut facts = self.read(&filter.sql()).await?;
        facts.sort_by(|a, b| b.anchor().cmp(&a.anchor()).then(a.id.cmp(&b.id)));
        Ok(facts)
    }

    pub async fn count(&self, filter: &FactFilter) -> Result<usize> {
        self.table.count(Some(filter.sql())).await
    }

    /// Facts, superseded ones included, citing any of `seqs`.
    pub async fn with_evidence(&self, seqs: &[Seq]) -> Result<Vec<Fact>> {
        let mut facts: Vec<Fact> = Vec::new();
        for chunk in seqs.chunks(50) {
            let clauses: Vec<String> = chunk
                .iter()
                .map(|seq| format!("evidence_seqs LIKE '%,{seq},%'"))
                .collect();
            for fact in self.read(&clauses.join(" OR ")).await? {
                if !facts.iter().any(|known| known.id == fact.id) {
                    facts.push(fact);
                }
            }
        }
        Ok(facts)
    }

    pub async fn superseded_by(&self, id: &str) -> Result<Vec<Fact>> {
        self.read(&format!("superseded_by = {}", quote(id))).await
    }

    /// A closed fact saying the same as `candidate` when the candidate was said while that
    /// fact still held: the same old statement found again (a retried or re-mined span).
    pub async fn closed_duplicate(&self, candidate: &Fact) -> Result<Option<Fact>> {
        let closed = self
            .read(&format!(
                "subject = {} AND content_hash = {} AND (superseded_by IS NOT NULL OR expired_at IS NOT NULL)",
                quote(&candidate.subject),
                quote(&candidate.content_hash)
            ))
            .await?;
        Ok(closed
            .into_iter()
            .find(|fact| candidate.observed_at() <= fact.expired_at.unwrap_or(fact.recorded_at)))
    }

    /// Hard delete: the rows and their vectors.
    pub async fn delete(&self, ids: &[String]) -> Result<()> {
        let _writes = self.table.write_lock().await;
        for chunk in ids.chunks(100) {
            self.table
                .delete(&in_list("id", chunk.iter().map(String::as_str)))
                .await?;
        }
        Ok(())
    }

    /// The nearest active facts on the candidate's subject: exact content matches first, then
    /// by vector, then by BM25 (which also finds facts still without a vector).
    pub async fn neighbours(
        &self,
        candidate: &Fact,
        vector: Option<&[f32]>,
        limit: usize,
    ) -> Result<Vec<Neighbour>> {
        if !self.table.exists().await? {
            return Ok(Vec::new());
        }
        let scope = FactFilter {
            subject: Some(candidate.subject.clone()),
            ..FactFilter::default()
        }
        .sql();
        let mut found: Vec<Neighbour> = Vec::new();
        let same: Vec<FactRow> = self
            .table
            .rows(
                &format!(
                    "{scope} AND content_hash = {}",
                    quote(&candidate.content_hash)
                ),
                columns(),
                limit,
                0,
            )
            .await?;
        for row in same {
            found.push(Neighbour {
                fact: row.into_fact()?,
                similarity: 1.0,
            });
        }
        let model = self.embedder.model_id();
        let mut hits = Vec::new();
        if let Some(vector) = vector
            && self.table.vectors_ready(model, vector.len()).await?
        {
            let embedded = format!("{scope} AND embedding_model = {}", quote(model));
            hits.extend(
                self.table
                    .nearest(vector, Some(&embedded), columns(), limit)
                    .await?,
            );
        }
        hits.extend(
            self.table
                .full_text(&candidate.text, Some(&scope), columns(), limit, 0)
                .await?,
        );
        for hit in hits {
            let similarity = similarity(&hit);
            let fact = serde_json::from_value::<FactRow>(hit)?.into_fact()?;
            if fact.id != candidate.id && !found.iter().any(|known| known.fact.id == fact.id) {
                found.push(Neighbour { fact, similarity });
            }
        }
        found.truncate(limit);
        Ok(found)
    }

    /// Hybrid search: vector and BM25 hits fused by RRF (k = 60), events weighted by recency.
    /// A time hint adds the rankings of the facts dated to that time, so they rank higher while
    /// lasting facts stay eligible. An empty query lists the newest facts.
    pub async fn search(&self, query: &FactQuery) -> Result<Vec<ScoredFact>> {
        if !self.table.exists().await? {
            return Ok(Vec::new());
        }
        if query.text.trim().is_empty() {
            return Ok(self
                .all(&query.filter)
                .await?
                .into_iter()
                .skip(query.offset)
                .take(query.limit)
                .map(|fact| ScoredFact { fact, score: 0.0 })
                .collect());
        }
        let vector = self
            .table
            .query_vector(self.embedder.as_ref(), &query.text)
            .await?;
        let probe = Probe {
            text: &query.text,
            vector: vector.as_deref(),
            model: self.embedder.model_id(),
            columns: columns(),
            pool: search_pool(query.limit),
        };
        let filter = query.filter.sql();
        let mut rows: HashMap<String, Value> = HashMap::new();
        let mut lists = self.table.rankings(&probe, &filter, &mut rows).await?;
        if let Some(range) = query.time_hint {
            let within = format!("({filter}) AND {}", dated_within(range));
            lists.extend(self.table.rankings(&probe, &within, &mut rows).await?);
        }

        let mut scored = Vec::new();
        for (id, score) in rrf(&lists, RRF_K) {
            let Some(row) = rows.remove(&id) else {
                continue;
            };
            let fact = serde_json::from_value::<FactRow>(row)?.into_fact()?;
            let weight = if fact.kind.decays() {
                recency_factor(query.now - fact.anchor(), query.recency)
            } else {
                1.0
            };
            scored.push(ScoredFact {
                fact,
                score: score * weight,
            });
        }
        scored.sort_by(|a, b| b.score.total_cmp(&a.score).then(a.fact.id.cmp(&b.fact.id)));
        Ok(scored
            .into_iter()
            .skip(query.offset)
            .take(query.limit)
            .collect())
    }

    /// Gives every fact without a vector from the current embedder one (after a model change or
    /// while the embedder was failing). The rows stay where they are: a failure half way loses
    /// nothing, and the next run continues. Idle work, never on the turn path.
    pub async fn reembed(&self) -> Result<usize> {
        self.table.reembed(self.embedder.as_ref(), columns()).await
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
