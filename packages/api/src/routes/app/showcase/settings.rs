use super::types::ShowcaseSettings;
use crate::{
    audit_branch, ensure_permission, entity::app, error::ApiError, middleware::jwt::AppUser,
    permission::role_permission::RolePermissions, state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, State},
};
use sea_orm::{ActiveModelTrait, ActiveValue::Set, EntityTrait, IntoActiveModel};

/// Read what the app's store page shows. Owner only; an app that never chose shows nothing.
#[utoipa::path(
    get,
    path = "/apps/{app_id}/settings/showcase",
    tag = "apps",
    description = "Read which parts of the app its public store page shows. Everything starts hidden.",
    params(("app_id" = String, Path, description = "Application ID")),
    responses(
        (status = 200, description = "Current store page settings", body = ShowcaseSettings),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Only the app owner can read these settings"),
        (status = 404, description = "Application not found")
    )
)]
#[tracing::instrument(name = "GET /apps/{app_id}/settings/showcase", skip(state, user))]
pub async fn get_showcase_settings(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(app_id): Path<String>,
) -> Result<Json<ShowcaseSettings>, ApiError> {
    ensure_permission!(user, &app_id, &state, RolePermissions::Owner);

    let row = app::Entity::find_by_id(&app_id)
        .one(&state.db)
        .await?
        .ok_or(ApiError::NOT_FOUND)?;
    Ok(Json(ShowcaseSettings::from_column(row.showcase.as_ref())))
}

/// Replace what the app's store page shows. Owner only.
#[utoipa::path(
    put,
    path = "/apps/{app_id}/settings/showcase",
    tag = "apps",
    description = "Choose which parts of the app its public store page shows: data, ways to use it, pages, widgets, templates, flows, requirements and activity.",
    params(("app_id" = String, Path, description = "Application ID")),
    request_body = ShowcaseSettings,
    responses(
        (status = 200, description = "The stored settings", body = ShowcaseSettings),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Only the app owner can change these settings"),
        (status = 404, description = "Application not found"),
        (status = 409, description = "BOT_WORKSPACE_LOCKED: a bot's workspace has no store page")
    )
)]
#[tracing::instrument(
    name = "PUT /apps/{app_id}/settings/showcase",
    skip(state, user, settings)
)]
pub async fn put_showcase_settings(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(app_id): Path<String>,
    Json(settings): Json<ShowcaseSettings>,
) -> Result<Json<ShowcaseSettings>, ApiError> {
    ensure_permission!(user, &app_id, &state, RolePermissions::Owner);

    let row = app::Entity::find_by_id(&app_id)
        .one(&state.db)
        .await?
        .ok_or(ApiError::NOT_FOUND)?;
    if row.bot_id.is_some() {
        return Err(crate::routes::bots::workspace::locked(&app_id));
    }

    let settings = settings.normalized();
    if ShowcaseSettings::from_column(row.showcase.as_ref()) == settings {
        return Ok(Json(settings));
    }

    let encoded = serde_json::to_value(&settings).map_err(|error| {
        ApiError::internal_error(flow_like_types::anyhow!(
            "encode showcase settings for app {app_id}: {error}"
        ))
    })?;
    let mut active = row.into_active_model();
    active.showcase = Set(Some(encoded.clone()));
    active.updated_at = Set(chrono::Utc::now().fixed_offset());
    active.update(&state.db).await?;
    super::evict(&state, &app_id);

    audit_branch!(
        state,
        user,
        app_id,
        "app.settings.showcase",
        "App",
        app_id,
        encoded
    );

    Ok(Json(settings))
}
