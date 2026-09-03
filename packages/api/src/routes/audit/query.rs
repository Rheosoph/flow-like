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
    permission::{global_permission::GlobalPermission, role_permission::RolePermissions},
    state::AppState,
};

/// Audit chains carry every administrative change of an app, so reading one is an
/// Owner-level capability. Chains that are not an app the caller belongs to — the root
/// chain (`None`) and package chains — are platform-admin territory.
pub(super) async fn ensure_chain_access(
    user: &AppUser,
    state: &AppState,
    chain_id: Option<&str>,
) -> Result<(), ApiError> {
    if let Some(chain_id) = chain_id
        && let Ok(permission) = user.app_permission(chain_id, state).await
    {
        if permission.has_permission(RolePermissions::Owner) {
            return Ok(());
        }
        return Err(ApiError::FORBIDDEN);
    }

    user.check_global_permission(state, GlobalPermission::Admin)
        .await
        .map(|_| ())
}

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
    description = "Query audit trail entries. App chains require Owner permission on that app; the root chain and package chains require the Admin global permission.",
    params(AuditQueryParams),
    responses(
        (status = 200, description = "Audit entries", body = Vec<AuditEntryOutput>),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden")
    )
)]
#[tracing::instrument(name = "GET /audit/entries", skip_all)]
pub async fn query_audit_entries(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Query(params): Query<AuditQueryParams>,
) -> Result<Json<Vec<AuditEntryOutput>>, ApiError> {
    user.sub()?;
    ensure_chain_access(&user, &state, params.chain_id.as_deref()).await?;

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
