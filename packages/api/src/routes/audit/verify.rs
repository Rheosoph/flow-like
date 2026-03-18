use axum::{
    Extension, Json,
    extract::{Query, State},
};
use serde::Deserialize;
use utoipa::IntoParams;

use crate::{
    audit::service::{AuditService, ChainVerification},
    error::ApiError,
    middleware::jwt::AppUser,
    state::AppState,
};

#[derive(Debug, Deserialize, IntoParams)]
pub struct VerifyParams {
    /// Chain ID (app_id or package_id). Omit for platform root chain.
    pub chain_id: Option<String>,
    /// Start sequence (inclusive)
    pub from: Option<i64>,
    /// End sequence (inclusive)
    pub to: Option<i64>,
}

#[utoipa::path(
    get,
    path = "/audit/verify",
    tag = "audit",
    description = "Verify the integrity of an audit hash chain. Replays the chain and checks every entry hash.",
    params(VerifyParams),
    responses(
        (status = 200, description = "Chain verification result", body = ChainVerification),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden")
    )
)]
#[tracing::instrument(name = "GET /audit/verify", skip(state, user))]
pub async fn verify_chain(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Query(params): Query<VerifyParams>,
) -> Result<Json<ChainVerification>, ApiError> {
    let _sub = user.sub()?;

    // If verifying a specific chain, optionally check access
    if let Some(ref chain_id) = params.chain_id {
        let _ = user.app_permission(chain_id, &state).await;
    }

    let result = AuditService::verify_chain(
        &state.db,
        params.chain_id.as_deref(),
        params.from,
        params.to,
    )
    .await
    .map_err(|e| ApiError::internal_error(e))?;

    Ok(Json(result))
}
