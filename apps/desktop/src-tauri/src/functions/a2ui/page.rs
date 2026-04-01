use crate::{
    functions::TauriFunctionError,
    state::{TauriFlowLikeState, TauriSettingsState},
};
use flow_like::{a2ui::widget::Page, app::App, bit::Metadata};
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

    if let Some(board_id_filter) = &board_id {
        match app.open_board(board_id_filter.clone(), None, None).await {
            Ok(board) => {
                let board_guard = board.lock().await;
                match board_guard.load_all_pages(None).await {
                    Ok(pages) => {
                        for page in pages {
                            result.push(PageInfo {
                                app_id: app_id.clone(),
                                page_id: page.id.clone(),
                                board_id: Some(board_id_filter.clone()),
                                name: page.name.clone(),
                                description: page.title.clone(),
                            });
                        }
                    }
                    Err(e) => tracing::error!(
                        "Failed to load pages for board {}: {:?}",
                        board_id_filter,
                        e
                    ),
                }
            }
            Err(e) => tracing::error!("Failed to open board {}: {:?}", board_id_filter, e),
        }
    } else {
        for board_id in app.boards.iter() {
            match app.open_board(board_id.to_string(), None, None).await {
                Ok(board) => {
                    let board_guard = board.lock().await;
                    match board_guard.load_all_pages(None).await {
                        Ok(pages) => {
                            for page in pages {
                                result.push(PageInfo {
                                    app_id: app_id.clone(),
                                    page_id: page.id.clone(),
                                    board_id: Some(board_id.clone()),
                                    name: page.name.clone(),
                                    description: page.title.clone(),
                                });
                            }
                        }
                        Err(e) => {
                            tracing::error!("Failed to load pages for board {}: {:?}", board_id, e)
                        }
                    }
                }
                Err(e) => tracing::error!("Failed to open board {}: {:?}", board_id, e),
            }
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
) -> Result<Page, TauriFunctionError> {
    let flow_like_state = TauriFlowLikeState::construct(&handler).await?;

    if let Some(page) = flow_like_state.page_registry.get(&page_id) {
        return Ok(page.value().clone());
    }

    let app = App::load(app_id, flow_like_state.clone()).await?;

    if let Some(bid) = board_id {
        let board = app.open_board(bid, None, None).await?;
        let board_guard = board.lock().await;
        if let Ok(page) = board_guard.load_page(&page_id, None).await {
            return Ok(page);
        }
        return Err(TauriFunctionError::new("Page not found in specified board"));
    }

    for bid in app.boards.iter() {
        if let Ok(board) = app.open_board(bid.clone(), None, None).await {
            let board_guard = board.lock().await;
            if let Ok(page) = board_guard.load_page(&page_id, None).await {
                return Ok(page);
            }
        }
    }

    Err(TauriFunctionError::new("Page not found"))
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
            if let Ok(pages) = board_guard.load_all_pages(None).await {
                for page in pages {
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
