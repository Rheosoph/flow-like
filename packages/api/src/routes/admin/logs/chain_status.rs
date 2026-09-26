//! Audit trail health for the admin dashboard: worker progress, key setup, the epoch
//! timeline and the platform chain.

use std::collections::HashSet;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use axum::extract::State;
use axum::{Extension, Json};
use chrono::{DateTime, FixedOffset};
use flow_like_types::tokio;
use sea_orm::{
    ColumnTrait, ConnectionTrait, DatabaseConnection, DbErr, EntityTrait, Order, PaginatorTrait,
    QueryFilter, QueryOrder, QuerySelect, Statement,
};
use serde::Serialize;
use utoipa::ToSchema;

use crate::audit::verify::{self, ChainReport, EpochReport, INVALID_SEAL_ID};
use crate::audit::{PLATFORM_CHAIN, RetentionClass, signer};
use crate::entity::{audit_archive, audit_epoch, audit_held_chain, audit_record, audit_seal};
use crate::error::ApiError;
use crate::middleware::jwt::AppUser;
use crate::permission::global_permission::GlobalPermission;
use crate::state::AppState;

/// The dashboard polls this endpoint; the timeline check and the table totals are the
/// expensive part and are shared for this long.
const SLOW_TTL: Duration = Duration::from_secs(60);
const RECENT_CHAINS: usize = 20;
const RECENT_SEAL_SAMPLE: u64 = 500;
/// Below this planner estimate a table is small enough to count exactly.
const EXACT_COUNT_BELOW: i64 = 100_000;

const PENDING_SQL: &str = r#"SELECT count(*) AS pending, min("timestamp") AS oldest FROM "AuditRecord" WHERE "sealId" IS NULL"#;
const OLDEST_UNANCHORED_SQL: &str = r#"SELECT min(s."sealedAt") AS oldest FROM "AuditSeal" s WHERE s."epochSeq" IS NULL AND NOT EXISTS (SELECT 1 FROM "AuditHeldChain" h WHERE h."chainId" = s."chainId")"#;
const ESTIMATE_SQL: &str = r#"SELECT CAST(GREATEST(reltuples, 0) AS BIGINT) AS estimate FROM pg_class WHERE relname = $1 ORDER BY reltuples DESC LIMIT 1"#;
const CHAINS_SQL: &str = r#"SELECT count(*) AS chains FROM (SELECT "chainId" FROM "AuditSeal" UNION SELECT "chainId" FROM "AuditRecord" WHERE "sealId" IS NULL OR "sealId" = $1) chains"#;
/// `n_distinct` is negative when the planner stores it as a fraction of the rows.
const CHAINS_ESTIMATE_SQL: &str = r#"SELECT CAST(CASE WHEN s.n_distinct < 0 THEN -s.n_distinct * GREATEST(c.reltuples, 0) ELSE s.n_distinct END AS BIGINT) AS estimate FROM pg_stats s JOIN pg_class c ON c.relname = s.tablename WHERE s.tablename = 'AuditSeal' AND s.attname = 'chainId' LIMIT 1"#;

#[derive(Clone)]
struct SlowParts {
    epochs: EpochReport,
    total_records: u64,
    total_seals: u64,
    chains: u64,
    legacy_entries: u64,
}

static SLOW: OnceLock<tokio::sync::Mutex<Option<(Instant, SlowParts)>>> = OnceLock::new();

#[derive(Debug, Serialize, ToSchema)]
pub struct LatestArchive {
    /// `YYYY-MM`.
    pub period: String,
    pub created_at_ms: i64,
    pub record_count: i64,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct RecentChain {
    pub chain_id: String,
    pub latest_seal_seq: i64,
    pub latest_sealed_at_ms: i64,
    /// Records of the chain waiting for a seal.
    pub pending: u64,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ChainStatusResponse {
    /// Key id of this process's audit signer, else the configured `AUDIT_KID`.
    pub signing_kid: Option<String>,
    /// Key ids whose public keys this process verifies with.
    pub verifying_kids: Vec<String>,
    pub epochs: EpochReport,
    pub latest_epoch_at_ms: Option<i64>,
    pub pending_records: u64,
    /// Timestamp of the oldest record waiting for a seal.
    pub oldest_pending_ms: Option<i64>,
    /// Records whose MAC failed before sealing.
    pub quarantined_records: u64,
    pub unanchored_seals: u64,
    /// `sealedAt` of the oldest seal still waiting for its signed epoch, held chains
    /// excluded. Older than `epoch_interval_seconds` plus a tick means signing stalls.
    pub oldest_unanchored_ms: Option<i64>,
    /// Chains whose seal failed its hash or MAC before signing; they need an operator.
    pub held_chains: u64,
    /// Exact for small tables, the planner's estimate for large ones.
    pub total_records: u64,
    /// Exact for small tables, the planner's estimate for large ones.
    pub total_seals: u64,
    /// Distinct chains with seals or records.
    pub chains: u64,
    pub latest_archive: Option<LatestArchive>,
    /// Rows of the pre-2026-09 audit table still waiting for export and deletion.
    pub legacy_entries: u64,
    /// Incremental verification of the platform chain.
    pub platform: ChainReport,
    /// The most recently sealed chains.
    pub recent_chains: Vec<RecentChain>,
    /// Age of the oldest pending record at which the worker counts as behind.
    pub pending_alert_seconds: u32,
    /// How long a seal may wait for its signed epoch.
    pub epoch_interval_seconds: u32,
}

#[utoipa::path(
    get,
    path = "/admin/logs/chain-status",
    tag = "audit",
    description = "Health of the audit trail for the admin dashboard: signing and verification keys, the epoch timeline, records waiting for a seal, quarantined records, unanchored seals, archive and legacy export progress, the platform chain and the most recently sealed chains. Counts, hashes and ids only, no record content. Requires the ReadLogs global permission.",
    responses(
        (status = 200, description = "Audit trail status", body = ChainStatusResponse),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden")
    ),
    security(
        ("bearer_auth" = []),
        ("pat" = [])
    )
)]
#[tracing::instrument(name = "GET /admin/logs/chain-status", skip_all)]
pub async fn chain_status(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
) -> Result<Json<ChainStatusResponse>, ApiError> {
    user.check_global_permission(&state, GlobalPermission::ReadLogs)
        .await?;
    let db = &state.db;
    let slow = slow_parts(db).await?;
    let (
        (pending_records, oldest_pending_ms),
        quarantined_records,
        unanchored_seals,
        oldest_unanchored_ms,
        held_chains,
        latest_epoch_at_ms,
        latest_archive,
        platform,
        recent_chains,
    ) = tokio::try_join!(
        pending(db),
        audit_record::Entity::find()
            .filter(audit_record::Column::SealId.eq(INVALID_SEAL_ID))
            .count(db),
        audit_seal::Entity::find()
            .filter(audit_seal::Column::EpochSeq.is_null())
            .count(db),
        oldest_unanchored(db),
        audit_held_chain::Entity::find().count(db),
        latest_epoch_at(db),
        latest_archive(db),
        verify::verify_chain(db, PLATFORM_CHAIN, false),
        recent_chains(db),
    )?;

    Ok(Json(ChainStatusResponse {
        signing_kid: state.audit_kid.clone(),
        verifying_kids: signer::verifying_kids(),
        epochs: slow.epochs,
        latest_epoch_at_ms,
        pending_records,
        oldest_pending_ms,
        quarantined_records,
        unanchored_seals,
        oldest_unanchored_ms,
        held_chains,
        total_records: slow.total_records,
        total_seals: slow.total_seals,
        chains: slow.chains,
        latest_archive,
        legacy_entries: slow.legacy_entries,
        platform,
        recent_chains,
        pending_alert_seconds: state.platform_config.audit.retention.pending_alert_seconds,
        epoch_interval_seconds: state.platform_config.audit.retention.epoch_interval_seconds,
    }))
}

async fn slow_parts(db: &DatabaseConnection) -> Result<SlowParts, DbErr> {
    let mut cached = SLOW.get_or_init(Default::default).lock().await;
    if let Some((at, parts)) = cached.as_ref()
        && at.elapsed() < SLOW_TTL
    {
        return Ok(parts.clone());
    }
    let (epochs, total_records, total_seals, legacy_entries) = tokio::try_join!(
        verify::verify_epochs(db, false),
        row_count(db, "AuditRecord"),
        row_count(db, "AuditSeal"),
        row_count(db, "AuditEntry"),
    )?;
    let chains = chain_count(db, total_seals).await?;
    let parts = SlowParts {
        epochs,
        total_records,
        total_seals,
        chains,
        legacy_entries,
    };
    *cached = Some((Instant::now(), parts.clone()));
    Ok(parts)
}

/// The planner's estimate, or an exact count when the table is small or the engine has
/// no estimate, so a large table is never scanned for a dashboard number.
async fn row_count(db: &DatabaseConnection, table: &'static str) -> Result<u64, DbErr> {
    let estimate = estimate(
        db,
        Statement::from_sql_and_values(
            db.get_database_backend(),
            ESTIMATE_SQL,
            [sea_orm::Value::from(table)],
        ),
    )
    .await;
    if estimate >= EXACT_COUNT_BELOW {
        return Ok(estimate as u64);
    }
    let row = db
        .query_one_raw(Statement::from_string(
            db.get_database_backend(),
            format!(r#"SELECT count(*) AS total FROM "{table}""#),
        ))
        .await?;
    let total: i64 = match row {
        Some(row) => row.try_get("", "total")?,
        None => 0,
    };
    Ok(total.max(0) as u64)
}

/// Distinct chains. Counting them exactly reads every seal, so large seal tables use the
/// planner's distinct estimate where the engine keeps one.
async fn chain_count(db: &DatabaseConnection, total_seals: u64) -> Result<u64, DbErr> {
    if total_seals >= EXACT_COUNT_BELOW as u64 {
        let estimate = estimate(
            db,
            Statement::from_string(db.get_database_backend(), CHAINS_ESTIMATE_SQL),
        )
        .await;
        if estimate > 0 {
            return Ok(estimate as u64);
        }
    }
    let row = db
        .query_one_raw(Statement::from_sql_and_values(
            db.get_database_backend(),
            CHAINS_SQL,
            [sea_orm::Value::from(INVALID_SEAL_ID)],
        ))
        .await?;
    let chains: i64 = match row {
        Some(row) => row.try_get("", "chains")?,
        None => 0,
    };
    Ok(chains.max(0) as u64)
}

/// A catalog estimate, 0 when the engine does not expose it.
async fn estimate(db: &DatabaseConnection, statement: Statement) -> i64 {
    match db.query_one_raw(statement).await {
        Ok(Some(row)) => row.try_get::<i64>("", "estimate").unwrap_or(0),
        Ok(None) => 0,
        Err(error) => {
            tracing::debug!(%error, "audit status: catalog estimate unavailable");
            0
        }
    }
}

async fn pending(db: &DatabaseConnection) -> Result<(u64, Option<i64>), DbErr> {
    let row = db
        .query_one_raw(Statement::from_string(
            db.get_database_backend(),
            PENDING_SQL,
        ))
        .await?;
    let Some(row) = row else {
        return Ok((0, None));
    };
    let pending: i64 = row.try_get("", "pending")?;
    let oldest: Option<DateTime<FixedOffset>> = row.try_get("", "oldest")?;
    Ok((
        pending.max(0) as u64,
        oldest.map(|oldest| oldest.timestamp_millis()),
    ))
}

async fn oldest_unanchored(db: &DatabaseConnection) -> Result<Option<i64>, DbErr> {
    let row = db
        .query_one_raw(Statement::from_string(
            db.get_database_backend(),
            OLDEST_UNANCHORED_SQL,
        ))
        .await?;
    let oldest: Option<DateTime<FixedOffset>> = match row {
        Some(row) => row.try_get("", "oldest")?,
        None => None,
    };
    Ok(oldest.map(|oldest| oldest.timestamp_millis()))
}

async fn latest_epoch_at(db: &DatabaseConnection) -> Result<Option<i64>, DbErr> {
    Ok(audit_epoch::Entity::find()
        .order_by(audit_epoch::Column::Seq, Order::Desc)
        .one(db)
        .await?
        .map(|epoch| epoch.created_at.timestamp_millis()))
}

async fn latest_archive(db: &DatabaseConnection) -> Result<Option<LatestArchive>, DbErr> {
    Ok(audit_archive::Entity::find()
        .filter(audit_archive::Column::Part.eq(0))
        .filter(audit_archive::Column::Period.ne("legacy"))
        .filter(audit_archive::Column::UploadedAt.is_not_null())
        .order_by(audit_archive::Column::Period, Order::Desc)
        .one(db)
        .await?
        .map(|archive| LatestArchive {
            period: archive.period,
            created_at_ms: archive.created_at.timestamp_millis(),
            record_count: archive.record_count,
        }))
}

/// Seals are sampled through the `(class, lastAt)` index, newest first, then each
/// chain's newest seal and pending count is read by its own index.
async fn recent_chains(db: &DatabaseConnection) -> Result<Vec<RecentChain>, DbErr> {
    let mut sample: Vec<(String, DateTime<FixedOffset>)> = Vec::new();
    for class in [RetentionClass::Evidence, RetentionClass::Activity] {
        let rows: Vec<(String, DateTime<FixedOffset>)> = audit_seal::Entity::find()
            .select_only()
            .columns([audit_seal::Column::ChainId, audit_seal::Column::SealedAt])
            .filter(audit_seal::Column::Class.eq(class.as_str()))
            .order_by(audit_seal::Column::LastAt, Order::Desc)
            .limit(RECENT_SEAL_SAMPLE)
            .into_tuple()
            .all(db)
            .await?;
        sample.extend(rows);
    }
    sample.sort_by_key(|entry| std::cmp::Reverse(entry.1));
    let mut seen = HashSet::new();
    let chain_ids: Vec<String> = sample
        .into_iter()
        .filter_map(|(chain_id, _)| seen.insert(chain_id.clone()).then_some(chain_id))
        .take(RECENT_CHAINS)
        .collect();

    let mut chains = Vec::with_capacity(chain_ids.len());
    for chain_id in chain_ids {
        let Some(latest) = audit_seal::Entity::find()
            .filter(audit_seal::Column::ChainId.eq(&chain_id))
            .order_by(audit_seal::Column::Seq, Order::Desc)
            .one(db)
            .await?
        else {
            continue;
        };
        let pending = audit_record::Entity::find()
            .filter(audit_record::Column::SealId.is_null())
            .filter(audit_record::Column::ChainId.eq(&chain_id))
            .count(db)
            .await?;
        chains.push(RecentChain {
            chain_id,
            latest_seal_seq: latest.seq,
            latest_sealed_at_ms: latest.sealed_at.timestamp_millis(),
            pending,
        });
    }
    Ok(chains)
}
