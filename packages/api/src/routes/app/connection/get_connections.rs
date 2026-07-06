use crate::{
    ensure_permission,
    entity::app_connection,
    error::ApiError,
    middleware::jwt::AppUser,
    permission::role_permission::RolePermissions,
    routes::app::connection::{
        AppConnectionInfo, app_meta_lookup, role_name_lookup, to_connection_info,
    },
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
pub struct AppConnectionsResponse {
    /// Apps that have (or requested) access to this app
    pub incoming: Vec<AppConnectionInfo>,
    /// Apps this app has (or requested) access to
    pub outgoing: Vec<AppConnectionInfo>,
}

#[utoipa::path(
    get,
    path = "/apps/{app_id}/connections",
    tag = "team",
    description = "List app connections: apps with access to this app and apps this app can access.",
    params(
        ("app_id" = String, Path, description = "Application ID")
    ),
    responses(
        (status = 200, description = "App connections", body = AppConnectionsResponse),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden")
    ),
    security(
        ("bearer_auth" = []),
        ("api_key" = []),
        ("pat" = [])
    )
)]
#[tracing::instrument(name = "GET /apps/{app_id}/connections", skip(state, user))]
pub async fn get_connections(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(app_id): Path<String>,
) -> Result<Json<AppConnectionsResponse>, ApiError> {
    ensure_permission!(user, &app_id, &state, RolePermissions::ReadTeam);

    let connections = app_connection::Entity::find()
        .filter(
            app_connection::Column::TargetAppId
                .eq(&app_id)
                .or(app_connection::Column::SourceAppId.eq(&app_id)),
        )
        .order_by_asc(app_connection::Column::CreatedAt)
        .all(&state.db)
        .await?;

    let role_ids: Vec<String> = connections
        .iter()
        .filter_map(|c| c.role_id.clone())
        .collect();
    let other_app_ids: Vec<String> = connections
        .iter()
        .map(|c| {
            if c.target_app_id == app_id {
                c.source_app_id.clone()
            } else {
                c.target_app_id.clone()
            }
        })
        .collect();

    let role_names = role_name_lookup(&state, &role_ids).await?;
    let app_meta = app_meta_lookup(&state, &other_app_ids).await?;

    let mut incoming = Vec::new();
    let mut outgoing = Vec::new();

    for connection in connections {
        let is_incoming = connection.target_app_id == app_id;
        let other_app_id = if is_incoming {
            connection.source_app_id.clone()
        } else {
            connection.target_app_id.clone()
        };
        let info = to_connection_info(connection, &role_names, &app_meta, &other_app_id);
        if is_incoming {
            incoming.push(info);
        } else {
            outgoing.push(info);
        }
    }

    Ok(Json(AppConnectionsResponse { incoming, outgoing }))
}
