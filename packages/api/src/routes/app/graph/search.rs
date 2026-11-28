use crate::{
    ensure_any_permission, error::ApiError, middleware::jwt::AppUser,
    permission::role_permission::RolePermissions, routes::app::db::ScopeParams, state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, Query, State},
};
use flow_like_query::graph::GraphOperation;
use flow_like_storage::databases::graph::SubgraphNode;
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
        (status = 400, description = "Bad request, or the search failed"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 503, description = "The query service is unavailable"),
        (status = 504, description = "The search ran out of time")
    ),
    security(
        ("bearer_auth" = []),
        ("api_key" = []),
        ("pat" = [])
    )
)]
#[tracing::instrument(
    name = "POST /apps/{app_id}/graph/{overlay_id}/search",
    skip(state, user, scope, payload)
)]
pub async fn search_nodes(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((app_id, overlay_id)): Path<(String, String)>,
    Query(scope): Query<ScopeParams>,
    Json(payload): Json<SearchNodesPayload>,
) -> Result<Json<Vec<SubgraphNode>>, ApiError> {
    let permission = ensure_any_permission!(
        user,
        &app_id,
        &state,
        RolePermissions::ReadFiles,
        RolePermissions::ReadDatabase
    );

    let results = super::read_overlay(
        &state,
        &user,
        &permission,
        &app_id,
        &overlay_id,
        &scope,
        |overlay| GraphOperation::Search {
            limit: payload.limit.unwrap_or(overlay.default_limit),
            overlay: Box::new(overlay),
            query: payload.query,
        },
    )
    .await?;

    Ok(Json(results))
}
