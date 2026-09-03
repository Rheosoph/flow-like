use crate::{
    audit_branch, ensure_permission, entity::page, error::ApiError, middleware::jwt::AppUser,
    permission::role_permission::RolePermissions, state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, Query, State},
};
use flow_like_types::anyhow;
use sea_orm::sea_query::ExprTrait;
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
use serde::Deserialize;
use utoipa::IntoParams;

#[derive(Debug, Deserialize, IntoParams)]
pub struct PageBoardQuery {
    /// Exact owning board. A mismatch is rejected instead of deleting a same-id page elsewhere.
    pub board_id: Option<String>,
}

#[utoipa::path(
    delete,
    path = "/apps/{app_id}/pages/{page_id}",
    tag = "pages",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        ("page_id" = String, Path, description = "Page ID"),
        PageBoardQuery
    ),
    responses(
        (status = 200, description = "Page deleted"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden")
    )
)]
#[tracing::instrument(name = "DELETE /apps/{app_id}/pages/{page_id}", skip(state, user))]
pub async fn delete_page(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((app_id, page_id)): Path<(String, String)>,
    Query(params): Query<PageBoardQuery>,
) -> Result<Json<()>, ApiError> {
    let permission = ensure_permission!(user, &app_id, &state, RolePermissions::WriteBoards);
    let sub = permission.sub()?;

    // Resolve credentials and storage before taking mutation locks so lock holders never wait for
    // another pooled database connection during app setup.
    let app = state
        .scoped_app(
            &sub,
            &app_id,
            &state,
            crate::credentials::CredentialsAccess::EditApp,
        )
        .await?;

    // Match upsert's lock order: global page id first, then the owning board discovered below.
    // The guard stays live through storage cleanup and DB deletion so a concurrent upsert cannot
    // recreate or move the id between those two operations.
    let mut page_id_guard = super::page_id_mutation_guard(&state, &page_id).await?;

    // Delete the storage object via the owning board so legacy
    // app-level copies are evicted alongside the canonical board-scoped
    // file (`Board::delete_page` removes both). If we can't determine
    // the board (e.g. orphaned row, board missing) the DB delete still
    // proceeds — leaving a stale blob is preferable to refusing to
    // remove the row.
    let row = page::Entity::find_by_id(&page_id)
        .filter(page::Column::AppId.eq(&app_id))
        .one(page_id_guard.connection())
        .await?;
    let board_id = row.and_then(|row| row.board_id);
    if let Some(requested_board_id) = params.board_id.filter(|id| !id.trim().is_empty())
        && board_id.as_deref() != Some(requested_board_id.as_str())
    {
        return Err(ApiError::NOT_FOUND);
    }
    if let Some(board_id) = board_id.as_deref() {
        page_id_guard
            .acquire_additional_board(&state, &app_id, board_id)
            .await?;
    }

    if let Some(board_id) = board_id {
        if let Ok(board) = app.open_board(board_id.clone(), None, None).await {
            let mut board_guard = board.lock().await;
            if let Err(e) = board_guard.delete_page(&page_id, None).await {
                tracing::warn!(
                    "delete_page storage cleanup failed for board {}: {e}",
                    board_id
                );
            }
            if let Err(e) = board_guard.save(None).await {
                tracing::warn!("delete_page board save failed for {}: {e}", board_id);
            }
        } else {
            tracing::warn!(
                "delete_page could not open board {} for page {} — DB row will still be removed",
                board_id,
                page_id
            );
        }
    } else {
        tracing::warn!(
            "delete_page found no board_id for page {} — DB row will still be removed",
            page_id
        );
    }

    page::Entity::delete_many()
        .filter(
            page::Column::AppId
                .eq(app_id.clone())
                .and(page::Column::Id.eq(page_id.clone())),
        )
        .exec(page_id_guard.connection())
        .await
        .map_err(|e| ApiError::internal_error(anyhow!("delete page row: {e}")))?;

    page_id_guard.release().await?;

    audit_branch!(
        state,
        user,
        app_id,
        "page.delete",
        "Page",
        page_id,
        "Page deleted"
    );
    Ok(Json(()))
}
