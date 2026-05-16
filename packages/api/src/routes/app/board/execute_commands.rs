use std::sync::Arc;

use crate::{
    ensure_permission, error::ApiError, middleware::jwt::AppUser,
    permission::role_permission::RolePermissions, state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, State},
};
use flow_like::flow::board::commands::GenericCommand;
use serde::Deserialize;
use utoipa::ToSchema;

#[derive(Clone, Deserialize, ToSchema)]
pub struct ExecuteCommandsBody {
    #[schema(value_type = Vec<Object>)]
    pub commands: Vec<GenericCommand>,
}

#[utoipa::path(
    post,
    path = "/apps/{app_id}/board/{board_id}",
    tag = "boards",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        ("board_id" = String, Path, description = "Board ID")
    ),
    request_body = ExecuteCommandsBody,
    responses(
        (status = 200, description = "Commands executed, returns resulting commands", body = Vec<Object>),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden")
    )
)]
#[tracing::instrument(
    name = "POST /apps/{app_id}/board/{board_id}",
    skip(state, user, params)
)]
pub async fn execute_commands(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((app_id, board_id)): Path<(String, String)>,
    Json(params): Json<ExecuteCommandsBody>,
) -> Result<Json<Vec<GenericCommand>>, ApiError> {
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

    let commands = board.execute_commands(params.commands, flow_state).await?;

    board.save(None).await?;

    Ok(Json(commands))
}
