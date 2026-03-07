use crate::{
    entity::app,
    error::ApiError,
    middleware::jwt::AppUser,
    routes::app::ensure_app_publicly_visible,
    state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, State},
};
use flow_like::app::App;
use sea_orm::EntityTrait;

#[utoipa::path(
    get,
    path = "/apps/{app_id}",
    tag = "apps",
    description = "Get application details. Returns scoped data for members, or basic info for public apps.",
    params(
        ("app_id" = String, Path, description = "Application ID")
    ),
    responses(
        (status = 200, description = "Application details", body = Object),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden – app is private and user is not a member"),
        (status = 404, description = "Application not found")
    )
)]
#[tracing::instrument(name = "GET /apps/{app_id}", skip(state, user))]
pub async fn get_app(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(app_id): Path<String>,
) -> Result<Json<App>, ApiError> {
    if let Ok(_perm) = user.app_permission(&app_id, &state).await {
        let scoped_app = state.master_app(&user.sub()?, &app_id, &state).await?;

        let app = app::Entity::find_by_id(&app_id)
            .one(&state.db)
            .await?
            .ok_or(ApiError::NOT_FOUND)?;

        let mut app: App = app.into();
        app.bits = scoped_app.bits.clone();
        app.boards = scoped_app.boards.clone();
        app.templates = scoped_app.templates.clone();
        app.events = scoped_app.events.clone();

        return Ok(Json(app));
    }

    user.sub()?;
    let app = ensure_app_publicly_visible(&app_id, &state).await?;
    Ok(Json(app.into()))
}
