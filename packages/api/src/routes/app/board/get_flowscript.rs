use crate::{
    ensure_permission,
    error::ApiError,
    middleware::jwt::AppUser,
    permission::role_permission::RolePermissions,
    routes::app::{
        board::secrets::filter_board_secrets,
        wasm_catalog::{app_wasm_nodes, hydrate_board_wasm_metadata},
    },
    state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, Query, State},
};
use flow_like::flow::ast::{RenderOptions, board_to_flowscript};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Clone, Debug, Deserialize, ToSchema)]
pub struct FlowScriptQuery {
    pub version: Option<String>,
    pub anchors: Option<bool>,
}

#[derive(Clone, Serialize, ToSchema)]
pub struct FlowScriptResponse {
    pub flowscript: String,
}

#[utoipa::path(
    get,
    path = "/apps/{app_id}/board/{board_id}/flowscript",
    tag = "boards",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        ("board_id" = String, Path, description = "Board ID"),
        ("version" = Option<String>, Query, description = "Version in MAJOR_MINOR_PATCH format (e.g., 1_0_3)"),
        ("anchors" = Option<bool>, Query, description = "Include `//@n:<id>` anchor comments for stable round-trip editing (default: true)")
    ),
    responses(
        (status = 200, description = "The board rendered as FlowScript source text", body = FlowScriptResponse),
        (status = 401, description = "Unauthorized"),
        (status = 404, description = "Board not found")
    )
)]
#[tracing::instrument(
    name = "GET /apps/{app_id}/board/{board_id}/flowscript",
    skip(state, user, params)
)]
pub async fn get_flowscript(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((app_id, board_id)): Path<(String, String)>,
    Query(params): Query<FlowScriptQuery>,
) -> Result<Json<FlowScriptResponse>, ApiError> {
    let permission = ensure_permission!(user, &app_id, &state, RolePermissions::ReadBoards);
    let sub = permission.sub()?;

    let version_opt = if let Some(ver_str) = params.version {
        // Malformed `version` is client input: map parse failures to 400 (not 500), and avoid
        // relying on a `From<ParseIntError>` impl for `ApiError`.
        let parts = ver_str
            .split('_')
            .map(str::parse::<u32>)
            .collect::<Result<Vec<u32>, _>>()
            .map_err(|e| ApiError::bad_request(format!("invalid version `{ver_str}`: {e}")))?;
        match parts.as_slice() {
            [maj, min, pat] => Some((*maj, *min, *pat)),
            _ => {
                return Err(ApiError::bad_request(
                    "version must be in MAJOR_MINOR_PATCH format",
                ));
            }
        }
    } else {
        None
    };

    let mut board = state
        .master_board(&sub, &app_id, &board_id, &state, version_opt)
        .await?;

    let builtin_nodes = state.registry.as_ref().get_nodes_shared();
    let wasm_nodes = app_wasm_nodes(&state, &app_id).await?;
    hydrate_board_wasm_metadata(&mut board, &wasm_nodes, &builtin_nodes);

    filter_board_secrets(&mut board);

    let render_options = RenderOptions {
        anchors: params.anchors.unwrap_or(true),
        ..RenderOptions::default()
    };

    Ok(Json(FlowScriptResponse {
        flowscript: board_to_flowscript(&board, &render_options),
    }))
}
