use crate::{
    ensure_permission, error::ApiError, middleware::jwt::AppUser,
    permission::role_permission::RolePermissions, state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, State},
};
use flow_like::a2ui::widget::Version;

#[utoipa::path(
    get,
    path = "/apps/{app_id}/widgets/{widget_id}/versions",
    tag = "widgets",
    description = "List the published versions of a widget, newest first.",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        ("widget_id" = String, Path, description = "Widget ID")
    ),
    responses(
        (status = 200, description = "Versions as (major, minor, patch) tuples, newest first", body = Vec<(u32, u32, u32)>),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden")
    ),
    security(
        ("bearer_auth" = []),
        ("api_key" = []),
        ("pat" = [])
    )
)]
#[tracing::instrument(
    name = "GET /apps/{app_id}/widgets/{widget_id}/versions",
    skip(state, user)
)]
pub async fn get_widget_versions(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((app_id, widget_id)): Path<(String, String)>,
) -> Result<Json<Vec<Version>>, ApiError> {
    ensure_permission!(user, &app_id, &state, RolePermissions::ReadWidgets);

    let app = state.master_app(&user.sub()?, &app_id, &state).await?;
    let versions = app.get_widget_versions(&widget_id).await?;

    Ok(Json(versions))
}
