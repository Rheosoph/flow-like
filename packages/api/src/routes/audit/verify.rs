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
    description = "Verify hashes, sequence continuity, branch anchors and signatures in an audit chain. The response distinguishes legacy and unsigned entries from fully authenticated entries. App chains require Owner; other chains require Admin.",
    params(VerifyParams),
    responses(
        (status = 200, description = "Chain verification result", body = ChainVerification),
        (status = 400, description = "Invalid sequence range"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden")
    )
)]
#[tracing::instrument(name = "GET /audit/verify", skip_all)]
pub async fn verify_chain(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Query(params): Query<VerifyParams>,
) -> Result<Json<ChainVerification>, ApiError> {
    user.sub()?;
    super::query::ensure_chain_access(&user, &state, params.chain_id.as_deref()).await?;
    validate_range(params.from, params.to)?;

    let result = AuditService::verify_chain(
        &state.db,
        state.db_dialect,
        params.chain_id.as_deref(),
        params.from,
        params.to,
    )
    .await
    .map_err(ApiError::internal_error)?;

    Ok(Json(result))
}

fn validate_range(from: Option<i64>, to: Option<i64>) -> Result<(), ApiError> {
    if from.is_some_and(|value| value < 1)
        || to.is_some_and(|value| value < 1)
        || matches!((from, to), (Some(from), Some(to)) if from > to)
    {
        return Err(ApiError::bad_request(
            "Audit sequences must be positive and from must not exceed to",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_invalid_ranges() {
        for (from, to) in [(Some(0), None), (None, Some(-1)), (Some(4), Some(3))] {
            assert!(validate_range(from, to).is_err());
        }
        for (from, to) in [(None, None), (Some(1), Some(1)), (Some(2), None)] {
            assert!(validate_range(from, to).is_ok());
        }
    }
}
