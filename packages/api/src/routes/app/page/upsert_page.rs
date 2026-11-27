use crate::{
    audit_branch, ensure_permission,
    entity::page,
    error::ApiError,
    middleware::jwt::AppUser,
    permission::role_permission::RolePermissions,
    routes::app::board::sink_rebind::{rebind_latest_sinks, try_rebind_latest_sinks},
    state::AppState,
    utils::fork::upload::upload_id_map,
};
use axum::{
    Extension, Json,
    extract::{Path, State},
};
use flow_like::{
    a2ui::{
        page_targets::{RetargetedAction, retarget_page_workflow_actions},
        widget::Page,
    },
    app::remap::{ForkIdMap, remap_page},
};
use flow_like_types::{anyhow, proto};
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
fn payload_revision(page: &Page) -> chrono::DateTime<chrono::FixedOffset> {
    let stamped: chrono::DateTime<chrono::Utc> = page.updated_at.into();
    let ceiling = chrono::Utc::now() + chrono::Duration::minutes(5);
    stamped.min(ceiling).fixed_offset()
}

/// The page an upload pushed, moved into the upload's id space together with its board and the
/// nodes, pages, widgets and events it names.
fn rekey_uploaded_page(page: Page, app_id: &str, maps: &ForkIdMap) -> Page {
    let new_page_id = maps.mint(&page.id);
    let mut proto = proto::Page::from(page);
    let unrewritten = remap_page(&mut proto, &new_page_id, maps);
    if !unrewritten.is_empty() {
        tracing::warn!(
            app_id = %app_id,
            page_id = %new_page_id,
            payloads = ?unrewritten,
            "uploaded page has payloads that could not be re-keyed; they keep the local ids"
        );
    }
    Page::from(proto)
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
        (status = 400, description = "Page payload is missing board_id, or names a board that does not belong to this app"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 423, description = "Another writer holds this board's mutation lease (code BOARD_LOCKED). Nothing was written; retry the identical request shortly."),
        (status = 503, description = "The event triggers that run the page's board could not be switched to run as you first (code SINK_REBIND_UNAVAILABLE). Nothing was written; retry the identical request shortly.")
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
    page.id = page_id;
    if let Some(maps) = upload_id_map(&state, &app_id).await? {
        page = rekey_uploaded_page(page, &app_id, &maps);
    }
    let page_id = page.id.clone();

    // Pages are owned by a board on disk; the path is
    // `apps/{app}/_{board_id}/{page_id}.page`. Without a board id we
    // can't materialize a write target.
    let board_id = page
        .board_id
        .clone()
        .ok_or_else(|| ApiError::bad_request("page payload is missing `board_id`".to_string()))?;

    // A page copied out of another app keeps that app's `workflow_event` targets, and the runtime
    // prefers the action's context over the surface it renders on. Normalizing before the write is
    // what keeps a stored page from firing a foreign app's nodes.
    let retargeted = retarget_page_workflow_actions(&mut page, &app_id);
    if !retargeted.is_empty() {
        tracing::warn!(
            app_id = %app_id,
            page_id = %page_id,
            board_id = %board_id,
            retargeted = retargeted.len(),
            changes = ?retargeted,
            "page carried workflow targets from another app; rewrote them to the owning app"
        );
    }

    // Lock order is global page id, then owning board. Delete takes the same order. Holding this
    // replica-safe lease through storage and the DB write closes the race where two different
    // board guards could both observe a missing globally unique page id and materialize
    // conflicting files.
    let mut page_id_guard = super::page_id_mutation_guard(&state, &page_id).await?;
    page_id_guard
        .acquire_additional_board(&state, &app_id, &board_id)
        .await?;
    // Page ids are the database primary key and board file name. Reject cross-app or cross-board
    // reuse before writing storage; silently moving the DB row would leave the old board's page
    // file/page_ids entry behind and make exact-board reads disagree.
    let existing = page::Entity::find_by_id(&page_id).one(&state.db).await?;
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

    // The lease above keeps every other board writer out, and the shared cache revalidates its
    // entry against storage, so this is the stored draft without a full reload when it is current.
    let mut board = match state
        .master_board(&sub, &app_id, &board_id, &state, None)
        .await
    {
        Ok(board) => board,
        Err(error) => {
            return Err(board_open_error(&state, &sub, &app_id, &board_id, &page_id, error).await);
        }
    };

    // A page is part of its board's draft, so its editor is a board editor.
    try_rebind_latest_sinks(&state, &app_id, &board_id, &user).await?;
    page_id_guard.ensure_held()?;
    let board_before_page = board.content_hash();
    board.save_page(&page, None).await?;
    // A rewritten board gets a new storage identity even when its content is the same, and every
    // API instance then rehydrates it on its next read. Persist the board only when the page
    // changed it: a new page id, or page event nodes whose options follow the page.
    let mut seeded = None;
    if board.content_hash() != board_before_page {
        page_id_guard.ensure_held()?;
        let put = board.save(None).await?;
        seeded = state.seed_board_cache(&app_id, &board_id, board, &put);
    }
    rebind_latest_sinks(&state, &app_id, &board_id, &user).await;

    let is_new = existing.is_none();
    let mut row = page::ActiveModel {
        id: Set(page_id.clone()),
        name: Set(page.name.clone()),
        description: Set(page.title.clone()),
        app_id: Set(app_id.to_string()),
        board_id: Set(Some(board_id.clone())),
        version: Set(page.version.map(|v| format!("{}.{}.{}", v.0, v.1, v.2))),
        updated_at: Set(payload_revision(&page)),
        ..Default::default()
    };
    if is_new {
        row.created_at = Set(chrono::Utc::now().fixed_offset());
    }
    state
        .transaction(|txn| {
            let row = row.clone();
            Box::pin(async move {
                if is_new {
                    page::Entity::insert(row).exec(txn).await?;
                } else {
                    row.update(txn).await?;
                }
                Ok::<(), ApiError>(())
            })
        })
        .await?;

    page_id_guard.release().await;
    if let Some(cached) = &seeded {
        crate::routes::app::prerun_shared::prewarm_draft_revision(
            &state, &app_id, &board_id, cached,
        )
        .await;
    }

    if retargeted.is_empty() {
        audit_branch!(state, user, app_id, "page.upsert", "Page", page_id);
    } else {
        audit_branch!(
            state,
            user,
            app_id,
            "page.upsert",
            "Page",
            page_id,
            retarget_audit_details(&board_id, &retargeted)
        );
    }
    Ok(Json(page))
}

/// A board this app does not own is a bad request, not a server fault — but the manifest alone
/// cannot decide that. `manifest.app` is a last-write-wins document several endpoints rewrite
/// without a shared lock, so a board created moments ago can be missing from it while its file is
/// perfectly readable; only a failed open plus an absent manifest entry means the board is really
/// not here.
async fn board_open_error(
    state: &AppState,
    sub: &str,
    app_id: &str,
    board_id: &str,
    page_id: &str,
    error: flow_like_types::Error,
) -> ApiError {
    if let Some(upgrade) = ApiError::from_board_format_error(&error) {
        return upgrade;
    }
    match state.master_app(sub, app_id, state).await {
        Ok(app) if !app.boards.iter().any(|listed| listed == board_id) => {
            ApiError::bad_request(format!(
                "board '{board_id}' is not a board of app '{app_id}', so page '{page_id}' has nowhere to live"
            ))
        }
        _ => ApiError::internal_error(anyhow!("open board {board_id}: {error}")),
    }
}

const MAX_AUDITED_FOREIGN_APPS: usize = 8;

/// Page content decides how many actions get retargeted, so the record carries counts and a
/// capped sample of the foreign apps instead of the change list.
fn retarget_audit_details(board_id: &str, retargeted: &[RetargetedAction]) -> serde_json::Value {
    let mut foreign_app_ids: Vec<&str> = Vec::new();
    let mut foreign_app_ids_truncated = false;
    let mut dropped_board_count = 0usize;
    for change in retargeted {
        match change.field {
            "appId" if !foreign_app_ids.contains(&change.from.as_str()) => {
                if foreign_app_ids.len() < MAX_AUDITED_FOREIGN_APPS {
                    foreign_app_ids.push(&change.from);
                } else {
                    foreign_app_ids_truncated = true;
                }
            }
            "boardId" => dropped_board_count += 1,
            _ => {}
        }
    }
    serde_json::json!({
        "board_id": board_id,
        "retargeted_count": retargeted.len(),
        "foreign_app_ids": foreign_app_ids,
        "foreign_app_ids_truncated": foreign_app_ids_truncated,
        "dropped_board_count": dropped_board_count,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn change(field: &'static str, from: &str) -> RetargetedAction {
        RetargetedAction {
            component_id: "c".to_string(),
            field,
            from: from.to_string(),
            to: None,
        }
    }

    #[test]
    fn retarget_details_stay_bounded() {
        let mut retargeted: Vec<RetargetedAction> = (0..40)
            .flat_map(|i| {
                [
                    change("appId", &format!("app-{}", i % 10)),
                    change("boardId", "b"),
                ]
            })
            .collect();
        retargeted.push(change("appId", "app-0"));

        let details = retarget_audit_details("board", &retargeted);

        assert_eq!(details["board_id"], "board");
        assert_eq!(details["retargeted_count"], 81);
        assert_eq!(details["dropped_board_count"], 40);
        assert_eq!(details["foreign_app_ids_truncated"], true);
        let ids = details["foreign_app_ids"].as_array().unwrap();
        assert_eq!(ids.len(), MAX_AUDITED_FOREIGN_APPS);
        assert_eq!(ids[0], "app-0");
        assert!(details.get("app_id").is_none());
    }

    #[test]
    fn retarget_details_dedupe_without_truncating() {
        let retargeted = vec![
            change("appId", "other"),
            change("appId", "other"),
            change("boardId", "b"),
        ];

        let details = retarget_audit_details("board", &retargeted);

        assert_eq!(details["foreign_app_ids"], serde_json::json!(["other"]));
        assert_eq!(details["foreign_app_ids_truncated"], false);
        assert_eq!(details["dropped_board_count"], 1);
    }
}
