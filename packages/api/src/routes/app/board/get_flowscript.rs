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
use flow_like::flow::ast::{
    FlowScriptFile, RenderOptions, board_to_flowscript, board_to_flowscript_file,
    board_to_flowscript_scoped,
};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Clone, Debug, Deserialize, ToSchema)]
pub struct FlowScriptQuery {
    pub version: Option<String>,
    pub anchors: Option<bool>,
    /// Comma-separated node ids to scope the render to. When set, only the events, functions and
    /// detached chains containing those nodes (plus the functions they reference and all
    /// variables/interfaces) are rendered, and the response carries `scope_anchors` for the
    /// matching scoped apply. Mutually exclusive with `file`.
    pub node_ids: Option<String>,
    /// Render exactly one virtual FlowScript file of the board: `"main"` (the root — globals,
    /// interfaces and every root-level event/function) or a module layer id (that module's own
    /// sections, unwrapped). The response carries `scope_anchors` for the matching scoped apply
    /// (pass the same id as `module` on `flowscript/apply`). Mutually exclusive with `node_ids`.
    pub file: Option<String>,
}

#[derive(Clone, Serialize, ToSchema)]
pub struct FlowScriptResponse {
    pub flowscript: String,
    /// Present only for a scoped render (`node_ids` set): the anchors (event entry node id /
    /// function layer id) of the rendered sections. Pass them back as `scope_anchors` on
    /// `flowscript/apply` so the unrendered rest of the board is never treated as deleted.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scope_anchors: Option<Vec<String>>,
}

#[utoipa::path(
    get,
    path = "/apps/{app_id}/board/{board_id}/flowscript",
    tag = "boards",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        ("board_id" = String, Path, description = "Board ID"),
        ("version" = Option<String>, Query, description = "Version in MAJOR_MINOR_PATCH format (e.g., 1_0_3)"),
        ("anchors" = Option<bool>, Query, description = "Include `//@n:<id>` anchor comments for stable round-trip editing (default: true)"),
        ("node_ids" = Option<String>, Query, description = "Comma-separated node ids: render only the events, functions and detached chains containing them (selection-scoped editing) and return their `scope_anchors`. Mutually exclusive with `file`"),
        ("file" = Option<String>, Query, description = "Render exactly one virtual FlowScript file: `main` (the board root) or a module layer id (that module's own sections). Returns `scope_anchors` for the matching scoped apply. Mutually exclusive with `node_ids`")
    ),
    responses(
        (status = 200, description = "The board rendered as FlowScript source text", body = FlowScriptResponse),
        (status = 400, description = "Both `file` and `node_ids` were set, or `file` names an unknown/non-module layer"),
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

    if params.file.is_some() && params.node_ids.is_some() {
        return Err(ApiError::bad_request(
            "`file` and `node_ids` are mutually exclusive: request a whole virtual file or a node selection, not both",
        ));
    }

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

    let node_ids: Vec<String> = params
        .node_ids
        .as_deref()
        .map(|raw| {
            raw.split(',')
                .map(str::trim)
                .filter(|id| !id.is_empty())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    if params.node_ids.is_some() {
        let scoped = board_to_flowscript_scoped(&board, &node_ids, &render_options);
        return Ok(Json(FlowScriptResponse {
            flowscript: scoped.text,
            scope_anchors: Some(scoped.scope_anchors),
        }));
    }

    if let Some(file) = params.file.as_deref() {
        let file = if file == "main" {
            FlowScriptFile::Main
        } else {
            FlowScriptFile::Module(file.to_string())
        };
        let scoped = board_to_flowscript_file(&board, &file, &render_options)
            .map_err(|error| ApiError::bad_request(error.to_string()))?;
        return Ok(Json(FlowScriptResponse {
            flowscript: scoped.text,
            scope_anchors: Some(scoped.scope_anchors),
        }));
    }

    Ok(Json(FlowScriptResponse {
        flowscript: board_to_flowscript(&board, &render_options),
        scope_anchors: None,
    }))
}
