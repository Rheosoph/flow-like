use crate::{
    ensure_permission,
    entity::{
        app,
        sea_orm_active_enums::{Status, Visibility},
    },
    error::ApiError,
    middleware::jwt::AppUser,
    permission::role_permission::RolePermissions,
    state::AppState,
    utils::fork::preview::compute_app_content_size_and_count,
};
use axum::{
    Extension, Json,
    extract::{Path, State},
};
use sea_orm::{ActiveModelTrait, ActiveValue::Set, EntityTrait, IntoActiveModel};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Clone, Debug, Default, Deserialize, Serialize, ToSchema)]
pub struct FinalizeOnlineForkBody {
    /// Override the destination's final visibility. Must be one of
    /// `Private` (default) or `Prototype`. The fork is materialized
    /// with `Offline` until this call.
    #[serde(default)]
    pub visibility: Option<FinalizeVisibility>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum FinalizeVisibility {
    #[default]
    Private,
    Prototype,
}

impl From<FinalizeVisibility> for Visibility {
    fn from(v: FinalizeVisibility) -> Self {
        match v {
            FinalizeVisibility::Private => Visibility::Private,
            FinalizeVisibility::Prototype => Visibility::Prototype,
        }
    }
}

#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct FinalizeOnlineForkResponse {
    pub app_id: String,
    /// Total bytes uploaded by the desktop into the destination's
    /// content prefix (`apps/{app_id}/{metadata,upload,storage}/`).
    pub total_size_bytes: u64,
    pub total_object_count: u64,
    /// New visibility post-finalize.
    pub visibility: String,
    /// New status post-finalize (always `Active`).
    pub status: String,
}

/// Finalize an offline → online fork.
///
/// The desktop side splits its work into two streams:
/// - **Content** (metadata/, upload/, storage/) is uploaded via the
///   scoped credentials returned by `/fork/online/begin` directly to
///   the content store at `apps/{app_id}/...`.
/// - **Meta** (boards, events, widgets, templates, pages) is pushed
///   via the **normal app-edit endpoints** (`PUT /apps/{id}/board/`,
///   `POST /apps/{id}/events/upsert`, etc.). Those endpoints already
///   know how to write to the meta store + insert DB rows correctly,
///   so the fork code doesn't reinvent any of that.
///
/// This endpoint just:
/// 1. Verifies the **content-store** upload doesn't exceed the
///    deployment's caps (the meta side is naturally bounded by the
///    app-edit endpoints' own quotas).
/// 2. Flips the App row's visibility from `Offline` →
///    `Private` / `Prototype` and status from `Inactive` → `Active`.
/// 3. Records the content-store total in `total_size`.
///
/// Notably it does **not** read the manifest, traverse `*.event`
/// files, or insert event/page/widget/template DB rows — that's the
/// desktop's responsibility via the existing edit endpoints.
#[utoipa::path(
    post,
    path = "/apps/{app_id}/fork/online/finalize",
    tag = "forking",
    description = "Finalize an offline → online fork after the desktop has uploaded the content bundle and synced its meta artifacts via the normal app-edit endpoints.",
    params(
        ("app_id" = String, Path, description = "Destination application id (returned by /fork/online/begin)"),
    ),
    request_body = FinalizeOnlineForkBody,
    responses(
        (status = 200, description = "Fork finalized; the app is now visible to its owner", body = FinalizeOnlineForkResponse),
        (status = 400, description = "Uploaded content exceeds the deployment's size or file-count cap"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden — caller is not the destination's owner"),
        (status = 404, description = "Destination app not found")
    )
)]
#[tracing::instrument(
    name = "POST /apps/{app_id}/fork/online/finalize",
    skip(state, user, body)
)]
pub async fn finalize_online_fork(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(app_id): Path<String>,
    Json(body): Json<FinalizeOnlineForkBody>,
) -> Result<Json<FinalizeOnlineForkResponse>, ApiError> {
    ensure_permission!(user, &app_id, &state, RolePermissions::Owner);

    let app_row = app::Entity::find_by_id(app_id.as_str())
        .one(&state.db)
        .await?
        .ok_or(ApiError::NOT_FOUND)?;

    // Sanity: `Offline` + `Inactive` is the shape `begin_online_fork`
    // creates. Anything else means the caller is finalizing a fork
    // that already finalized (or an unrelated app).
    if !matches!(app_row.visibility, Visibility::Offline) {
        return Err(ApiError::bad_request(format!(
            "app {} is not in offline-bundle state (visibility={:?}); fork is already finalized or wasn't started via /fork/online/begin",
            app_id, app_row.visibility
        )));
    }

    // Re-verify the content-store upload size (the body summary at
    // begin was the desktop's claim — this is the authoritative
    // measurement of what actually arrived).
    let (total_size_bytes, total_object_count) =
        compute_app_content_size_and_count(&state, &app_id).await?;
    let max_size = state.platform_config.forking.max_size_bytes;
    let max_count = state.platform_config.forking.max_file_count;
    if total_size_bytes > max_size {
        return Err(ApiError::bad_request(format!(
            "uploaded content exceeds the deployment's fork size cap ({} bytes > {} bytes)",
            total_size_bytes, max_size
        )));
    }
    if total_object_count > max_count {
        return Err(ApiError::bad_request(format!(
            "uploaded content exceeds the deployment's fork file-count cap ({} > {})",
            total_object_count, max_count
        )));
    }

    let visibility: Visibility = body.visibility.unwrap_or_default().into();
    let mut active = app_row.into_active_model();
    active.visibility = Set(visibility.clone());
    active.status = Set(Status::Active);
    active.total_size = Set(total_size_bytes as i64);
    active.updated_at = Set(chrono::Utc::now().naive_utc());
    active.update(&state.db).await?;

    Ok(Json(FinalizeOnlineForkResponse {
        app_id,
        total_size_bytes,
        total_object_count,
        visibility: format!("{:?}", visibility),
        status: format!("{:?}", Status::Active),
    }))
}
