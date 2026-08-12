use crate::{
    functions::TauriFunctionError,
    state::{TauriFlowLikeState, TauriSettingsState},
};
use flow_like::{a2ui::widget::Page, app::App, bit::Metadata, flow::board::LoadedPages};
use serde::Serialize;
use std::collections::HashMap;
use tauri::AppHandle;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PageInfo {
    pub app_id: String,
    pub page_id: String,
    pub board_id: Option<String>,
    pub name: String,
    pub description: Option<String>,
    /// Payload revision, so a listing can tell a stale local copy from a current one.
    pub updated_at: Option<String>,
    /// The board lists this page but its payload could not be read here. The entry is still
    /// reported so it can be shown and re-synced instead of silently vanishing.
    pub unavailable: bool,
}

fn page_revision(page: &Page) -> Option<String> {
    let datetime: chrono::DateTime<chrono::Utc> = page.updated_at.into();
    Some(datetime.to_rfc3339())
}

fn collect_board_pages(app_id: &str, board_id: &str, loaded: LoadedPages, out: &mut Vec<PageInfo>) {
    for page in loaded.pages {
        out.push(PageInfo {
            app_id: app_id.to_string(),
            page_id: page.id.clone(),
            board_id: Some(board_id.to_string()),
            name: page.name.clone(),
            description: page.title.clone(),
            updated_at: page_revision(&page),
            unavailable: false,
        });
    }

    for unreadable in loaded.unreadable {
        tracing::warn!(
            "Board {} lists page {} but its payload is unreadable: {}",
            board_id,
            unreadable.page_id,
            unreadable.reason
        );
        out.push(PageInfo {
            app_id: app_id.to_string(),
            page_id: unreadable.page_id.clone(),
            board_id: Some(board_id.to_string()),
            // Nothing else survives an unreadable payload; the id is all this host knows.
            name: unreadable.page_id,
            description: None,
            updated_at: None,
            unavailable: true,
        });
    }
}

#[tauri::command(async)]
pub async fn get_pages(
    handler: AppHandle,
    app_id: String,
    board_id: Option<String>,
) -> Result<Vec<PageInfo>, TauriFunctionError> {
    let flow_like_state = TauriFlowLikeState::construct(&handler).await?;
    let app = App::load(app_id.clone(), flow_like_state).await?;

    let mut result = Vec::new();

    let board_ids: Vec<String> = match &board_id {
        Some(board_id_filter) => vec![board_id_filter.clone()],
        None => app.boards.clone(),
    };

    for board_id in board_ids {
        match app.open_board(board_id.clone(), None, None).await {
            Ok(board) => {
                let board_guard = board.lock().await;
                match board_guard.load_all_pages(None).await {
                    Ok(loaded) => collect_board_pages(&app_id, &board_id, loaded, &mut result),
                    Err(e) => {
                        tracing::error!("Failed to load pages for board {}: {:?}", board_id, e)
                    }
                }
            }
            Err(e) => tracing::error!("Failed to open board {}: {:?}", board_id, e),
        }
    }

    Ok(result)
}

#[tauri::command(async)]
pub async fn get_page(
    handler: AppHandle,
    app_id: String,
    page_id: String,
    board_id: Option<String>,
    version: Option<(u32, u32, u32)>,
) -> Result<Page, TauriFunctionError> {
    let flow_like_state = TauriFlowLikeState::construct(&handler).await?;

    // The legacy open-page registry is keyed only by bare page id, so consulting it here can
    // return a page from another app/board before either requested scope is validated. Nothing
    // currently inserts into that registry; always resolve through the authoritative app/board
    // storage until the cache key carries (app_id, board_id, page_id).
    let app = App::load(app_id, flow_like_state.clone()).await?;

    if let Some(bid) = board_id {
        let board = app
            .open_board(bid.clone(), None, version)
            .await
            .map_err(|error| {
                TauriFunctionError::new(&format!(
                    "Failed to open board '{}' while looking up page '{}': {}",
                    bid, page_id, error
                ))
            })?;
        let board_guard = board.lock().await;
        if !board_guard
            .get_page_ids()
            .iter()
            .any(|candidate| candidate == &page_id)
        {
            return Err(TauriFunctionError::new("Page not found in specified board"));
        }
        return load_page_from_board(&board_guard, &page_id, &bid, version).await;
    }

    for bid in app.boards.iter() {
        let board = app
            .open_board(bid.clone(), None, version)
            .await
            .map_err(|error| {
                TauriFunctionError::new(&format!(
                    "Failed to open board '{}' while looking up page '{}': {}",
                    bid, page_id, error
                ))
            })?;
        let board_guard = board.lock().await;
        if !board_guard
            .get_page_ids()
            .iter()
            .any(|candidate| candidate == &page_id)
        {
            continue;
        }
        return load_page_from_board(&board_guard, &page_id, bid, version).await;
    }

    Err(TauriFunctionError::new("Page not found"))
}

/// A pinned board version must read the page snapshot published with it — the current
/// page file belongs to the draft board and can have diverged arbitrarily.
async fn load_page_from_board(
    board: &flow_like::flow::board::Board,
    page_id: &str,
    board_id: &str,
    version: Option<(u32, u32, u32)>,
) -> Result<Page, TauriFunctionError> {
    let loaded = match version {
        Some(version) => board.load_versioned_page(page_id, version, None).await,
        None => board.load_page(page_id, None).await,
    };

    loaded.map_err(|error| {
        TauriFunctionError::new(&format!(
            "Failed to load page '{}' from board '{}': {}",
            page_id, board_id, error
        ))
    })
}

#[derive(serde::Serialize)]
pub struct PageWithBoardId {
    pub page: Page,
    pub board_id: Option<String>,
}

#[tauri::command(async)]
pub async fn get_page_by_route(
    handler: AppHandle,
    app_id: String,
    route: String,
) -> Result<Option<PageWithBoardId>, TauriFunctionError> {
    let flow_like_state = TauriFlowLikeState::construct(&handler).await?;
    let app = App::load(app_id, flow_like_state).await?;

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
                    if page.route == route {
                        return Ok(Some(PageWithBoardId {
                            page,
                            board_id: Some(board_id.clone()),
                        }));
                    }
                }
            }
        }
    }

    Ok(None)
}

#[tauri::command(async)]
pub async fn create_page(
    handler: AppHandle,
    app_id: String,
    page_id: String,
    name: String,
    route: String,
    board_id: String,
    title: Option<String>,
) -> Result<Page, TauriFunctionError> {
    let flow_like_state = TauriFlowLikeState::construct(&handler).await?;
    let app = App::load(app_id, flow_like_state).await?;

    let mut page = Page::new(&page_id, &name, &route);
    if let Some(t) = title {
        page = page.with_title(t);
    }
    page = page.with_board_id(board_id.clone());

    let board = app.open_board(board_id, None, None).await?;
    let result_page;
    {
        let mut board_guard = board.lock().await;
        board_guard.save_page(&page, None).await?;
        board_guard.save(None).await?;
        result_page = page;
    }

    Ok(result_page)
}

#[tauri::command(async)]
pub async fn update_page(
    handler: AppHandle,
    app_id: String,
    page: Page,
) -> Result<(), TauriFunctionError> {
    let flow_like_state = TauriFlowLikeState::construct(&handler).await?;
    let app = App::load(app_id, flow_like_state.clone()).await?;

    if flow_like_state.page_registry.contains_key(&page.id) {
        flow_like_state
            .page_registry
            .insert(page.id.clone(), page.clone());
    }

    let board_id = page
        .board_id
        .clone()
        .ok_or_else(|| TauriFunctionError::new("Page must have a board_id"))?;

    let board = app.open_board(board_id, None, None).await?;
    {
        let mut board_guard = board.lock().await;
        board_guard.save_page(&page, None).await?;
        board_guard.save(None).await?;
    }

    Ok(())
}

#[tauri::command(async)]
pub async fn delete_page(
    handler: AppHandle,
    app_id: String,
    page_id: String,
    board_id: String,
) -> Result<(), TauriFunctionError> {
    let flow_like_state = TauriFlowLikeState::construct(&handler).await?;
    let app = App::load(app_id, flow_like_state.clone()).await?;

    flow_like_state.page_registry.remove(&page_id);

    let board = app.open_board(board_id, None, None).await?;
    {
        let mut board_guard = board.lock().await;
        board_guard.delete_page(&page_id, None).await?;
        board_guard.save(None).await?;
    }

    Ok(())
}

#[tauri::command(async)]
pub async fn get_open_pages(
    handler: AppHandle,
) -> Result<Vec<(String, String, String)>, TauriFunctionError> {
    let profile = TauriSettingsState::current_profile(&handler).await?;
    let flow_like_state = TauriFlowLikeState::construct(&handler).await?;

    let mut page_app_lookup = HashMap::new();

    for app in profile.hub_profile.apps.unwrap_or_default().iter() {
        if let Ok(app) = App::load(app.app_id.clone(), flow_like_state.clone()).await {
            for page_id in app.page_ids.iter() {
                page_app_lookup.insert(page_id.clone(), app.id.clone());
            }
        }
    }

    let mut pages = Vec::new();
    for entry in flow_like_state.page_registry.iter() {
        let page_id = entry.key().clone();
        let page = entry.value();
        if let Some(app_id) = page_app_lookup.get(&page_id) {
            pages.push((app_id.clone(), page_id, page.name.clone()));
        }
    }

    Ok(pages)
}

#[tauri::command(async)]
pub async fn close_page(handler: AppHandle, page_id: String) -> Result<(), TauriFunctionError> {
    let flow_like_state = TauriFlowLikeState::construct(&handler).await?;
    flow_like_state.page_registry.remove(&page_id);
    Ok(())
}

#[tauri::command(async)]
pub async fn get_page_meta(
    handler: AppHandle,
    app_id: String,
    page_id: String,
    language: Option<String>,
) -> Result<Metadata, TauriFunctionError> {
    let flow_like_state = TauriFlowLikeState::construct(&handler).await?;
    let app = App::load(app_id, flow_like_state).await?;
    let meta = app.get_page_meta(&page_id, language).await?;
    Ok(meta)
}

#[tauri::command(async)]
pub async fn push_page_meta(
    handler: AppHandle,
    app_id: String,
    page_id: String,
    metadata: Metadata,
    language: Option<String>,
) -> Result<(), TauriFunctionError> {
    let flow_like_state = TauriFlowLikeState::construct(&handler).await?;
    let app = App::load(app_id, flow_like_state).await?;
    app.push_page_meta(&page_id, language, metadata).await?;
    Ok(())
}
