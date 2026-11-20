use crate::{
    ensure_any_permission, error::ApiError, middleware::jwt::AppUser,
    permission::role_permission::RolePermissions, routes::app::db::ScopeParams, state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, Query, State},
};
use flow_like_query::graph::{GraphOperation, ObjectRef};
use flow_like_storage::databases::graph::GraphPathsResult;
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
        (status = 400, description = "Bad request, or the path search failed"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "Overlay not found"),
        (status = 503, description = "The query service is unavailable"),
        (status = 504, description = "The path search ran out of time")
    ),
    security(
        ("bearer_auth" = []),
        ("api_key" = []),
        ("pat" = [])
    )
)]
#[tracing::instrument(
    name = "POST /apps/{app_id}/graph/{overlay_id}/paths",
    skip(state, user, scope, payload)
)]
pub async fn find_paths(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((app_id, overlay_id)): Path<(String, String)>,
    Query(scope): Query<ScopeParams>,
    Json(payload): Json<PathsPayload>,
) -> Result<Json<GraphPathsResult>, ApiError> {
    let permission = ensure_any_permission!(
        user,
        &app_id,
        &state,
        RolePermissions::ReadFiles,
        RolePermissions::ReadDatabase
    );

    let result = super::read_overlay(
        &state,
        &user,
        &permission,
        &app_id,
        &overlay_id,
        &scope,
        |overlay| GraphOperation::Paths {
            limit: payload.limit.unwrap_or(overlay.default_limit),
            overlay: Box::new(overlay),
            from: ObjectRef {
                label: payload.from_label,
                id: payload.from_id,
            },
            to: ObjectRef {
                label: payload.to_label,
                id: payload.to_id,
            },
            max_depth: payload.max_depth,
        },
    )
    .await?;

    Ok(Json(result))
}
