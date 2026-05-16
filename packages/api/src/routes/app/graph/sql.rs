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
use flow_like_catalog_core::DEFAULT_GRAPH_QUERY_LIMIT;
use flow_like_storage::databases::graph::{GraphStore, lancegraph};
use utoipa::ToSchema;

#[derive(Debug, serde::Deserialize, ToSchema)]
pub struct SqlPayload {
    pub query: String,
    pub limit: Option<usize>,
}

#[utoipa::path(
    post,
    path = "/apps/{app_id}/graph/{overlay_id}/sql",
    tag = "graph",
    description = "Execute a SQL query against graph overlay tables via DataFusion.",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        ("overlay_id" = String, Path, description = "Overlay ID"),
        ("scope" = Option<String>, Query, description = "Scope: 'user' or omit for project")
    ),
    request_body = SqlPayload,
    responses(
        (status = 200, description = "Query results", body = Vec<flow_like_types::Value>),
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
    name = "POST /apps/{app_id}/graph/{overlay_id}/sql",
    skip(state, user, payload)
)]
pub async fn run_sql(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((app_id, overlay_id)): Path<(String, String)>,
    Query(scope): Query<ScopeParams>,
    Json(payload): Json<SqlPayload>,
) -> Result<Json<Vec<flow_like_types::Value>>, ApiError> {
    ensure_permission!(user, &app_id, &state, RolePermissions::ReadFiles);

    let connection = resolve_connection(&state, &user, &app_id, &scope).await?;
    let overlay = lancegraph::load_overlay(&connection, &overlay_id).await?;
    let store = lancegraph::LanceGraphStore::new(connection, overlay, None).await?;

    let results = store
        .sql(
            &payload.query,
            Some(payload.limit.unwrap_or(DEFAULT_GRAPH_QUERY_LIMIT)),
        )
        .await?;

    Ok(Json(results))
}
