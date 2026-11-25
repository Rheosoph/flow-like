//! Entity upkeep and the owner's entity edits. Maintenance gives facts stored before entities
//! existed their subject entity and drops entities no fact uses any more. Renames, merges and
//! forgetting an entity run under the lease like every other memory write.

use std::collections::{BTreeMap, BTreeSet};

use super::*;
use crate::entities::{Entity, EntityKind, OWNER_ENTITY, Resolver, alias_key};
use crate::tools::forget_facts;

/// The entity or the conversation scope a request names does not exist.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NoEntity(pub String);

impl std::fmt::Display for NoEntity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for NoEntity {}

/// What one maintenance run changes about entities, prepared without the lease.
#[derive(Default)]
pub(super) struct EntityUpkeep {
    writes: Vec<Entity>,
    /// Subject strings of old facts and the entity each now belongs to.
    assignments: BTreeMap<String, String>,
    orphans: Vec<String>,
}

impl EntityUpkeep {
    pub(super) async fn apply(self, stores: &MemoryStores) -> Result<()> {
        stores.entities.upsert(&self.writes).await?;
        for (subject, entity) in &self.assignments {
            stores.facts.assign_subject(subject, entity).await?;
        }
        stores.entities.delete(&self.orphans).await
    }
}

impl MemoryEngine {
    pub(super) async fn prepare_entity_upkeep(&self, now: DateTime<Utc>) -> Result<EntityUpkeep> {
        let stores = &self.stores;
        let mut resolver = Resolver::new(stores.entities.all().await?, now);
        let mut upkeep = EntityUpkeep::default();
        let pending = stores.facts.without_subject_entity(BACKFILL_BATCH).await?;
        let backfill_done = pending.len() < BACKFILL_BATCH;
        for fact in pending {
            if upkeep.assignments.contains_key(&fact.subject) {
                continue;
            }
            if let Some(entity) = resolver.subject(&fact.subject, &fact) {
                upkeep.assignments.insert(fact.subject.clone(), entity);
            }
        }
        upkeep.writes = resolver.writes();
        let used: BTreeSet<String> = stores
            .facts
            .edges(&FactFilter {
                include_superseded: true,
                ..FactFilter::default()
            })
            .await?
            .into_iter()
            .flat_map(|(subject, object)| std::iter::once(subject).chain(object))
            .chain(upkeep.assignments.values().cloned())
            .collect();
        if backfill_done {
            upkeep.orphans = stores
                .entities
                .all()
                .await?
                .into_iter()
                .filter(|entity| !entity.is_owner() && !used.contains(&entity.id))
                .filter(|entity| !upkeep.writes.iter().any(|written| written.id == entity.id))
                .map(|entity| entity.id)
                .collect();
        }
        Ok(upkeep)
    }

    /// Renames an entity or changes its kind; the old name stays an alias.
    pub async fn update_entity(
        &self,
        id: &str,
        name: Option<&str>,
        kind: Option<EntityKind>,
        now: DateTime<Utc>,
    ) -> Result<Entity> {
        let name = name
            .map(|name| crate::text::truncate_chars(&crate::text::one_line(name), 80))
            .filter(|name| !name.is_empty());
        let id = id.to_owned();
        self.entity_commit(now, false, move |stores| {
            let (id, name) = (id.clone(), name.clone());
            async move {
                let mut entity = live_entity(&stores, &id).await?;
                if entity.is_owner() && (name.is_some() || kind.is_some()) {
                    flow_like_types::bail!("The owner's entity keeps its name and kind");
                }
                if let Some(name) = name {
                    if let Some(key) = alias_key(&name)
                        && !entity.aliases.contains(&key)
                    {
                        entity.aliases.push(key);
                    }
                    entity.name = name;
                    entity.named = true;
                }
                if let Some(kind) = kind {
                    entity.kind = kind;
                }
                entity.updated_at = now;
                stores
                    .entities
                    .upsert(std::slice::from_ref(&entity))
                    .await?;
                Ok(entity)
            }
        })
        .await
    }

    /// Merges `source` into `into`: its names join `into` and every fact about it moves there.
    /// The source row stays, pointing at `into`.
    pub async fn merge_entity(
        &self,
        source: &str,
        into: &str,
        now: DateTime<Utc>,
    ) -> Result<Entity> {
        let (source, into) = (source.to_owned(), into.to_owned());
        self.entity_commit(now, false, move |stores| {
            let (source, into) = (source.clone(), into.clone());
            async move {
                let mut from = live_entity(&stores, &source).await?;
                let mut target = live_entity(&stores, &into).await?;
                if from.id == target.id {
                    flow_like_types::bail!("Entity {source} is already {into}");
                }
                if from.is_owner() {
                    flow_like_types::bail!(
                        "The owner's entity can only take others in, not merge away"
                    );
                }
                for alias in &from.aliases {
                    if target.aliases.len() >= crate::entities::MAX_ALIASES {
                        break;
                    }
                    if !target.aliases.contains(alias)
                        && !(target.is_owner() && alias.starts_with("user."))
                    {
                        target.aliases.push(alias.clone());
                    }
                }
                if target.kind == EntityKind::Thing && !target.is_owner() {
                    target.kind = from.kind;
                }
                target.updated_at = now;
                from.merged_into = Some(target.id.clone());
                from.updated_at = now;
                stores.facts.reassign_entity(&from.id, &target.id).await?;
                stores.entities.upsert(&[from, target.clone()]).await?;
                Ok(target)
            }
        })
        .await
    }

    /// Forgets an entity and every fact with it as subject or object, superseded ones
    /// included. The history keeps what was said.
    pub async fn forget_entity(&self, id: &str, now: DateTime<Utc>) -> Result<Vec<Fact>> {
        let id = id.to_owned();
        self.entity_commit(now, true, move |stores| {
            let id = id.clone();
            async move {
                let entity = live_entity(&stores, &id).await?;
                if entity.id == OWNER_ENTITY {
                    flow_like_types::bail!(
                        "Everything about the owner is most of the memory; reset the memory instead"
                    );
                }
                let facts = stores
                    .facts
                    .all(&FactFilter {
                        include_superseded: true,
                        entity: Some(entity.id.clone()),
                        ..FactFilter::default()
                    })
                    .await?;
                forget_facts(&stores, &facts, false).await?;
                let mut gone = vec![entity.id.clone()];
                gone.extend(
                    stores
                        .entities
                        .all()
                        .await?
                        .into_iter()
                        .filter(|merged| merged.merged_into.as_deref() == Some(entity.id.as_str()))
                        .map(|merged| merged.id),
                );
                stores.entities.delete(&gone).await?;
                Ok(facts)
            }
        })
        .await
    }

    async fn entity_commit<T, F, Fut>(
        &self,
        now: DateTime<Utc>,
        deletion: bool,
        apply: F,
    ) -> Result<T>
    where
        T: Send + 'static,
        F: Fn(Arc<MemoryStores>) -> Fut + Clone + Send + 'static,
        Fut: Future<Output = Result<T>> + Send + 'static,
    {
        let started = Instant::now();
        for attempt in 0..CLAIM_ATTEMPTS {
            let revision = self.snapshot().await?.revision;
            let commit_now = now + Duration::from_std(started.elapsed())?;
            let apply = apply.clone();
            let work = move |stores: Arc<MemoryStores>, state: CompactionState| async move {
                Ok((apply(stores).await?, state))
            };
            let outcome = if deletion {
                self.commit_deletion(revision, commit_now, work).await?
            } else {
                self.commit(revision, commit_now, work).await?
            };
            match outcome {
                CommitOutcome::Applied(value, _) => return Ok(value),
                CommitOutcome::Stale(state) if attempt + 1 == CLAIM_ATTEMPTS => {
                    return Err(StateConflict {
                        expected: revision,
                        found: state.revision,
                    }
                    .into());
                }
                CommitOutcome::Stale(_) => {}
            }
        }
        unreachable!("the last attempt returns its conflict")
    }
}

/// The live entity an id names, following merges.
async fn live_entity(stores: &MemoryStores, id: &str) -> Result<Entity> {
    let entities = stores.entities.all().await?;
    let mut current = entities
        .iter()
        .find(|entity| entity.id == id)
        .ok_or_else(|| NoEntity(format!("No entity {id}")))?;
    for _ in 0..8 {
        match &current.merged_into {
            Some(target) => {
                current = entities
                    .iter()
                    .find(|entity| &entity.id == target)
                    .ok_or_else(|| {
                        flow_like_types::anyhow!(
                            "Entity {id} was merged into a missing entity {target}"
                        )
                    })?;
            }
            None => return Ok(current.clone()),
        }
    }
    Err(flow_like_types::anyhow!("Entity {id} has a merge cycle"))
}
