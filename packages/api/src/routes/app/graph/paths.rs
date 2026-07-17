use crate::{
    ensure_any_permission, error::ApiError, middleware::jwt::AppUser,
    permission::role_permission::RolePermissions, routes::app::db::ScopeParams, state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, Query, State},
};
use flow_like_storage::databases::graph::{GraphPathsResult, GraphStore, lancegraph};
use utoipa::ToSchema;

#[derive(Debug, serde::Deserialize, ToSchema)]
pub struct PathsPayload {
    pub from_label: String,
    #[schema(value_type = Object)]
    pub from_id: flow_like_types::Value,
    pub to_label: String,
    #[schema(value_type = Object)]
    pub to_id: flow_like_types::Value,
    #[serde(default = "default_max_depth")]
    pub max_depth: usize,
    pub limit: Option<usize>,
}

fn default_max_depth() -> usize {
    4
}

#[utoipa::path(
    post,
    path = "/apps/{app_id}/graph/{overlay_id}/paths",
    tag = "graph",
    description = "Find the shortest connections between two objects, including up to two alternative routes.",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        ("overlay_id" = String, Path, description = "Overlay ID"),
        ("scope" = Option<String>, Query, description = "Scope: 'user' or omit for project")
    ),
    request_body = PathsPayload,
    responses(
        (status = 200, description = "Paths between the two objects", body = Object),
        (status = 400, description = "Bad request"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "Overlay not found")
    ),
    security(
        ("bearer_auth" = []),
        ("api_key" = []),
        ("pat" = [])
    )
)]
#[tracing::instrument(
    name = "POST /apps/{app_id}/graph/{overlay_id}/paths",
    skip(state, user, payload)
)]
pub async fn find_paths(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((app_id, overlay_id)): Path<(String, String)>,
    Query(scope): Query<ScopeParams>,
    Json(payload): Json<PathsPayload>,
) -> Result<Json<GraphPathsResult>, ApiError> {
    ensure_any_permission!(
        user,
        &app_id,
        &state,
        RolePermissions::ReadFiles,
        RolePermissions::ReadDatabase
    );

    let (connection, overlay) =
        super::load_scoped_overlay(&state, &user, &app_id, &overlay_id, &scope).await?;
    let store = lancegraph::LanceGraphStore::new(connection, overlay, None).await?;

    let result = store
        .shortest_paths(
            (payload.from_label, payload.from_id),
            (payload.to_label, payload.to_id),
            payload.max_depth,
            payload.limit,
        )
        .await?;

    Ok(Json(result))
}
