use crate::{
    ensure_permission,
    entity::{app_connection, sea_orm_active_enums::AppConnectionStatus},
    error::ApiError,
    middleware::jwt::AppUser,
    permission::role_permission::RolePermissions,
    routes::app::connection::app_meta_lookup,
    state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, State},
};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QueryOrder};
use serde::Serialize;
use utoipa::ToSchema;

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct AccessibleApp {
    /// ID of the app this app has access to
    pub app_id: String,
    /// Name of the accessible app
    pub name: Option<String>,
    /// Description of the accessible app
    pub description: Option<String>,
    /// Icon of the accessible app
    pub icon: Option<String>,
}

#[utoipa::path(
    get,
    path = "/apps/{app_id}/connections/accessible",
    tag = "team",
    description = "List all apps this app has been granted access to. Used to pick a remote project, e.g. for remote database nodes.",
    params(
        ("app_id" = String, Path, description = "Application ID")
    ),
    responses(
        (status = 200, description = "Accessible apps", body = Vec<AccessibleApp>),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden")
    ),
    security(
        ("bearer_auth" = []),
        ("api_key" = []),
        ("pat" = [])
    )
)]
#[tracing::instrument(name = "GET /apps/{app_id}/connections/accessible", skip(state, user))]
pub async fn get_accessible_apps(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(app_id): Path<String>,
) -> Result<Json<Vec<AccessibleApp>>, ApiError> {
    ensure_permission!(user, &app_id, &state, RolePermissions::ReadBoards);

    let connections = app_connection::Entity::find()
        .filter(
            app_connection::Column::SourceAppId
                .eq(&app_id)
                .and(app_connection::Column::Status.eq(AppConnectionStatus::Active)),
        )
        .order_by_asc(app_connection::Column::CreatedAt)
        .all(&state.db)
        .await?;

    let target_app_ids: Vec<String> = connections
        .iter()
        .map(|c| c.target_app_id.clone())
        .collect();
    let app_meta = app_meta_lookup(&state, &target_app_ids).await?;

    let accessible = connections
        .into_iter()
        .map(|connection| {
            let (name, description, icon) = app_meta
                .get(&connection.target_app_id)
                .map(|preview| {
                    (
                        Some(preview.name.clone()),
                        preview.description.clone(),
                        preview.icon.clone(),
                    )
                })
                .unwrap_or((None, None, None));
            AccessibleApp {
                app_id: connection.target_app_id,
                name,
                description,
                icon,
            }
        })
        .collect();

    Ok(Json(accessible))
}
