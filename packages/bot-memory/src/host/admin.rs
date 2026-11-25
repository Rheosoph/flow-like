//! What the memory tab does with a bot's facts without running a turn: list them with their
//! evidence, forget one the way `memory_forget` does, reset the whole memory. The stores are
//! opened on the bot's workspace connection with an embedder and a model that refuse to work,
//! because none of these calls embeds or summarises.

use crate::{
    CompactionConfig, Embedder, Entity, EntityKind, Fact, LlmRequest, MemoryEngine, MemoryLlm,
    MemoryStores, NoEntity, WriteGate,
    store::FactFilter,
    tools::{MemoryForgetArgs, WriteOutcome, memory_forget},
};
use async_trait::async_trait;
use chrono::Utc;
use chrono_tz::Tz;
use flow_like_storage::lancedb::{Connection, table::WriteOptions};
use flow_like_types::{Result, anyhow};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;

/// Facts one listing returns at most.
pub const MAX_LISTED_FACTS: usize = 500;
/// Entities one listing returns at most.
pub const MAX_LISTED_ENTITIES: usize = 1_000;

/// Active facts per entity, as subject or object.
async fn fact_counts(stores: &MemoryStores) -> Result<HashMap<String, usize>> {
    let mut counts = HashMap::new();
    for (subject, object) in stores.facts.edges(&FactFilter::default()).await? {
        *counts.entry(subject).or_default() += 1;
        if let Some(object) = object {
            *counts.entry(object).or_default() += 1;
        }
    }
    Ok(counts)
}

async fn entity_view(engine: &MemoryEngine, entity: &Entity) -> Result<EntityView> {
    let stores = engine.stores();
    let counts = fact_counts(stores).await?;
    Ok(EntityView::new(
        entity,
        counts.get(&entity.id).copied().unwrap_or_default(),
        stores.room_scope().map(str::to_owned),
    ))
}

/// One piece of evidence: where in the chat the fact was said.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct EvidenceView {
    pub seq: u64,
    pub message_id: String,
    pub quote: String,
    pub at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub speaker: Option<crate::model::Speaker>,
}

/// A fact as the memory tab shows it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct FactView {
    pub id: String,
    pub text: String,
    pub kind: String,
    pub subject: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subject_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub object_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub relation: Option<String>,
    pub source: String,
    pub confidence: f32,
    pub recorded_at: String,
    pub valid_from: Option<String>,
    pub valid_to: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub geojson: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub time: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<String>,
    pub superseded_by: Option<String>,
    pub evidence: Vec<EvidenceView>,
    /// An opaque room scope. Its positions cannot be opened in the owner's direct chat.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub conversation_scope: Option<String>,
}

/// An entity as the memory tab shows it. Ids are unique within one conversation scope.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct EntityView {
    pub id: String,
    pub name: String,
    pub kind: String,
    /// The bot's owner.
    pub owner: bool,
    /// The participant account a group member's entity stands for.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub participant: Option<String>,
    /// Other names used for it.
    pub aliases: Vec<String>,
    /// Active facts with it as subject or object.
    pub fact_count: usize,
    pub created_at: String,
    pub updated_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub conversation_scope: Option<String>,
}

impl EntityView {
    fn new(entity: &Entity, fact_count: usize, scope: Option<String>) -> Self {
        Self {
            id: entity.id.clone(),
            name: entity.name.clone(),
            kind: entity.kind.as_str().to_string(),
            owner: entity.is_owner(),
            participant: entity.participant().map(str::to_owned),
            aliases: entity
                .other_names()
                .into_iter()
                .map(str::to_owned)
                .collect(),
            fact_count,
            created_at: entity.created_at.to_rfc3339(),
            updated_at: entity.updated_at.to_rfc3339(),
            conversation_scope: scope,
        }
    }
}

/// A rename or a kind change from the memory tab.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct EntityUpdate {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub kind: Option<String>,
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
            subject_id: fact.subject_id,
            object_id: fact.object_id,
            relation: fact.relation,
            confidence: fact.confidence,
            recorded_at: fact.recorded_at.to_rfc3339(),
            valid_from: fact.valid_from.map(|at| at.to_rfc3339()),
            valid_to: fact.valid_to.map(|at| at.to_rfc3339()),
            geojson: fact.geojson,
            time: fact.time.map(|at| at.to_rfc3339()),
            expires_at: fact.expires_at.map(|at| at.to_rfc3339()),
            superseded_by: fact.superseded_by,
            evidence: fact
                .evidence
                .into_iter()
                .map(|evidence| EvidenceView {
                    seq: evidence.seq,
                    message_id: evidence.message_id,
                    quote: evidence.quote,
                    at: evidence.at.to_rfc3339(),
                    speaker: evidence.speaker,
                })
                .collect(),
            conversation_scope: None,
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
        Ok(self
            .listed(include_superseded)
            .await?
            .into_iter()
            .map(|(fact, _)| fact)
            .collect())
    }

    /// Facts for the memory tab, with room evidence distinguished from direct-chat positions.
    pub async fn list_views(&self, include_superseded: bool) -> Result<Vec<FactView>> {
        Ok(self
            .listed(include_superseded)
            .await?
            .into_iter()
            .map(|(fact, scope)| {
                let mut view = FactView::from(fact);
                view.conversation_scope = scope;
                view
            })
            .collect())
    }

    async fn room_engines(&self) -> Result<Vec<MemoryEngine>> {
        Ok(self
            .engine
            .stores()
            .rooms(Arc::new(Offline))
            .await?
            .into_iter()
            .filter(|stores| stores.room_scope() != self.engine.stores().room_scope())
            .map(|stores| Self::over(stores).engine)
            .collect())
    }

    async fn listed(&self, include_superseded: bool) -> Result<Vec<(Fact, Option<String>)>> {
        let filter = FactFilter {
            include_superseded,
            ..FactFilter::default()
        };
        let rooms = self.room_engines().await?;
        let mut facts = Vec::new();
        for engine in std::iter::once(&self.engine).chain(rooms.iter()) {
            let stores = engine.stores();
            stores.refresh().await?;
            let scope = stores.room_scope().map(str::to_owned);
            facts.extend(
                stores
                    .facts
                    .all(&filter)
                    .await?
                    .into_iter()
                    .map(|fact| (fact, scope.clone())),
            );
        }
        facts.sort_by(|a, b| b.0.anchor().cmp(&a.0.anchor()).then(a.0.id.cmp(&b.0.id)));
        facts.truncate(MAX_LISTED_FACTS);
        Ok(facts)
    }

    /// Deletes one fact (a unique id prefix is enough) under the engine's lease, old table
    /// versions included. The fact's evidence stays in the history.
    pub async fn forget(&self, id: &str) -> Result<Fact> {
        let rooms = self.room_engines().await?;
        let engines: Vec<_> = std::iter::once(&self.engine).chain(rooms.iter()).collect();
        let mut found = Vec::new();
        for engine in &engines {
            engine.stores().refresh().await?;
            if let Some(fact) = engine.stores().facts.get(id).await? {
                found.push((*engine, fact));
            }
        }
        if found.is_empty() {
            for engine in engines {
                found.extend(
                    engine
                        .stores()
                        .facts
                        .find_prefix(id)
                        .await?
                        .into_iter()
                        .map(|fact| (engine, fact)),
                );
            }
        }
        let (engine, fact) = match found.len() {
            0 => return Err(anyhow!("No fact {id}")),
            1 => found.remove(0),
            count => return Err(anyhow!("{count} facts match {id}; give a unique full id")),
        };
        let args = MemoryForgetArgs { id: fact.id };
        match memory_forget(engine, &args, WriteGate::default(), Utc::now()).await? {
            WriteOutcome::Done(fact) => Ok(fact),
            WriteOutcome::NeedsConfirmation { .. } | WriteOutcome::Rejected { .. } => {
                Err(anyhow!("Forgetting fact {id} was refused"))
            }
        }
    }

    /// The people, places, organizations, projects and things the facts are about, across the
    /// direct chat and every room: the owner first, then by how much is remembered.
    pub async fn list_entities(&self) -> Result<Vec<EntityView>> {
        let rooms = self.room_engines().await?;
        let mut views = Vec::new();
        for engine in std::iter::once(&self.engine).chain(rooms.iter()) {
            let stores = engine.stores();
            stores.refresh().await?;
            let scope = stores.room_scope().map(str::to_owned);
            let counts = fact_counts(stores).await?;
            views.extend(
                stores
                    .entities
                    .all()
                    .await?
                    .iter()
                    .filter(|entity| entity.is_active())
                    .map(|entity| {
                        EntityView::new(
                            entity,
                            counts.get(&entity.id).copied().unwrap_or_default(),
                            scope.clone(),
                        )
                    }),
            );
        }
        views.sort_by(|a, b| {
            b.owner
                .cmp(&a.owner)
                .then(b.fact_count.cmp(&a.fact_count))
                .then(a.name.to_lowercase().cmp(&b.name.to_lowercase()))
                .then(a.id.cmp(&b.id))
        });
        views.truncate(MAX_LISTED_ENTITIES);
        Ok(views)
    }

    /// Renames an entity or changes its kind; the old name stays one of its names.
    pub async fn update_entity(
        &self,
        scope: Option<&str>,
        id: &str,
        update: &EntityUpdate,
    ) -> Result<EntityView> {
        let kind = update
            .kind
            .as_deref()
            .map(|kind| {
                EntityKind::parse(kind).ok_or_else(|| {
                    anyhow!("'{kind}' is not an entity kind; use person, place, organization, project or thing")
                })
            })
            .transpose()?;
        let engine = self.engine_for(scope).await?;
        let entity = engine
            .update_entity(id, update.name.as_deref(), kind, Utc::now())
            .await?;
        entity_view(&engine, &entity).await
    }

    /// Merges `source` into `into` within one conversation scope.
    pub async fn merge_entities(
        &self,
        scope: Option<&str>,
        source: &str,
        into: &str,
    ) -> Result<EntityView> {
        let engine = self.engine_for(scope).await?;
        let entity = engine.merge_entity(source, into, Utc::now()).await?;
        entity_view(&engine, &entity).await
    }

    /// Forgets an entity with every fact about it; returns the forgotten facts.
    pub async fn forget_entity(&self, scope: Option<&str>, id: &str) -> Result<Vec<FactView>> {
        let engine = self.engine_for(scope).await?;
        Ok(engine
            .forget_entity(id, Utc::now())
            .await?
            .into_iter()
            .map(FactView::from)
            .collect())
    }

    async fn engine_for(&self, scope: Option<&str>) -> Result<MemoryEngine> {
        let Some(scope) = scope.filter(|scope| !scope.is_empty()) else {
            return Ok(self.engine.clone());
        };
        self.room_engines()
            .await?
            .into_iter()
            .find(|engine| engine.stores().room_scope() == Some(scope))
            .ok_or_else(|| {
                NoEntity(format!(
                    "This bot has no memory for conversation scope {scope}"
                ))
                .into()
            })
    }

    /// Clears direct and room memories while keeping their revisions, so prepared drafts
    /// cannot restore the removed content.
    pub async fn reset(&self) -> Result<()> {
        for room in self.room_engines().await? {
            room.reset(Utc::now()).await?;
        }
        self.engine.reset(Utc::now()).await
    }
}
