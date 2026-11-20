use crate::{
    ensure_any_permission, error::ApiError, middleware::jwt::AppUser,
    permission::role_permission::RolePermissions, routes::app::db::ScopeParams, state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, Query, State},
};
use flow_like_query::graph::{GraphOperation, ObjectRef};
use flow_like_storage::databases::graph::SubgraphResult;
use utoipa::ToSchema;

#[derive(Debug, serde::Deserialize, ToSchema)]
pub struct ChildrenPayload {
    pub label: String,
    pub node_id: flow_like_types::Value,
    pub limit: Option<usize>,
}

#[utoipa::path(
    post,
    path = "/apps/{app_id}/graph/{overlay_id}/children",
    tag = "graph",
    description = "Expand a parent object's containment children (one hop), following only edges flagged as hierarchy edges.",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        ("overlay_id" = String, Path, description = "Overlay ID"),
        ("scope" = Option<String>, Query, description = "Scope: 'user' or omit for project")
    ),
    request_body = ChildrenPayload,
    responses(
        (status = 200, description = "Subgraph result", body = Object),
        (status = 400, description = "Bad request, or the expansion failed"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 503, description = "The query service is unavailable"),
        (status = 504, description = "The expansion ran out of time")
    ),
    security(
        ("bearer_auth" = []),
        ("api_key" = []),
        ("pat" = [])
    )
)]
#[tracing::instrument(
    name = "POST /apps/{app_id}/graph/{overlay_id}/children",
    skip(state, user, scope, payload)
)]
pub async fn children(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((app_id, overlay_id)): Path<(String, String)>,
    Query(scope): Query<ScopeParams>,
    Json(payload): Json<ChildrenPayload>,
) -> Result<Json<SubgraphResult>, ApiError> {
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
        |overlay| GraphOperation::Children {
            limit: payload.limit.unwrap_or(overlay.default_limit),
            overlay: Box::new(overlay),
            object: ObjectRef {
                label: payload.label,
                id: payload.node_id,
            },
        },
    )
    .await?;

    Ok(Json(result))
}
