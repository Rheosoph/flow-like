use crate::{
    audit_branch, ensure_permission, error::ApiError, middleware::jwt::AppUser,
    permission::role_permission::RolePermissions, state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, State},
};

#[utoipa::path(
    delete,
    path = "/apps/{app_id}/board/{board_id}",
    tag = "boards",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        ("board_id" = String, Path, description = "Board ID")
    ),
    responses(
        (status = 200, description = "Board deleted"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "Board not found")
    )
)]
#[tracing::instrument(name = "DELETE /apps/{app_id}/board/{board_id}", skip(state, user))]
pub async fn delete_board(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((app_id, board_id)): Path<(String, String)>,
) -> Result<Json<()>, ApiError> {
    let permission = ensure_permission!(user, &app_id, &state, RolePermissions::WriteBoards);
    let sub = permission.sub()?;
    let _mutation_guard = state.board_mutation_guard(&app_id, &board_id).await?;

    let mut app = state
        .scoped_app(
            &sub,
            &app_id,
            &state,
            crate::credentials::CredentialsAccess::EditApp,
        )
        .await?;
    app.delete_board(&board_id).await?;
    app.save().await?;

    if let Err(err) =
        crate::routes::app::board::scoring::delete_board_score(&state.db, &app_id, &board_id).await
    {
        tracing::warn!("failed to delete board score for {app_id}/{board_id}: {err:?}");
    }

    audit_branch!(
        state,
        user,
        app_id,
        "board.delete",
        "Board",
        board_id,
        "Board deleted"
    );
    Ok(Json(()))
}
