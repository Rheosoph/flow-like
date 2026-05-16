use crate::{
    ensure_permission,
    error::ApiError,
    middleware::jwt::AppUser,
    permission::role_permission::RolePermissions,
    routes::app::db::{ScopeParams, resolve_connection},
    state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, Query, State},
};
use flow_like_storage::databases::graph::{GraphStore, SubgraphNode, lancegraph};
use utoipa::ToSchema;

#[derive(Debug, serde::Deserialize, ToSchema)]
pub struct SearchNodesPayload {
    pub query: String,
    pub limit: Option<usize>,
}

#[utoipa::path(
    post,
    path = "/apps/{app_id}/graph/{overlay_id}/search",
    tag = "graph",
    description = "Search graph nodes by caption or identifier, including nodes not currently loaded in the visualization.",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        ("overlay_id" = String, Path, description = "Overlay ID"),
        ("scope" = Option<String>, Query, description = "Scope: 'user' or omit for project")
    ),
    request_body = SearchNodesPayload,
    responses(
        (status = 200, description = "Matching graph nodes", body = Vec<Object>),
        (status = 400, description = "Bad request"),
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
    name = "POST /apps/{app_id}/graph/{overlay_id}/search",
    skip(state, user, payload)
)]
pub async fn search_nodes(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((app_id, overlay_id)): Path<(String, String)>,
    Query(scope): Query<ScopeParams>,
    Json(payload): Json<SearchNodesPayload>,
) -> Result<Json<Vec<SubgraphNode>>, ApiError> {
    ensure_permission!(user, &app_id, &state, RolePermissions::ReadFiles);

    let connection = resolve_connection(&state, &user, &app_id, &scope).await?;
    let overlay = lancegraph::load_overlay(&connection, &overlay_id).await?;
    let store = lancegraph::LanceGraphStore::new(connection, overlay, None).await?;
    let results = store.search_nodes(&payload.query, payload.limit).await?;

    Ok(Json(results))
}
