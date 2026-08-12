use crate::{
    ensure_permission,
    error::ApiError,
    middleware::jwt::AppUser,
    permission::role_permission::RolePermissions,
    routes::app::wasm_catalog::{
        app_wasm_nodes, hydrate_board_wasm_metadata, sanitize_wasm_command_metadata,
    },
    state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, State},
};
use flow_like::flow::board::commands::GenericCommand;
use flow_like_types::anyhow;
use serde::Deserialize;
use utoipa::ToSchema;

#[derive(Clone, Deserialize, ToSchema)]
pub struct ExecuteCommandsBody {
    #[schema(value_type = Vec<Object>)]
    pub commands: Vec<GenericCommand>,
}

#[utoipa::path(
    patch,
    path = "/apps/{app_id}/board/{board_id}/undo",
    tag = "boards",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        ("board_id" = String, Path, description = "Board ID")
    ),
    request_body = ExecuteCommandsBody,
    responses(
        (status = 200, description = "Undo operation completed"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden")
    )
)]
#[tracing::instrument(
    name = "PATCH /apps/{app_id}/board/{board_id}/undo",
    skip(state, user, params)
)]
pub async fn undo_board(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((app_id, board_id)): Path<(String, String)>,
    Json(mut params): Json<ExecuteCommandsBody>,
) -> Result<Json<()>, ApiError> {
    let permission = ensure_permission!(user, &app_id, &state, RolePermissions::WriteBoards);
    let sub = permission.sub()?;
    let _mutation_guard = state.board_mutation_guard(&app_id, &board_id).await?;

    let mut board = state
        .master_board(&sub, &app_id, &board_id, &state, None)
        .await?;

    let flow_state = board
        .app_state
        .clone()
        .ok_or(ApiError::internal_error(anyhow!(
            "No app state found for board"
        )))?
        .clone();
    let wasm_nodes = app_wasm_nodes(&state, &app_id).await?;
    let builtin_nodes = state.registry.as_ref().get_nodes();
    if hydrate_board_wasm_metadata(&mut board, &wasm_nodes, &builtin_nodes) {
        board.mark_changed();
    }
    sanitize_wasm_command_metadata(&mut params.commands, &wasm_nodes, &builtin_nodes)?;

    board.undo(params.commands, flow_state.clone()).await?;
    board.save(None).await?;

    Ok(Json(()))
}

#[utoipa::path(
    patch,
    path = "/apps/{app_id}/board/{board_id}/redo",
    tag = "boards",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        ("board_id" = String, Path, description = "Board ID")
    ),
    request_body = ExecuteCommandsBody,
    responses(
        (status = 200, description = "Redo operation completed"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden")
    )
)]
#[tracing::instrument(
    name = "PATCH /apps/{app_id}/board/{board_id}/redo",
    skip(state, user, params)
)]
pub async fn redo_board(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((app_id, board_id)): Path<(String, String)>,
    Json(mut params): Json<ExecuteCommandsBody>,
) -> Result<Json<()>, ApiError> {
    let permission = ensure_permission!(user, &app_id, &state, RolePermissions::WriteBoards);
    let sub = permission.sub()?;
    let _mutation_guard = state.board_mutation_guard(&app_id, &board_id).await?;

    let mut board = state
        .master_board(&sub, &app_id, &board_id, &state, None)
        .await?;

    let flow_state = board
        .app_state
        .clone()
        .ok_or(ApiError::internal_error(anyhow!(
            "No app state found for board"
        )))?
        .clone();
    let wasm_nodes = app_wasm_nodes(&state, &app_id).await?;
    let builtin_nodes = state.registry.as_ref().get_nodes();
    if hydrate_board_wasm_metadata(&mut board, &wasm_nodes, &builtin_nodes) {
        board.mark_changed();
    }
    sanitize_wasm_command_metadata(&mut params.commands, &wasm_nodes, &builtin_nodes)?;

    board.redo(params.commands, flow_state.clone()).await?;
    board.save(None).await?;

    Ok(Json(()))
}
