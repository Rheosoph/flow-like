use super::{
    assignment::linear_assignment,
    embedding::{cosine, normalized},
    types::{
        AppearanceObservation, AssociationStatus, EntityAssociation, EntityCandidate, TrackState,
        UnresolvedObservation, UnresolvedReason,
    },
};
use flow_like_types::{Result, anyhow};
use std::{
    cmp::Reverse,
    collections::{BTreeMap, BTreeSet, VecDeque},
};

const GALLERY_SIZE: usize = 8;
const MAX_CANDIDATES: usize = 3;
/// Timestamps further ahead of the wall clock are clamped, so one fast camera clock cannot expire
/// the entities of the others. Never more than half the entity lifetime, see
/// [`AssociationConfig::max_clock_lead_ms`].
const MAX_CLOCK_LEAD_MS: i64 = 60_000;
const LIMITS: Limits = Limits {
    entities: 1024,
    tracks: 8192,
};

/// `(camera_id, session_id, tracker_id)`: within it one entity is at most one track per batch.
type Partition = (String, String, String);
/// Track ids are only unique per tracker instance, so the partition is part of the key.
type LocalKey = (Partition, u64);

#[derive(Clone, Copy, Debug)]
pub struct AssociationConfig {
    pub min_similarity: f32,
    pub ambiguity_margin: f32,
    pub min_observations: u32,
    pub entity_ttl_ms: i64,
}

impl AssociationConfig {
    pub fn new(
        min_similarity: f64,
        ambiguity_margin: f64,
        min_observations: i64,
        entity_ttl_ms: i64,
    ) -> Result<Self> {
        let min_observations = u32::try_from(min_observations)
            .ok()
            .filter(|count| *count >= 1)
            .ok_or_else(|| {
                anyhow!(
                    "Pin 'min_observations' must be between 1 and {}, got {min_observations}",
                    u32::MAX
                )
            })?;
        if entity_ttl_ms <= 0 {
            return Err(anyhow!(
                "Pin 'entity_ttl_ms' must be greater than 0, got {entity_ttl_ms}"
            ));
        }
        Ok(Self {
            min_similarity: unit_interval("min_similarity", min_similarity)?,
            ambiguity_margin: unit_interval("ambiguity_margin", ambiguity_margin)?,
            min_observations,
            entity_ttl_ms,
        })
    }
}

impl AssociationConfig {
    /// How far an observation may run ahead of the wall clock before it is clamped. Bounded by
    /// half the entity lifetime so a clamped clock cannot expire correctly timed entities.
    pub fn max_clock_lead_ms(&self) -> i64 {
        MAX_CLOCK_LEAD_MS.min(self.entity_ttl_ms / 2)
    }
}

fn unit_interval(pin: &str, value: f64) -> Result<f32> {
    if value.is_finite() && (0.0..=1.0).contains(&value) {
        Ok(value as f32)
    } else {
        Err(anyhow!("Pin '{pin}' must be between 0 and 1, got {value}"))
    }
}

#[derive(Debug, Default)]
pub struct AssociationOutput {
    pub associations: Vec<EntityAssociation>,
    pub unresolved: Vec<UnresolvedObservation>,
    pub entity_count: usize,
    /// Observations whose timestamp was clamped to the maximum clock lead past the wall clock
    pub clamped_timestamps: usize,
    /// Observations already older than the entity lifetime relative to the task clock; the state
    /// they create expires on the next call
    pub late_timestamps: usize,
}

#[derive(Clone, Copy, Debug)]
struct Limits {
    entities: usize,
    tracks: usize,
}

#[derive(Debug)]
struct Entity {
    gallery: VecDeque<Vec<f32>>,
    prototype: Vec<f32>,
    class_idx: i32,
    last_seen_ms: i64,
}

impl Entity {
    fn new(embedding: Vec<f32>, class_idx: i32, seen_ms: i64) -> Self {
        Self {
            gallery: VecDeque::from([embedding.clone()]),
            prototype: embedding,
            class_idx,
            last_seen_ms: seen_ms,
        }
    }

    fn add_exemplar(&mut self, embedding: &[f32]) {
        if self.gallery.len() >= GALLERY_SIZE {
            self.gallery.pop_front();
        }
        self.gallery.push_back(embedding.to_vec());
        let mut sum = vec![0.0; embedding.len()];
        for exemplar in &self.gallery {
            add_into(&mut sum, exemplar);
        }
        self.prototype = normalized(&sum).unwrap_or_else(|| embedding.to_vec());
    }

    fn touch(&mut self, seen_ms: i64) {
        self.last_seen_ms = self.last_seen_ms.max(seen_ms);
    }
}

#[derive(Debug)]
struct Binding {
    entity_id: u64,
    /// Similarity of the last embedding compared with the entity
    similarity: f32,
    last_seen_ms: i64,
}

#[derive(Debug)]
struct Pending {
    sum: Vec<f32>,
    count: u32,
    last_seen_ms: i64,
}

/// One call's observations with their usable embeddings and observation times.
struct Batch<'a> {
    observations: &'a [AppearanceObservation],
    /// `None` for lost observations, which never contribute appearance evidence
    embeddings: Vec<Option<Vec<f32>>>,
    times: Vec<i64>,
    clamped: usize,
}

impl<'a> Batch<'a> {
    fn new(observations: &'a [AppearanceObservation], wall_now_ms: i64, max_lead_ms: i64) -> Self {
        let ceiling = wall_now_ms.saturating_add(max_lead_ms);
        Self {
            observations,
            embeddings: observations
                .iter()
                .map(|observation| {
                    (observation.state != TrackState::Lost)
                        .then(|| normalized(&observation.embedding))
                        .flatten()
                })
                .collect(),
            times: observations
                .iter()
                .map(|observation| observed_at(observation.timestamp_ms, ceiling, wall_now_ms))
                .collect(),
            clamped: observations
                .iter()
                .filter(|observation| observation.timestamp_ms > ceiling)
                .count(),
        }
    }
}

/// Embedded observations of one unbound local track in this batch, or a single observation without
/// a track id.
struct Group {
    key: Option<LocalKey>,
    partition: Partition,
    members: Vec<usize>,
    sum: Vec<f32>,
    embedding: Vec<f32>,
    class_idx: i32,
    seen_ms: i64,
}

impl Group {
    /// `None` when `members` is empty or one of them has no usable embedding.
    fn new(key: Option<LocalKey>, members: Vec<usize>, batch: &Batch) -> Option<Self> {
        let first = &batch.observations[*members.first()?];
        let mut sum = vec![0.0; batch.embeddings[members[0]].as_ref()?.len()];
        let mut classes = BTreeMap::<i32, usize>::new();
        let mut seen_ms = i64::MIN;
        for &index in &members {
            add_into(&mut sum, batch.embeddings[index].as_ref()?);
            *classes
                .entry(batch.observations[index].bbox.class_idx)
                .or_default() += 1;
            seen_ms = seen_ms.max(batch.times[index]);
        }
        let class_idx = classes
            .into_iter()
            .max_by_key(|(class_idx, count)| (*count, Reverse(*class_idx)))
            .map(|(class_idx, _)| class_idx)?;
        Some(Self {
            embedding: normalized(&sum)?,
            partition: partition_of(first),
            key,
            members,
            sum,
            class_idx,
            seen_ms,
        })
    }
}

/// An unbound group taking part in the assignment of its partition.
struct Row {
    group: usize,
    embedding: Vec<f32>,
    observations: u32,
    ranked: Vec<EntityCandidate>,
}

#[derive(Clone, Debug)]
enum Resolution {
    Associated {
        entity_id: u64,
        similarity: f32,
        status: AssociationStatus,
    },
    Unresolved {
        reason: UnresolvedReason,
        observations: u32,
        candidates: Vec<EntityCandidate>,
    },
}

impl Resolution {
    fn tracked(binding: &Binding) -> Self {
        Self::Associated {
            entity_id: binding.entity_id,
            similarity: binding.similarity,
            status: AssociationStatus::Tracked,
        }
    }
}

/// Cross-camera identity state of one association task. Every update is deterministic for a
/// given state and input.
#[derive(Debug)]
pub struct Associator {
    entities: BTreeMap<u64, Entity>,
    bindings: BTreeMap<LocalKey, Binding>,
    pending: BTreeMap<LocalKey, Pending>,
    next_entity_id: u64,
    dimension: Option<usize>,
    /// Latest observation time processed; expiry never runs behind it
    clock_ms: Option<i64>,
    limits: Limits,
}

impl Default for Associator {
    fn default() -> Self {
        Self::with_limits(LIMITS)
    }
}

impl Associator {
    pub fn new() -> Self {
        Self::default()
    }

    fn with_limits(limits: Limits) -> Self {
        Self {
            entities: BTreeMap::new(),
            bindings: BTreeMap::new(),
            pending: BTreeMap::new(),
            next_entity_id: 1,
            dimension: None,
            clock_ms: None,
            limits,
        }
    }

    /// Assigns global entities to `observations`. Time follows the observation timestamps;
    /// `wall_now_ms` fills missing ones and caps those running ahead of it. An empty batch
    /// changes nothing, and a rejected batch leaves the state untouched.
    pub fn associate(
        &mut self,
        observations: &[AppearanceObservation],
        config: &AssociationConfig,
        wall_now_ms: i64,
    ) -> Result<AssociationOutput> {
        if observations.is_empty() {
            return Ok(AssociationOutput {
                entity_count: self.entities.len(),
                ..Default::default()
            });
        }
        let batch = Batch::new(observations, wall_now_ms, config.max_clock_lead_ms());
        let now = batch
            .times
            .iter()
            .copied()
            .chain(self.clock_ms)
            .max()
            .unwrap_or(wall_now_ms);
        let dimension = self.batch_dimension(
            &batch.embeddings,
            self.retained_dimension(now, config.entity_ttl_ms),
        )?;
        self.clock_ms = Some(now);
        self.expire(now, config.entity_ttl_ms);
        self.dimension = dimension;

        let mut resolutions: Vec<Option<Resolution>> = vec![None; observations.len()];
        let mut tracks = BTreeMap::<LocalKey, Vec<usize>>::new();
        let mut untracked = Vec::new();
        for (index, observation) in observations.iter().enumerate() {
            let key = local_key(observation);
            if observation.state == TrackState::Lost {
                resolutions[index] = Some(self.passive(key.as_ref()));
                continue;
            }
            match key {
                Some(key) => tracks.entry(key).or_default().push(index),
                None => untracked.push(index),
            }
        }

        let mut groups = Vec::new();
        let mut occupied = BTreeMap::<Partition, BTreeSet<u64>>::new();
        for (key, members) in tracks {
            let embedded = members
                .iter()
                .copied()
                .filter(|&index| batch.embeddings[index].is_some())
                .collect();
            let group = Group::new(Some(key.clone()), embedded, &batch);
            let seen_ms = members
                .iter()
                .map(|&index| batch.times[index])
                .max()
                .unwrap_or(now);
            let embedding = group.as_ref().map(|group| group.embedding.as_slice());
            match self.observe_bound(&key, embedding, seen_ms, config) {
                Some(binding) => {
                    occupied.entry(key.0).or_default().insert(binding.entity_id);
                    resolve(&mut resolutions, &members, Resolution::tracked(binding));
                }
                None => groups.extend(group),
            }
        }
        groups.extend(
            untracked
                .into_iter()
                .filter_map(|index| Group::new(None, vec![index], &batch)),
        );

        let mut partitions = BTreeMap::<Partition, Vec<usize>>::new();
        for (index, group) in groups.iter().enumerate() {
            partitions
                .entry(group.partition.clone())
                .or_default()
                .push(index);
        }
        let no_occupancy = BTreeSet::new();
        for (partition, members) in &partitions {
            let camera_occupied: BTreeSet<u64>;
            let occupied = if members.iter().all(|&index| groups[index].key.is_none()) {
                // An untracked detection may duplicate a tracked one of the same camera, whichever
                // tracker that belongs to.
                camera_occupied = occupied
                    .iter()
                    .filter(|((camera, session, _), _)| {
                        camera == &partition.0 && session == &partition.1
                    })
                    .flat_map(|(_, entities)| entities.iter().copied())
                    .collect();
                &camera_occupied
            } else {
                occupied.get(partition).unwrap_or(&no_occupancy)
            };
            let mut rows = Vec::new();
            for &index in members {
                let group = &groups[index];
                let (embedding, observations, ready) = match &group.key {
                    Some(key) => self.accumulate(key, group, config.min_observations),
                    None => (group.embedding.clone(), 0, true),
                };
                let ranked = self.rank(&embedding, group.class_idx, occupied);
                if ready {
                    rows.push(Row {
                        group: index,
                        embedding,
                        observations,
                        ranked,
                    });
                } else {
                    let resolution = Resolution::Unresolved {
                        reason: UnresolvedReason::Pending,
                        observations,
                        candidates: top_candidates(&ranked),
                    };
                    resolve(&mut resolutions, &group.members, resolution);
                }
            }
            self.assign(&rows, &groups, config, &mut resolutions);
        }

        self.enforce_limits();

        let mut output = AssociationOutput {
            entity_count: self.entities.len(),
            clamped_timestamps: batch.clamped,
            late_timestamps: batch
                .times
                .iter()
                .filter(|&&time| now.saturating_sub(time) > config.entity_ttl_ms)
                .count(),
            ..Default::default()
        };
        for (index, (observation, resolution)) in observations.iter().zip(resolutions).enumerate() {
            let timestamp_ms = batch.times[index];
            let resolution = resolution.unwrap_or_else(|| Resolution::Unresolved {
                reason: UnresolvedReason::MissingEmbedding,
                observations: self.pending_count(local_key(observation).as_ref()),
                candidates: Vec::new(),
            });
            match resolution {
                Resolution::Associated {
                    entity_id,
                    similarity,
                    status,
                } => output.associations.push(EntityAssociation {
                    bbox: observation.bbox.clone(),
                    camera_id: observation.camera_id.clone(),
                    session_id: observation.session_id.clone(),
                    tracker_id: observation.tracker_id.clone(),
                    track_id: observation.track_id,
                    timestamp_ms,
                    detection_index: observation.detection_index,
                    entity_id,
                    similarity,
                    status,
                }),
                Resolution::Unresolved {
                    reason,
                    observations,
                    candidates,
                } => output.unresolved.push(UnresolvedObservation {
                    bbox: observation.bbox.clone(),
                    camera_id: observation.camera_id.clone(),
                    session_id: observation.session_id.clone(),
                    tracker_id: observation.tracker_id.clone(),
                    track_id: observation.track_id,
                    timestamp_ms,
                    detection_index: observation.detection_index,
                    reason,
                    observations,
                    candidates,
                }),
            }
        }
        Ok(output)
    }

    fn expire(&mut self, now: i64, ttl_ms: i64) {
        self.entities
            .retain(|_, entity| alive(now, entity.last_seen_ms, ttl_ms));
        let entities = &self.entities;
        self.bindings.retain(|_, binding| {
            alive(now, binding.last_seen_ms, ttl_ms) && entities.contains_key(&binding.entity_id)
        });
        self.pending
            .retain(|_, pending| alive(now, pending.last_seen_ms, ttl_ms));
    }

    /// The embedding length still in use once everything unseen at `now` has expired.
    fn retained_dimension(&self, now: i64, ttl_ms: i64) -> Option<usize> {
        let retained = self
            .entities
            .values()
            .any(|entity| alive(now, entity.last_seen_ms, ttl_ms))
            || self
                .pending
                .values()
                .any(|pending| alive(now, pending.last_seen_ms, ttl_ms));
        self.dimension.filter(|_| retained)
    }

    fn batch_dimension(
        &self,
        embeddings: &[Option<Vec<f32>>],
        mut expected: Option<usize>,
    ) -> Result<Option<usize>> {
        for (index, embedding) in embeddings.iter().enumerate() {
            let Some(embedding) = embedding else {
                continue;
            };
            match expected {
                Some(dimension) if dimension != embedding.len() => {
                    return Err(anyhow!(
                        "Pin 'observations': observation {index} has an embedding of length {}, but this association task uses length {dimension}. Use one appearance model per task_id",
                        embedding.len()
                    ));
                }
                Some(_) => {}
                None => expected = Some(embedding.len()),
            }
        }
        Ok(expected)
    }

    fn binding(&self, key: &LocalKey) -> Option<&Binding> {
        self.bindings
            .get(key)
            .filter(|binding| self.entities.contains_key(&binding.entity_id))
    }

    /// A lost track's box is a prediction: it reports its binding without changing any state.
    fn passive(&self, key: Option<&LocalKey>) -> Resolution {
        match key.and_then(|key| self.binding(key)) {
            Some(binding) => Resolution::tracked(binding),
            None => Resolution::Unresolved {
                reason: UnresolvedReason::Lost,
                observations: self.pending_count(key),
                candidates: Vec::new(),
            },
        }
    }

    /// Updates a bound track and returns its binding; `None` when the track is not bound. Without
    /// an embedding the track keeps its last similarity and only refreshes its last sighting.
    fn observe_bound(
        &mut self,
        key: &LocalKey,
        embedding: Option<&[f32]>,
        seen_ms: i64,
        config: &AssociationConfig,
    ) -> Option<&Binding> {
        let binding = self.bindings.get_mut(key)?;
        let entity = self.entities.get_mut(&binding.entity_id)?;
        if let Some(embedding) = embedding {
            binding.similarity = cosine(embedding, &entity.prototype);
            if binding.similarity >= config.min_similarity {
                entity.add_exemplar(embedding);
            }
        }
        entity.touch(seen_ms);
        binding.last_seen_ms = binding.last_seen_ms.max(seen_ms);
        Some(binding)
    }

    /// Adds the group to its pending track and returns the accumulated embedding, the number of
    /// observations so far and whether that is enough to resolve the track.
    fn accumulate(
        &mut self,
        key: &LocalKey,
        group: &Group,
        min_observations: u32,
    ) -> (Vec<f32>, u32, bool) {
        let pending = self.pending.entry(key.clone()).or_insert_with(|| Pending {
            sum: vec![0.0; group.sum.len()],
            count: 0,
            last_seen_ms: group.seen_ms,
        });
        add_into(&mut pending.sum, &group.sum);
        pending.count = pending
            .count
            .saturating_add(u32::try_from(group.members.len()).unwrap_or(u32::MAX));
        pending.last_seen_ms = pending.last_seen_ms.max(group.seen_ms);
        let embedding = normalized(&pending.sum).unwrap_or_else(|| group.embedding.clone());
        (embedding, pending.count, pending.count >= min_observations)
    }

    /// Entities of the same class that are not occupied in the partition, most similar first.
    fn rank(
        &self,
        embedding: &[f32],
        class_idx: i32,
        occupied: &BTreeSet<u64>,
    ) -> Vec<EntityCandidate> {
        let mut ranked: Vec<EntityCandidate> = self
            .entities
            .iter()
            .filter(|(id, entity)| entity.class_idx == class_idx && !occupied.contains(id))
            .map(|(id, entity)| EntityCandidate {
                entity_id: *id,
                similarity: cosine(embedding, &entity.prototype),
            })
            .collect();
        ranked.sort_by(|a, b| {
            b.similarity
                .total_cmp(&a.similarity)
                .then(a.entity_id.cmp(&b.entity_id))
        });
        ranked
    }

    fn assign(
        &mut self,
        rows: &[Row],
        groups: &[Group],
        config: &AssociationConfig,
        resolutions: &mut [Option<Resolution>],
    ) {
        let min = config.min_similarity;
        let columns: Vec<u64> = rows
            .iter()
            .flat_map(|row| above_threshold(&row.ranked, min))
            .map(|candidate| candidate.entity_id)
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        let similarities: Vec<f32> = rows
            .iter()
            .flat_map(|row| {
                columns.iter().map(|entity_id| {
                    above_threshold(&row.ranked, min)
                        .find(|candidate| candidate.entity_id == *entity_id)
                        .map_or(f32::NEG_INFINITY, |candidate| candidate.similarity)
                })
            })
            .collect();
        let similarity = |row: usize, column: usize| similarities[row * columns.len() + column];
        let assignment = linear_assignment(rows.len(), columns.len(), 1.0 - min, |row, column| {
            let value = similarity(row, column);
            if value >= min {
                1.0 - value
            } else {
                f32::INFINITY
            }
        });
        let matches: BTreeMap<usize, usize> = assignment.matches.into_iter().collect();

        for (index, row) in rows.iter().enumerate() {
            let group = &groups[row.group];
            let contested = |matched: Option<(u64, f32)>| {
                let runner_up = row
                    .ranked
                    .iter()
                    .find(|candidate| matched.is_none_or(|(id, _)| candidate.entity_id != id));
                runner_up.is_some_and(|candidate| {
                    candidate.similarity >= min
                        && matched.is_none_or(|(_, similarity)| {
                            similarity - candidate.similarity <= config.ambiguity_margin
                        })
                })
            };
            let matched = matches
                .get(&index)
                .map(|&column| (columns[column], similarity(index, column)));

            let resolution = if contested(matched) {
                Resolution::Unresolved {
                    reason: UnresolvedReason::Ambiguous,
                    observations: row.observations,
                    candidates: top_candidates(&row.ranked),
                }
            } else if let Some((entity_id, similarity)) = matched {
                if let Some(entity) = self.entities.get_mut(&entity_id) {
                    entity.add_exemplar(&row.embedding);
                    entity.touch(group.seen_ms);
                }
                self.bind(group, entity_id, similarity);
                Resolution::Associated {
                    entity_id,
                    similarity,
                    status: AssociationStatus::Matched,
                }
            } else if group.key.is_some() {
                let entity_id = self.next_entity_id;
                self.next_entity_id += 1;
                self.entities.insert(
                    entity_id,
                    Entity::new(row.embedding.clone(), group.class_idx, group.seen_ms),
                );
                self.bind(group, entity_id, 1.0);
                Resolution::Associated {
                    entity_id,
                    similarity: 1.0,
                    status: AssociationStatus::Created,
                }
            } else {
                Resolution::Unresolved {
                    reason: UnresolvedReason::Untracked,
                    observations: row.observations,
                    candidates: top_candidates(&row.ranked),
                }
            };
            resolve(resolutions, &group.members, resolution);
        }
    }

    fn bind(&mut self, group: &Group, entity_id: u64, similarity: f32) {
        let Some(key) = &group.key else {
            return;
        };
        self.pending.remove(key);
        self.bindings.insert(
            key.clone(),
            Binding {
                entity_id,
                similarity,
                last_seen_ms: group.seen_ms,
            },
        );
    }

    fn pending_count(&self, key: Option<&LocalKey>) -> u32 {
        key.and_then(|key| self.pending.get(key))
            .map_or(0, |pending| pending.count)
    }

    fn enforce_limits(&mut self) {
        let evicted: BTreeSet<u64> =
            trim_oldest(&mut self.entities, self.limits.entities, |e| e.last_seen_ms)
                .into_iter()
                .collect();
        if !evicted.is_empty() {
            self.bindings
                .retain(|_, binding| !evicted.contains(&binding.entity_id));
        }
        trim_oldest(&mut self.bindings, self.limits.tracks, |b| b.last_seen_ms);
        trim_oldest(&mut self.pending, self.limits.tracks, |p| p.last_seen_ms);
    }
}

fn partition_of(observation: &AppearanceObservation) -> Partition {
    (
        observation.camera_id.clone(),
        observation.session_id.clone(),
        observation.tracker_id.clone(),
    )
}

fn local_key(observation: &AppearanceObservation) -> Option<LocalKey> {
    Some((partition_of(observation), observation.track_id?))
}

fn resolve(resolutions: &mut [Option<Resolution>], members: &[usize], resolution: Resolution) {
    for &index in members {
        resolutions[index] = Some(resolution.clone());
    }
}

/// The timestamp capped at `ceiling`, or the wall clock when the observation carries none.
fn observed_at(timestamp_ms: i64, ceiling: i64, wall_now_ms: i64) -> i64 {
    let timestamp_ms = timestamp_ms.min(ceiling);
    if timestamp_ms > 0 {
        timestamp_ms
    } else {
        wall_now_ms
    }
}

fn alive(now: i64, seen_ms: i64, ttl_ms: i64) -> bool {
    now.saturating_sub(seen_ms) <= ttl_ms
}

fn above_threshold(
    ranked: &[EntityCandidate],
    min_similarity: f32,
) -> impl Iterator<Item = &EntityCandidate> {
    ranked
        .iter()
        .take_while(move |candidate| candidate.similarity >= min_similarity)
}

fn top_candidates(ranked: &[EntityCandidate]) -> Vec<EntityCandidate> {
    ranked.iter().take(MAX_CANDIDATES).cloned().collect()
}

fn add_into(target: &mut [f32], values: &[f32]) {
    for (sum, value) in target.iter_mut().zip(values) {
        *sum += value;
    }
}

/// Removes the least recently seen entries beyond `limit` (ties by key) and returns their keys.
fn trim_oldest<K: Ord + Clone, V>(
    map: &mut BTreeMap<K, V>,
    limit: usize,
    seen_ms: impl Fn(&V) -> i64,
) -> Vec<K> {
    let excess = map.len().saturating_sub(limit);
    if excess == 0 {
        return Vec::new();
    }
    let mut by_age: Vec<(i64, K)> = map
        .iter()
        .map(|(key, value)| (seen_ms(value), key.clone()))
        .collect();
    by_age.sort();
    by_age
        .into_iter()
        .take(excess)
        .map(|(_, key)| {
            map.remove(&key);
            key
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use flow_like_catalog_core::BoundingBox;
    use flow_like_types::json::to_value;

    const A: [f32; 4] = [1.0, 0.0, 0.0, 0.0];
    const B: [f32; 4] = [0.0, 1.0, 0.0, 0.0];
    const C: [f32; 4] = [0.0, 0.0, 1.0, 0.0];
    const D: [f32; 4] = [0.0, 0.0, 0.0, 1.0];
    const NEAR_A: [f32; 4] = [0.95, 0.1, 0.05, 0.0];
    const NEAR_B: [f32; 4] = [0.1, 0.95, 0.0, 0.05];
    const BETWEEN_A_B: [f32; 4] = [1.0, 1.0, 0.0, 0.0];

    fn config(min_observations: u32) -> AssociationConfig {
        AssociationConfig {
            min_observations,
            ..AssociationConfig::new(0.6, 0.05, 3, 10_000).unwrap()
        }
    }

    fn obs(camera: &str, track: Option<u64>, ts: i64, embedding: &[f32]) -> AppearanceObservation {
        AppearanceObservation {
            bbox: BoundingBox {
                x2: 10.0,
                y2: 20.0,
                score: 0.9,
                ..Default::default()
            },
            embedding: embedding.to_vec(),
            camera_id: camera.into(),
            session_id: "s".into(),
            track_id: track,
            timestamp_ms: ts,
            detection_index: None,
            ..Default::default()
        }
    }

    /// A wall clock long after the recorded timestamps most tests use.
    const WALL_MS: i64 = 1_700_000_000_000;
    const DAY_MS: i64 = 86_400_000;

    fn with_class(mut observation: AppearanceObservation, class_idx: i32) -> AppearanceObservation {
        observation.bbox.class_idx = class_idx;
        observation
    }

    fn with_tracker(
        mut observation: AppearanceObservation,
        tracker: &str,
    ) -> AppearanceObservation {
        observation.tracker_id = tracker.into();
        observation
    }

    fn lost(mut observation: AppearanceObservation) -> AppearanceObservation {
        observation.state = TrackState::Lost;
        observation
    }

    fn run_at(
        associator: &mut Associator,
        observations: &[AppearanceObservation],
        config: &AssociationConfig,
        wall_now_ms: i64,
    ) -> AssociationOutput {
        let output = associator
            .associate(observations, config, wall_now_ms)
            .unwrap();
        assert_eq!(
            output.associations.len() + output.unresolved.len(),
            observations.len(),
            "every observation is reported exactly once: {output:?}"
        );
        output
    }

    fn run(
        associator: &mut Associator,
        observations: &[AppearanceObservation],
        config: &AssociationConfig,
    ) -> AssociationOutput {
        run_at(associator, observations, config, WALL_MS)
    }

    fn single_association(output: &AssociationOutput) -> (u64, AssociationStatus, f32) {
        assert_eq!(output.associations.len(), 1, "{output:?}");
        let association = &output.associations[0];
        (
            association.entity_id,
            association.status,
            association.similarity,
        )
    }

    fn single_unresolved(output: &AssociationOutput) -> &UnresolvedObservation {
        assert_eq!(output.unresolved.len(), 1, "{output:?}");
        &output.unresolved[0]
    }

    fn key(camera: &str, track: u64) -> LocalKey {
        ((camera.into(), "s".into(), String::new()), track)
    }

    #[test]
    fn first_sighting_stays_pending_until_min_observations() {
        let mut associator = Associator::new();
        let config = config(3);
        for (count, ts) in [(1, 1000), (2, 1100)] {
            let output = run(&mut associator, &[obs("cam1", Some(1), ts, &A)], &config);
            let unresolved = single_unresolved(&output);
            assert_eq!(unresolved.reason, UnresolvedReason::Pending);
            assert_eq!(unresolved.observations, count);
            assert!(output.associations.is_empty());
            assert_eq!(output.entity_count, 0);
        }

        let output = run(&mut associator, &[obs("cam1", Some(1), 1200, &A)], &config);
        assert_eq!(
            single_association(&output),
            (1, AssociationStatus::Created, 1.0)
        );
        assert_eq!(output.entity_count, 1);
        assert!(associator.pending.is_empty());

        let output = run(&mut associator, &[obs("cam1", Some(1), 1300, &A)], &config);
        let (entity_id, status, similarity) = single_association(&output);
        assert_eq!((entity_id, status), (1, AssociationStatus::Tracked));
        assert!((similarity - 1.0).abs() < 1e-5);
    }

    #[test]
    fn same_person_on_a_second_camera_is_matched() {
        let mut associator = Associator::new();
        let config = config(1);
        run(&mut associator, &[obs("cam1", Some(1), 1000, &A)], &config);

        let output = run(
            &mut associator,
            &[obs("cam2", Some(7), 2000, &NEAR_A)],
            &config,
        );
        let (entity_id, status, similarity) = single_association(&output);
        assert_eq!((entity_id, status), (1, AssociationStatus::Matched));
        assert!(similarity > 0.9);
        assert_eq!(output.associations[0].camera_id, "cam2");

        let output = run(
            &mut associator,
            &[obs("cam2", Some(7), 2100, &NEAR_A)],
            &config,
        );
        assert_eq!(single_association(&output).1, AssociationStatus::Tracked);
        assert_eq!(output.entity_count, 1);
    }

    #[test]
    fn cross_camera_sightings_in_one_batch_share_an_entity() {
        let mut associator = Associator::new();
        let output = run(
            &mut associator,
            &[
                obs("cam2", Some(1), 1000, &NEAR_A),
                obs("cam1", Some(1), 1000, &A),
            ],
            &config(1),
        );
        let results: Vec<_> = output
            .associations
            .iter()
            .map(|a| (a.camera_id.as_str(), a.entity_id, a.status))
            .collect();
        assert_eq!(
            results,
            vec![
                ("cam2", 1, AssociationStatus::Matched),
                ("cam1", 1, AssociationStatus::Created),
            ]
        );
    }

    #[test]
    fn two_people_stay_distinct() {
        let mut associator = Associator::new();
        let config = config(1);
        let output = run(
            &mut associator,
            &[
                obs("cam1", Some(1), 1000, &A),
                obs("cam1", Some(2), 1000, &B),
            ],
            &config,
        );
        let created: Vec<_> = output
            .associations
            .iter()
            .map(|a| (a.track_id, a.entity_id, a.status))
            .collect();
        assert_eq!(
            created,
            vec![
                (Some(1), 1, AssociationStatus::Created),
                (Some(2), 2, AssociationStatus::Created),
            ]
        );

        let output = run(
            &mut associator,
            &[
                obs("cam2", Some(5), 2000, &NEAR_B),
                obs("cam2", Some(6), 2000, &NEAR_A),
            ],
            &config,
        );
        let matched: Vec<_> = output
            .associations
            .iter()
            .map(|a| (a.track_id, a.entity_id, a.status))
            .collect();
        assert_eq!(
            matched,
            vec![
                (Some(5), 2, AssociationStatus::Matched),
                (Some(6), 1, AssociationStatus::Matched),
            ]
        );
        assert_eq!(output.entity_count, 2);
    }

    #[test]
    fn ambiguity_yields_unresolved_and_keeps_evidence_pending() {
        let mut associator = Associator::new();
        let config = config(1);
        run(
            &mut associator,
            &[
                obs("cam1", Some(1), 1000, &A),
                obs("cam1", Some(2), 1000, &B),
            ],
            &config,
        );

        for (count, ts) in [(1, 2000), (2, 2100)] {
            let output = run(
                &mut associator,
                &[obs("cam2", Some(9), ts, &BETWEEN_A_B)],
                &config,
            );
            let unresolved = single_unresolved(&output);
            assert_eq!(unresolved.reason, UnresolvedReason::Ambiguous);
            assert_eq!(unresolved.observations, count);
            let candidates: Vec<u64> = unresolved.candidates.iter().map(|c| c.entity_id).collect();
            assert_eq!(candidates, vec![1, 2]);
            assert!(!associator.bindings.contains_key(&key("cam2", 9)));
        }

        let output = run(
            &mut associator,
            &[obs("cam2", Some(9), 2200, &[1.0, 0.05, 0.0, 0.0])],
            &config,
        );
        let (entity_id, status, similarity) = single_association(&output);
        assert_eq!((entity_id, status), (1, AssociationStatus::Matched));
        assert!(
            similarity < 0.9,
            "accumulated evidence is used: {similarity}"
        );
        assert!(associator.pending.is_empty());
        assert_eq!(associator.bindings[&key("cam2", 9)].entity_id, 1);
    }

    #[test]
    fn same_camera_occupancy_prevents_two_tracks_taking_one_entity() {
        let mut associator = Associator::new();
        let config = config(1);
        run(&mut associator, &[obs("cam1", Some(1), 1000, &A)], &config);
        let second_track = normalized(&BETWEEN_A_B).unwrap();
        assert!(cosine(&second_track, &A) >= config.min_similarity);

        let output = run(
            &mut associator,
            &[
                obs("cam1", Some(1), 2000, &A),
                obs("cam1", Some(2), 2000, &second_track),
                obs("cam2", Some(4), 2000, &NEAR_A),
            ],
            &config,
        );
        let results: Vec<_> = output
            .associations
            .iter()
            .map(|a| (a.camera_id.as_str(), a.track_id, a.entity_id, a.status))
            .collect();
        assert_eq!(
            results,
            vec![
                ("cam1", Some(1), 1, AssociationStatus::Tracked),
                ("cam1", Some(2), 2, AssociationStatus::Created),
                ("cam2", Some(4), 1, AssociationStatus::Matched),
            ]
        );
    }

    #[test]
    fn contested_entity_in_one_partition_is_ambiguous_for_the_weaker_track() {
        let mut associator = Associator::new();
        let config = config(1);
        run(&mut associator, &[obs("cam1", Some(1), 1000, &A)], &config);

        let output = run(
            &mut associator,
            &[
                obs("cam2", Some(1), 2000, &NEAR_A),
                obs("cam2", Some(2), 2000, &A),
            ],
            &config,
        );
        assert_eq!(
            single_association(&output).0,
            1,
            "the closer track takes the entity"
        );
        assert_eq!(output.associations[0].track_id, Some(2));
        let unresolved = single_unresolved(&output);
        assert_eq!(unresolved.track_id, Some(1));
        assert_eq!(unresolved.reason, UnresolvedReason::Ambiguous);
        assert!(associator.pending.contains_key(&key("cam2", 1)));
        assert_eq!(output.entity_count, 1);
    }

    #[test]
    fn class_mismatch_never_matches() {
        let mut associator = Associator::new();
        let config = config(1);
        run(&mut associator, &[obs("cam1", Some(1), 1000, &A)], &config);

        let output = run(
            &mut associator,
            &[with_class(obs("cam2", Some(2), 2000, &A), 1)],
            &config,
        );
        assert_eq!(
            single_association(&output),
            (2, AssociationStatus::Created, 1.0)
        );
        assert_eq!(associator.entities[&2].class_idx, 1);

        let output = run(
            &mut associator,
            &[with_class(obs("cam3", None, 3000, &A), 2)],
            &config,
        );
        let unresolved = single_unresolved(&output);
        assert_eq!(unresolved.reason, UnresolvedReason::Untracked);
        assert!(unresolved.candidates.is_empty());
    }

    #[test]
    fn ttl_expiry_forgets_entities_and_bindings() {
        let mut associator = Associator::new();
        let config = config(1);
        run(&mut associator, &[obs("cam1", Some(1), 1000, &A)], &config);
        run(&mut associator, &[obs("cam2", Some(1), 5000, &B)], &config);

        let output = run(
            &mut associator,
            &[obs("cam3", Some(1), 12_000, &C)],
            &config,
        );
        assert_eq!(single_association(&output).0, 3);
        assert_eq!(output.entity_count, 2);
        assert!(!associator.entities.contains_key(&1));
        assert!(!associator.bindings.contains_key(&key("cam1", 1)));
        assert!(associator.bindings.contains_key(&key("cam2", 1)));
    }

    #[test]
    fn local_track_whose_entity_expired_starts_over() {
        let mut associator = Associator::new();
        let config = config(2);
        let first = [
            obs("cam1", Some(1), 1000, &A),
            obs("cam1", Some(1), 1033, &A),
        ];
        let output = run(&mut associator, &first, &config);
        assert!(
            output
                .associations
                .iter()
                .all(|a| a.entity_id == 1 && a.status == AssociationStatus::Created)
        );

        let output = run(
            &mut associator,
            &[obs("cam1", Some(1), 20_000, &A)],
            &config,
        );
        let unresolved = single_unresolved(&output);
        assert_eq!(unresolved.reason, UnresolvedReason::Pending);
        assert_eq!(unresolved.observations, 1);
        assert_eq!(output.entity_count, 0);

        let output = run(
            &mut associator,
            &[obs("cam1", Some(1), 20_100, &A)],
            &config,
        );
        assert_eq!(
            single_association(&output),
            (2, AssociationStatus::Created, 1.0)
        );
    }

    #[test]
    fn bound_track_whose_entity_was_evicted_is_unbound() {
        let mut associator = Associator::with_limits(Limits {
            entities: 1,
            tracks: 8,
        });
        let config = config(1);
        run(&mut associator, &[obs("cam1", Some(1), 1000, &A)], &config);

        let output = run(&mut associator, &[obs("cam1", Some(2), 2000, &B)], &config);
        assert_eq!(single_association(&output).0, 2);
        assert_eq!(output.entity_count, 1);
        assert!(!associator.bindings.contains_key(&key("cam1", 1)));

        let output = run(&mut associator, &[obs("cam1", Some(1), 3000, &A)], &config);
        assert_eq!(
            single_association(&output),
            (3, AssociationStatus::Created, 1.0)
        );
    }

    #[test]
    fn track_caps_evict_least_recently_seen() {
        let mut associator = Associator::with_limits(Limits {
            entities: 8,
            tracks: 2,
        });
        for track in 1..=3 {
            run(
                &mut associator,
                &[obs("cam1", Some(track), 1000 * track as i64, &A)],
                &config(5),
            );
        }
        let keys: Vec<_> = associator.pending.keys().cloned().collect();
        assert_eq!(keys, vec![key("cam1", 2), key("cam1", 3)]);

        run(
            &mut associator,
            &[
                obs("cam1", Some(4), 4000, &A),
                obs("cam1", Some(5), 4000, &B),
                obs("cam1", Some(6), 4000, &C),
            ],
            &config(1),
        );
        assert_eq!(associator.bindings.len(), 2);
        assert_eq!(associator.entities.len(), 3);
    }

    #[test]
    fn dimension_mismatch_errors_without_touching_state() {
        let mut associator = Associator::new();
        let config = config(1);
        run(&mut associator, &[obs("cam1", Some(1), 1000, &A)], &config);

        let error = associator
            .associate(
                &[obs("cam1", Some(2), 5000, &[1.0, 0.0, 0.0])],
                &config,
                WALL_MS,
            )
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("length 3") && error.contains("length 4"),
            "{error}"
        );
        assert_eq!(associator.entities.len(), 1);
        assert!(associator.pending.is_empty());
        assert_eq!(associator.clock_ms, Some(1000), "the clock stays put");

        let mut fresh = Associator::new();
        let error = fresh
            .associate(
                &[
                    obs("cam1", Some(1), 1000, &A),
                    obs("cam1", Some(2), 1000, &[0.0, 1.0]),
                ],
                &config,
                WALL_MS,
            )
            .unwrap_err()
            .to_string();
        assert!(error.contains("observation 1"), "{error}");
        assert!(fresh.entities.is_empty() && fresh.dimension.is_none());
        assert_eq!(fresh.clock_ms, None);
    }

    #[test]
    fn lost_observations_do_not_count_towards_the_embedding_length() {
        let mut associator = Associator::new();
        let config = config(1);
        run(&mut associator, &[obs("cam1", Some(1), 1000, &A)], &config);
        let output = run(
            &mut associator,
            &[lost(obs("cam1", Some(1), 2000, &[1.0, 0.0]))],
            &config,
        );
        assert_eq!(single_association(&output).0, 1);
        assert_eq!(associator.dimension, Some(4));
    }

    #[test]
    fn embedding_length_can_change_once_everything_expired() {
        let mut associator = Associator::new();
        let config = config(1);
        run(&mut associator, &[obs("cam1", Some(1), 1000, &A)], &config);
        let output = run(
            &mut associator,
            &[obs("cam1", Some(2), 20_000, &[0.0, 1.0])],
            &config,
        );
        assert_eq!(single_association(&output).0, 2);
        assert_eq!(associator.dimension, Some(2));
    }

    #[test]
    fn output_order_is_deterministic() {
        let batches = [
            vec![
                obs("cam2", Some(3), 1000, &C),
                obs("cam1", Some(2), 1000, &B),
                obs("cam1", Some(1), 1000, &A),
                obs("cam1", None, 1000, &D),
            ],
            vec![
                obs("cam3", Some(1), 2000, &NEAR_B),
                obs("cam3", None, 2000, &NEAR_A),
                obs("cam3", Some(2), 2000, &[]),
                obs("cam3", Some(3), 2000, &BETWEEN_A_B),
            ],
        ];
        let outputs: Vec<Vec<_>> = (0..2)
            .map(|_| {
                let mut associator = Associator::new();
                batches
                    .iter()
                    .map(|batch| {
                        let output = run(&mut associator, batch, &config(1));
                        (
                            to_value(&output.associations).unwrap(),
                            to_value(&output.unresolved).unwrap(),
                        )
                    })
                    .collect()
            })
            .collect();
        assert_eq!(outputs[0], outputs[1]);

        let mut associator = Associator::new();
        let output = run(&mut associator, &batches[0], &config(1));
        let order: Vec<_> = output
            .associations
            .iter()
            .map(|a| (a.camera_id.as_str(), a.track_id, a.entity_id))
            .collect();
        assert_eq!(
            order,
            vec![
                ("cam2", Some(3), 3),
                ("cam1", Some(2), 2),
                ("cam1", Some(1), 1),
            ],
            "input order is kept; ids follow sorted camera, session and track"
        );
        assert_eq!(
            single_unresolved(&output).reason,
            UnresolvedReason::Untracked
        );
    }

    #[test]
    fn entity_ids_are_never_reused() {
        let mut associator = Associator::new();
        let config = config(1);
        run(
            &mut associator,
            &[
                obs("cam1", Some(1), 1000, &A),
                obs("cam1", Some(2), 1000, &B),
            ],
            &config,
        );
        let output = run(
            &mut associator,
            &[obs("cam1", Some(3), 50_000, &A)],
            &config,
        );
        assert_eq!(output.entity_count, 1);
        assert_eq!(
            single_association(&output),
            (3, AssociationStatus::Created, 1.0)
        );
    }

    #[test]
    fn empty_input_yields_empty_output() {
        let mut associator = Associator::new();
        let output = run(&mut associator, &[], &config(1));
        assert!(output.associations.is_empty() && output.unresolved.is_empty());
        assert_eq!(output.entity_count, 0);
        assert_eq!(associator.clock_ms, None);

        run(
            &mut associator,
            &[obs("cam1", Some(1), 1000, &A)],
            &config(1),
        );
        let output = run(&mut associator, &[], &config(1));
        assert!(output.associations.is_empty() && output.unresolved.is_empty());
        assert_eq!(output.entity_count, 1);
    }

    #[test]
    fn recorded_footage_survives_idle_calls_and_a_later_wall_clock() {
        let mut associator = Associator::new();
        let config = config(1);
        run(&mut associator, &[obs("cam1", Some(1), 1000, &A)], &config);

        let output = run_at(&mut associator, &[], &config, WALL_MS + DAY_MS);
        assert_eq!(output.entity_count, 1, "an empty batch expires nothing");
        assert_eq!(associator.clock_ms, Some(1000));
        assert_eq!(associator.bindings[&key("cam1", 1)].last_seen_ms, 1000);

        let output = run_at(
            &mut associator,
            &[obs("cam2", Some(7), 2000, &NEAR_A)],
            &config,
            WALL_MS + 2 * DAY_MS,
        );
        let (entity_id, status, _) = single_association(&output);
        assert_eq!((entity_id, status), (1, AssociationStatus::Matched));
        assert_eq!(output.associations[0].timestamp_ms, 2000);
        assert_eq!(output.clamped_timestamps, 0);
        assert_eq!(associator.clock_ms, Some(2000));
    }

    #[test]
    fn a_fast_camera_clock_cannot_expire_other_cameras_entities() {
        let mut associator = Associator::new();
        let config = AssociationConfig {
            entity_ttl_ms: 2 * MAX_CLOCK_LEAD_MS,
            ..config(1)
        };
        run(
            &mut associator,
            &[obs("cam1", Some(1), WALL_MS, &A)],
            &config,
        );

        let output = run(
            &mut associator,
            &[obs("cam2", Some(1), WALL_MS + 3_600_000, &B)],
            &config,
        );
        let ceiling = WALL_MS + config.max_clock_lead_ms();
        assert_eq!(config.max_clock_lead_ms(), MAX_CLOCK_LEAD_MS);
        assert_eq!(output.clamped_timestamps, 1);
        assert_eq!(output.associations[0].timestamp_ms, ceiling);
        assert_eq!(output.entity_count, 2);
        assert_eq!(associator.clock_ms, Some(ceiling));

        let output = run_at(
            &mut associator,
            &[obs("cam1", Some(1), WALL_MS + 1000, &A)],
            &config,
            WALL_MS + 1000,
        );
        assert_eq!(single_association(&output).1, AssociationStatus::Tracked);
        assert_eq!(output.entity_count, 2);
        assert_eq!(
            associator.clock_ms,
            Some(ceiling),
            "the clock never runs back"
        );
    }

    #[test]
    fn clock_lead_never_exceeds_half_the_entity_lifetime() {
        let mut associator = Associator::new();
        let config = AssociationConfig {
            entity_ttl_ms: 30_000,
            ..config(1)
        };
        assert_eq!(config.max_clock_lead_ms(), 15_000);
        run(
            &mut associator,
            &[obs("cam1", Some(1), WALL_MS, &A)],
            &config,
        );
        for _ in 0..3 {
            let output = run(
                &mut associator,
                &[obs("cam2", Some(1), WALL_MS * 1000, &B)],
                &config,
            );
            assert_eq!(output.clamped_timestamps, 1);
        }
        let output = run(
            &mut associator,
            &[obs("cam1", Some(1), WALL_MS, &A)],
            &config,
        );
        let (entity_id, status, _) = single_association(&output);
        assert_eq!(
            (entity_id, status),
            (1, AssociationStatus::Tracked),
            "a clamped clock must not expire correctly timed entities"
        );
    }

    #[test]
    fn observations_older_than_the_lifetime_are_counted_as_late() {
        let mut associator = Associator::new();
        let config = config(1);
        let output = run(
            &mut associator,
            &[obs("cam1", Some(1), WALL_MS, &A)],
            &config,
        );
        assert_eq!(output.late_timestamps, 0);
        let output = run(
            &mut associator,
            &[
                obs("cam2", Some(1), WALL_MS - config.entity_ttl_ms - 1, &B),
                obs("cam1", Some(1), WALL_MS, &A),
            ],
            &config,
        );
        assert_eq!(output.late_timestamps, 1);
    }

    #[test]
    fn microsecond_timestamps_are_clamped() {
        let mut associator = Associator::new();
        let output = run(
            &mut associator,
            &[
                obs("cam1", Some(1), WALL_MS * 1000, &A),
                obs("cam1", Some(2), WALL_MS, &B),
            ],
            &config(1),
        );
        let ceiling = WALL_MS + config(1).max_clock_lead_ms();
        assert_eq!(output.clamped_timestamps, 1);
        let times: Vec<i64> = output.associations.iter().map(|a| a.timestamp_ms).collect();
        assert_eq!(times, vec![ceiling, WALL_MS]);
        assert_eq!(associator.clock_ms, Some(ceiling));
        assert_eq!(associator.entities[&1].last_seen_ms, ceiling);
    }

    #[test]
    fn trackers_sharing_a_track_id_never_share_a_binding() {
        let mut associator = Associator::new();
        let config = config(1);
        run(
            &mut associator,
            &[with_tracker(obs("cam1", Some(1), 1000, &A), "t1")],
            &config,
        );

        let output = run(
            &mut associator,
            &[
                with_tracker(obs("cam1", Some(1), 2000, &A), "t1"),
                with_tracker(obs("cam1", Some(1), 2000, &B), "t2"),
                with_tracker(obs("cam1", Some(3), 2000, &NEAR_A), "t2"),
            ],
            &config,
        );
        let results: Vec<_> = output
            .associations
            .iter()
            .map(|a| (a.tracker_id.as_str(), a.track_id, a.entity_id, a.status))
            .collect();
        assert_eq!(
            results,
            vec![
                ("t1", Some(1), 1, AssociationStatus::Tracked),
                ("t2", Some(1), 2, AssociationStatus::Created),
                ("t2", Some(3), 1, AssociationStatus::Matched),
            ],
            "another tracker's track 1 is a different track, and its partition is not occupied"
        );
        let bound: Vec<_> = associator
            .bindings
            .iter()
            .map(|((partition, track), binding)| (partition.2.as_str(), *track, binding.entity_id))
            .collect();
        assert_eq!(bound, vec![("t1", 1, 1), ("t2", 1, 2), ("t2", 3, 1)]);
    }

    #[test]
    fn lost_observations_are_passive() {
        let mut associator = Associator::new();
        let config = config(1);
        run(&mut associator, &[obs("cam1", Some(1), 1000, &A)], &config);

        let output = run(
            &mut associator,
            &[
                lost(obs("cam1", Some(1), 2000, &B)),
                lost(obs("cam1", Some(2), 2000, &A)),
            ],
            &config,
        );
        assert_eq!(
            single_association(&output),
            (1, AssociationStatus::Tracked, 1.0)
        );
        let unresolved = single_unresolved(&output);
        assert_eq!(unresolved.track_id, Some(2));
        assert_eq!(unresolved.reason, UnresolvedReason::Lost);
        assert!(unresolved.candidates.is_empty() && unresolved.observations == 0);
        let entity = &associator.entities[&1];
        assert_eq!((entity.gallery.len(), entity.last_seen_ms), (1, 1000));
        assert_eq!(associator.bindings[&key("cam1", 1)].last_seen_ms, 1000);
        assert!(associator.pending.is_empty());
        assert_eq!(associator.clock_ms, Some(2000));

        let output = run(
            &mut associator,
            &[
                lost(obs("cam1", Some(1), 2100, &A)),
                obs("cam1", Some(3), 2100, &NEAR_A),
            ],
            &config,
        );
        let results: Vec<_> = output
            .associations
            .iter()
            .map(|a| (a.track_id, a.entity_id, a.status))
            .collect();
        assert_eq!(
            results,
            vec![
                (Some(1), 1, AssociationStatus::Tracked),
                (Some(3), 1, AssociationStatus::Matched),
            ],
            "a lost track does not occupy its entity"
        );

        let output = run(
            &mut associator,
            &[lost(obs("cam1", Some(1), 11_500, &A))],
            &config,
        );
        assert_eq!(single_unresolved(&output).reason, UnresolvedReason::Lost);
        assert_eq!(
            output.entity_count, 1,
            "lost sightings never refreshed the binding"
        );
    }

    #[test]
    fn bound_track_without_embedding_keeps_its_entity() {
        let mut associator = Associator::new();
        let config = config(1);
        run(&mut associator, &[obs("cam1", Some(1), 1000, &A)], &config);
        assert_eq!(associator.bindings[&key("cam1", 1)].similarity, 1.0);
        run(
            &mut associator,
            &[obs("cam1", Some(1), 2000, &NEAR_A)],
            &config,
        );
        let stored = associator.bindings[&key("cam1", 1)].similarity;
        assert!(stored > 0.9 && stored < 1.0, "{stored}");

        let output = run(
            &mut associator,
            &[
                obs("cam1", Some(1), 3000, &[]),
                obs("cam1", Some(2), 3000, &NEAR_A),
                obs("cam1", Some(5), 3000, &[]),
            ],
            &config,
        );
        let results: Vec<_> = output
            .associations
            .iter()
            .map(|a| (a.track_id, a.entity_id, a.status, a.similarity))
            .collect();
        assert_eq!(
            results,
            vec![
                (Some(1), 1, AssociationStatus::Tracked, stored),
                (Some(2), 2, AssociationStatus::Created, 1.0),
            ],
            "the embedding-less track still occupies entity 1"
        );
        let unresolved = single_unresolved(&output);
        assert_eq!(
            (unresolved.track_id, unresolved.reason),
            (Some(5), UnresolvedReason::MissingEmbedding)
        );
        let entity = &associator.entities[&1];
        assert_eq!((entity.gallery.len(), entity.last_seen_ms), (2, 3000));
        assert_eq!(associator.bindings[&key("cam1", 1)].last_seen_ms, 3000);

        let output = run(
            &mut associator,
            &[
                obs("cam1", Some(1), 4000, &A),
                obs("cam1", Some(1), 4033, &[]),
            ],
            &config,
        );
        let fresh = associator.bindings[&key("cam1", 1)].similarity;
        assert!(
            output
                .associations
                .iter()
                .all(|a| a.entity_id == 1 && a.similarity == fresh)
                && output.associations.len() == 2
        );
        assert_eq!(associator.bindings[&key("cam1", 1)].last_seen_ms, 4033);
    }

    #[test]
    fn observations_without_usable_embeddings_are_reported() {
        let mut associator = Associator::new();
        let config = config(3);
        let output = run(
            &mut associator,
            &[
                obs("cam1", Some(1), 1000, &[]),
                obs("cam1", Some(2), 1000, &[0.0, 0.0]),
                obs("cam1", None, 1000, &[f32::NAN, 1.0]),
            ],
            &config,
        );
        assert!(output.associations.is_empty());
        assert_eq!(output.unresolved.len(), 3);
        assert!(
            output
                .unresolved
                .iter()
                .all(|u| u.reason == UnresolvedReason::MissingEmbedding && u.observations == 0)
        );
        assert!(associator.pending.is_empty() && associator.dimension.is_none());

        let output = run(
            &mut associator,
            &[
                obs("cam1", Some(1), 2000, &A),
                obs("cam1", Some(1), 2033, &[]),
            ],
            &config,
        );
        assert_eq!(output.unresolved[0].reason, UnresolvedReason::Pending);
        assert_eq!(
            output.unresolved[1].reason,
            UnresolvedReason::MissingEmbedding
        );
        assert_eq!(output.unresolved[1].observations, 1);
    }

    #[test]
    fn untracked_observations_match_but_never_create() {
        let mut associator = Associator::new();
        let config = config(1);
        let output = run(&mut associator, &[obs("cam1", None, 1000, &A)], &config);
        assert_eq!(
            single_unresolved(&output).reason,
            UnresolvedReason::Untracked
        );
        assert_eq!(output.entity_count, 0);

        run(&mut associator, &[obs("cam1", Some(1), 2000, &A)], &config);
        let output = run(
            &mut associator,
            &[
                obs("cam2", None, 3000, &NEAR_A),
                obs("cam2", None, 3000, &C),
            ],
            &config,
        );
        let (entity_id, status, _) = single_association(&output);
        assert_eq!((entity_id, status), (1, AssociationStatus::Matched));
        let unresolved = single_unresolved(&output);
        assert_eq!(unresolved.reason, UnresolvedReason::Untracked);
        assert_eq!(unresolved.candidates.len(), 1);
        assert_eq!(unresolved.observations, 0);
        assert_eq!(output.entity_count, 1);
        assert!(associator.pending.is_empty() && associator.bindings.len() == 1);
    }

    #[test]
    fn untracked_observation_cannot_take_an_entity_held_by_any_tracker_on_its_camera() {
        let mut associator = Associator::new();
        let config = config(1);
        run(
            &mut associator,
            &[with_tracker(obs("cam1", Some(1), 1000, &A), "t1")],
            &config,
        );
        let output = run(
            &mut associator,
            &[
                with_tracker(obs("cam1", Some(1), 2000, &A), "t1"),
                obs("cam1", None, 2000, &NEAR_A),
            ],
            &config,
        );
        assert_eq!(output.associations.len(), 1);
        assert_eq!(output.associations[0].entity_id, 1);
        assert_eq!(output.associations[0].track_id, Some(1));
        assert_eq!(output.unresolved.len(), 1);
        assert_eq!(output.unresolved[0].reason, UnresolvedReason::Untracked);
    }

    #[test]
    fn untracked_observation_cannot_take_an_entity_occupied_on_its_camera() {
        let mut associator = Associator::new();
        let config = config(1);
        run(&mut associator, &[obs("cam1", Some(1), 1000, &A)], &config);
        let output = run(
            &mut associator,
            &[
                obs("cam1", Some(1), 2000, &A),
                obs("cam1", None, 2000, &NEAR_A),
            ],
            &config,
        );
        assert_eq!(single_association(&output).1, AssociationStatus::Tracked);
        assert_eq!(
            single_unresolved(&output).reason,
            UnresolvedReason::Untracked
        );
    }

    #[test]
    fn several_frames_of_one_track_resolve_together() {
        let mut associator = Associator::new();
        let config = config(3);
        let frames: Vec<_> = (0..3)
            .map(|frame| {
                let mut observation = obs("cam1", Some(4), 1000 + 33 * frame, &A);
                observation.detection_index = Some(frame as u32);
                observation.bbox.x1 = frame as f32;
                observation
            })
            .collect();
        let output = run(&mut associator, &frames, &config);
        assert_eq!(output.associations.len(), 3);
        for (frame, association) in output.associations.iter().enumerate() {
            assert_eq!(association.entity_id, 1);
            assert_eq!(association.status, AssociationStatus::Created);
            assert_eq!(association.detection_index, Some(frame as u32));
            assert_eq!(association.timestamp_ms, 1000 + 33 * frame as i64);
            assert_eq!(association.bbox.x1, frame as f32);
        }
        assert_eq!(output.entity_count, 1);
        assert_eq!(associator.entities[&1].gallery.len(), 1);

        let output = run(&mut associator, &frames[..2], &config);
        assert!(
            output
                .associations
                .iter()
                .all(|a| a.entity_id == 1 && a.status == AssociationStatus::Tracked)
        );
    }

    #[test]
    fn bound_track_updates_gallery_only_when_similar() {
        let mut associator = Associator::new();
        let config = config(1);
        run(&mut associator, &[obs("cam1", Some(1), 1000, &A)], &config);

        let output = run(&mut associator, &[obs("cam1", Some(1), 2000, &B)], &config);
        let (entity_id, status, similarity) = single_association(&output);
        assert_eq!((entity_id, status), (1, AssociationStatus::Tracked));
        assert!(similarity.abs() < 1e-6);
        assert_eq!(associator.entities[&1].gallery.len(), 1);
        assert_eq!(associator.entities[&1].last_seen_ms, 2000);

        for ts in 0..20 {
            run(
                &mut associator,
                &[obs("cam1", Some(1), 3000 + ts, &NEAR_A)],
                &config,
            );
        }
        let entity = &associator.entities[&1];
        assert_eq!(entity.gallery.len(), GALLERY_SIZE);
        assert!(cosine(&entity.prototype, &normalized(&NEAR_A).unwrap()) > 0.999);
    }

    #[test]
    fn pending_candidates_are_the_closest_three() {
        let mut associator = Associator::new();
        run(
            &mut associator,
            &[
                obs("cam1", Some(1), 1000, &A),
                obs("cam1", Some(2), 1000, &B),
                obs("cam1", Some(3), 1000, &C),
                obs("cam1", Some(4), 1000, &D),
            ],
            &config(1),
        );
        let output = run(
            &mut associator,
            &[obs("cam2", Some(1), 2000, &[1.0, 1.0, 0.5, 0.0])],
            &config(3),
        );
        let unresolved = single_unresolved(&output);
        assert_eq!(unresolved.reason, UnresolvedReason::Pending);
        let candidates: Vec<u64> = unresolved.candidates.iter().map(|c| c.entity_id).collect();
        assert_eq!(candidates, vec![1, 2, 3]);
        assert!(unresolved.candidates[0].similarity >= unresolved.candidates[2].similarity);
    }

    #[test]
    fn timestamps_fall_back_to_the_current_time() {
        let mut associator = Associator::new();
        let output = associator
            .associate(&[obs("cam1", Some(1), 0, &A)], &config(1), 77_000)
            .unwrap();
        assert_eq!(output.associations[0].timestamp_ms, 77_000);
        assert_eq!(associator.entities[&1].last_seen_ms, 77_000);
    }

    #[test]
    fn config_errors_name_the_pin() {
        let cases = [
            (AssociationConfig::new(1.5, 0.05, 3, 1000), "min_similarity"),
            (
                AssociationConfig::new(f64::NAN, 0.05, 3, 1000),
                "min_similarity",
            ),
            (
                AssociationConfig::new(0.6, -0.1, 3, 1000),
                "ambiguity_margin",
            ),
            (
                AssociationConfig::new(0.6, 0.05, 0, 1000),
                "min_observations",
            ),
            (
                AssociationConfig::new(0.6, 0.05, -2, 1000),
                "min_observations",
            ),
            (AssociationConfig::new(0.6, 0.05, 3, 0), "entity_ttl_ms"),
        ];
        for (result, pin) in cases {
            let error = result.unwrap_err().to_string();
            assert!(error.contains(pin), "{error}");
        }
        assert!(AssociationConfig::new(0.0, 1.0, 1, 1).is_ok());
    }
}
