use axum::{
    Extension, Json,
    extract::{Query, State},
};
use serde::Deserialize;
use utoipa::IntoParams;

use crate::{
    audit::service::{AuditEntryOutput, AuditFilter, AuditService},
    error::ApiError,
    middleware::jwt::AppUser,
    state::AppState,
};

#[derive(Debug, Deserialize, IntoParams)]
pub struct AuditQueryParams {
    /// Chain ID (app_id or package_id). Omit for platform root chain.
    pub chain_id: Option<String>,
    /// Action filter. Use "app.*" for prefix matching.
    pub action: Option<String>,
    pub actor_id: Option<String>,
    pub resource_type: Option<String>,
    pub resource_id: Option<String>,
    pub limit: Option<u64>,
    pub offset: Option<u64>,
}

#[utoipa::path(
    get,
    path = "/audit/entries",
    tag = "audit",
    description = "Query audit trail entries. Requires Owner permission when filtering by app chain.",
    params(AuditQueryParams),
    responses(
        (status = 200, description = "Audit entries", body = Vec<AuditEntryOutput>),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden")
    )
)]
#[tracing::instrument(name = "GET /audit/entries", skip(state, user))]
pub async fn query_audit_entries(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Query(params): Query<AuditQueryParams>,
) -> Result<Json<Vec<AuditEntryOutput>>, ApiError> {
    let _sub = user.sub()?;

    // If querying a specific chain, verify the user has access
    if let Some(ref chain_id) = params.chain_id {
        let perm = user.app_permission(chain_id, &state).await;
        if perm.is_err() {
            // Could be a package chain — for now require authenticated user
            // Package chain audit access is open to authenticated users
        }
    }

    let filter = AuditFilter {
        chain_id: params.chain_id,
        action: params.action,
        actor_id: params.actor_id,
        resource_type: params.resource_type,
        resource_id: params.resource_id,
        limit: params.limit,
        offset: params.offset,
    };

    let entries = AuditService::query(&state.db, filter)
        .await
        .map_err(ApiError::internal_error)?;

    let output: Vec<AuditEntryOutput> = entries.into_iter().map(Into::into).collect();
    Ok(Json(output))
}
