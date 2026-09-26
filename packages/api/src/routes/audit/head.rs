use axum::{
    Extension, Json,
    extract::{Query, State},
};
use sea_orm::{ColumnTrait, EntityTrait, Order, QueryFilter, QueryOrder};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

use crate::{
    audit::{
        verify::{self, HeadCheck},
        wire::{EpochLine, SealLine},
    },
    entity::{audit_epoch, audit_seal},
    error::ApiError,
    middleware::jwt::AppUser,
    state::AppState,
};

use super::{chain_or_platform, ensure_chain_access};

#[derive(Debug, Deserialize, IntoParams)]
pub struct HeadParams {
    /// Chain id: an app or package id, `<id>#activity`, or `platform` (the default).
    pub chain_id: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct AuditHead {
    pub chain_id: String,
    /// Newest seal of the chain that an epoch anchors; `epoch_proof` proves its
    /// inclusion in `epoch`.
    pub seal: Option<SealLine>,
    /// The signed epoch that anchors `seal`.
    pub epoch: Option<EpochLine>,
}

#[derive(Clone, Debug, Deserialize, ToSchema)]
pub struct HeadCheckBody {
    /// Sequence of the retained epoch.
    pub seq: i64,
    /// Hex hash of the retained epoch.
    pub hash: String,
    /// Chain of the retained seal (`AuditHead.chain_id`). With `seal_seq` and
    /// `seal_hash`, the check also detects that the chain's newest seals were removed.
    pub chain_id: Option<String>,
    pub seal_seq: Option<i64>,
    /// Hex hash of the retained seal.
    pub seal_hash: Option<String>,
}

#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct HeadCheckResponse {
    pub result: HeadCheck,
}

#[utoipa::path(
    get,
    path = "/audit/head",
    tag = "audit",
    description = "Newest anchored seal of an audit chain with the signed epoch that anchors it. Keep it outside the platform: its epoch can later be checked with /audit/head/check to prove that nothing was removed or rewritten. App chains require Owner on that app; other chains require the Admin global permission.",
    params(HeadParams),
    responses(
        (status = 200, description = "Chain head; seal and epoch are null until the chain has an anchored seal", body = AuditHead),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden")
    ),
    security(
        ("bearer_auth" = []),
        ("pat" = [])
    )
)]
#[tracing::instrument(name = "GET /audit/head", skip_all)]
pub async fn chain_head(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Query(params): Query<HeadParams>,
) -> Result<Json<AuditHead>, ApiError> {
    let chain_id = chain_or_platform(params.chain_id);
    ensure_chain_access(&user, &state, &chain_id).await?;
    // Without any epoch nothing is anchored; skip walking the chain's index.
    if audit_epoch::Entity::find().one(&state.db).await?.is_none() {
        return Ok(Json(AuditHead {
            chain_id,
            seal: None,
            epoch: None,
        }));
    }
    let seal = audit_seal::Entity::find()
        .filter(audit_seal::Column::ChainId.eq(&chain_id))
        .filter(audit_seal::Column::EpochSeq.is_not_null())
        .order_by(audit_seal::Column::Seq, Order::Desc)
        .one(&state.db)
        .await?;
    let epoch = match seal.as_ref().and_then(|seal| seal.epoch_seq) {
        Some(epoch_seq) => {
            audit_epoch::Entity::find_by_id(epoch_seq)
                .one(&state.db)
                .await?
        }
        None => None,
    };
    Ok(Json(AuditHead {
        chain_id,
        seal: seal.as_ref().map(SealLine::from),
        epoch: epoch.as_ref().map(EpochLine::from),
    }))
}

#[utoipa::path(
    post,
    path = "/audit/head/check",
    tag = "audit",
    description = "Compare a head retained earlier (from /audit/head) with the platform. `matches` means the epoch and, when sent, the chain's seal are unchanged; `differs` means the timeline or the chain was truncated or rewritten; `archived` means they were pruned and must be checked against the monthly archive. Any authenticated caller may check an epoch; sending the chain's seal requires access to that chain.",
    request_body = HeadCheckBody,
    responses(
        (status = 200, description = "Result of the comparison", body = HeadCheckResponse),
        (status = 400, description = "Invalid sequence or hash"),
        (status = 401, description = "Unauthorized")
    ),
    security(
        ("bearer_auth" = []),
        ("api_key" = []),
        ("pat" = [])
    )
)]
#[tracing::instrument(name = "POST /audit/head/check", skip_all)]
pub async fn check_head(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Json(body): Json<HeadCheckBody>,
) -> Result<Json<HeadCheckResponse>, ApiError> {
    if matches!(user, AppUser::Unauthorized) {
        return Err(ApiError::UNAUTHORIZED);
    }
    if body.seq < 1 {
        return Err(ApiError::bad_request(format!(
            "epoch sequence {} must be at least 1",
            body.seq
        )));
    }
    let hash = hex_hash(&body.hash, "epoch hash")?;
    let seal = match (
        body.chain_id.as_deref(),
        body.seal_seq,
        body.seal_hash.as_deref(),
    ) {
        (Some(chain_id), Some(seq), Some(seal_hash)) => {
            // Whether a chain holds a seal is the chain's business.
            ensure_chain_access(&user, &state, chain_id).await?;
            Some((chain_id, seq, hex_hash(seal_hash, "seal hash")?))
        }
        (None, None, None) => None,
        _ => {
            return Err(ApiError::bad_request(
                "chain_id, seal_seq and seal_hash must be sent together",
            ));
        }
    };
    let retained = seal
        .as_ref()
        .map(|(chain_id, seq, hash)| verify::RetainedSeal {
            chain_id,
            seq: *seq,
            hash,
        });
    let result = verify::check_head(&state.db, body.seq, &hash, retained).await?;
    Ok(Json(HeadCheckResponse { result }))
}

fn hex_hash(value: &str, what: &str) -> Result<Vec<u8>, ApiError> {
    hex::decode(value.trim())
        .ok()
        .filter(|hash| hash.len() == 32)
        .ok_or_else(|| ApiError::bad_request(format!("{what} must be 64 hex characters")))
}
