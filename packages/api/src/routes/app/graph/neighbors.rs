use crate::{
    ensure_any_permission, error::ApiError, middleware::jwt::AppUser,
    permission::role_permission::RolePermissions, routes::app::db::ScopeParams, state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, Query, State},
};
use flow_like_catalog_core::DEFAULT_GRAPH_NEIGHBORS_DIRECTION;
use flow_like_storage::databases::graph::{
    GraphStore, SubgraphResult, TraversalDirection, lancegraph,
};
use utoipa::ToSchema;

#[derive(Debug, serde::Deserialize, ToSchema)]
pub struct NeighborsPayload {
    pub label: String,
    pub node_id: flow_like_types::Value,
    #[serde(default = "default_depth")]
    pub depth: usize,
    #[serde(default = "default_direction")]
    pub direction: String,
    pub limit: Option<usize>,
}

fn default_depth() -> usize {
    1
}

fn default_direction() -> String {
    DEFAULT_GRAPH_NEIGHBORS_DIRECTION.to_string()
}

#[utoipa::path(
    post,
    path = "/apps/{app_id}/graph/{overlay_id}/neighbors",
    tag = "graph",
    description = "Find neighbors of a node by traversing edges.",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        ("overlay_id" = String, Path, description = "Overlay ID"),
        ("scope" = Option<String>, Query, description = "Scope: 'user' or omit for project")
    ),
    request_body = NeighborsPayload,
    responses(
        (status = 200, description = "Subgraph result", body = Object),
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
    name = "POST /apps/{app_id}/graph/{overlay_id}/neighbors",
    skip(state, user, scope, payload)
)]
pub async fn neighbors(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((app_id, overlay_id)): Path<(String, String)>,
    Query(scope): Query<ScopeParams>,
    Json(payload): Json<NeighborsPayload>,
) -> Result<Json<SubgraphResult>, ApiError> {
    ensure_any_permission!(
        user,
        &app_id,
        &state,
        RolePermissions::ReadFiles,
        RolePermissions::ReadDatabase
    );

    let direction = match payload.direction.to_lowercase().as_str() {
        "outgoing" | "out" => TraversalDirection::Outgoing,
        "incoming" | "in" => TraversalDirection::Incoming,
        _ => TraversalDirection::Both,
    };

    let (connection, overlay) =
        super::load_scoped_overlay(&state, &user, &app_id, &overlay_id, &scope).await?;
    let store = lancegraph::LanceGraphStore::new(connection, overlay, None).await?;

    let result = store
        .neighbors(
            &payload.label,
            payload.node_id,
            payload.depth,
            direction,
            payload.limit,
        )
        .await?;

    Ok(Json(result))
}
