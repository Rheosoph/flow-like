use crate::{
    ensure_permission,
    entity::app_package,
    entity::wasm_package_version,
    error::ApiError,
    middleware::jwt::AppUser,
    permission::role_permission::RolePermissions,
    routes::app::{board::secrets::filter_board_secrets, template::get_template::VersionQuery},
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
use flow_like_wasm::manifest::PackageNodeEntry;
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
use serde::Serialize;

use super::super::internal::get_nodes::package_node_to_node;

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
    skip(state, user)
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

    filter_board_secrets(&mut board);

    // 2. Load catalog (builtin + app WASM nodes)
    let packages = app_package::Entity::find()
        .filter(app_package::Column::AppId.eq(&app_id))
        .filter(app_package::Column::Stale.eq(false))
        .all(&state.db)
        .await?;

    let mut wasm_nodes: Vec<Node> = Vec::with_capacity(packages.len() * 5);

    for pkg in &packages {
        if pkg.stale {
            continue;
        }

        let version_record = wasm_package_version::Entity::find()
            .filter(wasm_package_version::Column::PackageId.eq(&pkg.package_id))
            .filter(wasm_package_version::Column::Version.eq(&pkg.version))
            .one(&state.db)
            .await?;

        let version_record = match version_record {
            Some(v) => v,
            None => continue,
        };

        let entries: Vec<PackageNodeEntry> =
            serde_json::from_value(version_record.nodes).unwrap_or_default();

        for entry in &entries {
            wasm_nodes.push(package_node_to_node(entry, &pkg.package_id));
        }
    }

    let mut catalog = state.registry.as_ref().get_nodes();
    catalog.extend(wasm_nodes);

    // 3. Load app
    let app = state.master_app(&sub, &app_id, &state).await?;

    Ok(Json(WorkspaceResponse {
        board,
        catalog,
        app,
    }))
}
