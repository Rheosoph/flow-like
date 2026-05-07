//! Admin endpoints for fork-orphan janitoring.
//!
//! Cross-mode fork sessions (online↔offline) materialize a destination
//! storage prefix `apps/{id}/...` BEFORE every DB row is committed. If
//! a session is interrupted (desktop crash mid-upload, finalize call
//! never arrives, fork session expires) the prefix can outlive any
//! matching `App` row. This module exposes the two primitives needed
//! to garbage-collect them: a dry-run listing and a per-prefix delete.

use axum::{
    Extension, Json,
    extract::{Path, State},
};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::{
    error::ApiError,
    middleware::jwt::AppUser,
    permission::global_permission::GlobalPermission,
    state::AppState,
    utils::fork::cleanup::{
        OrphanPrefix, delete_orphan_app_prefix, find_orphan_app_prefixes,
    },
};

#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct OrphanForkEntry {
    pub app_id: String,
    pub object_count: u64,
    pub total_size_bytes: u64,
}

impl From<OrphanPrefix> for OrphanForkEntry {
    fn from(o: OrphanPrefix) -> Self {
        Self {
            app_id: o.app_id,
            object_count: o.object_count,
            total_size_bytes: o.total_size_bytes,
        }
    }
}

#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct ListOrphanForksResponse {
    pub orphans: Vec<OrphanForkEntry>,
    /// Sum of `object_count` across all orphan prefixes — useful for
    /// budgeting a cleanup pass.
    pub total_objects: u64,
    /// Sum of `total_size_bytes` across all orphan prefixes.
    pub total_size_bytes: u64,
}

/// List every `apps/{id}/...` prefix on the master store with no
/// matching `App` row. Pure read; no mutation.
#[utoipa::path(
    get,
    path = "/admin/forks/orphans",
    tag = "admin",
    description = "List every storage prefix under apps/ that has no matching App row.",
    responses(
        (status = 200, description = "Orphan prefixes (may be empty)", body = ListOrphanForksResponse),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden — Admin permission required")
    )
)]
#[tracing::instrument(name = "GET /admin/forks/orphans", skip(state, user))]
pub async fn list_orphan_forks(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
) -> Result<Json<ListOrphanForksResponse>, ApiError> {
    user.check_global_permission(&state, GlobalPermission::Admin)
        .await?;

    let orphans = find_orphan_app_prefixes(&state).await?;
    let total_objects: u64 = orphans.iter().map(|o| o.object_count).sum();
    let total_size_bytes: u64 = orphans.iter().map(|o| o.total_size_bytes).sum();

    Ok(Json(ListOrphanForksResponse {
        orphans: orphans.into_iter().map(Into::into).collect(),
        total_objects,
        total_size_bytes,
    }))
}

#[derive(Clone, Debug, Default, Deserialize, ToSchema)]
pub struct DeleteOrphanForkBody {
    /// Pass `true` to confirm the deletion. Without this flag the
    /// endpoint returns 400 — guards against an accidental DELETE
    /// from a misconfigured client.
    #[serde(default)]
    pub confirm: bool,
}

#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct DeleteOrphanForkResponse {
    pub app_id: String,
    pub deleted_objects: u64,
}

/// Delete every object under `apps/{app_id}/...`. Re-checks that the
/// id is still in the orphan list at call time so a concurrent
/// upload-in-progress can't be wiped out.
#[utoipa::path(
    post,
    path = "/admin/forks/orphans/{app_id}/delete",
    tag = "admin",
    description = "Delete every object under apps/{app_id}/ on the master store. Refuses if the id is not currently an orphan or `confirm` is not set.",
    params(
        ("app_id" = String, Path, description = "Orphan storage prefix to delete"),
    ),
    request_body = DeleteOrphanForkBody,
    responses(
        (status = 200, description = "Prefix deleted", body = DeleteOrphanForkResponse),
        (status = 400, description = "App is no longer an orphan, or confirm flag missing"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden — Admin permission required"),
        (status = 404, description = "No matching orphan prefix")
    )
)]
#[tracing::instrument(name = "POST /admin/forks/orphans/{app_id}/delete", skip(state, user, body))]
pub async fn delete_orphan_fork(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(app_id): Path<String>,
    Json(body): Json<DeleteOrphanForkBody>,
) -> Result<Json<DeleteOrphanForkResponse>, ApiError> {
    user.check_global_permission(&state, GlobalPermission::Admin)
        .await?;

    if !body.confirm {
        return Err(ApiError::bad_request(
            "set `confirm: true` in the body to acknowledge the destructive operation",
        ));
    }

    // Recompute the orphan list at call time so an in-progress upload
    // (whose DB row may have been inserted between list and delete)
    // is not wiped out.
    let orphans = find_orphan_app_prefixes(&state).await?;
    let still_orphan = orphans.iter().any(|o| o.app_id == app_id);
    if !still_orphan {
        return Err(ApiError::not_found(format!(
            "app_id {} is not currently an orphan prefix",
            app_id
        )));
    }

    let deleted = delete_orphan_app_prefix(&state, &app_id).await?;
    Ok(Json(DeleteOrphanForkResponse {
        app_id,
        deleted_objects: deleted,
    }))
}
