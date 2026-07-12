use crate::{
    ensure_permission,
    error::ApiError,
    middleware::jwt::AppUser,
    permission::role_permission::RolePermissions,
    routes::app::{
        db::{ScopeParams, resolve_connection},
        wasm_catalog::app_wasm_nodes,
    },
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
    let permission = ensure_permission!(user, &app_id, &state, RolePermissions::ReadBoards);

    let mut nodes = state.registry.as_ref().get_nodes();
    if permission.has_permission(RolePermissions::ReadDatabase)
        || permission.has_permission(RolePermissions::ReadFiles)
    {
        match resolve_connection(&state, &user, &app_id, &ScopeParams { scope: None }).await {
            Ok(connection) => {
                match flow_like_storage::databases::graph::lancegraph::list_overlays(&connection)
                    .await
                {
                    Ok(ontologies) => {
                        let ontologies = ontologies
                            .into_iter()
                            .map(crate::routes::app::graph::list_overlays::def_to_overlay)
                            .collect::<Vec<_>>();
                        let bindings =
                            flow_like_catalog_core::ontology_binding_nodes(&ontologies, &nodes);
                        nodes.extend(bindings);
                    }
                    Err(error) => tracing::warn!(
                        app_id,
                        %error,
                        "Could not load Data Studio bindings; returning the base catalog"
                    ),
                }
            }
            Err(error) => tracing::warn!(
                app_id,
                %error,
                "Could not open the project database for Data Studio bindings"
            ),
        }
    }
    nodes.extend(app_wasm_nodes(&state, &app_id).await?);

    Ok(Json(nodes))
}
