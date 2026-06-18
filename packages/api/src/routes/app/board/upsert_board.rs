use crate::{
    audit_branch, ensure_permission, error::ApiError, middleware::jwt::AppUser,
    permission::role_permission::RolePermissions, state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, State},
};
use flow_like::flow::{
    board::{Board, ExecutionMode, ExecutionStage},
    execution::LogLevel,
};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Clone, Deserialize, ToSchema)]
pub struct UpsertBoard {
    pub name: Option<String>,
    pub description: Option<String>,
    #[schema(value_type = Option<String>)]
    pub stage: Option<ExecutionStage>,
    #[schema(value_type = Option<i32>)]
    pub log_level: Option<LogLevel>,
    #[schema(value_type = Option<String>)]
    pub execution_mode: Option<ExecutionMode>,
    #[schema(value_type = Option<Object>)]
    pub template: Option<Board>,
}

#[derive(Deserialize, Serialize, ToSchema)]
pub struct UpsertBoardResponse {
    pub id: String,
}

#[utoipa::path(
    put,
    path = "/apps/{app_id}/board/{board_id}",
    tag = "boards",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        ("board_id" = String, Path, description = "Board ID")
    ),
    request_body = UpsertBoard,
    responses(
        (status = 200, description = "Board created or updated", body = UpsertBoardResponse),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden")
    )
)]
#[tracing::instrument(
    name = "PUT /apps/{app_id}/board/{board_id}",
    skip(state, user, params)
)]
pub async fn upsert_board(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((app_id, board_id)): Path<(String, String)>,
    Json(params): Json<UpsertBoard>,
) -> Result<Json<UpsertBoardResponse>, ApiError> {
    let permission = ensure_permission!(user, &app_id, &state, RolePermissions::WriteBoards);
    let sub = permission.sub()?;

    let mut app = state.master_app(&sub, &app_id, &state).await?;
    let mut id = board_id.clone();
    let created_board = !app.boards.contains(&board_id);
    let template = params.template.clone();
    if created_board {
        id = app.create_board(Some(board_id.clone()), None).await?;
        app.save().await?;
    }

    let board = app.open_board(id, Some(false), None).await?;
    let mut board = board.lock().await;

    if created_board {
        if let Some(data) = template {
            board.variables = data.variables;
            board.comments = data.comments;
            board.nodes = data.nodes;
            board.layers = data.layers;
            board.parent = data.parent;
            board.refs = data.refs;
            board.version = data.version;
            board.viewport = data.viewport;
            board.page_ids = data.page_ids;
            board.created_at = data.created_at;
            board.updated_at = data.updated_at;
        }
    }

    board.name = params.name.unwrap_or(board.name.clone());
    board.description = params.description.unwrap_or(board.description.clone());
    board.stage = params.stage.unwrap_or(board.stage.clone());
    board.log_level = params.log_level.unwrap_or(board.log_level);
    board.execution_mode = params
        .execution_mode
        .unwrap_or(board.execution_mode.clone());
    board.mark_changed();
    board.save(None).await?;

    if let Err(err) =
        crate::routes::app::board::scoring::persist_board_score(&state.db, &app_id, &board).await
    {
        tracing::warn!(
            "failed to persist board score for {app_id}/{}: {err:?}",
            board.id
        );
    }

    audit_branch!(
        state,
        user,
        app_id,
        "board.update",
        "Board",
        board.id,
        "Board created or updated"
    );
    Ok(Json(UpsertBoardResponse {
        id: board.id.clone(),
    }))
}
