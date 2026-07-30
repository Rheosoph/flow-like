use crate::{
    ensure_permission, entity::page, error::ApiError, middleware::jwt::AppUser,
    permission::role_permission::RolePermissions, state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, Query, State},
};
use flow_like::a2ui::widget::Page;
use flow_like_types::anyhow;
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
use serde::Deserialize;
use utoipa::{IntoParams, ToSchema};

#[derive(Deserialize, Debug, IntoParams, ToSchema)]
pub struct VersionQuery {
    /// expected format: "MAJOR_MINOR_PATCH", e.g. "1_0_3"
    pub version: Option<String>,
    /// Exact owning board. When supplied, lookup never falls through to another board.
    pub board_id: Option<String>,
}

#[utoipa::path(
    get,
    path = "/apps/{app_id}/pages/{page_id}",
    tag = "pages",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        ("page_id" = String, Path, description = "Page ID"),
        VersionQuery
    ),
    responses(
        (status = 200, description = "Page details", body = Object),
        (status = 401, description = "Unauthorized"),
        (status = 404, description = "Page not found")
    )
)]
#[tracing::instrument(name = "GET /apps/{app_id}/pages/{page_id}", skip(state, user, params))]
pub async fn get_page(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((app_id, page_id)): Path<(String, String)>,
    Query(params): Query<VersionQuery>,
) -> Result<Json<Page>, ApiError> {
    ensure_permission!(user, &app_id, &state, RolePermissions::ExecuteEvents);

    let requested_board_id = params.board_id.filter(|id| !id.trim().is_empty());
    let version_opt = if let Some(ver_str) = params.version {
        let parts = ver_str
            .split('_')
            .map(str::parse::<u32>)
            .collect::<Result<Vec<u32>, _>>()?;
        match parts.as_slice() {
            [maj, min, pat] => Some((*maj, *min, *pat)),
            _ => {
                return Err(ApiError::internal_error(anyhow!(
                    "version must be in MAJOR_MINOR_PATCH format"
                )));
            }
        }
    } else {
        None
    };

    // The Page DB row carries the owning `board_id`; trying that
    // board first avoids scanning every board on the app. A missing
    // hint, an unresolvable hint, or a load failure all fall through
    // to a full scan so stale/orphaned DB rows can't make a page
    // permanently unreachable. The same scan strategy is used by the
    // desktop's `get_page` Tauri command.
    let row = page::Entity::find_by_id(&page_id)
        .filter(page::Column::AppId.eq(&app_id))
        .one(&state.db)
        .await?;
    let board_hint = requested_board_id
        .clone()
        .or_else(|| row.and_then(|r| r.board_id));

    let app = state.master_app(&user.sub()?, &app_id, &state).await?;

    let try_board = |board_id: String| {
        let app = &app;
        let page_id = &page_id;
        async move {
            let board = app.open_board(board_id, None, version_opt).await.ok()?;
            let board_guard = board.lock().await;
            match version_opt {
                Some(v) => board_guard.load_versioned_page(page_id, v, None).await.ok(),
                None => board_guard.load_page(page_id, None).await.ok(),
            }
        }
    };

    if let Some(board_id) = board_hint
        && let Some(page) = try_board(board_id).await
    {
        return Ok(Json(page));
    }
    if requested_board_id.is_some() {
        return Err(ApiError::NOT_FOUND);
    }

    for board_id in app.boards.iter() {
        if let Some(page) = try_board(board_id.clone()).await {
            return Ok(Json(page));
        }
    }

    Err(ApiError::NOT_FOUND)
}
