use crate::{
    ensure_permission, entity::page, error::ApiError, middleware::jwt::AppUser,
    permission::role_permission::RolePermissions, state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, Query, State},
};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

#[derive(Debug, Deserialize, IntoParams, ToSchema)]
pub struct GetPagesParams {
    #[serde(alias = "boardId")]
    pub board_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct PageInfo {
    pub app_id: String,
    pub page_id: String,
    pub board_id: Option<String>,
    pub name: String,
    pub description: Option<String>,
}

#[utoipa::path(
    get,
    path = "/apps/{app_id}/pages",
    tag = "pages",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        GetPagesParams
    ),
    responses(
        (status = 200, description = "List of pages in the application", body = Vec<PageInfo>),
        (status = 401, description = "Unauthorized")
    )
)]
#[tracing::instrument(name = "GET /apps/{app_id}/pages", skip(state, user, query))]
pub async fn get_pages(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(app_id): Path<String>,
    Query(query): Query<GetPagesParams>,
) -> Result<Json<Vec<PageInfo>>, ApiError> {
    ensure_permission!(user, &app_id, &state, RolePermissions::ReadBoards);

    let mut query_builder = page::Entity::find().filter(page::Column::AppId.eq(&app_id));

    if let Some(ref board_id) = query.board_id {
        query_builder = query_builder.filter(page::Column::BoardId.eq(board_id));
    }

    let pages = query_builder.all(&state.db).await?;

    let result = pages
        .into_iter()
        .map(|page| PageInfo {
            app_id: app_id.clone(),
            page_id: page.id,
            board_id: page.board_id,
            name: page.name,
            description: page.description,
        })
        .collect();

    Ok(Json(result))
}
