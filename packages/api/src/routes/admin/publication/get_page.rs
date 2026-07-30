use crate::{
    entity::page, error::ApiError, middleware::jwt::AppUser,
    permission::global_permission::GlobalPermission, state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, Query, State},
};
use flow_like::a2ui::widget::Page;
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PageQuery {
    pub board_id: Option<String>,
}

#[utoipa::path(
    get,
    path = "/admin/publication/apps/{app_id}/page/{page_id}",
    tag = "admin",
    description = "Get full page data for admin review. Page is returned read-only.",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        ("page_id" = String, Path, description = "Page ID"),
        ("boardId" = Option<String>, Query, description = "Optional owning board ID")
    ),
    responses(
        (status = 200, description = "Full page data", body = Object),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "Page not found")
    )
)]
#[tracing::instrument(
    name = "GET /admin/publication/apps/{app_id}/page/{page_id}",
    skip(state, user, query)
)]
pub async fn get_page(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((app_id, page_id)): Path<(String, String)>,
    Query(query): Query<PageQuery>,
) -> Result<Json<Page>, ApiError> {
    user.check_global_permission(&state, GlobalPermission::ReadPublishing)
        .await?;

    let row = page::Entity::find_by_id(&page_id)
        .filter(page::Column::AppId.eq(&app_id))
        .one(&state.db)
        .await?
        .ok_or(ApiError::NOT_FOUND)?;

    let board_hint = query
        .board_id
        .filter(|board_id| !board_id.is_empty())
        .or(row.board_id);

    let app = state.master_app("admin", &app_id, &state).await?;

    let try_board = |board_id: String| {
        let app = &app;
        let page_id = &page_id;
        async move {
            let board = app.open_board(board_id, Some(false), None).await.ok()?;
            let board = board.lock().await;
            board.load_page(page_id, None).await.ok()
        }
    };

    if let Some(board_id) = board_hint {
        if let Some(page) = try_board(board_id).await {
            return Ok(Json(page));
        }
    }

    for board_id in app.boards.iter() {
        if let Some(page) = try_board(board_id.clone()).await {
            return Ok(Json(page));
        }
    }

    Err(ApiError::NOT_FOUND)
}
