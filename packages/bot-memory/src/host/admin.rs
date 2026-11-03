//! What the memory tab does with a bot's facts without running a turn: list them with their
//! evidence, forget one the way `memory_forget` does, reset the whole memory. The stores are
//! opened on the bot's workspace connection with an embedder and a model that refuse to work,
//! because none of these calls embeds or summarises.

use crate::{
    CompactionConfig, Embedder, Fact, LlmRequest, MemoryEngine, MemoryLlm, MemoryStores, WriteGate,
    store::FactFilter,
    tools::{MemoryForgetArgs, WriteOutcome, memory_forget},
};
use async_trait::async_trait;
use chrono::Utc;
use chrono_tz::Tz;
use flow_like_storage::lancedb::{Connection, table::WriteOptions};
use flow_like_types::{Result, anyhow};
use serde::Serialize;
use std::sync::Arc;

/// Facts one listing returns at most.
pub const MAX_LISTED_FACTS: usize = 500;

/// One piece of evidence: where in the chat the fact was said.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct EvidenceView {
    pub seq: u64,
    pub message_id: String,
    pub quote: String,
    pub at: String,
}

/// A fact as the memory tab shows it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct FactView {
    pub id: String,
    pub text: String,
    pub kind: String,
    pub subject: String,
    pub source: String,
    pub confidence: f32,
    pub recorded_at: String,
    pub valid_from: Option<String>,
    pub valid_to: Option<String>,
    pub superseded_by: Option<String>,
    pub evidence: Vec<EvidenceView>,
}

fn wire<T: Serialize>(value: &T) -> String {
    serde_json::to_value(value)
        .ok()
        .and_then(|value| value.as_str().map(str::to_owned))
        .unwrap_or_default()
}

impl From<Fact> for FactView {
    fn from(fact: Fact) -> Self {
        Self {
            kind: wire(&fact.kind),
            source: wire(&fact.source),
            id: fact.id,
            text: fact.text,
            subject: fact.subject,
            confidence: fact.confidence,
            recorded_at: fact.recorded_at.to_rfc3339(),
            valid_from: fact.valid_from.map(|at| at.to_rfc3339()),
            valid_to: fact.valid_to.map(|at| at.to_rfc3339()),
            superseded_by: fact.superseded_by,
            evidence: fact
                .evidence
                .into_iter()
                .map(|evidence| EvidenceView {
                    seq: evidence.seq,
                    message_id: evidence.message_id,
                    quote: evidence.quote,
                    at: evidence.at.to_rfc3339(),
                })
                .collect(),
        }
    }
}

struct Offline;

#[async_trait]
impl MemoryLlm for Offline {
    async fn complete(&self, _request: LlmRequest) -> Result<String> {
        Err(anyhow!("Managing facts runs no model"))
    }
}

#[async_trait]
impl Embedder for Offline {
    fn model_id(&self) -> &str {
        ""
    }

    async fn embed_documents(&self, _texts: &[String]) -> Result<Vec<Vec<f32>>> {
        Err(anyhow!("Managing facts embeds nothing"))
    }

    async fn embed_query(&self, _text: &str) -> Result<Vec<f32>> {
        Err(anyhow!("Managing facts embeds nothing"))
    }
}

pub struct FactAdmin {
    engine: MemoryEngine,
}

impl FactAdmin {
    pub async fn open(connection: Connection, write_options: Option<WriteOptions>) -> Self {
        let stores = MemoryStores::open(connection, Arc::new(Offline), write_options).await;
        Self::over(stores)
    }

    pub fn over(stores: MemoryStores) -> Self {
        Self {
            engine: MemoryEngine::new(
                stores,
                Arc::new(Offline),
                CompactionConfig::default(),
                Tz::UTC,
            ),
        }
    }

    /// The facts the bot holds, newest first; with `include_superseded` the replaced ones too.
    pub async fn list(&self, include_superseded: bool) -> Result<Vec<Fact>> {
        let stores = self.engine.stores();
        stores.refresh().await?;
        let filter = FactFilter {
            include_superseded,
            ..FactFilter::default()
        };
        let mut facts = stores.facts.all(&filter).await?;
        facts.truncate(MAX_LISTED_FACTS);
        Ok(facts)
    }

    /// Deletes one fact (a unique id prefix is enough) under the engine's lease, old table
    /// versions included. The fact's evidence stays in the history.
    pub async fn forget(&self, id: &str) -> Result<Fact> {
        let args = MemoryForgetArgs { id: id.to_owned() };
        match memory_forget(&self.engine, &args, WriteGate::default(), Utc::now()).await? {
            WriteOutcome::Done(fact) => Ok(fact),
            WriteOutcome::NeedsConfirmation { .. } | WriteOutcome::Rejected { .. } => {
                Err(anyhow!("Forgetting fact {id} was refused"))
            }
        }
    }

    /// Drops every table of the memory.
    pub async fn reset(&self) -> Result<()> {
        self.engine.stores().drop_all().await
    }
}
