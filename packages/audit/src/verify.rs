//! Verification of chains and of the epoch timeline.
//!
//! Verification reads short, independent batches and remembers authenticated progress.
//! Incremental checks recheck the cached boundary and new data; full checks reread the
//! retained history. Any failed check invalidates cached proof progress.

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex, PoisonError};

use sea_orm::{
    ActiveEnum, ColumnTrait, ConnectionTrait, DbErr, EntityTrait, Order, PaginatorTrait,
    QueryFilter, QueryOrder, QuerySelect,
};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::entity::{audit_epoch, audit_held_chain, audit_record, audit_seal, audit_watermark};

use super::crypto::{
    EpochFields, Hash, RecordFields, SealFields, ZERO_HASH, details_commitment, epoch_hash,
    ip_commitment, mac_matches, record_hash, seal_hash, seal_mac_matches, to_hash,
    watermark_batch_hash, watermark_hash,
};
use super::keys::accepted_entry_keys;
use super::merkle;
use super::record::chain_class;
use super::signer::{self, SignatureCheck};

/// `sealId` of a pending record whose MAC failed; the sealer never seals it.
pub const INVALID_SEAL_ID: &str = "invalid";
/// Watermark row of the epoch timeline.
pub const EPOCH_WATERMARK: &str = "~epochs";

const SEAL_BATCH: u64 = 200;
const RECORD_BATCH: i64 = 5_000;
const PENDING_SAMPLE: u64 = 1_000;

#[derive(Clone, Debug, Default, Serialize, Deserialize, ToSchema)]
pub struct ChainReport {
    pub chain_id: String,
    /// Every checked seal, record and epoch is intact and every epoch signature verified.
    pub valid: bool,
    /// Sequence of the first seal that failed a check.
    pub first_broken_seal: Option<i64>,
    /// What failed first, for operators.
    pub problem: Option<String>,
    pub seals_checked: u64,
    pub records_checked: u64,
    /// Personal values that expired; their commitments still verify.
    pub redacted_values: u64,
    /// Seals no epoch covers yet. They become anchored within one worker run.
    pub unanchored_seals: u64,
    /// Epochs signed with a key id that has no registered public key.
    pub unverifiable_epochs: u64,
    pub pending_records: u64,
    /// Pending records whose MAC does not verify, or that the worker quarantined.
    pub pending_invalid: u64,
    /// Seals up to this sequence were archived and pruned.
    pub pruned_before_seq: Option<i64>,
    pub latest_seal_seq: Option<i64>,
    /// Newest epoch that covers a seal of this chain.
    pub latest_epoch_seq: Option<i64>,
    /// Neither seals nor pending records exist. A deleted chain looks the same.
    pub empty: bool,
    /// First seal sequence this run checked. Seals before it were verified by an earlier
    /// run in this server process, or pruned; `full` re-checks from the watermark.
    pub checked_from_seq: i64,
    /// A seal of this chain failed its hash or MAC before an epoch signed it; the audit
    /// worker signs, archives and prunes none of its seals until an operator resolves it.
    pub held: bool,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, ToSchema)]
pub struct EpochReport {
    pub valid: bool,
    pub first_broken_epoch: Option<i64>,
    pub problem: Option<String>,
    pub epochs_checked: u64,
    pub unverifiable_epochs: u64,
    pub pruned_before_seq: Option<i64>,
    pub latest_epoch_seq: Option<i64>,
    pub latest_epoch_hash: Option<String>,
}

/// One revision covers both caches because chain proofs depend on epoch signatures.
static VERIFIED: LazyLock<Mutex<ProgressCache>> =
    LazyLock::new(|| Mutex::new(ProgressCache::default()));

#[derive(Default)]
struct ProgressCache {
    revision: u64,
    chains: HashMap<String, Progress>,
    epoch: Option<EpochProgress>,
    // Keep authenticated boundaries after invalidation so deletion cannot become an
    // apparently empty, valid history on the next request.
    retained_chains: HashMap<String, Progress>,
    retained_epoch: Option<EpochProgress>,
}

impl ProgressCache {
    fn invalidate(&mut self) {
        self.revision = self.revision.wrapping_add(1);
        self.chains.clear();
        self.epoch = None;
    }

    fn remember_chain(&mut self, revision: u64, chain_id: &str, progress: Progress) {
        if revision == self.revision
            && self
                .chains
                .get(chain_id)
                .is_none_or(|old| progress.seq >= old.seq)
        {
            self.chains.insert(chain_id.to_owned(), progress);
            if self
                .retained_chains
                .get(chain_id)
                .is_none_or(|old| progress.seq >= old.seq)
            {
                self.retained_chains.insert(chain_id.to_owned(), progress);
            }
        }
    }

    fn remember_epoch(&mut self, revision: u64, progress: EpochProgress) {
        if revision == self.revision && self.epoch.is_none_or(|old| progress.seq >= old.seq) {
            self.epoch = Some(progress);
            if self
                .retained_epoch
                .is_none_or(|old| progress.seq >= old.seq)
            {
                self.retained_epoch = Some(progress);
            }
        }
    }

    /// An older successful check must not restore success after another check failed.
    fn complete(&mut self, revision: u64, valid: bool) -> bool {
        if !valid {
            self.invalidate();
            return false;
        }
        revision == self.revision
    }
}

impl ChainReport {
    fn broken(&mut self, seq: i64, problem: impl Into<String>) {
        self.valid = false;
        if self.first_broken_seal.is_none() {
            self.first_broken_seal = Some(seq);
            self.problem = Some(problem.into());
        }
    }
}

/// Canonical order of a seal's records. Sorted in code, never by database collation,
/// so the sealer, the verifier and offline tools always agree.
pub fn sort_records<T: std::borrow::Borrow<audit_record::Model>>(records: &mut [T]) {
    records.sort_by(|left, right| {
        let (left, right) = (left.borrow(), right.borrow());
        (left.timestamp.timestamp_millis(), left.id.as_bytes())
            .cmp(&(right.timestamp.timestamp_millis(), right.id.as_bytes()))
    });
}

/// Hash a stored record and check its commitments against any raw values still present.
/// Returns the record hash and how many values were redacted, or what is wrong.
pub fn check_record(record: &audit_record::Model) -> Result<(Hash, u64), String> {
    let mut redacted = 0;
    match (&record.actor_ip, &record.ip_salt, &record.ip_commitment) {
        (Some(ip), Some(salt), Some(commitment)) => {
            let salt = to_hash(salt).ok_or("record IP salt has the wrong length")?;
            if ip_commitment(&salt, ip).as_slice() != commitment.as_slice() {
                return Err(format!(
                    "record {} IP does not match its commitment",
                    record.id
                ));
            }
        }
        (None, None, Some(_)) => redacted += 1,
        (None, None, None) => {}
        _ => return Err(format!("record {} has an incomplete IP triple", record.id)),
    }
    match (
        &record.details,
        &record.details_salt,
        &record.details_commitment,
    ) {
        (Some(details), Some(salt), Some(commitment)) => {
            let salt = to_hash(salt).ok_or("record details salt has the wrong length")?;
            if details_commitment(&salt, details).as_slice() != commitment.as_slice() {
                return Err(format!(
                    "record {} details do not match their commitment",
                    record.id
                ));
            }
        }
        (None, None, Some(_)) => redacted += 1,
        (None, None, None) => {}
        _ => return Err(format!("record {} has incomplete details", record.id)),
    }
    let actor_type = record.actor_type.to_value();
    let hash = record_hash(&RecordFields {
        id: &record.id,
        chain_id: &record.chain_id,
        timestamp_ms: record.timestamp.timestamp_millis(),
        actor_id: &record.actor_id,
        actor_type: &actor_type,
        action: &record.action,
        resource_type: &record.resource_type,
        resource_id: &record.resource_id,
        ip_commitment: record.ip_commitment.as_deref(),
        details_commitment: record.details_commitment.as_deref(),
    });
    Ok((hash, redacted))
}

pub fn seal_hash_of(seal: &audit_seal::Model) -> Hash {
    seal_hash(&SealFields {
        id: &seal.id,
        chain_id: &seal.chain_id,
        seq: seal.seq,
        class: &seal.class,
        prev_hash: &seal.prev_hash,
        record_count: i64::from(seal.record_count),
        records_root: &seal.records_root,
        first_at_ms: seal.first_at.timestamp_millis(),
        last_at_ms: seal.last_at.timestamp_millis(),
        sealed_at_ms: seal.sealed_at.timestamp_millis(),
    })
}

pub fn epoch_hash_of(epoch: &audit_epoch::Model) -> Hash {
    epoch_hash(&EpochFields {
        seq: epoch.seq,
        prev_hash: &epoch.prev_hash,
        seal_count: i64::from(epoch.seal_count),
        seals_root: &epoch.seals_root,
        created_at_ms: epoch.created_at.timestamp_millis(),
        kid: &epoch.kid,
    })
}

pub fn watermark_hash_of(watermark: &audit_watermark::Model) -> Hash {
    watermark_hash(
        &watermark.chain_id,
        watermark.seq,
        &watermark.hash,
        watermark.pruned_at.timestamp_millis(),
    )
}

/// Check an epoch on its own: stored hash and signature. `Err` names the problem;
/// `Ok(false)` means the key id has no registered public key.
pub fn check_epoch(epoch: &audit_epoch::Model) -> Result<bool, String> {
    let hash = epoch_hash_of(epoch);
    if hash.as_slice() != epoch.hash.as_slice() {
        return Err(format!(
            "epoch {} hash does not match its fields",
            epoch.seq
        ));
    }
    match signer::verify(&epoch.kid, &hash, &epoch.signature) {
        SignatureCheck::Valid => Ok(true),
        SignatureCheck::Invalid => Err(format!("epoch {} signature is invalid", epoch.seq)),
        SignatureCheck::Unavailable => Ok(false),
    }
}

/// Check a watermark: its inclusion in its prune run's batch and the batch signature.
/// `Ok(false)` means the key is unavailable.
pub fn check_watermark(watermark: &audit_watermark::Model) -> Result<bool, String> {
    let invalid = || {
        format!(
            "watermark of {} is not part of its batch",
            watermark.chain_id
        )
    };
    let root = to_hash(&watermark.batch_root).ok_or_else(invalid)?;
    let proof = merkle::decode_proof(&watermark.batch_proof).ok_or_else(invalid)?;
    if watermark.batch_index < 0
        || !merkle::verify_inclusion(
            &watermark_hash_of(watermark),
            watermark.batch_index as u64,
            watermark.batch_size.max(0) as u64,
            &proof,
            &root,
        )
    {
        return Err(invalid());
    }
    let batch = watermark_batch_hash(
        &root,
        i64::from(watermark.batch_size),
        watermark.pruned_at.timestamp_millis(),
        &watermark.kid,
    );
    match signer::verify(&watermark.kid, &batch, &watermark.signature) {
        SignatureCheck::Valid => Ok(true),
        SignatureCheck::Invalid => Err(format!(
            "watermark of {} has an invalid signature",
            watermark.chain_id
        )),
        SignatureCheck::Unavailable => Ok(false),
    }
}

/// Verify one chain. `full` ignores the cached progress.
pub async fn verify_chain<C: ConnectionTrait>(
    db: &C,
    chain_id: &str,
    full: bool,
) -> Result<ChainReport, DbErr> {
    let (revision, cached, resume) = {
        let mut cache = VERIFIED.lock().unwrap_or_else(PoisonError::into_inner);
        let cached = cache.retained_chains.get(chain_id).copied();
        let resume = (!full)
            .then(|| cache.chains.get(chain_id).copied())
            .flatten();
        if full {
            cache.chains.remove(chain_id);
        }
        (cache.revision, cached, resume)
    };
    let mut result = verify_chain_inner(db, chain_id, resume, revision, cached).await;
    let valid = result.as_ref().is_ok_and(|report| report.valid);
    let accepted = VERIFIED
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .complete(revision, valid);
    if valid
        && !accepted
        && let Ok(report) = &mut result
    {
        report.valid = false;
        report.problem =
            Some("another verification invalidated cached proof progress; retry".into());
    }
    result
}

async fn verify_chain_inner<C: ConnectionTrait>(
    db: &C,
    chain_id: &str,
    resume: Option<Progress>,
    revision: u64,
    cached: Option<Progress>,
) -> Result<ChainReport, DbErr> {
    let mut report = ChainReport {
        chain_id: chain_id.to_owned(),
        valid: true,
        ..Default::default()
    };
    let class = chain_class(chain_id).as_str();
    let keys: Vec<Hash> = accepted_entry_keys()
        .into_iter()
        .map(|(_, key)| key)
        .collect();
    report.held = audit_held_chain::Entity::find_by_id(chain_id)
        .one(db)
        .await?
        .is_some();
    // Without any epoch (no audit key) seals are only MAC-protected; their progress is
    // cached too, so a keyless deployment does not re-read whole chains on every check.
    let keyless = audit_epoch::Entity::find().one(db).await?.is_none();

    let watermark = audit_watermark::Entity::find_by_id(chain_id)
        .one(db)
        .await?;
    let (mut expected_seq, mut prev_hash) = match &watermark {
        Some(watermark) => {
            match check_watermark(watermark) {
                Ok(true) => {}
                Ok(false) => {
                    report.valid = false;
                    report.problem = Some("watermark key unavailable".into());
                }
                Err(problem) => report.broken(watermark.seq, problem),
            }
            report.pruned_before_seq = Some(watermark.seq);
            let hash = to_hash(&watermark.hash).unwrap_or(ZERO_HASH);
            (watermark.seq + 1, hash)
        }
        None => (1, ZERO_HASH),
    };
    let mut cached_boundary_seen = cached.is_none_or(|progress| progress.seq < expected_seq);
    if let Some(progress) = resume
        && progress.seq >= expected_seq
        && (progress.anchored || keyless)
    {
        // Recheck the boundary itself, including its records and epoch, before advancing.
        expected_seq = progress.seq;
        prev_hash = progress.prev_hash;
    }
    report.checked_from_seq = expected_seq;

    let mut epochs: HashMap<i64, LoadedEpoch> = HashMap::new();
    let mut progress: Option<Progress> = None;
    let mut all_anchored = true;

    'seals: loop {
        let seals = audit_seal::Entity::find()
            .filter(audit_seal::Column::ChainId.eq(chain_id))
            .filter(audit_seal::Column::Seq.gte(expected_seq))
            .order_by(audit_seal::Column::Seq, Order::Asc)
            .limit(SEAL_BATCH)
            .all(db)
            .await?;
        if seals.is_empty() {
            break;
        }
        let full_batch = seals.len() as u64 == SEAL_BATCH;
        let mut start = 0;
        while start < seals.len() {
            let mut end = start;
            let mut budget = 0i64;
            while end < seals.len() && (end == start || budget < RECORD_BATCH) {
                budget += i64::from(seals[end].record_count);
                end += 1;
            }
            let group = &seals[start..end];
            let records = audit_record::Entity::find()
                .filter(
                    audit_record::Column::SealId.is_in(group.iter().map(|seal| seal.id.clone())),
                )
                .order_by(audit_record::Column::SealId, Order::Asc)
                .order_by(audit_record::Column::ChainId, Order::Asc)
                .order_by(audit_record::Column::Timestamp, Order::Asc)
                .order_by(audit_record::Column::Id, Order::Asc)
                .all(db)
                .await?;
            let mut by_seal: HashMap<&str, Vec<&audit_record::Model>> = HashMap::new();
            for record in &records {
                if let Some(seal_id) = record.seal_id.as_deref() {
                    by_seal.entry(seal_id).or_default().push(record);
                }
            }
            let missing: Vec<i64> = group
                .iter()
                .filter_map(|seal| seal.epoch_seq)
                .filter(|seq| !epochs.contains_key(seq))
                .collect::<std::collections::BTreeSet<_>>()
                .into_iter()
                .collect();
            if !missing.is_empty() {
                for epoch in audit_epoch::Entity::find()
                    .filter(audit_epoch::Column::Seq.is_in(missing))
                    .all(db)
                    .await?
                {
                    let check = check_epoch(&epoch);
                    epochs.insert(
                        epoch.seq,
                        LoadedEpoch {
                            epoch,
                            check,
                            counted: false,
                        },
                    );
                }
            }

            for seal in group {
                report.seals_checked += 1;
                report.latest_seal_seq = Some(seal.seq);
                if seal.seq != expected_seq {
                    report.broken(expected_seq, format!("seal {expected_seq} is missing"));
                    break 'seals;
                }
                if seal.prev_hash.as_slice() != prev_hash.as_slice() {
                    report.broken(seal.seq, "seal does not link to its predecessor");
                    break 'seals;
                }
                if seal.class != class {
                    report.broken(seal.seq, "seal class does not match its chain");
                    break 'seals;
                }
                let hash = seal_hash_of(seal);
                if let Some(progress) = cached
                    && seal.seq == progress.seq
                {
                    if hash != progress.hash {
                        report.broken(seal.seq, "previously verified seal changed");
                        break 'seals;
                    }
                    cached_boundary_seen = true;
                }
                if hash.as_slice() != seal.hash.as_slice() {
                    report.broken(seal.seq, "seal hash does not match its fields");
                    break 'seals;
                }

                let mut members = by_seal.remove(seal.id.as_str()).unwrap_or_default();
                sort_records(&mut members);
                if members.len() != seal.record_count as usize {
                    report.broken(
                        seal.seq,
                        format!(
                            "seal holds {} records but {} remain",
                            seal.record_count,
                            members.len()
                        ),
                    );
                    break 'seals;
                }
                let mut leaves = Vec::with_capacity(members.len());
                for record in &members {
                    if record.chain_id != chain_id {
                        report.broken(seal.seq, format!("record {} moved chains", record.id));
                        break 'seals;
                    }
                    if record.mac.is_some() {
                        report.broken(seal.seq, format!("sealed record {} kept a MAC", record.id));
                        break 'seals;
                    }
                    match check_record(record) {
                        Ok((hash, redacted)) => {
                            report.redacted_values += redacted;
                            leaves.push(hash);
                        }
                        Err(problem) => {
                            report.broken(seal.seq, problem);
                            break 'seals;
                        }
                    }
                }
                report.records_checked += members.len() as u64;
                if merkle::root(&leaves).as_slice() != seal.records_root.as_slice() {
                    report.broken(seal.seq, "records do not match the seal's root");
                    break 'seals;
                }
                if members.first().map(|record| record.timestamp) != Some(seal.first_at)
                    || members.last().map(|record| record.timestamp) != Some(seal.last_at)
                {
                    report.broken(seal.seq, "record times fall outside the seal");
                    break 'seals;
                }

                match (
                    seal.epoch_seq,
                    seal.epoch_index,
                    seal.epoch_proof.as_deref(),
                ) {
                    (Some(epoch_seq), Some(index), Some(proof)) => {
                        let Some(loaded) = epochs.get_mut(&epoch_seq) else {
                            report.broken(seal.seq, format!("epoch {epoch_seq} is missing"));
                            break 'seals;
                        };
                        match &loaded.check {
                            Ok(true) => {}
                            Ok(false) => {
                                if !loaded.counted {
                                    loaded.counted = true;
                                    report.unverifiable_epochs += 1;
                                }
                                report.valid = false;
                                report.problem.get_or_insert_with(|| {
                                    format!("epoch {epoch_seq} key unavailable")
                                });
                            }
                            Err(problem) => {
                                report.broken(seal.seq, problem.clone());
                                break 'seals;
                            }
                        }
                        let epoch = &loaded.epoch;
                        let Some(proof) = merkle::decode_proof(proof) else {
                            report.broken(seal.seq, "seal epoch proof is malformed");
                            break 'seals;
                        };
                        let Some(seals_root) = to_hash(&epoch.seals_root) else {
                            report.broken(seal.seq, "epoch root has the wrong length");
                            break 'seals;
                        };
                        if index < 0
                            || !merkle::verify_inclusion(
                                &hash,
                                index as u64,
                                epoch.seal_count as u64,
                                &proof,
                                &seals_root,
                            )
                        {
                            report.broken(seal.seq, "seal is not part of its epoch");
                            break 'seals;
                        }
                        report.latest_epoch_seq = Some(epoch_seq);
                        if all_anchored {
                            progress = Some(Progress {
                                seq: seal.seq,
                                hash,
                                prev_hash,
                                anchored: true,
                            });
                        }
                    }
                    (None, None, None) => {
                        // Until an epoch signs it, the seal is protected by the entry key.
                        let intact = seal.mac.as_deref().is_some_and(|mac| {
                            keys.iter().any(|key| seal_mac_matches(key, &hash, mac))
                        });
                        if !intact {
                            report.broken(seal.seq, "unanchored seal fails its MAC");
                            break 'seals;
                        }
                        report.unanchored_seals += 1;
                        if keyless && all_anchored {
                            progress = Some(Progress {
                                seq: seal.seq,
                                hash,
                                prev_hash,
                                anchored: false,
                            });
                        } else {
                            all_anchored = false;
                        }
                    }
                    _ => {
                        report.broken(seal.seq, "seal has a partial epoch reference");
                        break 'seals;
                    }
                }
                prev_hash = hash;
                expected_seq = seal.seq + 1;
            }
            start = end;
            // Saved per group, so a run cut off by a deadline keeps what it verified.
            if report.valid {
                if let Some(progress) = progress {
                    VERIFIED
                        .lock()
                        .unwrap_or_else(PoisonError::into_inner)
                        .remember_chain(revision, chain_id, progress);
                }
            }
        }
        if !full_batch {
            break;
        }
    }

    if !cached_boundary_seen && let Some(progress) = cached {
        report.broken(progress.seq, "previously verified seal is missing");
    }

    report.pending_records = audit_record::Entity::find()
        .filter(audit_record::Column::ChainId.eq(chain_id))
        .filter(audit_record::Column::SealId.is_null())
        .count(db)
        .await?;
    let pending = audit_record::Entity::find()
        .filter(audit_record::Column::ChainId.eq(chain_id))
        .filter(audit_record::Column::SealId.is_null())
        .order_by(audit_record::Column::Timestamp, Order::Desc)
        .limit(PENDING_SAMPLE)
        .all(db)
        .await?;
    for record in &pending {
        let intact = check_record(record).is_ok_and(|(hash, _)| {
            record
                .mac
                .as_deref()
                .is_some_and(|mac| keys.iter().any(|key| mac_matches(key, &hash, mac)))
        });
        if !intact {
            report.pending_invalid += 1;
        }
    }
    report.pending_invalid += audit_record::Entity::find()
        .filter(audit_record::Column::ChainId.eq(chain_id))
        .filter(audit_record::Column::SealId.eq(INVALID_SEAL_ID))
        .count(db)
        .await?;
    if report.pending_invalid > 0 {
        report.valid = false;
        report
            .problem
            .get_or_insert_with(|| "a pending record fails its integrity check".into());
    }

    if report.seals_checked == 0 {
        let latest = audit_seal::Entity::find()
            .filter(audit_seal::Column::ChainId.eq(chain_id))
            .order_by(audit_seal::Column::Seq, Order::Desc)
            .one(db)
            .await?;
        if let Some(latest) = latest {
            report.latest_seal_seq = Some(latest.seq);
            report.latest_epoch_seq = latest.epoch_seq;
        }
    }
    report.empty = report.latest_seal_seq.is_none()
        && report.pending_records == 0
        && report.pruned_before_seq.is_none();

    Ok(report)
}

/// Progress of one chain: its last verified seal, and whether an epoch covered every
/// seal up to it. Unanchored progress counts only while no epoch exists at all.
#[derive(Clone, Copy, Debug)]
struct Progress {
    seq: i64,
    hash: Hash,
    prev_hash: Hash,
    anchored: bool,
}

#[derive(Clone, Copy, Debug)]
struct EpochProgress {
    seq: i64,
    hash: Hash,
    prev_hash: Hash,
}

struct LoadedEpoch {
    epoch: audit_epoch::Model,
    check: Result<bool, String>,
    counted: bool,
}

/// Verify the epoch timeline from its watermark: continuity, hashes and signatures.
/// `full` ignores cached progress; otherwise the last verified epoch and its successors
/// are read, so a deleted or changed boundary cannot pass an incremental check.
pub async fn verify_epochs<C: ConnectionTrait>(db: &C, full: bool) -> Result<EpochReport, DbErr> {
    let (revision, cached, resume) = {
        let mut cache = VERIFIED.lock().unwrap_or_else(PoisonError::into_inner);
        let cached = cache.retained_epoch;
        let resume = (!full).then_some(cache.epoch).flatten();
        if full {
            cache.chains.clear();
            cache.epoch = None;
        }
        (cache.revision, cached, resume)
    };
    let mut result = verify_epochs_inner(db, resume, revision, cached).await;
    let valid = result.as_ref().is_ok_and(|report| report.valid);
    let accepted = VERIFIED
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .complete(revision, valid);
    if valid
        && !accepted
        && let Ok(report) = &mut result
    {
        report.valid = false;
        report.problem =
            Some("another verification invalidated cached proof progress; retry".into());
    }
    result
}

async fn verify_epochs_inner<C: ConnectionTrait>(
    db: &C,
    resume: Option<EpochProgress>,
    revision: u64,
    cached: Option<EpochProgress>,
) -> Result<EpochReport, DbErr> {
    let mut report = EpochReport {
        valid: true,
        ..Default::default()
    };
    let broken = |report: &mut EpochReport, seq: i64, problem: String| {
        report.valid = false;
        if report.first_broken_epoch.is_none() {
            report.first_broken_epoch = Some(seq);
            report.problem = Some(problem);
        }
    };
    let watermark = audit_watermark::Entity::find_by_id(EPOCH_WATERMARK)
        .one(db)
        .await?;
    let (mut expected_seq, mut prev_hash) = match &watermark {
        Some(watermark) => {
            match check_watermark(watermark) {
                Ok(true) => {}
                Ok(false) => report.valid = false,
                Err(problem) => broken(&mut report, watermark.seq, problem),
            }
            report.pruned_before_seq = Some(watermark.seq);
            (
                watermark.seq + 1,
                to_hash(&watermark.hash).unwrap_or(ZERO_HASH),
            )
        }
        None => (1, ZERO_HASH),
    };
    let mut cached_boundary_seen = cached.is_none_or(|progress| progress.seq < expected_seq);
    if let Some(progress) = resume
        && progress.seq >= expected_seq
    {
        expected_seq = progress.seq;
        prev_hash = progress.prev_hash;
    }
    let mut progress = None;
    loop {
        let epochs = audit_epoch::Entity::find()
            .filter(audit_epoch::Column::Seq.gte(expected_seq))
            .order_by(audit_epoch::Column::Seq, Order::Asc)
            .limit(SEAL_BATCH * 5)
            .all(db)
            .await?;
        if epochs.is_empty() {
            break;
        }
        let full_batch = epochs.len() as u64 == SEAL_BATCH * 5;
        for epoch in &epochs {
            report.epochs_checked += 1;
            if epoch.seq != expected_seq {
                broken(
                    &mut report,
                    expected_seq,
                    format!("epoch {expected_seq} is missing"),
                );
                return Ok(report);
            }
            if epoch.prev_hash.as_slice() != prev_hash.as_slice() {
                broken(
                    &mut report,
                    epoch.seq,
                    "epoch does not link to its predecessor".into(),
                );
                return Ok(report);
            }
            if let Some(cached) = cached
                && epoch.seq == cached.seq
            {
                if epoch.hash.as_slice() != cached.hash.as_slice() {
                    broken(
                        &mut report,
                        epoch.seq,
                        "previously verified epoch changed".into(),
                    );
                    return Ok(report);
                }
                cached_boundary_seen = true;
            }
            match check_epoch(epoch) {
                Ok(true) => {}
                Ok(false) => {
                    report.unverifiable_epochs += 1;
                    report.valid = false;
                }
                Err(problem) => {
                    broken(&mut report, epoch.seq, problem);
                    return Ok(report);
                }
            }
            let hash = to_hash(&epoch.hash).unwrap_or(ZERO_HASH);
            progress = Some(EpochProgress {
                seq: epoch.seq,
                hash,
                prev_hash,
            });
            prev_hash = hash;
            expected_seq = epoch.seq + 1;
            report.latest_epoch_seq = Some(epoch.seq);
            report.latest_epoch_hash = Some(hex::encode(&epoch.hash));
        }
        if !full_batch {
            break;
        }
    }
    if !cached_boundary_seen && let Some(cached) = cached {
        broken(
            &mut report,
            cached.seq,
            "previously verified epoch is missing".into(),
        );
    }
    if report.valid
        && let Some(progress) = progress
    {
        VERIFIED
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remember_epoch(revision, progress);
    }
    Ok(report)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum HeadCheck {
    /// The epoch exists with that hash.
    Matches,
    /// No epoch with that hash exists at that sequence: the timeline was truncated or
    /// rewritten after the head was retained.
    Differs,
    /// The epoch was archived and pruned; check it against the monthly archive.
    Archived,
}

/// A chain's seal retained together with a head.
#[derive(Clone, Copy, Debug)]
pub struct RetainedSeal<'a> {
    pub chain_id: &'a str,
    pub seq: i64,
    pub hash: &'a [u8],
}

/// Compare a head retained earlier with the timeline, and with it the chain's seal when
/// one was retained: an epoch covers many chains and stays intact when one chain loses
/// its newest seals, the retained seal does not.
pub async fn check_head<C: ConnectionTrait>(
    db: &C,
    seq: i64,
    hash: &[u8],
    seal: Option<RetainedSeal<'_>>,
) -> Result<HeadCheck, DbErr> {
    let epoch = match audit_epoch::Entity::find_by_id(seq).one(db).await? {
        Some(epoch) if epoch.hash.as_slice() == hash => HeadCheck::Matches,
        Some(_) => HeadCheck::Differs,
        None => {
            let watermark = audit_watermark::Entity::find_by_id(EPOCH_WATERMARK)
                .one(db)
                .await?;
            against_watermark(watermark.as_ref(), seq, hash)
        }
    };
    let Some(seal) = seal else {
        return Ok(epoch);
    };
    let chain = match audit_seal::Entity::find()
        .filter(audit_seal::Column::ChainId.eq(seal.chain_id))
        .filter(audit_seal::Column::Seq.eq(seal.seq))
        .one(db)
        .await?
    {
        Some(stored) if stored.hash.as_slice() == seal.hash => HeadCheck::Matches,
        Some(_) => HeadCheck::Differs,
        None => {
            let watermark = audit_watermark::Entity::find_by_id(seal.chain_id)
                .one(db)
                .await?;
            against_watermark(watermark.as_ref(), seal.seq, seal.hash)
        }
    };
    Ok(match (epoch, chain) {
        (HeadCheck::Differs, _) | (_, HeadCheck::Differs) => HeadCheck::Differs,
        (HeadCheck::Matches, HeadCheck::Matches) => HeadCheck::Matches,
        _ => HeadCheck::Archived,
    })
}

/// A retained `(seq, hash)` that is no longer stored: only a watermark whose signature
/// verifies may say it was pruned.
fn against_watermark(
    watermark: Option<&audit_watermark::Model>,
    seq: i64,
    hash: &[u8],
) -> HeadCheck {
    match watermark {
        Some(watermark) if check_watermark(watermark) != Ok(true) => HeadCheck::Differs,
        Some(watermark) if watermark.seq == seq && watermark.hash.as_slice() == hash => {
            HeadCheck::Matches
        }
        Some(watermark) if watermark.seq > seq => HeadCheck::Archived,
        _ => HeadCheck::Differs,
    }
}

/// Forget cached progress, for tests and after a prune rewrote a watermark. `None`
/// forgets every chain and the epoch timeline.
pub fn forget_progress(chain_id: Option<&str>) {
    let mut cache = VERIFIED.lock().unwrap_or_else(PoisonError::into_inner);
    // A pruned epoch also invalidates chains that used its signature.
    if let Some(chain_id) = chain_id.filter(|chain| *chain != EPOCH_WATERMARK) {
        cache.revision = cache.revision.wrapping_add(1);
        cache.chains.remove(chain_id);
        cache.retained_chains.remove(chain_id);
    } else {
        cache.invalidate();
        cache.retained_chains.clear();
        cache.retained_epoch = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn progress(seq: i64) -> Progress {
        Progress {
            seq,
            hash: [seq as u8; 32],
            prev_hash: ZERO_HASH,
            anchored: true,
        }
    }

    #[test]
    fn failed_check_invalidates_chains_and_the_epoch_together() {
        let mut cache = ProgressCache::default();
        cache.remember_chain(0, "app", progress(1));
        cache.remember_epoch(
            0,
            EpochProgress {
                seq: 1,
                hash: [1; 32],
                prev_hash: ZERO_HASH,
            },
        );
        assert!(!cache.complete(0, false));
        assert!(cache.chains.is_empty());
        assert!(cache.epoch.is_none());
        assert_eq!(cache.revision, 1);
        assert_eq!(cache.retained_chains["app"].seq, 1);
        assert_eq!(cache.retained_epoch.unwrap().seq, 1);
    }

    #[test]
    fn concurrent_old_success_cannot_restore_invalidated_progress() {
        let mut cache = ProgressCache::default();
        let old_revision = cache.revision;
        cache.complete(old_revision, false);
        cache.remember_chain(old_revision, "app", progress(2));
        cache.remember_epoch(
            old_revision,
            EpochProgress {
                seq: 2,
                hash: [2; 32],
                prev_hash: ZERO_HASH,
            },
        );
        assert!(!cache.complete(old_revision, true));
        assert!(cache.chains.is_empty());
        assert!(cache.epoch.is_none());
        cache.remember_chain(cache.revision, "app", progress(2));
        assert!(cache.complete(cache.revision, true));
        assert_eq!(cache.chains["app"].seq, 2);
    }

    #[test]
    fn slower_success_cannot_move_verified_progress_backwards() {
        let mut cache = ProgressCache::default();
        cache.remember_chain(0, "app", progress(2));
        cache.remember_chain(0, "app", progress(1));
        assert_eq!(cache.chains["app"].seq, 2);
    }
}
