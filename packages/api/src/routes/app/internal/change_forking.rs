use crate::{
    audit_branch, ensure_permission, entity::app, error::ApiError, middleware::jwt::AppUser,
    permission::role_permission::RolePermissions, state::AppState, utils::fork::ForkPolicy,
};
use axum::{
    Extension, Json,
    extract::{Path, State},
};
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, EntityTrait, IntoActiveModel, QueryFilter,
};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Clone, Serialize, Deserialize, ToSchema)]
pub struct UpdateForkingBody {
    /// When true, members with read access can create a fork of this app.
    /// Defaults to false; project owners must opt in. Omit to leave unchanged.
    #[serde(default)]
    pub allow_forking: Option<bool>,
    /// What a fork of this app copies. Owner-defined — the person forking
    /// gets no choice. Omit to leave unchanged.
    #[serde(default)]
    pub fork_policy: Option<ForkPolicy>,
}

#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct ForkSettingsResponse {
    pub allow_forking: bool,
    /// Resolved policy — a never-configured app reports the permissive
    /// default, which is what a fork would actually copy.
    pub fork_policy: ForkPolicy,
}

/// Read the project-level fork settings: the Fork-an-app opt-in and the
/// owner-defined policy describing what a fork of this app includes.
#[utoipa::path(
    get,
    path = "/apps/{app_id}/settings/forking",
    tag = "forking",
    params(
        ("app_id" = String, Path, description = "Application ID")
    ),
    responses(
        (status = 200, description = "Current fork settings", body = ForkSettingsResponse),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "Application not found")
    )
)]
#[tracing::instrument(name = "GET /apps/{app_id}/settings/forking", skip(state, user))]
pub async fn get_forking(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(app_id): Path<String>,
) -> Result<Json<ForkSettingsResponse>, ApiError> {
    ensure_permission!(user, &app_id, &state, RolePermissions::Owner);

    let app_row = app::Entity::find()
        .filter(app::Column::Id.eq(&app_id))
        .one(&state.db)
        .await?
        .ok_or(ApiError::NOT_FOUND)?;

    Ok(Json(ForkSettingsResponse {
        allow_forking: app_row.allow_forking,
        fork_policy: ForkPolicy::from_app_row(&app_row),
    }))
}

/// Update the project-level Fork-an-app opt-in and/or the owner-defined
/// fork policy. Only the app's Owner role can change these; both are
/// enforced server-side on every fork request and on the preview endpoint.
#[utoipa::path(
    patch,
    path = "/apps/{app_id}/settings/forking",
    tag = "forking",
    params(
        ("app_id" = String, Path, description = "Application ID")
    ),
    request_body = UpdateForkingBody,
    responses(
        (status = 200, description = "Fork settings updated"),
        (status = 400, description = "Nothing to update"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "Application not found")
    )
)]
#[tracing::instrument(
    name = "PATCH /apps/{app_id}/settings/forking",
    skip(state, user, body)
)]
pub async fn change_forking(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(app_id): Path<String>,
    Json(body): Json<UpdateForkingBody>,
) -> Result<Json<()>, ApiError> {
    ensure_permission!(user, &app_id, &state, RolePermissions::Owner);

    if body.allow_forking.is_none() && body.fork_policy.is_none() {
        return Err(ApiError::bad_request(
            "supply allow_forking, fork_policy, or both",
        ));
    }

    let app_row = app::Entity::find()
        .filter(app::Column::Id.eq(&app_id))
        .one(&state.db)
        .await?
        .ok_or(ApiError::NOT_FOUND)?;

    let allow_changed = body
        .allow_forking
        .is_some_and(|allow| allow != app_row.allow_forking);
    let policy_changed = body
        .fork_policy
        .as_ref()
        .is_some_and(|policy| *policy != ForkPolicy::from_app_row(&app_row));

    if !allow_changed && !policy_changed {
        return Ok(Json(()));
    }

    let mut detail = Vec::new();
    let mut active = app_row.into_active_model();
    if let Some(allow) = body.allow_forking.filter(|_| allow_changed) {
        active.allow_forking = Set(allow);
        detail.push(format!("allow_forking = {allow}"));
    }
    if let Some(policy) = body.fork_policy.filter(|_| policy_changed) {
        let encoded = serde_json::to_value(&policy).map_err(|e| {
            ApiError::internal_error(flow_like_types::anyhow!("encode policy: {e}"))
        })?;
        detail.push(format!("fork_policy = {encoded}"));
        active.fork_policy = Set(Some(encoded));
    }
    active.updated_at = Set(chrono::Utc::now().naive_utc());
    active.update(&state.db).await?;

    audit_branch!(
        state,
        user,
        app_id,
        "app.settings.forking",
        "App",
        app_id,
        detail.join(", ")
    );

    Ok(Json(()))
}
