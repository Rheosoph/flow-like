use crate::{
    entity::app,
    error::ApiError,
    middleware::jwt::AppUser,
    permission::fork_permission::{ForkTargetKind, check_can_fork},
    state::AppState,
    utils::fork::{
        ForkPolicy,
        preview::{
            ForkSizeBreakdown, RemoteTokenSite, compute_fork_size_breakdown,
            detect_remote_token_sites,
        },
    },
};
use axum::{
    Extension, Json,
    extract::{Path, Query, State},
};
use sea_orm::EntityTrait;
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

#[derive(Clone, Debug, Default, Deserialize, IntoParams)]
pub struct ForkPreviewQuery {
    /// Where the fork would land. Defaults to `online` (same backend);
    /// `offline` exercises the unauthenticated public-fork gate when the
    /// caller is anonymous.
    pub target: Option<ForkPreviewTarget>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ForkPreviewTarget {
    #[default]
    Online,
    Offline,
}

impl From<ForkPreviewTarget> for ForkTargetKind {
    fn from(t: ForkPreviewTarget) -> Self {
        match t {
            ForkPreviewTarget::Online => ForkTargetKind::Online,
            ForkPreviewTarget::Offline => ForkTargetKind::Offline,
        }
    }
}

#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct ForkPreviewResponse {
    pub source_app_id: String,
    /// Sum of every object's size under `apps/{source_app_id}/...`.
    pub total_size_bytes: u64,
    /// Object count under the same prefix.
    pub total_object_count: u64,
    /// Hard cap from the deployment config (`forking.max_size_bytes`).
    pub max_size_bytes: u64,
    /// Hard cap from the deployment config (`forking.max_file_count`).
    pub max_file_count: u64,
    /// Whether the fork — after applying the owner's policy — fits within
    /// both caps. An app that is too large as a whole can still be
    /// forkable if the policy excludes its database or files.
    pub within_limits: bool,
    /// The source owner's fork policy. Read-only for the forker: the owner
    /// decides what a fork of their app contains.
    pub fork_policy: ForkPolicy,
    /// Per-category sizes of the source, so the UI can show what each
    /// category costs and what the policy excludes.
    pub size_breakdown: ForkSizeBreakdown,
    /// Bytes that will actually be copied once the policy is applied.
    pub selected_size_bytes: u64,
    /// Objects that will actually be copied once the policy is applied.
    pub selected_object_count: u64,
    /// True if any remote-event tokens were detected on the source —
    /// caller should prompt the user for one before invoking the fork.
    pub requires_token: bool,
    /// Per-site detail (HTTP auth_token, PAT, OAuth) so the UI can
    /// explain *which* events need re-auth.
    pub remote_token_sites: Vec<RemoteTokenSite>,
    /// Whether the project owner has opted into the Fork-an-app
    /// feature (`App.allow_forking`).
    pub allow_forking: bool,
    /// Whether the calling user passes the permission gate for the
    /// requested `target` (auth + read-set OR public-free anonymous).
    pub user_can_fork: bool,
    /// Reason `user_can_fork` is false, when applicable. Empty string
    /// when the caller is allowed.
    pub disallow_reason: String,
}

/// Pre-fork dry run. Returns the size + count totals, detected
/// remote-token sites, and the permission-gate verdict for the caller.
/// Does **not** allocate any state — safe to call as a probe before
/// committing to a full fork.
#[utoipa::path(
    get,
    path = "/apps/{app_id}/fork/preview",
    tag = "forking",
    description = "Compute the size and remote-token requirements of a hypothetical fork of this app, and report whether the caller is allowed to perform it.",
    params(
        ("app_id" = String, Path, description = "Source application ID"),
        ForkPreviewQuery
    ),
    responses(
        (status = 200, description = "Fork preview", body = ForkPreviewResponse),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "Source app not found")
    )
)]
#[tracing::instrument(name = "GET /apps/{app_id}/fork/preview", skip(state, user, params))]
pub async fn get_fork_preview(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(app_id): Path<String>,
    Query(params): Query<ForkPreviewQuery>,
) -> Result<Json<ForkPreviewResponse>, ApiError> {
    let target_kind = params.target.unwrap_or_default().into();

    // The caller-facing permission verdict is reported in the response
    // body rather than via 403 so the UI can show *why* the button is
    // disabled (forking off, no read access, anonymous online, etc.).
    let (user_can_fork, disallow_reason, app_row) =
        match check_can_fork(&user, &app_id, &state, target_kind).await {
            Ok(row) => (true, String::new(), row),
            Err(err) => {
                let reason = err.to_string();
                // Still need the row to fill out size / requires_token
                // when it's just a permission miss. NOT_FOUND propagates.
                let row = app::Entity::find_by_id(app_id.as_str())
                    .one(&state.db)
                    .await?
                    .ok_or(ApiError::NOT_FOUND)?;
                (false, reason, row)
            }
        };

    let fork_policy = ForkPolicy::from_app_row(&app_row);
    let size_breakdown = compute_fork_size_breakdown(&state, &app_id).await?;
    let (total_size_bytes, total_object_count) = size_breakdown.total();
    let (selected_size_bytes, selected_object_count) = size_breakdown.selected(&fork_policy);
    let remote_token_sites = detect_remote_token_sites(&state, &app_id).await?;

    let max_size_bytes = state.platform_config.forking.max_size_bytes;
    let max_file_count = state.platform_config.forking.max_file_count;
    // Caps apply to what will actually be copied, not to the whole app —
    // otherwise the policy could never rescue an over-cap app, which is
    // the main reason to exclude a category.
    let within_limits =
        selected_size_bytes <= max_size_bytes && selected_object_count <= max_file_count;

    Ok(Json(ForkPreviewResponse {
        source_app_id: app_id,
        total_size_bytes,
        total_object_count,
        max_size_bytes,
        max_file_count,
        within_limits,
        fork_policy,
        size_breakdown,
        selected_size_bytes,
        selected_object_count,
        requires_token: !remote_token_sites.is_empty(),
        remote_token_sites,
        allow_forking: app_row.allow_forking,
        user_can_fork,
        disallow_reason,
    }))
}
