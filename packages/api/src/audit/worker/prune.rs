//! Pruning: activity past its retention window, and archived evidence past the
//! database window.
//!
//! Two phases keep the key service out of the hot path. At most once an hour the pruner
//! advances every due chain's watermark (and the epoch timeline's) under one batch
//! signature. Every tick it deletes what the watermarks cover, which needs no signature.
//!
//! Nothing is signed on the strength of a mutable column. Every candidate seal is
//! authenticated first — its fields must hash to its stored hash, its class must match
//! its chain, and its epoch's signature and inclusion proof must verify — and the
//! evidence horizon comes only from archive manifests whose signature verifies and whose
//! epochs link into one unbroken timeline. Verification starts at the watermarks, so
//! rows still waiting for deletion are ignored and a crash anywhere is harmless.

use std::collections::HashMap;
use std::sync::{Mutex, PoisonError};

use chrono::{DateTime, FixedOffset, NaiveDate, TimeDelta, Utc};
use flow_like::hub::AuditRetention;
use flow_like_types::Value;
use sea_orm::{
    ActiveEnum, ColumnTrait, Condition, ConnectionTrait, DatabaseConnection, DbErr, EntityTrait,
    QueryFilter, QueryOrder, QuerySelect, Statement,
};

use crate::audit::crypto::{manifest_hash, to_hash};
use crate::audit::level::RetentionClass;
use crate::audit::merkle;
use crate::audit::record::{ACTIVITY_SUFFIX, chain_class};
use crate::audit::signer::{self, SignatureCheck};
use crate::audit::verify::{
    EPOCH_WATERMARK, INVALID_SEAL_ID, check_epoch, forget_progress, seal_hash_of,
};
use crate::db::delete_in_batches;
use crate::entity::sea_orm_active_enums::AiRiskCategory;
use crate::entity::{audit_archive, audit_epoch, audit_record, audit_seal, audit_watermark};

use super::archive::{next_month, parse_period, start_of};
use super::lease::Lease;
use super::legacy::LEGACY_PERIOD;
use super::watermark::{self, MAX_BATCH, WatermarkTarget};
use super::{AuditWorkerContext, TickReport};

/// Minimum time between two watermark batches, and so between two prune signatures.
const SIGNING_INTERVAL: TimeDelta = TimeDelta::hours(1);
const MAX_SEALS: u64 = 2_000;
const MAX_RECORDS: u64 = 200_000;
const QUARANTINE_BATCH: u64 = 1_000;
const EPOCH_RANGE: i64 = 500;
const MAX_EPOCHS: i64 = 10_000;
/// Chains whose deletions one tick works through.
const CHAINS_PER_TICK: u64 = 200;

/// Apps whose newest assessment is high-risk, among apps with any high-risk version.
const HIGH_RISK_SQL: &str = r#"SELECT "appId", "version", "riskCategory" FROM "AiActAssessment" WHERE "appId" IN (SELECT "appId" FROM "AiActAssessment" WHERE "riskCategory" = $1)"#;

/// The chain's watermark sequence, 0 without one.
const WATERMARK_SEQ: &str =
    r#"COALESCE((SELECT w."seq" FROM "AuditWatermark" w WHERE w."chainId" = c."chainId"), 0)"#;

/// First epoch still referenced by a seal no watermark covers.
const BLOCKING_EPOCH_SQL: &str = r#"SELECT c."epochSeq" AS "epochSeq" FROM "AuditSeal" c WHERE c."epochSeq" IS NOT NULL AND c."seq" > COALESCE((SELECT w."seq" FROM "AuditWatermark" w WHERE w."chainId" = c."chainId"), 0) ORDER BY c."epochSeq" ASC LIMIT 1"#;

/// When the last watermark batch failed. A failing key service is tried about once an
/// hour, like a successful run.
static LAST_FAILED_BATCH: Mutex<Option<DateTime<Utc>>> = Mutex::new(None);

pub(super) async fn run(
    context: &AuditWorkerContext,
    lease: &Lease,
    now: DateTime<Utc>,
    report: &mut TickReport,
) -> flow_like_types::Result<()> {
    if context.signer().is_none() {
        return Ok(());
    }
    let horizon =
        verified_horizon(&context.db, now, context.config.retention.evidence_hot_days).await?;
    // Phase B needs no signature, so it runs even when phase A could not sign.
    let advanced = advance_watermarks(context, horizon, now).await;
    if advanced.is_err() {
        *LAST_FAILED_BATCH
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(now);
    }

    let mut pruner = Pruner {
        context,
        lease,
        budget: Budget::default(),
        report,
    };
    pruner.delete_covered().await?;
    if let Some(horizon) = horizon {
        pruner
            .delete_quarantined(horizon.end.fixed_offset(), false)
            .await?;
    }
    if let Some(bound) = days_before(now, window_days(&context.config.retention, true)) {
        // Quarantined records never join a chain, so their window alone decides.
        pruner
            .delete_quarantined(bound.fixed_offset(), true)
            .await?;
    }
    pruner.delete_epochs().await?;
    advanced
}

/// Deletions left in this tick.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Budget {
    seals: u64,
    records: u64,
}

impl Default for Budget {
    fn default() -> Self {
        Self {
            seals: MAX_SEALS,
            records: MAX_RECORDS,
        }
    }
}

impl Budget {
    /// Whether a seal of `records` records still fits. A tick that has deleted no record
    /// yet takes any seal, so a seal larger than the record budget cannot stall pruning.
    fn admits(&self, records: u64) -> bool {
        self.seals > 0 && (records <= self.records || self.records == MAX_RECORDS)
    }

    fn spend(&mut self, seals: u64, records: u64) {
        self.seals = self.seals.saturating_sub(seals);
        self.records = self.records.saturating_sub(records);
    }

    fn exhausted(&self) -> bool {
        self.seals == 0 || self.records == 0
    }
}

/// End of the newest month whose evidence may leave the database, and the newest epoch
/// the archives up to it hold.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Horizon {
    end: DateTime<Utc>,
    last_epoch: Option<i64>,
}

/// What a verified manifest says about its month.
#[derive(Clone, Debug, PartialEq, Eq)]
struct ArchivedMonth {
    month: NaiveDate,
    first_epoch: Option<i64>,
    last_epoch: Option<i64>,
    first_prev_hash: Option<String>,
    last_hash: Option<String>,
}

/// The months an archive row proves: only rows whose stored manifest verifies under a
/// registered key and whose object was uploaded count. Forged or unsigned rows are
/// ignored, so no month is ever pruned on the strength of a row alone.
fn archived_month(row: &audit_archive::Model) -> Option<ArchivedMonth> {
    if row.uploaded_at.is_none() {
        return None;
    }
    let manifest = row.manifest.as_ref()?;
    let kid = row.kid.as_deref()?;
    let signature = row.signature.as_deref()?;
    if signer::verify(kid, &manifest_hash(manifest), signature) != SignatureCheck::Valid {
        return None;
    }
    let epochs = manifest.get("epochs");
    Some(ArchivedMonth {
        month: parse_period(manifest.get("period")?.as_str()?)?,
        first_epoch: epochs
            .and_then(|epochs| epochs.get("first"))
            .and_then(Value::as_i64),
        last_epoch: epochs
            .and_then(|epochs| epochs.get("last"))
            .and_then(Value::as_i64),
        first_prev_hash: epochs
            .and_then(|epochs| epochs.get("first_prev_hash"))
            .and_then(Value::as_str)
            .map(str::to_owned),
        last_hash: epochs
            .and_then(|epochs| epochs.get("last_hash"))
            .and_then(Value::as_str)
            .map(str::to_owned),
    })
}

/// How far evidence may be pruned: verified archive manifests, oldest first, whose
/// epochs continue one unbroken timeline, while the month after them is older than the
/// database window. A missing, unsigned or forged manifest stops the walk.
async fn verified_horizon(
    db: &DatabaseConnection,
    now: DateTime<Utc>,
    hot_days: u32,
) -> Result<Option<Horizon>, DbErr> {
    let Some(cutoff) = days_before(now, hot_days) else {
        return Ok(None);
    };
    let mut rows = audit_archive::Entity::find()
        .filter(audit_archive::Column::Part.eq(0))
        .filter(audit_archive::Column::Period.ne(LEGACY_PERIOD))
        .all(db)
        .await?;
    rows.sort_by(|left, right| left.period.cmp(&right.period));
    Ok(horizon_of(rows.iter().map(archived_month), cutoff))
}

/// Walk verified months in period order while their epochs link into one timeline and
/// the month after them is older than `cutoff`.
fn horizon_of(
    months: impl IntoIterator<Item = Option<ArchivedMonth>>,
    cutoff: DateTime<Utc>,
) -> Option<Horizon> {
    let mut horizon: Option<Horizon> = None;
    let mut expected_prev = hex::encode(crate::audit::crypto::ZERO_HASH);
    for month in months {
        let Some(archived) = month else {
            break;
        };
        // A month with epochs must continue the timeline; an empty month holds none.
        if let Some(first_prev_hash) = &archived.first_prev_hash {
            if first_prev_hash != &expected_prev {
                break;
            }
            let Some(last_hash) = archived.last_hash.clone() else {
                break;
            };
            expected_prev = last_hash;
        }
        let end = start_of(next_month(archived.month));
        if end >= cutoff {
            break;
        }
        horizon = Some(Horizon {
            end,
            last_epoch: archived
                .last_epoch
                .max(horizon.and_then(|horizon| horizon.last_epoch)),
        });
    }
    horizon
}

/// Days an activity chain keeps its seals. High-risk AI apps keep them at least
/// `high_risk_activity_days` (EU AI Act art. 19, 26(6)).
fn window_days(retention: &AuditRetention, high_risk: bool) -> u32 {
    if high_risk {
        retention
            .activity_days
            .max(retention.high_risk_activity_days)
    } else {
        retention.activity_days
    }
}

fn days_before(now: DateTime<Utc>, days: u32) -> Option<DateTime<Utc>> {
    TimeDelta::try_days(i64::from(days)).and_then(|window| now.checked_sub_signed(window))
}

/// Activity chains of apps whose newest assessment (highest `version`) has risk `high`.
fn high_risk_chains_of(
    assessments: impl IntoIterator<Item = (String, i32, String)>,
    high: &str,
) -> Vec<String> {
    let mut newest: HashMap<String, (i32, String)> = HashMap::new();
    for (app_id, version, risk) in assessments {
        match newest.get(&app_id) {
            Some((known, _)) if *known >= version => {}
            _ => {
                newest.insert(app_id, (version, risk));
            }
        }
    }
    let mut chains: Vec<String> = newest
        .into_iter()
        .filter(|(_, (_, risk))| risk == high)
        .map(|(app_id, _)| format!("{app_id}{ACTIVITY_SUFFIX}"))
        .collect();
    chains.sort();
    chains
}

async fn high_risk_chains(db: &DatabaseConnection) -> flow_like_types::Result<Vec<String>> {
    let high = AiRiskCategory::High.to_value();
    let rows = db
        .query_all_raw(Statement::from_sql_and_values(
            db.get_database_backend(),
            HIGH_RISK_SQL,
            [sea_orm::Value::from(high.clone())],
        ))
        .await?;
    let mut assessments = Vec::with_capacity(rows.len());
    for row in rows {
        assessments.push((
            row.try_get::<String>("", "appId")?,
            row.try_get::<i32>("", "version")?,
            row.try_get::<String>("", "riskCategory")?,
        ));
    }
    Ok(high_risk_chains_of(assessments, &high))
}

/// Which chains an activity pass looks at.
enum Chains<'a> {
    Except(&'a [String]),
    Only(&'a [String]),
}

/// Whether phase A may sign now: at most one batch per [`SIGNING_INTERVAL`], counting
/// failed attempts, so a failing key service is not called on every tick.
async fn signing_due(db: &DatabaseConnection, now: DateTime<Utc>) -> Result<bool, DbErr> {
    let failed = *LAST_FAILED_BATCH
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    if failed.is_some_and(|at| now.signed_duration_since(at) < SIGNING_INTERVAL) {
        return Ok(false);
    }
    let newest = audit_watermark::Entity::find()
        .order_by_desc(audit_watermark::Column::PrunedAt)
        .one(db)
        .await?;
    Ok(!newest
        .is_some_and(|watermark| now.signed_duration_since(watermark.pruned_at) < SIGNING_INTERVAL))
}

/// Phase A. One signature covers every chain that has seals past its window, plus the
/// epoch timeline.
async fn advance_watermarks(
    context: &AuditWorkerContext,
    horizon: Option<Horizon>,
    now: DateTime<Utc>,
) -> flow_like_types::Result<()> {
    let Some(signer) = context.signer() else {
        return Ok(());
    };
    let db = &context.db;
    if !signing_due(db, now).await? {
        return Ok(());
    }

    let retention = &context.config.retention;
    let mut candidates: Vec<audit_seal::Model> = Vec::new();
    let high_risk = high_risk_chains(db).await?;
    if let Some(cutoff) = days_before(now, window_days(retention, false)) {
        candidates
            .extend(activity_candidates(db, cutoff, Chains::Except(&high_risk), MAX_BATCH).await?);
    }
    if !high_risk.is_empty()
        && let Some(cutoff) = days_before(now, window_days(retention, true))
    {
        let room = MAX_BATCH.saturating_sub(candidates.len());
        candidates.extend(activity_candidates(db, cutoff, Chains::Only(&high_risk), room).await?);
    }
    let activity = candidates.len();
    if let Some(horizon) = horizon {
        let room = MAX_BATCH.saturating_sub(candidates.len());
        candidates.extend(evidence_candidates(db, horizon, room).await?);
    }

    let mut epochs: HashMap<i64, Option<audit_epoch::Model>> = HashMap::new();
    let mut targets = Vec::with_capacity(candidates.len());
    for (index, seal) in candidates.iter().enumerate() {
        let cutoff = match (index < activity, horizon) {
            // Activity: the seal's own signed times decide.
            (true, _) => days_before(
                now,
                window_days(retention, high_risk.binary_search(&seal.chain_id).is_ok()),
            ),
            // Evidence: the signed epoch of the archived months decides.
            (false, Some(horizon)) => Some(horizon.end),
            (false, None) => None,
        };
        let Some(cutoff) = cutoff else {
            continue;
        };
        match authentic_target(db, seal, cutoff, horizon, index >= activity, &mut epochs).await? {
            Ok(target) => targets.push(target),
            Err(problem) => tracing::error!(
                target: "audit",
                chain_id = %seal.chain_id,
                seal_id = %seal.id,
                seq = seal.seq,
                %problem,
                "audit seal is not prunable; its chain keeps its records"
            ),
        }
    }
    if let Some(horizon) = horizon
        && targets.len() < MAX_BATCH
        && let Some(epochs) = epoch_target(db, horizon).await?
    {
        targets.push(epochs);
    }
    if targets.is_empty() {
        return Ok(());
    }

    let chains: Vec<String> = targets
        .iter()
        .map(|target| target.chain_id.clone())
        .collect();
    let count = targets.len();
    let watermarks = watermark::sign_batch(signer, targets, now).await?;
    watermark::upsert(db, watermarks).await.map_err(|error| {
        flow_like_types::anyhow!("writing {count} signed audit watermarks failed: {error}")
    })?;
    for chain in &chains {
        forget_progress(Some(chain));
    }
    tracing::info!(target: "audit", watermarks = count, "advanced audit watermarks");
    Ok(())
}

/// Authenticate a candidate before its hash is signed into a watermark: the seal must
/// hash to its stored hash, carry its chain's class, and be anchored in an epoch whose
/// signature and inclusion proof verify. Only then do its own signed times, or its
/// epoch, decide whether it is past its window.
async fn authentic_target(
    db: &DatabaseConnection,
    seal: &audit_seal::Model,
    cutoff: DateTime<Utc>,
    horizon: Option<Horizon>,
    evidence: bool,
    epochs: &mut HashMap<i64, Option<audit_epoch::Model>>,
) -> Result<Result<WatermarkTarget, String>, DbErr> {
    let hash = seal_hash_of(seal);
    if hash.as_slice() != seal.hash.as_slice() {
        return Ok(Err("seal hash does not match its fields".into()));
    }
    if seal.class != chain_class(&seal.chain_id).as_str() {
        return Ok(Err("seal class does not match its chain".into()));
    }
    let (Some(epoch_seq), Some(index), Some(proof)) = (
        seal.epoch_seq,
        seal.epoch_index,
        seal.epoch_proof.as_deref(),
    ) else {
        return Ok(Err("seal is not anchored in an epoch".into()));
    };
    if !epochs.contains_key(&epoch_seq) {
        let epoch = audit_epoch::Entity::find_by_id(epoch_seq).one(db).await?;
        epochs.insert(epoch_seq, epoch);
    }
    let Some(Some(epoch)) = epochs.get(&epoch_seq) else {
        return Ok(Err(format!("epoch {epoch_seq} is missing")));
    };
    if check_epoch(epoch) != Ok(true) {
        return Ok(Err(format!("epoch {epoch_seq} does not verify")));
    }
    let Some(seals_root) = to_hash(&epoch.seals_root) else {
        return Ok(Err(format!("epoch {epoch_seq} root has the wrong length")));
    };
    let (Ok(index), Ok(size)) = (u64::try_from(index), u64::try_from(epoch.seal_count)) else {
        return Ok(Err("seal has a negative epoch position".into()));
    };
    let Some(proof) = merkle::decode_proof(proof) else {
        return Ok(Err("seal epoch proof is malformed".into()));
    };
    if !merkle::verify_inclusion(&hash, index, size, &proof, &seals_root) {
        return Ok(Err(format!("seal is not part of epoch {epoch_seq}")));
    }
    let cutoff = cutoff.fixed_offset();
    if evidence {
        let covered = horizon
            .and_then(|horizon| horizon.last_epoch)
            .is_some_and(|last| epoch.seq <= last);
        if !covered {
            return Ok(Err(format!(
                "epoch {epoch_seq} is not covered by a verified archive"
            )));
        }
        if seal.last_at >= cutoff {
            return Ok(Err("seal is newer than the archived months".into()));
        }
    } else if seal.sealed_at >= cutoff || seal.last_at >= cutoff {
        return Ok(Err("seal is inside its retention window".into()));
    }
    Ok(Ok(WatermarkTarget {
        chain_id: seal.chain_id.clone(),
        seq: seal.seq,
        hash: seal.hash.clone(),
    }))
}

/// Per chain, the newest anchored activity seal sealed before `cutoff` above its
/// watermark. `sealedAt` grows with `seq` within a chain, so everything below it is due
/// as well; `lastAt` only lets the `(class, lastAt)` index narrow the scan.
async fn activity_candidates(
    db: &DatabaseConnection,
    cutoff: DateTime<Utc>,
    chains: Chains<'_>,
    limit: usize,
) -> Result<Vec<audit_seal::Model>, DbErr> {
    if limit == 0 {
        return Ok(Vec::new());
    }
    let mut values: Vec<sea_orm::Value> = vec![
        RetentionClass::Activity.as_str().into(),
        cutoff.fixed_offset().into(),
        (limit as i64).into(),
    ];
    let (list, negate) = match chains {
        Chains::Except(list) => (list, true),
        Chains::Only(list) => (list, false),
    };
    let filter = if list.is_empty() {
        if negate {
            String::new()
        } else {
            return Ok(Vec::new());
        }
    } else {
        let placeholders: Vec<String> = list
            .iter()
            .enumerate()
            .map(|(index, _)| format!("${}", values.len() + index + 1))
            .collect();
        values.extend(list.iter().map(|chain| sea_orm::Value::from(chain.clone())));
        format!(
            r#" AND c."chainId" {} IN ({})"#,
            if negate { "NOT" } else { "" },
            placeholders.join(", ")
        )
    };
    let sql = format!(
        r#"SELECT s.* FROM "AuditSeal" s JOIN (SELECT c."chainId", max(c."seq") AS "top" FROM "AuditSeal" c WHERE c."class" = $1 AND c."lastAt" < $2 AND c."sealedAt" < $2 AND c."epochSeq" IS NOT NULL AND c."seq" > {WATERMARK_SEQ}{filter} GROUP BY c."chainId" LIMIT $3) m ON m."chainId" = s."chainId" AND m."top" = s."seq""#
    );
    candidates(db, sql, values).await
}

/// Per chain, the newest evidence seal anchored by an epoch the archives up to
/// `horizon` hold.
async fn evidence_candidates(
    db: &DatabaseConnection,
    horizon: Horizon,
    limit: usize,
) -> Result<Vec<audit_seal::Model>, DbErr> {
    if limit == 0 {
        return Ok(Vec::new());
    }
    let sql = format!(
        r#"SELECT s.* FROM "AuditSeal" s JOIN (SELECT c."chainId", max(c."seq") AS "top" FROM "AuditSeal" c WHERE c."class" = $1 AND c."lastAt" < $2 AND c."epochSeq" <= $3 AND c."seq" > {WATERMARK_SEQ} GROUP BY c."chainId" LIMIT $4) m ON m."chainId" = s."chainId" AND m."top" = s."seq""#
    );
    let values: Vec<sea_orm::Value> = vec![
        RetentionClass::Evidence.as_str().into(),
        horizon.end.fixed_offset().into(),
        horizon.last_epoch.unwrap_or(-1).into(),
        (limit as i64).into(),
    ];
    candidates(db, sql, values).await
}

async fn candidates(
    db: &DatabaseConnection,
    sql: String,
    values: Vec<sea_orm::Value>,
) -> Result<Vec<audit_seal::Model>, DbErr> {
    audit_seal::Entity::find()
        .from_raw_sql(Statement::from_sql_and_values(
            db.get_database_backend(),
            sql,
            values,
        ))
        .all(db)
        .await
}

/// The newest epoch created before the horizon that no uncovered seal references, above
/// the timeline's current watermark. Uses committed watermarks, so an epoch freed by
/// this run's chain watermarks is pruned by the next run.
async fn epoch_target(
    db: &DatabaseConnection,
    horizon: Horizon,
) -> Result<Option<WatermarkTarget>, DbErr> {
    let blocking: Option<i64> = db
        .query_one_raw(Statement::from_string(
            db.get_database_backend(),
            BLOCKING_EPOCH_SQL,
        ))
        .await?
        .map(|row| row.try_get("", "epochSeq"))
        .transpose()?;
    let current = audit_watermark::Entity::find_by_id(EPOCH_WATERMARK)
        .one(db)
        .await?
        .map_or(0, |watermark| watermark.seq);
    let mut query = audit_epoch::Entity::find()
        .filter(audit_epoch::Column::CreatedAt.lt(horizon.end.fixed_offset()))
        .filter(audit_epoch::Column::Seq.gt(current));
    if let Some(blocking) = blocking {
        query = query.filter(audit_epoch::Column::Seq.lt(blocking));
    }
    if let Some(last) = horizon.last_epoch {
        query = query.filter(audit_epoch::Column::Seq.lte(last));
    }
    let epoch = query
        .order_by_desc(audit_epoch::Column::CreatedAt)
        .order_by_desc(audit_epoch::Column::Seq)
        .one(db)
        .await?;
    // The epoch's own signature covers its sequence and hash.
    Ok(epoch
        .filter(|epoch| check_epoch(epoch) == Ok(true))
        .map(|epoch| WatermarkTarget {
            chain_id: EPOCH_WATERMARK.to_owned(),
            seq: epoch.seq,
            hash: epoch.hash,
        }))
}

fn record_count(seal: &audit_seal::Model) -> u64 {
    u64::try_from(seal.record_count).unwrap_or(0)
}

/// A seal's records and the seal itself, in one transaction of at most
/// `max_records_per_seal + 1` rows. Returns the records deleted.
async fn delete_seal(context: &AuditWorkerContext, seal_id: &str) -> flow_like_types::Result<u64> {
    let seal_id = seal_id.to_owned();
    let records = crate::db::retry_transaction(
        &context.db,
        context.dialect,
        None,
        &crate::db::RetryPolicy::idempotent(),
        move |txn| {
            let seal_id = seal_id.clone();
            Box::pin(async move {
                let records = audit_record::Entity::delete_many()
                    .filter(audit_record::Column::SealId.eq(seal_id.as_str()))
                    .exec(txn)
                    .await?
                    .rows_affected;
                audit_seal::Entity::delete_many()
                    .filter(audit_seal::Column::Id.eq(seal_id.as_str()))
                    .exec(txn)
                    .await?;
                Ok::<_, DbErr>(records)
            })
        },
    )
    .await?;
    Ok(records)
}

/// Phase B: deletions behind the watermarks, bounded per tick. No signing.
struct Pruner<'a> {
    context: &'a AuditWorkerContext,
    lease: &'a Lease,
    budget: Budget,
    report: &'a mut TickReport,
}

impl Pruner<'_> {
    /// Work through the chains whose watermark still covers seals, by the
    /// `(pendingDelete)` index, then by each chain's `(chainId, seq)` index.
    async fn delete_covered(&mut self) -> flow_like_types::Result<()> {
        let db = &self.context.db;
        loop {
            if self.budget.exhausted() {
                return Ok(());
            }
            self.keepalive().await?;
            let pending = audit_watermark::Entity::find()
                .filter(audit_watermark::Column::PendingDelete.eq(true))
                .order_by_asc(audit_watermark::Column::PrunedAt)
                .limit(CHAINS_PER_TICK)
                .all(db)
                .await?;
            if pending.is_empty() {
                return Ok(());
            }
            let mut progressed = false;
            for watermark in &pending {
                if self.budget.exhausted() {
                    return Ok(());
                }
                if self.delete_chain(watermark).await? {
                    progressed = true;
                }
            }
            if !progressed {
                return Ok(());
            }
        }
    }

    /// Delete one chain's seals up to its watermark, oldest first. Returns whether the
    /// chain is done for now.
    async fn delete_chain(
        &mut self,
        watermark: &audit_watermark::Model,
    ) -> flow_like_types::Result<bool> {
        let context = self.context;
        loop {
            self.keepalive().await?;
            let seals = audit_seal::Entity::find()
                .filter(audit_seal::Column::ChainId.eq(watermark.chain_id.as_str()))
                .filter(audit_seal::Column::Seq.lte(watermark.seq))
                .order_by_asc(audit_seal::Column::Seq)
                .limit(self.budget.seals.min(CHAINS_PER_TICK))
                .all(&context.db)
                .await?;
            if seals.is_empty() {
                // Nothing left at or below this watermark until it moves again.
                audit_watermark::Entity::update_many()
                    .col_expr(
                        audit_watermark::Column::PendingDelete,
                        sea_orm::sea_query::Expr::value(false),
                    )
                    .filter(audit_watermark::Column::ChainId.eq(watermark.chain_id.as_str()))
                    .filter(audit_watermark::Column::Seq.eq(watermark.seq))
                    .exec(&context.db)
                    .await?;
                return Ok(false);
            }
            for seal in &seals {
                if !self.budget.admits(record_count(seal)) {
                    return Ok(true);
                }
                let records = delete_seal(context, &seal.id).await?;
                self.budget.spend(1, records);
                self.report.pruned_seals += 1;
                self.report.pruned_records += records;
            }
        }
    }

    /// Quarantined records past their window: those of pruned evidence months, and
    /// activity ones past the activity window.
    async fn delete_quarantined(
        &mut self,
        before: DateTime<FixedOffset>,
        activity: bool,
    ) -> flow_like_types::Result<()> {
        let context = self.context;
        loop {
            if self.budget.records == 0 {
                return Ok(());
            }
            self.keepalive().await?;
            let mut condition = Condition::all()
                .add(audit_record::Column::SealId.eq(INVALID_SEAL_ID))
                .add(audit_record::Column::Timestamp.lt(before));
            condition = if activity {
                condition.add(audit_record::Column::ChainId.like(format!("%{ACTIVITY_SUFFIX}")))
            } else {
                condition.add(audit_record::Column::ChainId.not_like(format!("%{ACTIVITY_SUFFIX}")))
            };
            let outcome = delete_in_batches::<audit_record::Entity>(
                &context.db,
                context.dialect,
                condition,
                self.budget.records.min(QUARANTINE_BATCH) as usize,
                Some(1),
            )
            .await?;
            self.budget.spend(0, outcome.rows);
            self.report.pruned_records += outcome.rows;
            if !outcome.stopped_early {
                return Ok(());
            }
        }
    }

    /// Epochs at or below the timeline's watermark, oldest first, once no seal
    /// references them any more.
    async fn delete_epochs(&mut self) -> flow_like_types::Result<()> {
        let db = &self.context.db;
        let Some(watermark) = audit_watermark::Entity::find_by_id(EPOCH_WATERMARK)
            .one(db)
            .await?
        else {
            return Ok(());
        };
        let referenced = audit_seal::Entity::find()
            .filter(audit_seal::Column::EpochSeq.is_not_null())
            .order_by_asc(audit_seal::Column::EpochSeq)
            .one(db)
            .await?
            .and_then(|seal| seal.epoch_seq);
        let through = referenced.map_or(watermark.seq, |seq| watermark.seq.min(seq - 1));
        let Some(first) = audit_epoch::Entity::find()
            .filter(audit_epoch::Column::Seq.lte(through))
            .order_by_asc(audit_epoch::Column::Seq)
            .one(db)
            .await?
            .map(|epoch| epoch.seq)
        else {
            return Ok(());
        };
        let through = through.min(first.saturating_add(MAX_EPOCHS - 1));
        let mut low = first;
        while low <= through {
            self.keepalive().await?;
            let high = through.min(low.saturating_add(EPOCH_RANGE - 1));
            let deleted = audit_epoch::Entity::delete_many()
                .filter(audit_epoch::Column::Seq.gte(low))
                .filter(audit_epoch::Column::Seq.lte(high))
                .exec(db)
                .await?
                .rows_affected;
            self.report.pruned_epochs += deleted;
            low = high + 1;
        }
        Ok(())
    }

    async fn keepalive(&self) -> flow_like_types::Result<()> {
        if self.lease.keepalive().await {
            Ok(())
        } else {
            Err(flow_like_types::anyhow!(
                "audit worker lost its lease while pruning"
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audit::crypto::ZERO_HASH;
    use crate::audit::signer::{AuditSigner, LocalSigner, register_verifying_key};
    use p256::ecdsa::SigningKey;
    use serde_json::json;

    fn at(text: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(text)
            .unwrap()
            .with_timezone(&Utc)
    }

    fn signer() -> LocalSigner {
        let local = LocalSigner::new(
            SigningKey::from_slice(&[51; 32]).unwrap(),
            Some("prune-test".into()),
        );
        register_verifying_key(local.kid(), local.verifying_key()).unwrap();
        local
    }

    /// A manifest row as the archiver writes it, signed unless `sign` is false.
    async fn archived(
        period: &str,
        epochs: Option<(i64, i64, &str, &str)>,
        sign: bool,
    ) -> audit_archive::Model {
        let signer = signer();
        let epochs = match epochs {
            Some((first, last, first_prev_hash, last_hash)) => json!({
                "first": first,
                "last": last,
                "first_prev_hash": first_prev_hash,
                "last_hash": last_hash,
            }),
            None => json!({
                "first": Value::Null,
                "last": Value::Null,
                "first_prev_hash": Value::Null,
                "last_hash": Value::Null,
            }),
        };
        let manifest = json!({
            "format": "flow-like.audit-archive/v1",
            "period": period,
            "epochs": epochs,
            "kid": signer.kid(),
        });
        let signature = signer.sign(&manifest_hash(&manifest)).await.unwrap();
        audit_archive::Model {
            period: period.into(),
            part: 0,
            object_key: format!("archive/{}/manifest.json", period.replacen('-', "/", 1)),
            sha256: vec![0; 32],
            byte_size: 1,
            first_epoch: None,
            last_epoch: None,
            epoch_count: 0,
            seal_count: 0,
            record_count: 0,
            created_at: at("2026-01-01T00:00:00Z").fixed_offset(),
            kid: sign.then(|| signer.kid().to_owned()),
            signature: sign.then(|| signature.to_vec()),
            manifest: Some(manifest),
            uploaded_at: Some(at("2026-01-01T00:00:00Z").fixed_offset()),
        }
    }

    fn zero() -> String {
        hex::encode(ZERO_HASH)
    }

    #[flow_like_types::tokio::test]
    async fn only_verified_linked_manifests_prove_a_month() {
        let first = archived("2025-06", Some((1, 10, &zero(), "aa")), true).await;
        assert_eq!(
            archived_month(&first).unwrap().last_epoch,
            Some(10),
            "a signed manifest counts"
        );
        let unsigned = archived("2025-07", Some((11, 20, "aa", "bb")), false).await;
        assert_eq!(
            archived_month(&unsigned),
            None,
            "an unsigned manifest does not"
        );
        let mut forged = first.clone();
        forged.manifest = Some(json!({"period": "2025-06", "epochs": {"last": 99}}));
        assert_eq!(
            archived_month(&forged),
            None,
            "a rewritten manifest does not"
        );
        let mut waiting = first.clone();
        waiting.uploaded_at = None;
        assert_eq!(
            archived_month(&waiting),
            None,
            "an unwritten manifest does not"
        );
    }

    fn month(period: &str, epochs: Option<(i64, i64, &str, &str)>) -> Option<ArchivedMonth> {
        Some(ArchivedMonth {
            month: parse_period(period).unwrap(),
            first_epoch: epochs.map(|(first, _, _, _)| first),
            last_epoch: epochs.map(|(_, last, _, _)| last),
            first_prev_hash: epochs.map(|(_, _, prev, _)| prev.to_owned()),
            last_hash: epochs.map(|(_, _, _, last)| last.to_owned()),
        })
    }

    #[test]
    fn the_horizon_follows_linked_months_inside_the_window() {
        let cutoff = at("2026-09-19T12:00:00Z") - TimeDelta::days(396);
        let months = [
            month("2025-06", Some((1, 10, &zero(), "aa"))),
            month("2025-07", None),
            month("2025-08", Some((11, 30, "aa", "bb"))),
        ];
        // 2025-08 ends 2025-09-01, inside the 396-day window; 2025-07 holds no epochs.
        assert_eq!(
            horizon_of(months.clone(), cutoff),
            Some(Horizon {
                end: at("2025-08-01T00:00:00Z"),
                last_epoch: Some(10),
            })
        );
        let broken = [
            month("2025-06", Some((1, 10, &zero(), "aa"))),
            month("2025-07", Some((11, 20, "ff", "bb"))),
        ];
        assert_eq!(
            horizon_of(broken, cutoff).map(|horizon| horizon.last_epoch),
            Some(Some(10)),
            "a month whose epochs do not link stops the walk"
        );
        assert_eq!(
            horizon_of([None, months[0].clone()], cutoff),
            None,
            "an unverified manifest stops the walk"
        );
        assert_eq!(horizon_of([], cutoff), None);
    }

    #[test]
    fn high_risk_apps_keep_activity_longer() {
        let retention = AuditRetention::default();
        assert_eq!(window_days(&retention, false), 90);
        assert_eq!(window_days(&retention, true), 183);
        let short = AuditRetention {
            activity_days: 400,
            ..AuditRetention::default()
        };
        assert_eq!(window_days(&short, true), 400);
        assert!(days_before(at("2026-09-19T00:00:00Z"), u32::MAX).is_none());
        assert_eq!(
            days_before(at("2026-09-19T00:00:00Z"), 90),
            Some(at("2026-06-21T00:00:00Z"))
        );
    }

    #[test]
    fn only_the_newest_assessment_decides_the_risk() {
        let chains = high_risk_chains_of(
            [
                ("downgraded".to_owned(), 1, "HIGH".to_owned()),
                ("downgraded".to_owned(), 2, "LIMITED".to_owned()),
                ("upgraded".to_owned(), 2, "HIGH".to_owned()),
                ("upgraded".to_owned(), 1, "MINIMAL".to_owned()),
                ("steady".to_owned(), 1, "HIGH".to_owned()),
            ],
            &AiRiskCategory::High.to_value(),
        );
        assert_eq!(chains, vec!["steady#activity", "upgraded#activity"]);
    }

    #[test]
    fn the_first_seal_of_a_tick_always_fits() {
        let mut budget = Budget::default();
        assert!(budget.admits(MAX_RECORDS + 1));
        budget.spend(1, 10);
        assert!(!budget.admits(MAX_RECORDS));
        assert!(budget.admits(MAX_RECORDS - 10));
        budget.spend(0, MAX_RECORDS);
        assert!(budget.exhausted());
        let no_seals = Budget {
            seals: 0,
            records: MAX_RECORDS,
        };
        assert!(!no_seals.admits(0));
    }
}
