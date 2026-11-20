use crate::{
    ensure_any_permission, error::ApiError, middleware::jwt::AppUser,
    permission::role_permission::RolePermissions, routes::app::db::ScopeParams, state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, Query, State},
};
use flow_like_catalog_core::DEFAULT_GRAPH_NEIGHBORS_DIRECTION;
use flow_like_query::graph::{GraphOperation, ObjectRef};
use flow_like_storage::databases::graph::{SubgraphResult, TraversalDirection};
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
    /// Relationship labels to follow. Omit or leave empty to follow all of them.
    #[serde(default)]
    pub edge_labels: Option<Vec<String>>,
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
        (status = 400, description = "Bad request, or the traversal failed"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 503, description = "The query service is unavailable"),
        (status = 504, description = "The traversal ran out of time")
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
    let permission = ensure_any_permission!(
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

    let result = super::read_overlay(
        &state,
        &user,
        &permission,
        &app_id,
        &overlay_id,
        &scope,
        |overlay| GraphOperation::Neighbors {
            limit: payload.limit.unwrap_or(overlay.default_limit),
            overlay: Box::new(overlay),
            object: ObjectRef {
                label: payload.label,
                id: payload.node_id,
            },
            depth: payload.depth,
            direction,
            edge_labels: payload.edge_labels,
        },
    )
    .await?;

    Ok(Json(result))
}
