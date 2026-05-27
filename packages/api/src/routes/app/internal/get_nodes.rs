use crate::{
    ensure_permission, error::ApiError, middleware::jwt::AppUser,
    permission::role_permission::RolePermissions, routes::app::wasm_catalog::app_wasm_nodes,
    state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, State},
};
use flow_like::flow::node::Node;

#[utoipa::path(
    get,
    path = "/apps/nodes",
    tag = "apps",
    responses(
        (status = 200, description = "List of available nodes", body = Vec<Object>),
        (status = 401, description = "Unauthorized")
    )
)]
#[tracing::instrument(name = "GET /apps/nodes", skip(state, user))]
pub async fn get_nodes(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
) -> Result<Json<Vec<Node>>, ApiError> {
    user.sub()?;

    let nodes = state.registry.as_ref();
    let nodes = nodes.get_nodes();

    Ok(Json(nodes))
}

#[utoipa::path(
    get,
    path = "/apps/{app_id}/nodes",
    tag = "apps",
    params(
        ("app_id" = String, Path, description = "Application ID")
    ),
    responses(
        (status = 200, description = "List of available nodes for this app", body = Vec<Object>),
        (status = 401, description = "Unauthorized")
    )
)]
#[tracing::instrument(name = "GET /apps/{app_id}/nodes", skip(state, user))]
pub async fn get_app_nodes(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(app_id): Path<String>,
) -> Result<Json<Vec<Node>>, ApiError> {
    ensure_permission!(user, &app_id, &state, RolePermissions::ReadBoards);

    let mut nodes = state.registry.as_ref().get_nodes();
    nodes.extend(app_wasm_nodes(&state, &app_id).await?);

    Ok(Json(nodes))
}
