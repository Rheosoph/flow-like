use crate::{
    ensure_permission, error::ApiError, middleware::jwt::AppUser,
    permission::role_permission::RolePermissions,
    routes::app::board::secrets::filter_board_secrets, state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, State},
};
use flow_like::flow::board::Board;
use futures::{StreamExt, stream};

#[utoipa::path(
    get,
    path = "/apps/{app_id}/board",
    tag = "boards",
    params(
        ("app_id" = String, Path, description = "Application ID")
    ),
    responses(
        (status = 200, description = "List of boards in the application", body = Vec<Object>),
        (status = 401, description = "Unauthorized")
    )
)]
#[tracing::instrument(name = "GET /apps/{app_id}/board", skip(state, user))]
pub async fn get_boards(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(app_id): Path<String>,
) -> Result<Json<Vec<Board>>, ApiError> {
    let permission = ensure_permission!(user, &app_id, &state, RolePermissions::ReadBoards);
    let sub = permission.sub()?;

    let app = state.master_app(&sub, &app_id, &state).await?;

    // `buffered` keeps the app's board order while unchanged boards resolve as conditional GETs.
    const BOARD_LOAD_CONCURRENCY: usize = 4;
    let loaded: Vec<_> = stream::iter(app.boards.clone())
        .map(|board_id| {
            let state = state.clone();
            let app_id = app_id.clone();
            async move {
                state
                    .master_board_shared(&app_id, &board_id, &state, None)
                    .await
            }
        })
        .buffered(BOARD_LOAD_CONCURRENCY)
        .collect()
        .await;

    let mut boards = Vec::with_capacity(loaded.len());
    for cached in loaded {
        let cached = match cached {
            Ok(cached) => cached,
            Err(error) => {
                if let Some(error) = ApiError::from_board_format_error(&error) {
                    return Err(error);
                }
                continue;
            }
        };
        let mut board = (*cached.board).clone();
        filter_board_secrets(&mut board);
        boards.push(board);
    }

    Ok(Json(boards))
}
