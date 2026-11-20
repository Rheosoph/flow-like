use crate::{
    ensure_any_permission, error::ApiError, middleware::jwt::AppUser,
    permission::role_permission::RolePermissions, routes::app::db::ScopeParams, state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, Query, State},
};
use flow_like_query::graph::GraphOperation;
use flow_like_storage::databases::graph::GraphAnalyticsResult;

#[derive(Debug, serde::Deserialize)]
pub struct AnalyticsParams {
    pub scope: Option<String>,
    /// Maximum number of edges sampled for the metrics computation.
    pub limit: Option<usize>,
}

#[utoipa::path(
    get,
    path = "/apps/{app_id}/graph/{overlay_id}/analytics",
    tag = "graph",
    description = "Structural analytics for a graph overlay: object counts, connected components, and the most connected and most central objects.",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        ("overlay_id" = String, Path, description = "Overlay ID"),
        ("scope" = Option<String>, Query, description = "Scope: 'user' or omit for project"),
        ("limit" = Option<usize>, Query, description = "Maximum number of edges sampled")
    ),
    responses(
        (status = 200, description = "Graph analytics", body = Object),
        (status = 400, description = "The analytics could not be computed"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "Overlay not found"),
        (status = 503, description = "The query service is unavailable"),
        (status = 504, description = "The analytics ran out of time")
    ),
    security(
        ("bearer_auth" = []),
        ("api_key" = []),
        ("pat" = [])
    )
)]
#[tracing::instrument(
    name = "GET /apps/{app_id}/graph/{overlay_id}/analytics",
    skip(state, user, params)
)]
pub async fn graph_analytics(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((app_id, overlay_id)): Path<(String, String)>,
    Query(params): Query<AnalyticsParams>,
) -> Result<Json<GraphAnalyticsResult>, ApiError> {
    let permission = ensure_any_permission!(
        user,
        &app_id,
        &state,
        RolePermissions::ReadFiles,
        RolePermissions::ReadDatabase
    );

    let scope = ScopeParams {
        scope: params.scope.clone(),
    };
    let result = super::read_overlay(
        &state,
        &user,
        &permission,
        &app_id,
        &overlay_id,
        &scope,
        |overlay| GraphOperation::Analytics {
            overlay: Box::new(overlay),
            edge_limit: params.limit,
        },
    )
    .await?;

    Ok(Json(result))
}
