//! Cryptographic audit log status for the dashboard.

use crate::audit::service::AuditService;
use crate::audit::sign;
use crate::entity::audit_entry;
use crate::error::ApiError;
use crate::middleware::jwt::AppUser;
use crate::permission::global_permission::GlobalPermission;
use crate::state::AppState;
use axum::extract::State;
use axum::{Extension, Json};
use chrono::{Duration, Utc};
use sea_orm::sea_query::Expr;
use sea_orm::{
    ColumnTrait, EntityTrait, Order, PaginatorTrait, QueryFilter, QueryOrder, QuerySelect,
};
use serde::Serialize;
use utoipa::ToSchema;

const AUTOMATIC_VERIFICATION_ENTRY_LIMIT: i64 = 1_000;

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
    let mut q = audit_entry::Entity::find();
    q = match chain_id {
        Some(cid) => q.filter(audit_entry::Column::ChainId.eq(cid)),
        None => q.filter(audit_entry::Column::ChainId.is_null()),
    };

    let entries = q.clone().count(&state.db).await? as i64;

    let mut tail_q = audit_entry::Entity::find();
    tail_q = match chain_id {
        Some(cid) => tail_q.filter(audit_entry::Column::ChainId.eq(cid)),
        None => tail_q.filter(audit_entry::Column::ChainId.is_null()),
    };
    let tail = tail_q
        .order_by(audit_entry::Column::Sequence, Order::Desc)
        .one(&state.db)
        .await?;

    let (last_sequence, last_entry_at, last_entry_hash, signed, kid) = match tail {
        Some(e) => (
            Some(e.sequence),
            Some(e.timestamp.to_rfc3339()),
            Some(e.entry_hash),
            e.signature.is_some(),
            e.kid,
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

    let total_entries = audit_entry::Entity::find().count(&state.db).await? as i64;
    let signed_entries = audit_entry::Entity::find()
        .filter(audit_entry::Column::Signature.is_not_null())
        .count(&state.db)
        .await? as i64;
    let unsigned_entries = (total_entries - signed_entries).max(0);

    let branch_chain_count = audit_entry::Entity::find()
        .filter(audit_entry::Column::ChainId.is_not_null())
        .select_only()
        .column(audit_entry::Column::ChainId)
        .group_by(audit_entry::Column::ChainId)
        .count(&state.db)
        .await? as i64;

    let last_24h_cutoff = Utc::now().fixed_offset() - Duration::hours(24);
    let last_24h_entries = audit_entry::Entity::find()
        .filter(audit_entry::Column::Timestamp.gte(last_24h_cutoff))
        .count(&state.db)
        .await? as i64;

    let root_chain = build_summary(&state, None, "Platform Root".to_string(), true).await?;

    let mut recent_branches: Vec<ChainSummary> = Vec::new();
    let recent = audit_entry::Entity::find()
        .filter(audit_entry::Column::ChainId.is_not_null())
        .order_by(audit_entry::Column::Timestamp, Order::Desc)
        .limit(50)
        .all(&state.db)
        .await?;
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    for entry in recent {
        if let Some(cid) = entry.chain_id.clone()
            && seen.insert(cid.clone())
        {
            let label = format!("{} :: {}", entry.resource_type, cid);
            let summary = build_summary(&state, Some(&cid), label, false).await?;
            recent_branches.push(summary);
            if recent_branches.len() >= 8 {
                break;
            }
        }
    }
    let _ = Expr::value(0); // keep Expr import used

    Ok(Json(ChainStatusResponse {
        signing_configured: sign::is_signing_configured(),
        current_kid: sign::current_kid().to_string(),
        total_entries,
        signed_entries,
        unsigned_entries,
        branch_chain_count,
        last_24h_entries,
        root_chain,
        recent_branches,
    }))
}
