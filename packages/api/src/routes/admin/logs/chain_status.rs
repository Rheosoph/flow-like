//! Cryptographic audit log status for the dashboard.

use crate::audit::service::{AuditService, chain_filter, newest_first};
use crate::audit::sign;
use crate::entity::audit_entry;
use crate::error::ApiError;
use crate::middleware::jwt::AppUser;
use crate::permission::global_permission::GlobalPermission;
use crate::state::AppState;
use axum::extract::State;
use axum::{Extension, Json};
use chrono::{DateTime, Duration, FixedOffset, Utc};
use flow_like_types::tokio;
use sea_orm::{
    ColumnTrait, EntityTrait, Order, PaginatorTrait, QueryFilter, QueryOrder, QuerySelect,
};
use serde::Serialize;
use std::sync::OnceLock;
use std::time::Instant;
use utoipa::ToSchema;

const AUTOMATIC_VERIFICATION_ENTRY_LIMIT: i64 = 1_000;
/// The dashboard polls this endpoint; the platform-wide counts scan the table.
const TOTALS_TTL: std::time::Duration = std::time::Duration::from_secs(60);

#[derive(Clone, Copy)]
struct Totals {
    total: i64,
    signed: i64,
    branches: i64,
    last_24h: i64,
}

static TOTALS: OnceLock<tokio::sync::Mutex<Option<(Instant, Totals)>>> = OnceLock::new();

type TailRow = (
    i64,
    DateTime<FixedOffset>,
    String,
    Option<String>,
    Option<String>,
);

#[derive(Debug, Serialize, ToSchema)]
pub struct ChainSummary {
    pub chain_id: Option<String>,
    pub label: String,
    pub entries: i64,
    pub last_sequence: Option<i64>,
    pub last_entry_at: Option<String>,
    pub last_entry_hash: Option<String>,
    pub signed: bool,
    pub kid: Option<String>,
    pub valid: Option<bool>,
    pub fully_authenticated: Option<bool>,
    pub first_broken_at: Option<i64>,
    pub unverifiable_signatures: Option<u64>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ChainStatusResponse {
    pub signing_configured: bool,
    pub current_kid: String,
    pub total_entries: i64,
    pub signed_entries: i64,
    pub unsigned_entries: i64,
    pub branch_chain_count: i64,
    pub last_24h_entries: i64,
    pub root_chain: ChainSummary,
    pub recent_branches: Vec<ChainSummary>,
}

async fn build_summary(
    state: &AppState,
    chain_id: Option<&str>,
    label: String,
    verify: bool,
) -> Result<ChainSummary, ApiError> {
    let tail: Option<TailRow> = newest_first(
        audit_entry::Entity::find()
            .select_only()
            .columns([
                audit_entry::Column::Sequence,
                audit_entry::Column::Timestamp,
                audit_entry::Column::EntryHash,
                audit_entry::Column::Signature,
                audit_entry::Column::Kid,
            ])
            .filter(chain_filter(chain_id)),
    )
    .into_tuple()
    .one(&state.db)
    .await?;

    // Sequences are contiguous from 1, so the tail already is the entry count.
    let entries = tail.as_ref().map_or(0, |tail| tail.0);
    let (last_sequence, last_entry_at, last_entry_hash, signed, kid) = match tail {
        Some((sequence, timestamp, entry_hash, signature, kid)) => (
            Some(sequence),
            Some(timestamp.to_rfc3339()),
            Some(entry_hash),
            signature.is_some(),
            kid,
        ),
        None => (None, None, None, false, None),
    };

    let (valid, fully_authenticated, first_broken_at, unverifiable_signatures) = if verify
        && entries > 0
        && entries <= AUTOMATIC_VERIFICATION_ENTRY_LIMIT
    {
        match AuditService::verify_chain(&state.db, state.db_dialect, chain_id, None, None).await {
            Ok(v) => (
                Some(v.valid),
                Some(v.fully_authenticated),
                v.first_broken_at,
                Some(v.unverifiable_signatures),
            ),
            Err(error) => {
                tracing::error!(%error, chain_id, "Audit chain status verification failed");
                (None, None, None, None)
            }
        }
    } else {
        (None, None, None, None)
    };

    Ok(ChainSummary {
        chain_id: chain_id.map(|s| s.to_string()),
        label,
        entries,
        last_sequence,
        last_entry_at,
        last_entry_hash,
        signed,
        kid,
        valid,
        fully_authenticated,
        first_broken_at,
        unverifiable_signatures,
    })
}

#[utoipa::path(
    get,
    path = "/admin/logs/chain-status",
    tag = "admin",
    responses(
        (status = 200, description = "Cryptographic audit chain status", body = ChainStatusResponse),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden")
    ),
    description = "Snapshot of the cryptographic audit logs for the dashboard."
)]
pub async fn chain_status(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
) -> Result<Json<ChainStatusResponse>, ApiError> {
    user.check_global_permission(&state, GlobalPermission::ReadLogs)
        .await?;

    let totals = totals(&state).await?;
    let root_chain = build_summary(&state, None, "Platform Root".to_string(), true).await?;

    let mut recent_branches: Vec<ChainSummary> = Vec::new();
    let recent: Vec<(Option<String>, String)> = audit_entry::Entity::find()
        .select_only()
        .columns([
            audit_entry::Column::ChainId,
            audit_entry::Column::ResourceType,
        ])
        .filter(audit_entry::Column::ChainId.is_not_null())
        .order_by(audit_entry::Column::Timestamp, Order::Desc)
        .limit(50)
        .into_tuple()
        .all(&state.db)
        .await?;
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    for (chain_id, resource_type) in recent {
        if let Some(cid) = chain_id
            && seen.insert(cid.clone())
        {
            let label = format!("{resource_type} :: {cid}");
            let summary = build_summary(&state, Some(&cid), label, false).await?;
            recent_branches.push(summary);
            if recent_branches.len() >= 8 {
                break;
            }
        }
    }

    Ok(Json(ChainStatusResponse {
        signing_configured: sign::is_signing_configured(),
        current_kid: sign::current_kid().to_string(),
        total_entries: totals.total,
        signed_entries: totals.signed,
        unsigned_entries: (totals.total - totals.signed).max(0),
        branch_chain_count: totals.branches,
        last_24h_entries: totals.last_24h,
        root_chain,
        recent_branches,
    }))
}

async fn totals(state: &AppState) -> Result<Totals, ApiError> {
    let mut cached = TOTALS.get_or_init(Default::default).lock().await;
    if let Some((at, totals)) = *cached
        && at.elapsed() < TOTALS_TTL
    {
        return Ok(totals);
    }
    let last_24h_cutoff = Utc::now().fixed_offset() - Duration::hours(24);
    let (total, signed, branches, last_24h) = tokio::try_join!(
        audit_entry::Entity::find().count(&state.db),
        audit_entry::Entity::find()
            .filter(audit_entry::Column::Signature.is_not_null())
            .count(&state.db),
        audit_entry::Entity::find()
            .filter(audit_entry::Column::ChainId.is_not_null())
            .select_only()
            .column(audit_entry::Column::ChainId)
            .group_by(audit_entry::Column::ChainId)
            .count(&state.db),
        audit_entry::Entity::find()
            .filter(audit_entry::Column::Timestamp.gte(last_24h_cutoff))
            .count(&state.db),
    )?;
    let totals = Totals {
        total: total as i64,
        signed: signed as i64,
        branches: branches as i64,
        last_24h: last_24h as i64,
    };
    *cached = Some((Instant::now(), totals));
    Ok(totals)
}
