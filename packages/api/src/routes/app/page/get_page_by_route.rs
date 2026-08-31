use crate::{
    ensure_permission, error::ApiError, middleware::jwt::AppUser,
    permission::role_permission::RolePermissions, state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, Query, State},
};
use flow_like::a2ui::widget::Page;
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

#[derive(Deserialize, Debug, IntoParams, ToSchema)]
pub struct RouteQuery {
    pub route: String,
}

#[derive(Serialize, ToSchema)]
pub struct PageWithBoardId {
    #[schema(value_type = Object)]
    pub page: Page,
    pub board_id: Option<String>,
}

#[utoipa::path(
    get,
    path = "/apps/{app_id}/pages/by-route",
    tag = "pages",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        RouteQuery
    ),
    responses(
        (status = 200, description = "Page matching the route", body = Option<PageWithBoardId>),
        (status = 401, description = "Unauthorized")
    )
)]
#[tracing::instrument(name = "GET /apps/{app_id}/pages/by-route", skip(state, user, params))]
pub async fn get_page_by_route(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(app_id): Path<String>,
    Query(params): Query<RouteQuery>,
) -> Result<Json<Option<PageWithBoardId>>, ApiError> {
    ensure_permission!(user, &app_id, &state, RolePermissions::ReadBoards);

    let app = state.master_app(&user.sub()?, &app_id, &state).await?;

    for board_id in app.boards.iter() {
        if let Ok(board) = app.open_board(board_id.to_string(), None, None).await {
            let board_guard = board.lock().await;
            if let Ok(loaded) = board_guard.load_all_pages(None).await {
                for unreadable in &loaded.unreadable {
                    tracing::warn!(
                        "Board {} lists page {} but its payload is unreadable: {}",
                        board_id,
                        unreadable.page_id,
                        unreadable.reason
                    );
                }
                for page in loaded.pages {
                    if page.route == params.route {
                        return Ok(Json(Some(PageWithBoardId {
                            page,
                            board_id: Some(board_id.clone()),
                        })));
                    }
                }
            }
        }
    }

    Ok(Json(None))
}
