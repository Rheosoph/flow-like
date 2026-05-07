use crate::{
    audit_branch, ensure_permission, entity::app, error::ApiError, middleware::jwt::AppUser,
    permission::role_permission::RolePermissions, state::AppState,
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
    /// Defaults to false; project owners must opt in.
    pub allow_forking: bool,
}

/// Toggle the project-level Fork-an-app opt-in. Only the app's Owner role
/// can change this; the value is checked server-side on every fork
/// request and on the public preview endpoint.
#[utoipa::path(
    patch,
    path = "/apps/{app_id}/settings/forking",
    tag = "forking",
    params(
        ("app_id" = String, Path, description = "Application ID")
    ),
    request_body = UpdateForkingBody,
    responses(
        (status = 200, description = "Forking flag updated"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "Application not found")
    )
)]
#[tracing::instrument(name = "PATCH /apps/{app_id}/settings/forking", skip(state, user, body))]
pub async fn change_forking(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(app_id): Path<String>,
    Json(body): Json<UpdateForkingBody>,
) -> Result<Json<()>, ApiError> {
    ensure_permission!(user, &app_id, &state, RolePermissions::Owner);

    let app_row = app::Entity::find()
        .filter(app::Column::Id.eq(&app_id))
        .one(&state.db)
        .await?
        .ok_or(ApiError::NOT_FOUND)?;

    if app_row.allow_forking == body.allow_forking {
        return Ok(Json(()));
    }

    let mut active = app_row.into_active_model();
    active.allow_forking = Set(body.allow_forking);
    active.updated_at = Set(chrono::Utc::now().naive_utc());
    active.update(&state.db).await?;

    audit_branch!(
        state,
        user,
        app_id,
        "app.settings.forking",
        "App",
        app_id,
        format!("allow_forking = {}", body.allow_forking)
    );

    Ok(Json(()))
}
