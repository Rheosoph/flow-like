use axum::{
    Extension, Json,
    extract::{Query, State},
};
use serde::Deserialize;
use utoipa::IntoParams;

use crate::{
    audit::verify::{self, ChainReport, EpochReport},
    error::ApiError,
    middleware::jwt::AppUser,
    permission::global_permission::GlobalPermission,
    state::AppState,
};

use super::{chain_or_platform, ensure_chain_access};

#[derive(Debug, Deserialize, IntoParams)]
pub struct VerifyParams {
    /// Chain id: an app or package id, `<id>#activity`, or `platform` (the default).
    pub chain_id: Option<String>,
    /// Re-check every seal from the watermark instead of continuing after the last one
    /// this server verified. Read-only; allowed to everyone who may read the chain.
    #[serde(default)]
    pub full: bool,
}

#[utoipa::path(
    get,
    path = "/audit/verify",
    tag = "audit",
    description = "Verify an audit chain: seal links, record hashes against each seal's root, epoch inclusion proofs and signatures, and the integrity of records and seals not signed yet. Expired personal values are reported as redacted, not as tampering. `checked_from_seq` says where this run started; `full=true` re-checks from the watermark. App chains require Owner on that app; the platform and package chains require the Admin global permission.",
    params(VerifyParams),
    responses(
        (status = 200, description = "Chain verification report", body = ChainReport),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden")
    ),
    security(
        ("bearer_auth" = []),
        ("pat" = [])
    )
)]
#[tracing::instrument(name = "GET /audit/verify", skip_all)]
pub async fn verify_chain(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Query(params): Query<VerifyParams>,
) -> Result<Json<ChainReport>, ApiError> {
    let chain_id = chain_or_platform(params.chain_id);
    ensure_chain_access(&user, &state, &chain_id).await?;
    let report = verify::verify_chain(&state.db, &chain_id, params.full).await?;
    Ok(Json(report))
}

#[derive(Debug, Deserialize, IntoParams)]
pub struct VerifyEpochsParams {
    /// Re-check every epoch instead of continuing after the last verified one.
    #[serde(default)]
    pub full: bool,
}

#[utoipa::path(
    get,
    path = "/audit/verify/epochs",
    tag = "audit",
    description = "Verify the platform-wide epoch timeline: continuity from its watermark, hashes and signatures. Requires the Admin global permission.",
    params(VerifyEpochsParams),
    responses(
        (status = 200, description = "Epoch timeline verification report", body = EpochReport),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden")
    ),
    security(
        ("bearer_auth" = []),
        ("pat" = [])
    )
)]
#[tracing::instrument(name = "GET /audit/verify/epochs", skip_all)]
pub async fn verify_epochs(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Query(params): Query<VerifyEpochsParams>,
) -> Result<Json<EpochReport>, ApiError> {
    user.sub()?;
    user.check_global_permission(&state, GlobalPermission::Admin)
        .await?;
    let report = verify::verify_epochs(&state.db, params.full).await?;
    Ok(Json(report))
}
