use crate::{
    ensure_any_permission, error::ApiError, middleware::jwt::AppUser,
    permission::role_permission::RolePermissions, routes::app::db::ScopeParams, state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, Query, State},
};
use flow_like_catalog_core::DEFAULT_GRAPH_QUERY_LIMIT;
use flow_like_query::graph::{GraphOperation, ObjectRef};
use flow_like_storage::databases::graph::SubgraphResult;
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
        (status = 400, description = "Bad request, or the extraction failed"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 503, description = "The query service is unavailable"),
        (status = 504, description = "The extraction ran out of time")
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
    let permission = ensure_any_permission!(
        user,
        &app_id,
        &state,
        RolePermissions::ReadFiles,
        RolePermissions::ReadDatabase
    );

    let seeds: Vec<ObjectRef> = payload
        .seeds
        .into_iter()
        .map(|seed| ObjectRef {
            label: seed.label,
            id: seed.id,
        })
        .collect();

    let result = super::read_overlay(
        &state,
        &user,
        &permission,
        &app_id,
        &overlay_id,
        &scope,
        |overlay| GraphOperation::Subgraph {
            overlay: Box::new(overlay),
            seeds,
            depth: payload.depth,
            limit: payload.limit.unwrap_or(DEFAULT_GRAPH_QUERY_LIMIT),
        },
    )
    .await?;

    Ok(Json(result))
}
