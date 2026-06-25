use std::sync::Arc;

use crate::{
    ensure_permission,
    error::ApiError,
    middleware::jwt::AppUser,
    permission::role_permission::RolePermissions,
    routes::app::wasm_catalog::{app_wasm_nodes, hydrate_board_wasm_metadata},
    state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, State},
};
use flow_like::flow::ast::ApplyFlowScriptResult;
use serde::Deserialize;
use utoipa::ToSchema;

#[derive(Clone, Deserialize, ToSchema)]
pub struct ApplyFlowScriptBody {
    pub flowscript: String,
    #[serde(default)]
    pub current_layer: Option<String>,
    #[serde(default)]
    pub allow_deletions: bool,
}

#[utoipa::path(
    post,
    path = "/apps/{app_id}/board/{board_id}/flowscript/apply",
    tag = "boards",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        ("board_id" = String, Path, description = "Board ID")
    ),
    request_body = ApplyFlowScriptBody,
    responses(
        (status = 200, description = "FlowScript applied, returns resulting commands", body = Object),
        (status = 400, description = "Invalid FlowScript or generated command plan"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden")
    )
)]
#[tracing::instrument(
    name = "POST /apps/{app_id}/board/{board_id}/flowscript/apply",
    skip(state, user, params)
)]
pub async fn apply_flowscript(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((app_id, board_id)): Path<(String, String)>,
    Json(params): Json<ApplyFlowScriptBody>,
) -> Result<Json<ApplyFlowScriptResult>, ApiError> {
    let permission = ensure_permission!(user, &app_id, &state, RolePermissions::WriteBoards);
    let sub = permission.sub()?;

    let mut board = state
        .master_board(&sub, &app_id, &board_id, &state, None)
        .await?;

    let flow_state = {
        if let Some(flow_state) = &board.app_state {
            flow_state.clone()
        } else {
            let flow_state = state
                .scoped_credentials(
                    &sub,
                    &app_id,
                    crate::credentials::CredentialsAccess::EditApp,
                )
                .await?
                .to_state(state.clone())
                .await?;
            Arc::new(flow_state)
        }
    };

    let wasm_nodes = app_wasm_nodes(&state, &app_id).await?;
    let builtin_nodes = state.registry.as_ref().get_nodes();
    if hydrate_board_wasm_metadata(&mut board, &wasm_nodes, &builtin_nodes) {
        board.mark_changed();
    }

    let mut catalog_nodes = builtin_nodes;
    catalog_nodes.extend(wasm_nodes);

    let result = flow_like::flow::ast::apply_flowscript_to_board(
        &mut board,
        &params.flowscript,
        &catalog_nodes,
        flow_state,
        params.current_layer,
        params.allow_deletions,
    )
    .await
    .map_err(|error| ApiError::bad_request(error.to_string()))?;

    if !result.commands.is_empty() {
        board.save(None).await?;
    }

    Ok(Json(result))
}
