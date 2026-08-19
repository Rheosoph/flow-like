use crate::{
    ensure_permission, error::ApiError, middleware::jwt::AppUser,
    permission::role_permission::RolePermissions, state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, State},
};
use flow_like::flow::variable::Variable;
use serde::Serialize;
use std::collections::HashMap;
use utoipa::ToSchema;

#[derive(Serialize, ToSchema)]
pub struct BoardVariables {
    pub board_id: String,
    pub board_name: String,
    /// The board's own variables. Secret values are never included.
    #[schema(value_type = HashMap<String, Object>)]
    pub variables: HashMap<String, Variable>,
    /// Only the schema refs the variables above reach, so struct variables resolve client-side.
    pub refs: HashMap<String, String>,
}

#[utoipa::path(
    get,
    path = "/apps/{app_id}/board/variables",
    tag = "boards",
    description = "List the variables of every board in the app without transferring the boards themselves. Secret values are stripped.",
    params(
        ("app_id" = String, Path, description = "Application ID")
    ),
    responses(
        (status = 200, description = "Per-board variable maps", body = Vec<BoardVariables>),
        (status = 401, description = "Unauthorized")
    )
)]
#[tracing::instrument(name = "GET /apps/{app_id}/board/variables", skip(state, user))]
pub async fn get_board_variables(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(app_id): Path<String>,
) -> Result<Json<Vec<BoardVariables>>, ApiError> {
    let permission = ensure_permission!(user, &app_id, &state, RolePermissions::ReadBoards);
    let sub = permission.sub()?;

    let app = state.master_app(&sub, &app_id, &state).await?;
    let mut result = Vec::with_capacity(app.boards.len());
    for board_id in app.boards.iter() {
        // Cached per revision; the second call for an unchanged board costs one conditional GET.
        let Ok(cached) = state
            .master_board_shared(&app_id, board_id, &state, None)
            .await
        else {
            continue;
        };
        let (variables, refs) = cached.board.public_variables();
        result.push(BoardVariables {
            board_id: cached.board.id.clone(),
            board_name: cached.board.name.clone(),
            variables,
            refs,
        });
    }

    Ok(Json(result))
}
