use crate::{
    audit_branch, ensure_permission, entity::app, error::ApiError, middleware::jwt::AppUser,
    permission::role_permission::RolePermissions, state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, State},
};
use sea_orm::{ActiveModelTrait, ActiveValue::Set, ConnectionTrait, EntityTrait, IntoActiveModel};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct BotAccessSettings {
    /// When true, no bot may use this app: every bot's request, including bots already
    /// attached to it or reaching it through their owner's profile, is refused with
    /// `APP_REFUSES_BOTS`. Members keep their own access.
    pub refuse_bots: bool,
}

/// Stores the setting on `app_id`; true when it changed. A bot's workspace always admits its bot.
pub(crate) async fn store_bot_access<C: ConnectionTrait>(
    db: &C,
    app_id: &str,
    refuse_bots: bool,
) -> Result<bool, ApiError> {
    let app_row = app::Entity::find_by_id(app_id)
        .one(db)
        .await?
        .ok_or(ApiError::NOT_FOUND)?;
    if app_row.bot_id.is_some() {
        return Err(crate::routes::bots::workspace::locked(app_id));
    }
    if app_row.refuse_bots.unwrap_or(false) == refuse_bots {
        return Ok(false);
    }
    let mut active = app_row.into_active_model();
    active.refuse_bots = Set(Some(refuse_bots));
    active.updated_at = Set(chrono::Utc::now().fixed_offset());
    active.update(db).await?;
    Ok(true)
}

/// Decide whether bots may use the app. Only the app's Owner role can change it; it takes
/// effect on the next request of every bot, including bots that already use the app.
#[utoipa::path(
    patch,
    path = "/apps/{app_id}/settings/bots",
    tag = "apps",
    params(
        ("app_id" = String, Path, description = "Application ID")
    ),
    request_body = BotAccessSettings,
    responses(
        (status = 200, description = "The stored setting", body = BotAccessSettings),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Only the app's owners can decide whether bots may use it"),
        (status = 404, description = "Application not found"),
        (status = 409, description = "BOT_WORKSPACE_LOCKED: the app is a bot's workspace, which always admits its bot")
    )
)]
#[tracing::instrument(name = "PATCH /apps/{app_id}/settings/bots", skip(state, user, body))]
pub async fn change_bot_access(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(app_id): Path<String>,
    Json(body): Json<BotAccessSettings>,
) -> Result<Json<BotAccessSettings>, ApiError> {
    ensure_permission!(user, &app_id, &state, RolePermissions::Owner);
    if store_bot_access(&state.db, &app_id, body.refuse_bots).await? {
        audit_branch!(
            state,
            user,
            app_id,
            "app.settings.bots",
            "App",
            app_id,
            serde_json::json!({ "refuse_bots": body.refuse_bots })
        );
    }
    Ok(Json(body))
}
