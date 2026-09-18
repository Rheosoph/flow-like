use axum::{
    Extension, Json,
    extract::{Query, State},
};
use serde::Deserialize;
use utoipa::IntoParams;

use crate::{
    audit::service::{AuditService, ChainHead},
    error::ApiError,
    middleware::jwt::AppUser,
    state::AppState,
};

#[derive(Debug, Deserialize, IntoParams)]
pub struct HeadParams {
    /// Chain ID (app_id or package_id). Omit for platform root chain.
    pub chain_id: Option<String>,
}

#[utoipa::path(
    get,
    path = "/audit/head",
    tag = "audit",
    description = "Newest entry of an audit chain. Keep it outside the platform and pass it to /audit/verify later to prove that no entries were removed. App chains require Owner; other chains require Admin.",
    params(HeadParams),
    responses(
        (status = 200, description = "Chain head, or null for an empty chain", body = Option<ChainHead>),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden")
    )
)]
#[tracing::instrument(name = "GET /audit/head", skip_all)]
pub async fn chain_head(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Query(params): Query<HeadParams>,
) -> Result<Json<Option<ChainHead>>, ApiError> {
    user.sub()?;
    super::query::ensure_chain_access(&user, &state, params.chain_id.as_deref()).await?;
    let head = AuditService::head(&state.db, params.chain_id.as_deref())
        .await
        .map_err(ApiError::internal_error)?;
    Ok(Json(head))
}
