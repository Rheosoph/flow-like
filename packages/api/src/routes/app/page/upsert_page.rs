use crate::{
    audit_branch,
    ensure_permission, entity::page, error::ApiError, middleware::jwt::AppUser,
    permission::role_permission::RolePermissions, state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, State},
};
use flow_like::a2ui::widget::Page;
use sea_orm::{ActiveModelTrait, ActiveValue::Set, ColumnTrait, EntityTrait, QueryFilter};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Clone, Deserialize, Serialize, ToSchema)]
pub struct PageUpsert {
    #[schema(value_type = Object)]
    pub page: Page,
}

#[utoipa::path(
    put,
    path = "/apps/{app_id}/pages/{page_id}",
    tag = "pages",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        ("page_id" = String, Path, description = "Page ID")
    ),
    request_body = PageUpsert,
    responses(
        (status = 200, description = "Page created or updated", body = Object),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden")
    )
)]
#[tracing::instrument(
    name = "PUT /apps/{app_id}/pages/{page_id}",
    skip(state, user, page_data)
)]
pub async fn upsert_page(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((app_id, page_id)): Path<(String, String)>,
    Json(page_data): Json<PageUpsert>,
) -> Result<Json<Page>, ApiError> {
    ensure_permission!(user, &app_id, &state, RolePermissions::WriteBoards);

    if page_id.is_empty() || app_id.is_empty() {
        return Err(ApiError::FORBIDDEN);
    }

    let mut app = state
        .scoped_app(
            &user.sub()?,
            &app_id,
            &state,
            crate::credentials::CredentialsAccess::EditApp,
        )
        .await?;

    let mut page = page_data.page;
    page.id = page_id.clone();

    app.save_page(&page).await?;

    let existing = page::Entity::find_by_id(&page_id)
        .filter(page::Column::AppId.eq(&app_id))
        .one(&state.db)
        .await?;

    if existing.is_none() {
        let new_page = page::ActiveModel {
            id: Set(page_id.clone()),
            name: Set(page.name.clone()),
            description: Set(page.title.clone()),
            app_id: Set(app_id.to_string()),
            board_id: Set(page.board_id.clone()),
            version: Set(page.version.map(|v| format!("{}.{}.{}", v.0, v.1, v.2))),
            created_at: Set(chrono::Utc::now().naive_utc()),
            updated_at: Set(chrono::Utc::now().naive_utc()),
        };

        page::Entity::insert(new_page)
            .exec_with_returning(&state.db)
            .await?;

        if !app.page_ids.contains(&page_id) {
            app.page_ids.push(page_id.clone());
            app.save().await?;
        }
    } else {
        let update_page = page::ActiveModel {
            id: Set(page_id.clone()),
            name: Set(page.name.clone()),
            description: Set(page.title.clone()),
            app_id: Set(app_id.to_string()),
            board_id: Set(page.board_id.clone()),
            version: Set(page.version.map(|v| format!("{}.{}.{}", v.0, v.1, v.2))),
            updated_at: Set(chrono::Utc::now().naive_utc()),
            ..Default::default()
        };

        update_page.update(&state.db).await?;
    }

    audit_branch!(state, user, app_id, "page.upsert", "Page", page_id, "Page created or updated");
    Ok(Json(page))
}
