use crate::{
    ensure_permission,
    error::ApiError,
    middleware::jwt::AppUser,
    permission::role_permission::RolePermissions,
    routes::app::{
        board::secrets::filter_board_secrets,
        db::{ScopeParams, resolve_connection},
        template::get_template::VersionQuery,
        wasm_catalog::{app_wasm_nodes, hydrate_board_wasm_metadata},
    },
    state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, Query, State},
};
use flow_like::{
    app::App,
    flow::{board::Board, node::Node},
};
use flow_like_types::anyhow;
use serde::Serialize;

#[derive(Serialize)]
pub struct WorkspaceResponse {
    pub board: Board,
    pub catalog: Vec<Node>,
    pub app: App,
}

#[utoipa::path(
    get,
    path = "/apps/{app_id}/board/{board_id}/workspace",
    tag = "boards",
    description = "Get combined board workspace: board + catalog + app. Reduces 3 HTTP calls to 1 for the board editor.",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        ("board_id" = String, Path, description = "Board ID"),
        ("version" = Option<String>, Query, description = "Board version in MAJOR_MINOR_PATCH format (e.g., 1_0_3)")
    ),
    responses(
        (status = 200, description = "Board workspace data", body = Object),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "Board not found")
    ),
    security(
        ("bearer_auth" = []),
        ("api_key" = []),
        ("pat" = [])
    )
)]
#[tracing::instrument(
    name = "GET /apps/{app_id}/board/{board_id}/workspace",
    skip(state, user, params)
)]
pub async fn workspace(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((app_id, board_id)): Path<(String, String)>,
    Query(params): Query<VersionQuery>,
) -> Result<Json<WorkspaceResponse>, ApiError> {
    let permission = ensure_permission!(user, &app_id, &state, RolePermissions::ReadBoards);
    let sub = permission.sub()?;

    // 1. Load board
    let version_opt = if let Some(ver_str) = params.version {
        let parts = ver_str
            .split('_')
            .map(str::parse::<u32>)
            .collect::<Result<Vec<u32>, _>>()?;
        match parts.as_slice() {
            [maj, min, pat] => Some((*maj, *min, *pat)),
            _ => {
                return Err(ApiError::internal_error(anyhow!(
                    "version must be in MAJOR_MINOR_PATCH format"
                )));
            }
        }
    } else {
        None
    };

    let mut board = state
        .master_board(&sub, &app_id, &board_id, &state, version_opt)
        .await?;

    let mut catalog = state.registry.as_ref().get_nodes();
    if permission.has_permission(RolePermissions::ReadDatabase)
        || permission.has_permission(RolePermissions::ReadFiles)
    {
        match resolve_connection(&state, &user, &app_id, &ScopeParams { scope: None }).await {
            Ok(connection) => {
                let (ontologies, imports) = flow_like_types::tokio::join!(
                    flow_like_storage::databases::graph::lancegraph::list_overlays(&connection),
                    flow_like_storage::databases::graph::lancegraph::list_ontology_imports(
                        &connection
                    )
                );
                match ontologies {
                    Ok(ontologies) => {
                        let ontologies = ontologies
                            .into_iter()
                            .map(crate::routes::app::graph::list_overlays::def_to_overlay)
                            .collect::<Vec<_>>();
                        let bindings =
                            flow_like_catalog_core::ontology_binding_nodes(&ontologies, &catalog);
                        catalog.extend(bindings);
                    }
                    Err(error) => tracing::warn!(
                        app_id,
                        %error,
                        "Could not load Data Studio bindings for the workspace"
                    ),
                }
                match imports {
                    Ok(imports) => {
                        let imports = imports
                            .into_iter()
                            .map(crate::routes::app::graph::list_imports::def_to_import)
                            .collect::<Result<Vec<_>, _>>();
                        match imports {
                            Ok(imports) => {
                                let bindings =
                                    flow_like_catalog_core::remote_ontology_binding_nodes(
                                        &imports, &catalog,
                                    );
                                catalog.extend(bindings);
                            }
                            Err(error) => tracing::warn!(
                                app_id,
                                %error,
                                "Could not decode remote Data Studio bindings for the workspace"
                            ),
                        }
                    }
                    Err(error) => tracing::warn!(
                        app_id,
                        %error,
                        "Could not load remote Data Studio bindings for the workspace"
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
    let wasm_nodes = app_wasm_nodes(&state, &app_id).await?;
    hydrate_board_wasm_metadata(&mut board, &wasm_nodes, &catalog);
    filter_board_secrets(&mut board);

    // 2. Load catalog (builtin + app WASM nodes)
    catalog.extend(wasm_nodes);

    // 3. Load app
    let app = state.master_app(&sub, &app_id, &state).await?;

    Ok(Json(WorkspaceResponse {
        board,
        catalog,
        app,
    }))
}
