use crate::{
    audit_branch, ensure_permission, entity::page, error::ApiError, middleware::jwt::AppUser,
    permission::role_permission::RolePermissions, state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, State},
};
use flow_like::a2ui::widget::Page;
use flow_like_types::anyhow;
use sea_orm::{ActiveModelTrait, ActiveValue::Set, EntityTrait};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Clone, Deserialize, Serialize, ToSchema)]
pub struct PageUpsert {
    #[schema(value_type = Object)]
    pub page: Page,
}

/// The row's revision is what `get_pages` hands to clients to decide whether their cached
/// page is stale, so it has to be the same clock the cached payload carries — `get_page`'s
/// merge already treats the payload timestamp as authoritative. A clock running far ahead
/// would freeze the revision for everyone else, so the future is capped.
fn payload_revision(page: &Page) -> chrono::NaiveDateTime {
    let stamped: chrono::DateTime<chrono::Utc> = page.updated_at.into();
    let ceiling = chrono::Utc::now() + chrono::Duration::minutes(5);
    stamped.min(ceiling).naive_utc()
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
        (status = 400, description = "Page payload is missing board_id"),
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
    let permission = ensure_permission!(user, &app_id, &state, RolePermissions::WriteBoards);
    let sub = permission.sub()?;

    if page_id.is_empty() || app_id.is_empty() {
        return Err(ApiError::FORBIDDEN);
    }

    let mut page = page_data.page;
    page.id = page_id.clone();

    // Pages are owned by a board on disk; the path is
    // `apps/{app}/_{board_id}/{page_id}.page`. Without a board id we
    // can't materialize a write target.
    let board_id = page
        .board_id
        .clone()
        .ok_or_else(|| ApiError::bad_request("page payload is missing `board_id`".to_string()))?;

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

    // Lock order is global page id, then owning board. Delete takes the same order. Holding this
    // replica-safe guard through the DB write closes the race where two different board guards
    // could both observe a missing globally unique page id and materialize conflicting files.
    let mut page_id_guard = super::page_id_mutation_guard(&state, &page_id).await?;
    page_id_guard
        .acquire_additional_board(&state, &app_id, &board_id)
        .await?;
    // Page ids are the database primary key and board file name. Reject cross-app or cross-board
    // reuse before writing storage; silently moving the DB row would leave the old board's page
    // file/page_ids entry behind and make exact-board reads disagree.
    let existing = page::Entity::find_by_id(&page_id)
        .one(page_id_guard.connection())
        .await?;
    if let Some(existing_page) = existing.as_ref() {
        if existing_page.app_id != app_id {
            return Err(ApiError::conflict(format!(
                "page id '{page_id}' is already owned by another app"
            )));
        }
        if existing_page
            .board_id
            .as_deref()
            .is_some_and(|existing_board| existing_board != board_id.as_str())
        {
            return Err(ApiError::conflict(format!(
                "page id '{page_id}' is already owned by board '{}'",
                existing_page.board_id.as_deref().unwrap_or_default()
            )));
        }
    }

    let board = app
        .open_board(board_id.clone(), None, None)
        .await
        .map_err(|e| ApiError::internal_error(anyhow!("open board {}: {e}", board_id)))?;

    if existing.is_none() {
        let new_page = page::ActiveModel {
            id: Set(page_id.clone()),
            name: Set(page.name.clone()),
            description: Set(page.title.clone()),
            app_id: Set(app_id.to_string()),
            board_id: Set(Some(board_id.clone())),
            version: Set(page.version.map(|v| format!("{}.{}.{}", v.0, v.1, v.2))),
            created_at: Set(chrono::Utc::now().naive_utc()),
            updated_at: Set(payload_revision(&page)),
        };

        page::Entity::insert(new_page)
            .exec_with_returning(page_id_guard.connection())
            .await?;
    } else {
        let update_page = page::ActiveModel {
            id: Set(page_id.clone()),
            name: Set(page.name.clone()),
            description: Set(page.title.clone()),
            app_id: Set(app_id.to_string()),
            board_id: Set(Some(board_id.clone())),
            version: Set(page.version.map(|v| format!("{}.{}.{}", v.0, v.1, v.2))),
            updated_at: Set(payload_revision(&page)),
            ..Default::default()
        };

        update_page.update(page_id_guard.connection()).await?;
    }

    {
        let mut board_guard = board.lock().await;
        board_guard.save_page(&page, None).await?;
        board_guard.save(None).await?;
    }

    page_id_guard.release().await?;

    audit_branch!(
        state,
        user,
        app_id,
        "page.upsert",
        "Page",
        page_id,
        "Page created or updated"
    );
    Ok(Json(page))
}
