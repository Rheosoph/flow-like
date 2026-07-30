use crate::{
    ensure_any_permission, error::ApiError, middleware::jwt::AppUser,
    permission::role_permission::RolePermissions, routes::app::db::ScopeParams, state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, Query, State},
};
use flow_like_catalog_core::DEFAULT_GRAPH_QUERY_LIMIT;
use flow_like_storage::databases::graph::{GraphStore, SubgraphResult, lancegraph};
use utoipa::ToSchema;

#[derive(Debug, serde::Deserialize, ToSchema)]
pub struct SubgraphPayload {
    pub seeds: Vec<SeedEntry>,
    #[serde(default = "default_depth")]
    pub depth: usize,
    pub limit: Option<usize>,
}

#[derive(Debug, serde::Deserialize, ToSchema)]
pub struct SeedEntry {
    pub label: String,
    pub id: flow_like_types::Value,
}

fn default_depth() -> usize {
    1
}

#[utoipa::path(
    post,
    path = "/apps/{app_id}/graph/{overlay_id}/subgraph",
    tag = "graph",
    description = "Extract a subgraph from multiple seed nodes, ready for visualization.",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        ("overlay_id" = String, Path, description = "Overlay ID"),
        ("scope" = Option<String>, Query, description = "Scope: 'user' or omit for project")
    ),
    request_body = SubgraphPayload,
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
    name = "POST /apps/{app_id}/graph/{overlay_id}/subgraph",
    skip(state, user, scope, payload)
)]
pub async fn subgraph(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((app_id, overlay_id)): Path<(String, String)>,
    Query(scope): Query<ScopeParams>,
    Json(payload): Json<SubgraphPayload>,
) -> Result<Json<SubgraphResult>, ApiError> {
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

    let seeds: Vec<(String, flow_like_types::Value)> =
        payload.seeds.into_iter().map(|s| (s.label, s.id)).collect();

    let result = store
        .subgraph(
            seeds,
            payload.depth,
            Some(payload.limit.unwrap_or(DEFAULT_GRAPH_QUERY_LIMIT)),
        )
        .await?;

    Ok(Json(result))
}
