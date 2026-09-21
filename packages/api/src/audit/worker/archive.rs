//! Monthly archive of the evidence class.
//!
//! A closed month is exported once, oldest first, as zstd-compressed JSON lines: its
//! epochs with their evidence seals and records, then evidence seals no epoch covers,
//! then quarantined records. Everything is re-verified before it is written, so a
//! failed check leaves the month unarchived. Large months are split into parts at
//! epoch boundaries; the manifest, written last, marks the month complete.

use std::collections::HashMap;

use chrono::{DateTime, Datelike, Days, FixedOffset, Months, NaiveDate, NaiveTime, Utc};
use flow_like_storage::Path;
use flow_like_storage::files::store::FlowLikeStore;
use flow_like_storage::object_store::{Error as StoreError, ObjectStoreExt};
use flow_like_types::Value;
use flow_like_types::base64::{Engine, engine::general_purpose::STANDARD};
use futures::TryStreamExt;
use sea_orm::sea_query::Expr;
use sea_orm::{
    ActiveValue::Set, ColumnTrait, Condition, ConnectionTrait, DatabaseConnection, EntityTrait,
    IntoActiveModel, QueryFilter, QueryOrder, QuerySelect, Statement, TryIntoModel,
};
use sha2::{Digest, Sha256};

use crate::audit::crypto::{Hash, ZERO_HASH, manifest_hash, to_hash};
use crate::audit::level::RetentionClass;
use crate::audit::merkle;
use crate::audit::record::{ACTIVITY_SUFFIX, chain_class};
use crate::audit::verify::{
    EPOCH_WATERMARK, INVALID_SEAL_ID, check_epoch, check_record, check_watermark, seal_hash_of,
    sort_records,
};
use crate::audit::wire::{EpochLine, Line, RawValues, RecordLine, SealLine, encode};
use crate::entity::{audit_archive, audit_epoch, audit_record, audit_seal, audit_watermark};

use super::bucket::{
    ArchiveWriter, WrittenObject, archive_manifest_key, archive_part_key, period_of, put_bytes,
};
use super::lease::Lease;
use super::legacy::LEGACY_PERIOD;
use super::{AuditWorkerContext, TickReport};

const MANIFEST_FORMAT: &str = "flow-like.audit-archive/v1";
const RECEIPT_FORMAT: &str = "flow-like.audit-archive-receipt/v1";
const MAX_RECEIPT_BYTES: usize = 1024 * 1024;
/// Uncompressed size after which the next epoch starts a new part.
const PART_BYTES: u64 = 2 * 1024 * 1024 * 1024;
const EPOCH_PAGE: u64 = 100;
const SEAL_PAGE: u64 = 200;
/// Records loaded at once, summed over whole seals.
const RECORD_PAGE: u64 = 5_000;
/// A seal sealed before `$1` that still waits for its epoch, held chains excluded.
const WAITING_BEFORE_SQL: &str = r#"SELECT 1 AS "waiting" FROM "AuditSeal" s WHERE s."epochSeq" IS NULL AND s."sealedAt" < $1 AND NOT EXISTS (SELECT 1 FROM "AuditHeldChain" h WHERE h."chainId" = s."chainId") LIMIT 1"#;

pub(super) async fn run(
    context: &AuditWorkerContext,
    lease: &Lease,
    now: DateTime<Utc>,
    report: &mut TickReport,
) -> flow_like_types::Result<()> {
    let Some(bucket) = context.bucket.as_deref() else {
        return Ok(());
    };
    let db = &context.db;
    let retention = &context.config.retention;
    // A configured key that is not connected yet signs nothing; the month waits.
    let signer = match (context.signing_configured(), context.signer()) {
        (true, None) => return Ok(()),
        (_, signer) => signer,
    };

    let last_archived = audit_archive::Entity::find()
        .filter(audit_archive::Column::Part.eq(0))
        .filter(audit_archive::Column::Period.ne(LEGACY_PERIOD))
        .order_by_desc(audit_archive::Column::Period)
        .one(db)
        .await?;
    if let Some(row) = &last_archived {
        // The database cannot authorize a completed month or a manifest re-upload.
        read_receipt(bucket, row).await?;
    }
    if let Some(row) = last_archived
        .as_ref()
        .filter(|row| row.uploaded_at.is_none())
    {
        upload_manifest(db, bucket, row, now).await?;
        report.archived_parts += 1;
        return Ok(());
    }
    let earliest = match last_archived {
        Some(_) => None,
        None => earliest_data(db).await?,
    };
    let Some(first_day) = candidate_month(
        last_archived.as_ref().map(|row| row.period.as_str()),
        earliest,
    ) else {
        return Ok(());
    };
    if !archivable(first_day, now, retention.archive_grace_days) {
        return Ok(());
    }
    let month = Month::new(first_day);
    if still_changing(context, month.end).await? {
        return Ok(());
    }

    let first_epoch = audit_epoch::Entity::find()
        .filter(audit_epoch::Column::CreatedAt.gte(month.start))
        .filter(audit_epoch::Column::CreatedAt.lt(month.end))
        .order_by_asc(audit_epoch::Column::Seq)
        .one(db)
        .await?;
    let last_epoch = audit_epoch::Entity::find()
        .filter(audit_epoch::Column::CreatedAt.gte(month.start))
        .filter(audit_epoch::Column::CreatedAt.lt(month.end))
        .order_by_desc(audit_epoch::Column::Seq)
        .one(db)
        .await?;
    let last_epoch_seq = last_epoch.as_ref().map(|epoch| epoch.seq);
    let raw = RawValues {
        ip: false,
        details: retention.details_days.is_none(),
    };
    let created_at = at_millis(now);
    let receipt_context = receipt_context(
        &month,
        first_epoch.as_ref(),
        last_epoch.as_ref(),
        raw,
        !context.signing_configured(),
    );
    let verified = verified_parts(db, bucket, &month.period, &receipt_context).await?;

    if let Progress::ContinueAfter(after) = verified.progress {
        let mut export = MonthExport {
            db,
            bucket,
            lease,
            month: &month,
            raw,
            include_unanchored: !context.signing_configured(),
            created_at,
            next_part: verified.rows.len() as i32 + 1,
            open: None,
            written: 0,
            previous_epoch: None,
            chain_tails: HashMap::new(),
            receipt_context: &receipt_context,
            previous_receipt: verified.last_receipt,
        };
        let streamed = match export.stream(after, last_epoch_seq).await {
            Ok(()) => export.finish_part(true).await,
            Err(error) => Err(error),
        };
        report.archived_parts += export.written;
        if let Err(error) = streamed {
            export.abort().await;
            return Err(error);
        }
    }

    let verified = verified_parts(db, bucket, &month.period, &receipt_context).await?;
    if !verified.rows.is_empty() && verified.progress != Progress::PartsComplete {
        return Err(flow_like_types::anyhow!(
            "audit archive {} has no completed final part",
            month.period
        ));
    }
    let parts = verified.rows;
    let mut manifest = manifest_json(
        &month,
        retention.archive_years_after_year_end,
        &parts,
        first_epoch.as_ref(),
        last_epoch.as_ref(),
        raw,
        created_at.timestamp_millis(),
    );
    if let Some(row) = recover_manifest(
        bucket,
        &month.period,
        &manifest,
        &parts,
        &receipt_context,
        verified.last_receipt.as_deref(),
        context.signing_configured(),
    )
    .await?
    {
        let row = audit_archive::Entity::insert(row.into_active_model())
            .exec_with_returning(db)
            .await?;
        upload_manifest(db, bucket, &row, now).await?;
        report.archived_parts += 1;
        return Ok(());
    }
    let signature = match signer {
        Some(signer) => {
            manifest["kid"] = Value::from(signer.kid());
            Some(signer.sign(&manifest_hash(&manifest)).await?)
        }
        None => None,
    };
    let key = archive_manifest_key(&month.period);
    let bytes = manifest_bytes(
        &manifest,
        signature.as_ref().map(|signature| signature.as_slice()),
    )?;
    let written = WrittenObject {
        sha256: Sha256::digest(&bytes).into(),
        byte_size: bytes.len() as u64,
    };
    // The signed manifest is committed before its upload, so a failed upload is retried
    // from this row without signing again.
    let mut row = archive_row(&month.period, 0, &key, written, created_at);
    row.first_epoch = Set(first_epoch.as_ref().map(|epoch| epoch.seq));
    row.last_epoch = Set(last_epoch_seq);
    row.epoch_count = Set(part_total(&parts, |part| part.epoch_count));
    row.seal_count = Set(part_total(&parts, |part| part.seal_count));
    row.record_count = Set(part_total(&parts, |part| part.record_count));
    row.kid = Set(signer.map(|signer| signer.kid().to_owned()));
    row.signature = Set(signature.map(|signature| signature.to_vec()));
    row.manifest = Set(Some(manifest));
    row.uploaded_at = Set(None);
    write_receipt(
        bucket,
        &row.clone().try_into_model()?,
        &receipt_context,
        verified.last_receipt.as_deref(),
        true,
    )
    .await?;
    let row = audit_archive::Entity::insert(row)
        .exec_with_returning(db)
        .await?;
    upload_manifest(db, bucket, &row, now).await?;
    report.archived_parts += 1;
    tracing::info!(
        target: "audit",
        period = %month.period,
        parts = parts.len(),
        "audit month archived"
    );
    Ok(())
}

/// The manifest object: the stored manifest plus its signature, when there is one.
/// Sorted keys survive JSONB reordering; integer sequences keep their wire type.
fn manifest_bytes(manifest: &Value, signature: Option<&[u8]>) -> flow_like_types::Result<Vec<u8>> {
    let mut signed = manifest.clone();
    if let Some(signature) = signature {
        signed["signature"] = Value::from(STANDARD.encode(signature));
    }
    signed.sort_all_objects();
    Ok(serde_json::to_vec(&signed)?)
}

/// Upload a committed manifest and mark it uploaded. The bytes are rebuilt from the row
/// and must match the digest recorded before the first attempt.
async fn upload_manifest(
    db: &DatabaseConnection,
    bucket: &FlowLikeStore,
    row: &audit_archive::Model,
    now: DateTime<Utc>,
) -> flow_like_types::Result<()> {
    read_receipt(bucket, row).await?;
    let manifest = row.manifest.as_ref().ok_or_else(|| {
        flow_like_types::anyhow!("audit archive manifest of {} was not stored", row.period)
    })?;
    let bytes = manifest_bytes(manifest, row.signature.as_deref())?;
    if Sha256::digest(&bytes).as_slice() != row.sha256.as_slice() {
        return Err(flow_like_types::anyhow!(
            "stored audit archive manifest of {} no longer matches its digest",
            row.period
        ));
    }
    put_bytes(bucket, &Path::from(row.object_key.as_str()), bytes).await?;
    audit_archive::Entity::update_many()
        .col_expr(
            audit_archive::Column::UploadedAt,
            Expr::value(Some(at_millis(now))),
        )
        .filter(audit_archive::Column::Period.eq(row.period.as_str()))
        .filter(audit_archive::Column::Part.eq(0))
        .exec(db)
        .await?;
    Ok(())
}

/// `now` at the database's millisecond precision, so stored and hashed times agree.
pub(super) fn at_millis(now: DateTime<Utc>) -> DateTime<FixedOffset> {
    DateTime::from_timestamp_millis(now.timestamp_millis())
        .unwrap_or(now)
        .fixed_offset()
}

/// First day of the month `date` falls in.
pub(super) fn month_of(date: NaiveDate) -> NaiveDate {
    date.with_day(1).unwrap_or(date)
}

/// First day of the month after `month`.
pub(super) fn next_month(month: NaiveDate) -> NaiveDate {
    month_of(month)
        .checked_add_months(Months::new(1))
        .unwrap_or(NaiveDate::MAX)
}

/// Midnight UTC at the start of `day`.
pub(super) fn start_of(day: NaiveDate) -> DateTime<Utc> {
    day.and_time(NaiveTime::MIN).and_utc()
}

/// First day of an archive `period` (`YYYY-MM`).
pub(super) fn parse_period(period: &str) -> Option<NaiveDate> {
    NaiveDate::parse_from_str(&format!("{period}-01"), "%Y-%m-%d").ok()
}

/// A month may be archived once `grace_days` have passed since it closed.
fn archivable(month: NaiveDate, now: DateTime<Utc>, grace_days: u32) -> bool {
    next_month(month)
        .checked_add_days(Days::new(u64::from(grace_days)))
        .is_some_and(|day| now >= start_of(day))
}

/// The month after the newest archived one, or the month of the oldest data while
/// nothing is archived yet.
fn candidate_month(
    last_archived: Option<&str>,
    earliest_data: Option<DateTime<Utc>>,
) -> Option<NaiveDate> {
    match last_archived {
        Some(period) => parse_period(period).map(next_month),
        None => earliest_data.map(|at| month_of(at.date_naive())),
    }
}

/// 31 December of the `years`th calendar year after the month.
fn retain_until(month: NaiveDate, years: u32) -> NaiveDate {
    i32::try_from(years)
        .ok()
        .and_then(|years| month.year().checked_add(years))
        .and_then(|year| NaiveDate::from_ymd_opt(year, 12, 31))
        .unwrap_or(NaiveDate::MAX)
}

/// Whether the part ends after `epoch_seq`. The month's last epoch never ends a part,
/// so the seals without an epoch and the quarantined records join it.
fn starts_new_part(uncompressed: u64, epoch_seq: i64, month_last_epoch: Option<i64>) -> bool {
    uncompressed > PART_BYTES && month_last_epoch.is_some_and(|last| epoch_seq < last)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Progress {
    /// Every part is recorded; only the manifest is missing.
    PartsComplete,
    /// Continue after this epoch; `None` starts at the beginning of the month.
    ContinueAfter(Option<i64>),
}

/// Oldest time any data that belongs in an archive was written.
async fn earliest_data(db: &DatabaseConnection) -> flow_like_types::Result<Option<DateTime<Utc>>> {
    let epoch = audit_epoch::Entity::find()
        .order_by_asc(audit_epoch::Column::Seq)
        .one(db)
        .await?
        .map(|epoch| epoch.created_at);
    let unanchored = audit_seal::Entity::find()
        .filter(audit_seal::Column::EpochSeq.is_null())
        .filter(audit_seal::Column::Class.eq(RetentionClass::Evidence.as_str()))
        .order_by_asc(audit_seal::Column::SealedAt)
        .one(db)
        .await?
        .map(|seal| seal.sealed_at);
    let quarantined = audit_record::Entity::find()
        .filter(audit_record::Column::SealId.eq(INVALID_SEAL_ID))
        .order_by_asc(audit_record::Column::Timestamp)
        .one(db)
        .await?
        .map(|record| record.timestamp);
    let pending = audit_record::Entity::find()
        .filter(audit_record::Column::SealId.is_null())
        .order_by_asc(audit_record::Column::Timestamp)
        .one(db)
        .await?
        .map(|record| record.timestamp);
    Ok([epoch, unanchored, quarantined, pending]
        .into_iter()
        .flatten()
        .min()
        .map(|at| at.with_timezone(&Utc)))
}

/// Whether the month can still change: a record before its end is pending, or, with an
/// audit key, a seal before its end still waits for its epoch. Seals of held chains
/// never get an epoch; they are an integrity incident that stays in the database,
/// reported by verification, and does not stop the month from being archived.
async fn still_changing(
    context: &AuditWorkerContext,
    end: DateTime<FixedOffset>,
) -> flow_like_types::Result<bool> {
    let pending = audit_record::Entity::find()
        .filter(audit_record::Column::SealId.is_null())
        .filter(audit_record::Column::Timestamp.lt(end))
        .one(&context.db)
        .await?;
    if pending.is_some() {
        return Ok(true);
    }
    if !context.signing_configured() {
        return Ok(false);
    }
    let db = &context.db;
    Ok(db
        .query_one_raw(Statement::from_sql_and_values(
            db.get_database_backend(),
            WAITING_BEFORE_SQL,
            [sea_orm::Value::from(end)],
        ))
        .await?
        .is_some())
}

async fn recorded_parts(
    db: &DatabaseConnection,
    period: &str,
) -> flow_like_types::Result<Vec<audit_archive::Model>> {
    Ok(audit_archive::Entity::find()
        .filter(audit_archive::Column::Period.eq(period))
        .filter(audit_archive::Column::Part.gt(0))
        .order_by_asc(audit_archive::Column::Part)
        .all(db)
        .await?)
}

struct VerifiedParts {
    rows: Vec<audit_archive::Model>,
    progress: Progress,
    last_receipt: Option<String>,
}

/// Receipts stay outside `archive/`, so archive-tier transitions never require a
/// restore to resume the worker. Only the worker's bucket identity can write them.
fn receipt_key(period: &str, part: i32) -> flow_like_types::Result<Path> {
    if parse_period(period).is_none() || part < 0 {
        return Err(flow_like_types::anyhow!(
            "invalid audit archive receipt location"
        ));
    }
    Ok(Path::from(format!(
        "receipts/archive/{}/{part:04}.json",
        period.replacen('-', "/", 1)
    )))
}

fn receipt_context(
    month: &Month,
    first: Option<&audit_epoch::Model>,
    last: Option<&audit_epoch::Model>,
    raw: RawValues,
    include_unanchored: bool,
) -> Value {
    serde_json::json!({
        "period": month.period,
        "first_epoch": first.map(EpochLine::from),
        "last_epoch": last.map(EpochLine::from),
        "raw_ip": raw.ip,
        "raw_details": raw.details,
        "include_unanchored": include_unanchored,
    })
}

fn receipt_row(row: &audit_archive::Model) -> flow_like_types::Result<Value> {
    let mut metadata = serde_json::to_value(row)?;
    // This acknowledgement changes after uploading the already committed manifest.
    metadata.as_object_mut().unwrap().remove("uploaded_at");
    Ok(metadata)
}

async fn write_receipt(
    bucket: &FlowLikeStore,
    row: &audit_archive::Model,
    context: &Value,
    previous: Option<&str>,
    complete: bool,
) -> flow_like_types::Result<String> {
    let receipt = serde_json::json!({
        "format": RECEIPT_FORMAT,
        "row": receipt_row(row)?,
        "context": context,
        "previous_receipt_sha256": previous,
        "complete": complete,
    });
    let bytes = serde_json::to_vec(&receipt)?;
    if bytes.len() > MAX_RECEIPT_BYTES {
        return Err(flow_like_types::anyhow!(
            "audit archive receipt exceeds its size limit"
        ));
    }
    let written = put_bytes(bucket, &receipt_key(&row.period, row.part)?, bytes).await?;
    Ok(hex::encode(written.sha256))
}

/// Check the trusted copy before using any archive row to resume or sign. The path
/// comes from the period and part, never from a database-controlled object key.
async fn read_receipt(
    bucket: &FlowLikeStore,
    row: &audit_archive::Model,
) -> flow_like_types::Result<(Value, String)> {
    let (receipt, digest) = load_receipt(bucket, &row.period, row.part)
        .await?
        .ok_or_else(|| {
            flow_like_types::anyhow!(
                "audit archive {} part {} has no trusted receipt",
                row.period,
                row.part
            )
        })?;
    if receipt["format"] != RECEIPT_FORMAT || receipt["row"] != receipt_row(row)? {
        return Err(flow_like_types::anyhow!(
            "audit archive {} part {} differs from its trusted receipt",
            row.period,
            row.part
        ));
    }
    Ok((receipt, digest))
}

async fn load_receipt(
    bucket: &FlowLikeStore,
    period: &str,
    part: i32,
) -> flow_like_types::Result<Option<(Value, String)>> {
    let key = receipt_key(period, part)?;
    let object = match bucket.as_generic().get(&key).await {
        Ok(object) => object,
        Err(StoreError::NotFound { .. }) => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    let mut stream = object.into_stream();
    let mut bytes = Vec::new();
    while let Some(chunk) = stream.try_next().await? {
        if bytes.len().saturating_add(chunk.len()) > MAX_RECEIPT_BYTES {
            return Err(flow_like_types::anyhow!(
                "audit archive receipt {key} exceeds its size limit"
            ));
        }
        bytes.extend_from_slice(&chunk);
    }
    let receipt = serde_json::from_slice(&bytes)?;
    Ok(Some((receipt, hex::encode(Sha256::digest(&bytes)))))
}

fn recovered_row(
    receipt: &Value,
    period: &str,
    part: i32,
    context: &Value,
    previous: Option<&str>,
    complete: bool,
) -> flow_like_types::Result<audit_archive::Model> {
    let row: audit_archive::Model = serde_json::from_value(receipt["row"].clone())?;
    let key = if part == 0 {
        archive_manifest_key(period)
    } else {
        archive_part_key(period, part)
    };
    if receipt["format"] != RECEIPT_FORMAT
        || &receipt["context"] != context
        || receipt["previous_receipt_sha256"] != serde_json::json!(previous)
        || receipt["complete"] != complete
        || row.period != period
        || row.part != part
        || row.object_key != key.to_string()
        || row.sha256.len() != 32
        || row.byte_size < 0
        || row.epoch_count < 0
        || row.seal_count < 0
        || row.record_count < 0
        || receipt["row"] != receipt_row(&row)?
    {
        return Err(flow_like_types::anyhow!(
            "audit archive {period} part {part} has inconsistent recovery metadata"
        ));
    }
    Ok(row)
}

/// The object was replayed and its compressed digest verified before this function.
/// Keep the original receipt timestamp if the upload survived a failed database commit.
async fn record_part_receipt(
    bucket: &FlowLikeStore,
    row: audit_archive::Model,
    context: &Value,
    previous: Option<&str>,
    complete: bool,
) -> flow_like_types::Result<(audit_archive::Model, String)> {
    if let Some((receipt, digest)) = load_receipt(bucket, &row.period, row.part).await? {
        let mut original =
            recovered_row(&receipt, &row.period, row.part, context, previous, complete)?;
        let mut replay = row;
        replay.created_at = original.created_at;
        if receipt_row(&replay)? != receipt_row(&original)? {
            return Err(flow_like_types::anyhow!(
                "audit archive {} part {} replay differs from its trusted receipt",
                original.period,
                original.part
            ));
        }
        original.uploaded_at = Some(original.created_at);
        return Ok((original, digest));
    }
    let digest = write_receipt(bucket, &row, context, previous, complete).await?;
    Ok((row, digest))
}

/// A receipt can precede the manifest's database commit. Recover its exact signed
/// bytes rather than creating another timestamp or spending another signature.
async fn recover_manifest(
    bucket: &FlowLikeStore,
    period: &str,
    expected: &Value,
    parts: &[audit_archive::Model],
    context: &Value,
    previous: Option<&str>,
    signing: bool,
) -> flow_like_types::Result<Option<audit_archive::Model>> {
    let Some((receipt, _)) = load_receipt(bucket, period, 0).await? else {
        return Ok(None);
    };
    let row = recovered_row(&receipt, period, 0, context, previous, true)?;
    let mut expected = expected.clone();
    expected["created_at_ms"] = Value::from(row.created_at.timestamp_millis());
    if let Some(kid) = &row.kid {
        expected["kid"] = Value::from(kid.clone());
    }
    if row.manifest.as_ref() != Some(&expected)
        || row.first_epoch != expected["epochs"]["first"].as_i64()
        || row.last_epoch != expected["epochs"]["last"].as_i64()
        || row.epoch_count != part_total(parts, |part| part.epoch_count)
        || row.seal_count != part_total(parts, |part| part.seal_count)
        || row.record_count != part_total(parts, |part| part.record_count)
    {
        return Err(flow_like_types::anyhow!(
            "audit archive {period} recovered manifest differs from its verified parts"
        ));
    }
    match (row.kid.as_deref(), row.signature.as_deref()) {
        (Some(kid), Some(signature))
            if crate::audit::signer::verify(kid, &manifest_hash(&expected), signature)
                == crate::audit::signer::SignatureCheck::Valid => {}
        (None, None) if !signing => {}
        _ => {
            return Err(flow_like_types::anyhow!(
                "audit archive {period} recovered manifest signature cannot be verified"
            ));
        }
    }
    let bytes = manifest_bytes(&expected, row.signature.as_deref())?;
    if bytes.len() as i64 != row.byte_size
        || Sha256::digest(&bytes).as_slice() != row.sha256.as_slice()
    {
        return Err(flow_like_types::anyhow!(
            "audit archive {period} recovered manifest differs from its trusted digest"
        ));
    }
    Ok(Some(row))
}

async fn verified_parts(
    db: &DatabaseConnection,
    bucket: &FlowLikeStore,
    period: &str,
    context: &Value,
) -> flow_like_types::Result<VerifiedParts> {
    let rows = recorded_parts(db, period).await?;
    let mut progress = Progress::ContinueAfter(None);
    let mut previous: Option<String> = None;
    for (index, row) in rows.iter().enumerate() {
        let (receipt, digest) = read_receipt(bucket, row).await?;
        progress = receipt_progress(row, &receipt, context, index, previous.as_deref(), progress)?;
        previous = Some(digest);
    }
    Ok(VerifiedParts {
        rows,
        progress,
        last_receipt: previous,
    })
}

fn receipt_progress(
    row: &audit_archive::Model,
    receipt: &Value,
    context: &Value,
    index: usize,
    previous: Option<&str>,
    progress: Progress,
) -> flow_like_types::Result<Progress> {
    if row.part as usize != index + 1
        || progress == Progress::PartsComplete
        || &receipt["context"] != context
        || receipt["previous_receipt_sha256"] != serde_json::json!(previous)
    {
        return Err(flow_like_types::anyhow!(
            "audit archive {} part {} does not continue its trusted receipt chain",
            row.period,
            row.part
        ));
    }
    let last_epoch = context["last_epoch"]["seq"].as_i64();
    match receipt["complete"].as_bool() {
        Some(true) if row.last_epoch == last_epoch => Ok(Progress::PartsComplete),
        Some(false)
            if row
                .last_epoch
                .zip(last_epoch)
                .is_some_and(|(done, last)| done < last) =>
        {
            Ok(Progress::ContinueAfter(row.last_epoch))
        }
        _ => Err(flow_like_types::anyhow!(
            "audit archive {} part {} has inconsistent completion bounds",
            row.period,
            row.part
        )),
    }
}

/// The archive row of an object just written; callers fill in the counts.
pub(super) fn archive_row(
    period: &str,
    part: i32,
    key: &Path,
    written: WrittenObject,
    created_at: DateTime<FixedOffset>,
) -> audit_archive::ActiveModel {
    audit_archive::ActiveModel {
        period: Set(period.to_owned()),
        part: Set(part),
        object_key: Set(key.to_string()),
        sha256: Set(written.sha256.to_vec()),
        byte_size: Set(i64::try_from(written.byte_size).unwrap_or(i64::MAX)),
        first_epoch: Set(None),
        last_epoch: Set(None),
        epoch_count: Set(0),
        seal_count: Set(0),
        record_count: Set(0),
        created_at: Set(created_at),
        kid: Set(None),
        signature: Set(None),
        manifest: Set(None),
        uploaded_at: Set(Some(created_at)),
    }
}

fn part_total(parts: &[audit_archive::Model], count: fn(&audit_archive::Model) -> i64) -> i64 {
    parts.iter().map(count).sum()
}

fn manifest_json(
    month: &Month,
    archive_years: u32,
    parts: &[audit_archive::Model],
    first_epoch: Option<&audit_epoch::Model>,
    last_epoch: Option<&audit_epoch::Model>,
    raw: RawValues,
    created_at_ms: i64,
) -> Value {
    let parts_json: Vec<Value> = parts
        .iter()
        .map(|part| {
            serde_json::json!({
                "part": part.part,
                "key": part.object_key,
                "sha256": hex::encode(&part.sha256),
                "byte_size": part.byte_size,
                "first_epoch": part.first_epoch,
                "last_epoch": part.last_epoch,
                "epoch_count": part.epoch_count,
                "seal_count": part.seal_count,
                "record_count": part.record_count,
            })
        })
        .collect();
    serde_json::json!({
        "format": MANIFEST_FORMAT,
        "period": month.period,
        "retain_until": retain_until(month.first_day, archive_years).format("%Y-%m-%d").to_string(),
        "parts": parts_json,
        "epochs": {
            "first": first_epoch.map(|epoch| epoch.seq),
            "last": last_epoch.map(|epoch| epoch.seq),
            "first_prev_hash": first_epoch.map(|epoch| hex::encode(&epoch.prev_hash)),
            "last_hash": last_epoch.map(|epoch| hex::encode(&epoch.hash)),
        },
        "seal_count": part_total(parts, |part| part.seal_count),
        "record_count": part_total(parts, |part| part.record_count),
        "raw_ip": raw.ip,
        "raw_details": raw.details,
        "created_at_ms": created_at_ms,
    })
}

/// A chain's watermark `(seq, hash)`, only when its signature verifies.
async fn verified_watermark(
    db: &DatabaseConnection,
    chain_id: &str,
) -> Result<Option<(i64, Vec<u8>)>, String> {
    let watermark = audit_watermark::Entity::find_by_id(chain_id)
        .one(db)
        .await
        .map_err(|error| error.to_string())?;
    Ok(watermark
        .filter(|watermark| check_watermark(watermark) == Ok(true))
        .map(|watermark| (watermark.seq, watermark.hash)))
}

struct Month {
    first_day: NaiveDate,
    period: String,
    start: DateTime<FixedOffset>,
    end: DateTime<FixedOffset>,
}

impl Month {
    fn new(first_day: NaiveDate) -> Self {
        Self {
            first_day,
            period: period_of(first_day),
            start: start_of(first_day).fixed_offset(),
            end: start_of(next_month(first_day)).fixed_offset(),
        }
    }
}

struct OpenPart {
    number: i32,
    key: Path,
    writer: ArchiveWriter,
    first_epoch: Option<i64>,
    last_epoch: Option<i64>,
    epochs: i64,
    seals: i64,
    records: i64,
}

struct MonthExport<'a> {
    db: &'a DatabaseConnection,
    bucket: &'a FlowLikeStore,
    lease: &'a Lease,
    month: &'a Month,
    raw: RawValues,
    /// Seals without an epoch belong in the archive only when there is no audit key;
    /// with one, an unanchored seal before the month's end is a held-back incident.
    include_unanchored: bool,
    created_at: DateTime<FixedOffset>,
    next_part: i32,
    open: Option<OpenPart>,
    /// Parts finished and recorded by this export.
    written: u64,
    /// The last epoch written, `(seq, hash)`: the month's epochs must be consecutive
    /// and linked, so a deleted or moved epoch aborts the month.
    previous_epoch: Option<(i64, Vec<u8>)>,
    /// Each evidence chain's last written seal, `(seq, hash)`: every seal must continue
    /// its chain, so a deleted or reclassified seal aborts the month.
    chain_tails: HashMap<String, (i64, Vec<u8>)>,
    receipt_context: &'a Value,
    previous_receipt: Option<String>,
}

impl MonthExport<'_> {
    /// The epoch before `epoch` must be the one written last, or, for the month's first
    /// epoch, the stored epoch before it, the timeline's verified watermark, or none.
    async fn check_epoch_link(&mut self, epoch: &audit_epoch::Model) -> Result<(), String> {
        let previous = match self.previous_epoch.take() {
            Some(previous) => Some(previous),
            None if epoch.seq == 1 => None,
            None => match audit_epoch::Entity::find_by_id(epoch.seq - 1)
                .one(self.db)
                .await
                .map_err(|error| error.to_string())?
            {
                Some(previous) => Some((previous.seq, previous.hash)),
                None => verified_watermark(self.db, EPOCH_WATERMARK)
                    .await?
                    .filter(|(seq, _)| *seq == epoch.seq - 1),
            },
        };
        let links = match &previous {
            Some((seq, hash)) => epoch.seq == seq + 1 && &epoch.prev_hash == hash,
            None => epoch.seq == 1 && epoch.prev_hash.as_slice() == ZERO_HASH.as_slice(),
        };
        if !links {
            return Err(format!(
                "epoch {} does not continue the epoch timeline",
                epoch.seq
            ));
        }
        self.previous_epoch = Some((epoch.seq, epoch.hash.clone()));
        Ok(())
    }

    /// `seal` must continue its chain: the seal written before it, or the chain's stored
    /// predecessor, or its verified watermark, or the start of the chain.
    async fn check_chain_link(&mut self, seal: &audit_seal::Model) -> Result<(), String> {
        let previous = match self.chain_tails.get(&seal.chain_id) {
            Some(tail) => Some(tail.clone()),
            None if seal.seq == 1 => None,
            None => match audit_seal::Entity::find()
                .filter(audit_seal::Column::ChainId.eq(seal.chain_id.as_str()))
                .filter(audit_seal::Column::Seq.eq(seal.seq - 1))
                .one(self.db)
                .await
                .map_err(|error| error.to_string())?
            {
                Some(previous) => Some((previous.seq, previous.hash)),
                None => verified_watermark(self.db, &seal.chain_id)
                    .await?
                    .filter(|(seq, _)| *seq == seal.seq - 1),
            },
        };
        let links = match &previous {
            Some((seq, hash)) => seal.seq == seq + 1 && &seal.prev_hash == hash,
            None => seal.seq == 1 && seal.prev_hash.as_slice() == ZERO_HASH.as_slice(),
        };
        if !links {
            return Err(format!(
                "seal {} (seq {}) does not continue chain {}",
                seal.id, seal.seq, seal.chain_id
            ));
        }
        self.chain_tails
            .insert(seal.chain_id.clone(), (seal.seq, seal.hash.clone()));
        Ok(())
    }
    async fn stream(
        &mut self,
        after: Option<i64>,
        month_last_epoch: Option<i64>,
    ) -> flow_like_types::Result<()> {
        let mut cursor = after;
        loop {
            self.keepalive().await?;
            let mut query = audit_epoch::Entity::find()
                .filter(audit_epoch::Column::CreatedAt.gte(self.month.start))
                .filter(audit_epoch::Column::CreatedAt.lt(self.month.end));
            if let Some(seq) = cursor {
                query = query.filter(audit_epoch::Column::Seq.gt(seq));
            }
            let epochs = query
                .order_by_asc(audit_epoch::Column::Seq)
                .limit(EPOCH_PAGE)
                .all(self.db)
                .await?;
            for epoch in &epochs {
                self.write_epoch(epoch).await?;
                cursor = Some(epoch.seq);
                if starts_new_part(self.uncompressed_bytes(), epoch.seq, month_last_epoch) {
                    self.finish_part(false).await?;
                }
            }
            if (epochs.len() as u64) < EPOCH_PAGE {
                break;
            }
        }
        if cursor != month_last_epoch {
            return Err(self.rejected(
                cursor,
                None,
                "archive epoch range changed during export".into(),
            ));
        }
        if self.include_unanchored {
            self.write_unanchored_seals().await?;
        }
        self.write_quarantined_records().await
    }

    async fn write_epoch(&mut self, epoch: &audit_epoch::Model) -> flow_like_types::Result<()> {
        match check_epoch(epoch) {
            Ok(true) => {}
            Ok(false) => {
                return Err(self.rejected(
                    Some(epoch.seq),
                    None,
                    format!(
                        "epoch {} is signed with key {}, which has no registered public key",
                        epoch.seq, epoch.kid
                    ),
                ));
            }
            Err(problem) => return Err(self.rejected(Some(epoch.seq), None, problem)),
        }
        if let Err(problem) = self.check_epoch_link(epoch).await {
            return Err(self.rejected(Some(epoch.seq), None, problem));
        }
        let Some(seals_root) = to_hash(&epoch.seals_root) else {
            return Err(self.rejected(
                Some(epoch.seq),
                None,
                format!("epoch {} root has the wrong length", epoch.seq),
            ));
        };
        let part = self.write(&Line::Epoch(EpochLine::from(epoch))).await?;
        part.epochs += 1;
        part.first_epoch = part.first_epoch.or(Some(epoch.seq));
        part.last_epoch = Some(epoch.seq);

        // Selected by epoch only: a reclassified seal must fail its check, not vanish.
        let mut after_index: Option<i32> = None;
        loop {
            self.keepalive().await?;
            let mut query =
                audit_seal::Entity::find().filter(audit_seal::Column::EpochSeq.eq(epoch.seq));
            if let Some(index) = after_index {
                query = query.filter(audit_seal::Column::EpochIndex.gt(index));
            }
            let seals = query
                .order_by_asc(audit_seal::Column::EpochIndex)
                .limit(SEAL_PAGE)
                .all(self.db)
                .await?;
            let (activity, evidence): (Vec<_>, Vec<_>) = seals
                .iter()
                .cloned()
                .partition(|seal| chain_class(&seal.chain_id) == RetentionClass::Activity);
            for seal in &activity {
                // Activity is never archived, but its seal must still be what the epoch
                // signed.
                let hash = seal_hash_of(seal);
                let checked = if hash.as_slice() != seal.hash.as_slice() {
                    Err(format!("seal {} hash does not match its fields", seal.id))
                } else if seal.class != RetentionClass::Activity.as_str() {
                    Err(format!("seal {} class does not match its chain", seal.id))
                } else {
                    verify_anchor(seal, &hash, epoch, &seals_root)
                };
                if let Err(problem) = checked {
                    return Err(self.rejected(Some(epoch.seq), Some(seal.id.as_str()), problem));
                }
            }
            self.write_seals(&evidence, Some((epoch, &seals_root)))
                .await?;
            if (seals.len() as u64) < SEAL_PAGE {
                return Ok(());
            }
            after_index = seals.last().and_then(|seal| seal.epoch_index);
        }
    }

    /// Evidence seals no epoch covers (deployments without an audit key), by chain and seq.
    async fn write_unanchored_seals(&mut self) -> flow_like_types::Result<()> {
        let mut cursor: Option<(String, i64)> = None;
        loop {
            self.keepalive().await?;
            let mut query = audit_seal::Entity::find()
                .filter(audit_seal::Column::EpochSeq.is_null())
                .filter(audit_seal::Column::Class.eq(RetentionClass::Evidence.as_str()))
                .filter(audit_seal::Column::SealedAt.gte(self.month.start))
                .filter(audit_seal::Column::SealedAt.lt(self.month.end));
            if let Some((chain_id, seq)) = &cursor {
                // A row-value keyset is a range on the (chainId, seq) index.
                query = query.filter(Expr::cust_with_values(
                    r#"("chainId", "seq") > ($1, $2)"#,
                    [
                        sea_orm::Value::from(chain_id.clone()),
                        sea_orm::Value::from(*seq),
                    ],
                ));
            }
            let seals = query
                .order_by_asc(audit_seal::Column::ChainId)
                .order_by_asc(audit_seal::Column::Seq)
                .limit(SEAL_PAGE)
                .all(self.db)
                .await?;
            self.write_seals(&seals, None).await?;
            if (seals.len() as u64) < SEAL_PAGE {
                return Ok(());
            }
            cursor = seals.last().map(|seal| (seal.chain_id.clone(), seal.seq));
        }
    }

    async fn write_quarantined_records(&mut self) -> flow_like_types::Result<()> {
        let raw = self.raw;
        let mut cursor: Option<(DateTime<FixedOffset>, String)> = None;
        loop {
            self.keepalive().await?;
            // Activity is never archived, quarantined or not.
            let mut query = audit_record::Entity::find()
                .filter(audit_record::Column::SealId.eq(INVALID_SEAL_ID))
                .filter(audit_record::Column::ChainId.not_like(format!("%{ACTIVITY_SUFFIX}")))
                .filter(audit_record::Column::Timestamp.gte(self.month.start))
                .filter(audit_record::Column::Timestamp.lt(self.month.end));
            if let Some((timestamp, id)) = &cursor {
                query = query.filter(
                    Condition::any()
                        .add(audit_record::Column::Timestamp.gt(*timestamp))
                        .add(
                            Condition::all()
                                .add(audit_record::Column::Timestamp.eq(*timestamp))
                                .add(audit_record::Column::Id.gt(id.as_str())),
                        ),
                );
            }
            let records = query
                .order_by_asc(audit_record::Column::Timestamp)
                .order_by_asc(audit_record::Column::Id)
                .limit(RECORD_PAGE)
                .all(self.db)
                .await?;
            for record in &records {
                let part = self
                    .write(&Line::Record(RecordLine::from_model(record, raw)))
                    .await?;
                part.records += 1;
            }
            if (records.len() as u64) < RECORD_PAGE {
                return Ok(());
            }
            cursor = records
                .last()
                .map(|record| (record.timestamp, record.id.clone()));
        }
    }

    /// Verify and write `seals`, each followed by its records in canonical order.
    /// `anchor` is the epoch the seals belong to and its root.
    async fn write_seals(
        &mut self,
        seals: &[audit_seal::Model],
        anchor: Option<(&audit_epoch::Model, &Hash)>,
    ) -> flow_like_types::Result<()> {
        let raw = self.raw;
        let epoch_seq = anchor.map(|(epoch, _)| epoch.seq);
        let mut start = 0;
        while start < seals.len() {
            let mut end = start;
            let mut budget = 0u64;
            while end < seals.len() && (end == start || budget < RECORD_PAGE) {
                budget += u64::try_from(seals[end].record_count).unwrap_or(0);
                end += 1;
            }
            let group = &seals[start..end];
            let records = audit_record::Entity::find()
                .filter(
                    audit_record::Column::SealId.is_in(group.iter().map(|seal| seal.id.clone())),
                )
                .all(self.db)
                .await?;
            let mut by_seal: HashMap<String, Vec<audit_record::Model>> = HashMap::new();
            for record in records {
                if let Some(seal_id) = record.seal_id.clone() {
                    by_seal.entry(seal_id).or_default().push(record);
                }
            }
            for seal in group {
                let mut members = by_seal.remove(&seal.id).unwrap_or_default();
                let checked = verify_seal(seal, &mut members).and_then(|hash| match anchor {
                    Some((epoch, seals_root)) => verify_anchor(seal, &hash, epoch, seals_root),
                    None => verify_unanchored(seal),
                });
                let checked = match checked {
                    Ok(()) => self.check_chain_link(seal).await,
                    Err(problem) => Err(problem),
                };
                if let Err(problem) = checked {
                    return Err(self.rejected(epoch_seq, Some(seal.id.as_str()), problem));
                }
                let part = self.write(&Line::Seal(SealLine::from(seal))).await?;
                part.seals += 1;
                for record in &members {
                    let part = self
                        .write(&Line::Record(RecordLine::from_model(record, raw)))
                        .await?;
                    part.records += 1;
                }
            }
            self.keepalive().await?;
            start = end;
        }
        Ok(())
    }

    /// Append a line, opening the next part on first use.
    async fn write(&mut self, line: &Line) -> flow_like_types::Result<&mut OpenPart> {
        if self.open.is_none() {
            let key = archive_part_key(&self.month.period, self.next_part);
            let writer = ArchiveWriter::create(self.bucket, key.clone()).await?;
            self.open = Some(OpenPart {
                number: self.next_part,
                key,
                writer,
                first_epoch: None,
                last_epoch: None,
                epochs: 0,
                seals: 0,
                records: 0,
            });
            self.next_part += 1;
        }
        let Some(part) = self.open.as_mut() else {
            return Err(flow_like_types::anyhow!(
                "audit archive part of {} was not opened",
                self.month.period
            ));
        };
        part.writer.write_line(&encode(line)).await?;
        Ok(part)
    }

    fn uncompressed_bytes(&self) -> u64 {
        self.open
            .as_ref()
            .map_or(0, |part| part.writer.uncompressed_bytes())
    }

    /// Complete the open part's upload and record it.
    async fn finish_part(&mut self, complete: bool) -> flow_like_types::Result<()> {
        let Some(part) = self.open.take() else {
            return Ok(());
        };
        let written = part.writer.finish().await?;
        let mut row = archive_row(
            &self.month.period,
            part.number,
            &part.key,
            written,
            self.created_at,
        );
        row.first_epoch = Set(part.first_epoch);
        row.last_epoch = Set(part.last_epoch);
        row.epoch_count = Set(part.epochs);
        row.seal_count = Set(part.seals);
        row.record_count = Set(part.records);
        let (row, receipt) = record_part_receipt(
            self.bucket,
            row.try_into_model()?,
            self.receipt_context,
            self.previous_receipt.as_deref(),
            complete,
        )
        .await?;
        audit_archive::Entity::insert(row.into_active_model())
            .exec_without_returning(self.db)
            .await?;
        self.previous_receipt = Some(receipt);
        self.written += 1;
        Ok(())
    }

    async fn abort(&mut self) {
        if let Some(part) = self.open.take() {
            part.writer.abort().await;
        }
    }

    /// `&mut self` keeps the future `Send`: the open upload is `Send` but not `Sync`.
    async fn keepalive(&mut self) -> flow_like_types::Result<()> {
        if self.lease.keepalive().await {
            Ok(())
        } else {
            Err(flow_like_types::anyhow!(
                "audit worker lost its lease while archiving {}",
                self.month.period
            ))
        }
    }

    fn rejected(
        &self,
        epoch: Option<i64>,
        seal_id: Option<&str>,
        problem: String,
    ) -> flow_like_types::Error {
        tracing::error!(
            target: "audit",
            period = %self.month.period,
            epoch = ?epoch,
            seal_id = ?seal_id,
            %problem,
            "audit archive verification failed; the month stays unarchived"
        );
        flow_like_types::anyhow!(
            "archiving audit month {} stopped: {problem}",
            self.month.period
        )
    }
}

/// Re-check a seal against its records: seal hash, record hashes and commitments, the
/// records root and the time range. Returns the seal hash.
fn verify_seal(
    seal: &audit_seal::Model,
    records: &mut [audit_record::Model],
) -> Result<Hash, String> {
    let hash = seal_hash_of(seal);
    if hash.as_slice() != seal.hash.as_slice() {
        return Err(format!("seal {} hash does not match its fields", seal.id));
    }
    if seal.class != chain_class(&seal.chain_id).as_str() {
        return Err(format!("seal {} class does not match its chain", seal.id));
    }
    if usize::try_from(seal.record_count).ok() != Some(records.len()) {
        return Err(format!(
            "seal {} holds {} records but {} remain",
            seal.id,
            seal.record_count,
            records.len()
        ));
    }
    sort_records(records);
    let mut leaves = Vec::with_capacity(records.len());
    for record in records.iter() {
        if record.chain_id != seal.chain_id {
            return Err(format!(
                "record {} of seal {} belongs to chain {}",
                record.id, seal.id, record.chain_id
            ));
        }
        if record.mac.is_some() {
            return Err(format!("sealed record {} kept a MAC", record.id));
        }
        leaves.push(check_record(record)?.0);
    }
    if merkle::root(&leaves).as_slice() != seal.records_root.as_slice() {
        return Err(format!("records of seal {} do not match its root", seal.id));
    }
    if records.first().map(|record| record.timestamp) != Some(seal.first_at)
        || records.last().map(|record| record.timestamp) != Some(seal.last_at)
    {
        return Err(format!("record times fall outside seal {}", seal.id));
    }
    Ok(hash)
}

fn verify_anchor(
    seal: &audit_seal::Model,
    hash: &Hash,
    epoch: &audit_epoch::Model,
    seals_root: &Hash,
) -> Result<(), String> {
    let (Some(index), Some(proof)) = (seal.epoch_index, seal.epoch_proof.as_deref()) else {
        return Err(format!("seal {} has a partial epoch reference", seal.id));
    };
    if seal.epoch_seq != Some(epoch.seq) {
        return Err(format!(
            "seal {} references epoch {:?}, not {}",
            seal.id, seal.epoch_seq, epoch.seq
        ));
    }
    let proof = merkle::decode_proof(proof)
        .ok_or_else(|| format!("seal {} epoch proof is malformed", seal.id))?;
    let (Ok(index), Ok(size)) = (u64::try_from(index), u64::try_from(epoch.seal_count)) else {
        return Err(format!("seal {} has a negative epoch position", seal.id));
    };
    if !merkle::verify_inclusion(hash, index, size, &proof, seals_root) {
        return Err(format!(
            "seal {} is not part of epoch {}",
            seal.id, epoch.seq
        ));
    }
    Ok(())
}

fn verify_unanchored(seal: &audit_seal::Model) -> Result<(), String> {
    if seal.epoch_seq.is_some() || seal.epoch_index.is_some() || seal.epoch_proof.is_some() {
        return Err(format!("seal {} has a partial epoch reference", seal.id));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audit::crypto::ZERO_HASH;
    use crate::audit::signer::{
        AuditSigner, LocalSigner, SignatureCheck, register_verifying_key, verify,
    };
    use crate::entity::sea_orm_active_enums::AuditActorType;
    use p256::ecdsa::SigningKey;

    fn day(year: i32, month: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(year, month, day).unwrap()
    }

    fn at(text: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(text)
            .unwrap()
            .with_timezone(&Utc)
    }

    fn part(number: i32, last_epoch: Option<i64>) -> audit_archive::Model {
        audit_archive::Model {
            period: "2026-09".into(),
            part: number,
            object_key: archive_part_key("2026-09", number).to_string(),
            sha256: vec![7; 32],
            byte_size: 1_234,
            first_epoch: last_epoch.map(|last| last - 8),
            last_epoch,
            epoch_count: 9,
            seal_count: 20,
            record_count: 300,
            created_at: at("2026-10-04T00:01:00Z").fixed_offset(),
            kid: None,
            signature: None,
            manifest: None,
            uploaded_at: Some(at("2026-10-04T00:01:00Z").fixed_offset()),
        }
    }

    fn epoch(seq: i64, prev: u8, hash: u8) -> audit_epoch::Model {
        audit_epoch::Model {
            seq,
            prev_hash: vec![prev; 32],
            seal_count: 1,
            seals_root: vec![0; 32],
            created_at: at("2026-09-02T00:00:00Z").fixed_offset(),
            hash: vec![hash; 32],
            kid: "kid".into(),
            signature: vec![0; 64],
        }
    }

    fn record(id: &str, ms: i64) -> audit_record::Model {
        audit_record::Model {
            id: id.into(),
            chain_id: "app".into(),
            timestamp: DateTime::from_timestamp_millis(ms).unwrap().fixed_offset(),
            actor_id: "user".into(),
            actor_type: AuditActorType::User,
            action: "board.update".into(),
            resource_type: "Board".into(),
            resource_id: "board".into(),
            ip_commitment: None,
            details_commitment: None,
            actor_ip: None,
            ip_salt: None,
            details: None,
            details_salt: None,
            mac: None,
            seal_id: Some("seal".into()),
        }
    }

    fn seal_over(records: &[audit_record::Model]) -> audit_seal::Model {
        let leaves: Vec<Hash> = records
            .iter()
            .map(|record| check_record(record).unwrap().0)
            .collect();
        let mut seal = audit_seal::Model {
            id: "seal".into(),
            chain_id: "app".into(),
            seq: 1,
            class: "evidence".into(),
            prev_hash: ZERO_HASH.to_vec(),
            record_count: records.len() as i32,
            records_root: merkle::root(&leaves).to_vec(),
            first_at: records[0].timestamp,
            last_at: records[records.len() - 1].timestamp,
            sealed_at: records[records.len() - 1].timestamp,
            hash: Vec::new(),
            epoch_seq: None,
            epoch_index: None,
            epoch_proof: None,
            mac: None,
            ip_pending: false,
            details_pending: false,
        };
        seal.hash = seal_hash_of(&seal).to_vec();
        seal
    }

    #[test]
    fn months_roll_over_in_utc() {
        assert_eq!(month_of(day(2026, 9, 17)), day(2026, 9, 1));
        assert_eq!(next_month(day(2026, 12, 1)), day(2027, 1, 1));
        assert_eq!(next_month(day(2026, 1, 31)), day(2026, 2, 1));
        assert_eq!(parse_period("2026-09"), Some(day(2026, 9, 1)));
        assert_eq!(parse_period(LEGACY_PERIOD), None);
        assert_eq!(start_of(day(2026, 9, 1)), at("2026-09-01T00:00:00Z"));
    }

    #[test]
    fn a_month_is_archivable_after_its_grace_days() {
        let august = day(2026, 8, 1);
        assert!(!archivable(august, at("2026-09-03T23:59:59Z"), 3));
        assert!(archivable(august, at("2026-09-04T00:00:00Z"), 3));
        assert!(archivable(august, at("2026-09-01T00:00:00Z"), 0));
        assert!(!archivable(august, at("2026-08-31T23:59:59Z"), 0));
        assert!(!archivable(day(2026, 12, 1), at("2027-01-02T12:00:00Z"), 3));
    }

    #[test]
    fn the_next_month_follows_the_archive_or_the_oldest_data() {
        assert_eq!(
            candidate_month(Some("2026-08"), None),
            Some(day(2026, 9, 1))
        );
        assert_eq!(
            candidate_month(Some("2026-12"), None),
            Some(day(2027, 1, 1))
        );
        assert_eq!(
            candidate_month(None, Some(at("2026-07-15T08:00:00Z"))),
            Some(day(2026, 7, 1))
        );
        assert_eq!(candidate_month(None, None), None);
    }

    #[test]
    fn retention_ends_with_the_calendar_year() {
        assert_eq!(retain_until(day(2026, 9, 1), 3), day(2029, 12, 31));
        assert_eq!(retain_until(day(2026, 1, 1), 0), day(2026, 12, 31));
        assert_eq!(retain_until(day(2026, 1, 1), u32::MAX), NaiveDate::MAX);
    }

    #[test]
    fn parts_split_only_between_epochs_before_the_last() {
        assert!(!starts_new_part(PART_BYTES, 3, Some(9)));
        assert!(starts_new_part(PART_BYTES + 1, 3, Some(9)));
        assert!(!starts_new_part(PART_BYTES + 1, 9, Some(9)));
        assert!(!starts_new_part(PART_BYTES + 1, 3, None));
    }

    fn receipt_bucket() -> FlowLikeStore {
        FlowLikeStore::Memory(std::sync::Arc::new(
            flow_like_storage::object_store::memory::InMemory::new(),
        ))
    }

    fn test_receipt_context() -> Value {
        receipt_context(
            &Month::new(day(2026, 9, 1)),
            Some(&epoch(1, 0, 1)),
            Some(&epoch(9, 8, 9)),
            RawValues::NONE,
            false,
        )
    }

    #[flow_like_types::tokio::test]
    async fn database_rows_cannot_authorize_archive_resume_without_bucket_receipts() {
        let bucket = receipt_bucket();
        // Both rows used to bypass the export loop and reach manifest signing.
        for last in [Some(9), None] {
            assert!(read_receipt(&bucket, &part(1, last)).await.is_err());
        }
        let row = part(1, Some(9));
        let context = test_receipt_context();
        write_receipt(&bucket, &row, &context, None, true)
            .await
            .unwrap();
        let (receipt, _) = read_receipt(&bucket, &row).await.unwrap();
        assert_eq!(
            receipt_progress(
                &row,
                &receipt,
                &context,
                0,
                None,
                Progress::ContinueAfter(None)
            )
            .unwrap(),
            Progress::PartsComplete
        );

        for changed in [
            audit_archive::Model {
                last_epoch: None,
                ..row.clone()
            },
            audit_archive::Model {
                record_count: 0,
                ..row.clone()
            },
            audit_archive::Model {
                sha256: vec![8; 32],
                ..row.clone()
            },
            audit_archive::Model {
                object_key: "archive/other.jsonl.zst".into(),
                ..row.clone()
            },
            audit_archive::Model {
                period: "2026-08".into(),
                ..row.clone()
            },
        ] {
            assert!(read_receipt(&bucket, &changed).await.is_err());
        }
        let mut changed_context = context.clone();
        changed_context["raw_details"] = Value::Bool(true);
        assert!(
            receipt_progress(
                &row,
                &receipt,
                &changed_context,
                0,
                None,
                Progress::ContinueAfter(None)
            )
            .is_err()
        );
        changed_context = context.clone();
        changed_context["last_epoch"]["seq"] = Value::from(10);
        assert!(
            receipt_progress(
                &row,
                &receipt,
                &changed_context,
                0,
                None,
                Progress::ContinueAfter(None)
            )
            .is_err()
        );
    }

    #[flow_like_types::tokio::test]
    async fn trusted_parts_resume_in_order_and_require_explicit_completion() {
        let bucket = receipt_bucket();
        let context = test_receipt_context();
        let first = part(1, Some(4));
        let last = part(2, Some(9));
        let first_digest = write_receipt(&bucket, &first, &context, None, false)
            .await
            .unwrap();
        write_receipt(&bucket, &last, &context, Some(&first_digest), true)
            .await
            .unwrap();
        let (receipt, _) = read_receipt(&bucket, &first).await.unwrap();
        let resumed = receipt_progress(
            &first,
            &receipt,
            &context,
            0,
            None,
            Progress::ContinueAfter(None),
        )
        .unwrap();
        assert_eq!(resumed, Progress::ContinueAfter(Some(4)));
        let (receipt, _) = read_receipt(&bucket, &last).await.unwrap();
        assert_eq!(
            receipt_progress(&last, &receipt, &context, 1, Some(&first_digest), resumed).unwrap(),
            Progress::PartsComplete
        );
        assert!(receipt_progress(&last, &receipt, &context, 0, None, resumed).is_err());
        assert!(
            receipt_progress(
                &last,
                &receipt,
                &context,
                1,
                Some("replaced receipt"),
                resumed
            )
            .is_err()
        );
        let mut unfinished = receipt.clone();
        unfinished["complete"] = Value::Bool(false);
        assert!(
            receipt_progress(
                &last,
                &unfinished,
                &context,
                1,
                Some(&first_digest),
                resumed
            )
            .is_err()
        );
    }

    #[flow_like_types::tokio::test]
    async fn receipt_reads_are_bounded_and_uploaded_acknowledgements_can_change() {
        let bucket = receipt_bucket();
        let mut row = part(1, Some(9));
        write_receipt(&bucket, &row, &test_receipt_context(), None, true)
            .await
            .unwrap();
        row.uploaded_at = None;
        assert!(read_receipt(&bucket, &row).await.is_ok());
        bucket
            .as_generic()
            .put(
                &receipt_key(&row.period, row.part).unwrap(),
                flow_like_storage::object_store::PutPayload::from(vec![
                    b' ';
                    MAX_RECEIPT_BYTES + 1
                ]),
            )
            .await
            .unwrap();
        assert!(read_receipt(&bucket, &row).await.is_err());
        assert!(receipt_key("../../other", 1).is_err());
    }

    #[flow_like_types::tokio::test]
    async fn replayed_part_recovers_the_original_receipt_after_a_failed_database_commit() {
        let bucket = receipt_bucket();
        let context = test_receipt_context();
        let key = archive_part_key("2026-09", 1);
        let mut writer = ArchiveWriter::create(&bucket, key.clone()).await.unwrap();
        writer
            .write_line(b"verified archive contents\n")
            .await
            .unwrap();
        let written = writer.finish().await.unwrap();
        let mut original = part(1, Some(9));
        original.sha256 = written.sha256.to_vec();
        original.byte_size = written.byte_size as i64;
        let original_digest = write_receipt(&bucket, &original, &context, None, true)
            .await
            .unwrap();

        // The process restarts after writing the receipt and before committing its row.
        let mut writer = ArchiveWriter::create(&bucket, key).await.unwrap();
        writer
            .write_line(b"verified archive contents\n")
            .await
            .unwrap();
        let replayed = writer.finish().await.unwrap();
        let mut replay = original.clone();
        replay.created_at = at("2026-10-05T09:00:00Z").fixed_offset();
        replay.uploaded_at = Some(replay.created_at);
        replay.sha256 = replayed.sha256.to_vec();
        replay.byte_size = replayed.byte_size as i64;
        let (recovered, digest) =
            record_part_receipt(&bucket, replay.clone(), &context, None, true)
                .await
                .unwrap();
        assert_eq!(recovered, original);
        assert_eq!(digest, original_digest);
        assert_eq!(
            read_receipt(&bucket, &recovered).await.unwrap().1,
            original_digest
        );

        let mut changed = replay.clone();
        changed.record_count += 1;
        assert!(
            record_part_receipt(&bucket, changed, &context, None, true)
                .await
                .is_err()
        );
        let mut changed = replay.clone();
        changed.sha256[0] ^= 1;
        assert!(
            record_part_receipt(&bucket, changed, &context, None, true)
                .await
                .is_err()
        );
        assert!(
            record_part_receipt(&bucket, replay.clone(), &context, Some("different"), true)
                .await
                .is_err()
        );
        assert!(
            record_part_receipt(&bucket, replay, &context, None, false)
                .await
                .is_err()
        );
    }

    #[flow_like_types::tokio::test]
    async fn manifest_receipt_recovers_original_signature_and_timestamp_without_resigning() {
        let bucket = receipt_bucket();
        let signer = LocalSigner::new(
            SigningKey::from_slice(&[42; 32]).unwrap(),
            Some("archive-recovery-test".into()),
        );
        register_verifying_key(signer.kid(), signer.verifying_key()).unwrap();
        let month = Month::new(day(2026, 9, 1));
        let parts = [part(1, Some(9))];
        let context = test_receipt_context();
        let original_time = at("2026-10-04T00:01:00Z").fixed_offset();
        let mut manifest = manifest_json(
            &month,
            3,
            &parts,
            Some(&epoch(1, 0, 1)),
            Some(&epoch(9, 8, 9)),
            RawValues::NONE,
            original_time.timestamp_millis(),
        );
        manifest["kid"] = Value::from(signer.kid());
        let signature = signer.sign(&manifest_hash(&manifest)).await.unwrap();
        let bytes = manifest_bytes(&manifest, Some(&signature)).unwrap();
        let mut original = archive_row(
            "2026-09",
            0,
            &archive_manifest_key("2026-09"),
            WrittenObject {
                sha256: Sha256::digest(&bytes).into(),
                byte_size: bytes.len() as u64,
            },
            original_time,
        )
        .try_into_model()
        .unwrap();
        original.first_epoch = Some(1);
        original.last_epoch = Some(9);
        original.epoch_count = parts[0].epoch_count;
        original.seal_count = parts[0].seal_count;
        original.record_count = parts[0].record_count;
        original.kid = Some(signer.kid().into());
        original.signature = Some(signature.to_vec());
        original.manifest = Some(manifest);
        original.uploaded_at = None;
        write_receipt(&bucket, &original, &context, Some("previous"), true)
            .await
            .unwrap();
        let expected = manifest_json(
            &month,
            3,
            &parts,
            Some(&epoch(1, 0, 1)),
            Some(&epoch(9, 8, 9)),
            RawValues::NONE,
            at("2026-10-05T00:00:00Z").timestamp_millis(),
        );
        let recovered = recover_manifest(
            &bucket,
            "2026-09",
            &expected,
            &parts,
            &context,
            Some("previous"),
            true,
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(recovered, original);
        assert_eq!(
            manifest_bytes(
                recovered.manifest.as_ref().unwrap(),
                recovered.signature.as_deref()
            )
            .unwrap(),
            bytes
        );
        let mut changed = expected.clone();
        changed["record_count"] = Value::from(301);
        assert!(
            recover_manifest(
                &bucket,
                "2026-09",
                &changed,
                &parts,
                &context,
                Some("previous"),
                true
            )
            .await
            .is_err()
        );
        assert!(
            recover_manifest(&bucket, "2026-09", &expected, &parts, &context, None, true)
                .await
                .is_err()
        );
        for change in ["digest", "size", "signature", "object_key"] {
            let corrupt_bucket = receipt_bucket();
            let mut corrupt = original.clone();
            match change {
                "digest" => corrupt.sha256[0] ^= 1,
                "size" => corrupt.byte_size += 1,
                "signature" => corrupt.signature.as_mut().unwrap()[0] ^= 1,
                "object_key" => corrupt.object_key = "archive/elsewhere.json".into(),
                _ => unreachable!(),
            }
            write_receipt(&corrupt_bucket, &corrupt, &context, Some("previous"), true)
                .await
                .unwrap();
            assert!(
                recover_manifest(
                    &corrupt_bucket,
                    "2026-09",
                    &expected,
                    &parts,
                    &context,
                    Some("previous"),
                    true
                )
                .await
                .is_err(),
                "{change}"
            );
        }
    }

    #[test]
    fn manifest_upload_bytes_survive_database_key_reordering() {
        let original: Value = serde_json::from_str(
            r#"{"period":"2026-09","parts":[{"sha256":"abc","part":1}],"epochs":{"last":9,"first":1}}"#,
        )
        .unwrap();
        let reordered: Value = serde_json::from_str(
            r#"{"epochs":{"first":1,"last":9},"parts":[{"part":1,"sha256":"abc"}],"period":"2026-09"}"#,
        )
        .unwrap();
        for signature in [None, Some([7u8; 64].as_slice())] {
            let before = manifest_bytes(&original, signature).unwrap();
            let after = manifest_bytes(&reordered, signature).unwrap();
            assert_eq!(before, after);
            assert_eq!(Sha256::digest(&before), Sha256::digest(&after));
            let parsed: Value = serde_json::from_slice(&after).unwrap();
            assert_eq!(parsed["epochs"]["first"].as_i64(), Some(1));
            assert_eq!(parsed["epochs"]["last"].as_i64(), Some(9));
            assert_eq!(parsed["parts"][0]["part"].as_i64(), Some(1));
        }
        let mut changed = reordered;
        changed["epochs"]["last"] = Value::from(10);
        assert_ne!(
            manifest_bytes(&original, None).unwrap(),
            manifest_bytes(&changed, None).unwrap()
        );
    }

    #[flow_like_types::tokio::test]
    async fn manifests_carry_the_month_and_a_verifiable_signature() {
        let signer = LocalSigner::new(
            SigningKey::from_slice(&[41; 32]).unwrap(),
            Some("archive-manifest-test".into()),
        );
        register_verifying_key(signer.kid(), signer.verifying_key()).unwrap();
        let month = Month::new(day(2026, 9, 1));
        let parts = [part(1, Some(9)), part(2, Some(17))];
        let (first, last) = (epoch(1, 0, 1), epoch(17, 16, 17));
        let raw = RawValues {
            ip: false,
            details: true,
        };
        let mut manifest = manifest_json(&month, 3, &parts, Some(&first), Some(&last), raw, 42);
        assert!(manifest.get("signature").is_none());
        manifest["kid"] = Value::from(signer.kid());
        let signature = signer.sign(&manifest_hash(&manifest)).await.unwrap();
        let bytes = manifest_bytes(&manifest, Some(&signature)).unwrap();
        assert!(manifest.get("signature").is_none());
        let manifest: Value = serde_json::from_slice(&bytes).unwrap();

        assert_eq!(manifest["format"], MANIFEST_FORMAT);
        assert_eq!(manifest["period"], "2026-09");
        assert_eq!(manifest["retain_until"], "2029-12-31");
        assert_eq!(
            manifest["parts"][1]["key"],
            "archive/2026/09/0002.jsonl.zst"
        );
        assert_eq!(manifest["parts"][0]["sha256"], hex::encode([7u8; 32]));
        assert_eq!(manifest["epochs"]["first"], 1);
        assert_eq!(manifest["epochs"]["last"], 17);
        assert_eq!(
            manifest["epochs"]["first_prev_hash"],
            hex::encode([0u8; 32])
        );
        assert_eq!(manifest["epochs"]["last_hash"], hex::encode([17u8; 32]));
        assert_eq!(manifest["seal_count"], 40);
        assert_eq!(manifest["record_count"], 600);
        assert_eq!(manifest["raw_ip"], false);
        assert_eq!(manifest["raw_details"], true);
        assert_eq!(manifest["kid"], "archive-manifest-test");
        assert_eq!(manifest["created_at_ms"], 42);

        let mut unsigned = manifest.clone();
        let encoded = unsigned
            .as_object_mut()
            .unwrap()
            .remove("signature")
            .unwrap();
        let decoded = STANDARD.decode(encoded.as_str().unwrap()).unwrap();
        assert_eq!(decoded, signature.to_vec());
        let digest = manifest_hash(&unsigned);
        assert_eq!(
            verify("archive-manifest-test", &digest, &decoded),
            SignatureCheck::Valid
        );
        unsigned["record_count"] = Value::from(601);
        assert_eq!(
            verify("archive-manifest-test", &manifest_hash(&unsigned), &decoded),
            SignatureCheck::Invalid
        );
    }

    #[test]
    fn empty_months_have_a_manifest_without_parts() {
        let month = Month::new(day(2026, 10, 1));
        let manifest = manifest_json(&month, 3, &[], None, None, RawValues::NONE, 1);
        assert_eq!(manifest["parts"], serde_json::json!([]));
        assert!(manifest["epochs"]["first"].is_null());
        assert_eq!(manifest["record_count"], 0);
        assert!(manifest.get("kid").is_none());
    }

    #[test]
    fn seals_verify_against_their_records_in_canonical_order() {
        let records = vec![record("a", 1_000), record("b", 1_000), record("c", 2_000)];
        let seal = seal_over(&records);
        let mut shuffled = vec![records[2].clone(), records[1].clone(), records[0].clone()];
        let hash = verify_seal(&seal, &mut shuffled).unwrap();
        assert_eq!(hash.to_vec(), seal.hash);

        let mut edited = records.clone();
        edited[1].action = "board.delete".into();
        assert!(verify_seal(&seal, &mut edited).is_err());
        let mut missing = records[..2].to_vec();
        assert!(verify_seal(&seal, &mut missing).is_err());
        let mut with_mac = records.clone();
        with_mac[0].mac = Some(vec![0; 32]);
        assert!(verify_seal(&seal, &mut with_mac).is_err());
        let mut activity = seal.clone();
        activity.class = "activity".into();
        activity.hash = seal_hash_of(&activity).to_vec();
        assert!(verify_seal(&activity, &mut records.clone()).is_err());
    }

    #[test]
    fn anchored_seals_prove_their_epoch() {
        let records = vec![record("a", 1_000)];
        let mut seal = seal_over(&records);
        let hash = verify_seal(&seal, &mut records.clone()).unwrap();
        let (root, proofs) = merkle::root_with_proofs(&[[9; 32], hash]);
        let mut anchor = epoch(3, 0, 3);
        anchor.seal_count = 2;
        anchor.seals_root = root.to_vec();
        assert!(verify_unanchored(&seal).is_ok());
        seal.epoch_seq = Some(3);
        seal.epoch_index = Some(1);
        seal.epoch_proof = Some(merkle::encode_proof(&proofs[1]));
        assert!(verify_anchor(&seal, &hash, &anchor, &root).is_ok());
        assert!(verify_unanchored(&seal).is_err());
        seal.epoch_index = Some(0);
        assert!(verify_anchor(&seal, &hash, &anchor, &root).is_err());
        seal.epoch_index = None;
        assert!(verify_anchor(&seal, &hash, &anchor, &root).is_err());
        seal.epoch_index = Some(1);
        seal.epoch_seq = Some(4);
        assert!(verify_anchor(&seal, &hash, &anchor, &root).is_err());
    }
}
