//! Admin endpoint for cache expiry reconciliation.
//!
//! `POST /admin/cache/sweep` removes entries whose lifetime has elapsed. Long-running
//! deployments already run the in-process ticker from `cache::sweeper`; this exists for
//! ad-hoc reconciliation and for deployments where the API has no persistent process to
//! host a ticker.

use axum::{Extension, Json, extract::State};
use serde::Serialize;
use utoipa::ToSchema;

use crate::{
    cache::{require_cache_store, sweeper::sweep_once},
    error::ApiError,
    middleware::jwt::AppUser,
    permission::global_permission::GlobalPermission,
    state::AppState,
};

#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct SweepCacheResponse {
    /// Number of expired entries removed.
    pub deleted: i64,
    /// Backend that performed the sweep. Backends with native expiry always report 0.
    pub backend: String,
}

/// POST /admin/cache/sweep
#[utoipa::path(
    post,
    path = "/admin/cache/sweep",
    tag = "admin",
    description = "Remove cache entries whose lifetime has elapsed. Backends that expire entries on their own report zero.",
    responses(
        (status = 200, description = "Sweep completed", body = SweepCacheResponse),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden — Admin permission required"),
        (status = 503, description = "Cache backend is not configured")
    )
)]
#[tracing::instrument(name = "POST /admin/cache/sweep", skip(state, user))]
pub async fn sweep_cache(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
) -> Result<Json<SweepCacheResponse>, ApiError> {
    user.check_global_permission(&state, GlobalPermission::Admin)
        .await?;

    let store = require_cache_store(&state.cache).await?;

    let deleted = sweep_once(store.as_ref()).await.map_err(|e| {
        tracing::error!(error = %e, "Admin cache sweep failed");
        ApiError::internal_error(flow_like_types::anyhow!("Cache sweep failed: {}", e))
    })?;

    Ok(Json(SweepCacheResponse {
        deleted,
        backend: store.backend_name().to_string(),
    }))
}
