use crate::{
    functions::TauriFunctionError,
    state::{TauriFlowLikeState, TauriSettingsState},
};
use flow_like::{
    a2ui::{page_targets::retarget_page_workflow_actions, widget::Page},
    app::App,
    bit::Metadata,
    flow::{
        board::{Board, LoadedPages},
        compiled::prerun::{PrerunPageExecution, decorate_page_actions, page_execution_revision},
        event::Event,
    },
    flow_like_storage::Path,
    state::FlowLikeState,
};
use serde::Serialize;
use std::{collections::HashMap, sync::Arc};
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

/// A board handle backed only by its storage location, holding no board content.
///
/// A page payload is addressed by `(app_id, board_id, page_id)` alone, so reading one never
/// needs the board file: the board lists page ids, it does not store them. A device that has
/// the app but not the board — the normal state until a board is downloaded — can still serve
/// that board's pages through this handle.
///
/// The handle is never saved. Writing a synthetic board file here would make later local board
/// reads succeed against an empty board and permanently suppress the real download.
fn detached_board(app_id: &str, board_id: &str, state: Arc<FlowLikeState>) -> Board {
    Board::new(
        Some(board_id.to_string()),
        Path::from("apps").child(app_id.to_string()),
        state,
    )
}

fn page_revision(page: &Page) -> Option<String> {
    let datetime: chrono::DateTime<chrono::Utc> = page.updated_at.into();
    Some(datetime.to_rfc3339())
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalPageBootstrap {
    pub event: Event,
    pub page: Option<Page>,
    /// Hash of the projected Page returned to the client.
    pub revision: Option<String>,
    pub execution_revision: Option<String>,
    pub canonical_route: Option<String>,
    pub route_miss: bool,
}

fn normalize_route_path(path: &str) -> String {
    let raw = path.trim();
    if raw.is_empty() {
        return "/".to_string();
    }
    let without_fragment = raw.split('#').next().unwrap_or_default();
    let without_query = without_fragment.split('?').next().unwrap_or_default();
    let with_leading_slash = if without_query.starts_with('/') {
        without_query.to_string()
    } else {
        format!("/{without_query}")
    };
    let normalized = with_leading_slash.trim_end_matches('/');
    if normalized.is_empty() {
        "/".to_string()
    } else {
        normalized.to_string()
    }
}

fn canonical_event_route(event: &Event) -> Option<String> {
    event
        .route
        .as_deref()
        .filter(|route| !route.trim().is_empty())
        .map(normalize_route_path)
        .or_else(|| event.is_default.then(|| "/".to_string()))
}

fn resolve_local_route(events: &[Event], requested_route: &str) -> (Option<usize>, bool) {
    let requested = normalize_route_path(requested_route);
    let default = events
        .iter()
        .position(|event| {
            event
                .route
                .as_deref()
                .is_some_and(|route| normalize_route_path(route) == "/")
        })
        .or_else(|| {
            events.iter().position(|event| {
                event.is_default
                    && event
                        .route
                        .as_deref()
                        .is_none_or(|route| route.trim().is_empty())
            })
        });

    if requested == "/" {
        return (default, false);
    }

    let matched = events.iter().position(|event| {
        event
            .route
            .as_deref()
            .is_some_and(|route| normalize_route_path(route) == requested)
    });
    (matched.or(default), matched.is_none())
}

async fn load_local_runtime_events(app: &App) -> Vec<Event> {
    let mut events = Vec::new();
    for event_id in &app.events {
        if let Ok(event) = app.get_event(event_id, None).await
            && event.active
            && event.event_type != "ontology_action"
        {
            events.push(event);
        }
    }
    events
}

/// Project an Event-bound Page from the exact board snapshot available on
/// this device. The returned action ids come from the same compiler used by
/// native execution; raw workflow targets remain only for legacy editors.
#[tauri::command(async)]
pub async fn get_local_page_bootstrap(
    handler: AppHandle,
    app_id: String,
    route: Option<String>,
    event_id: Option<String>,
) -> Result<LocalPageBootstrap, TauriFunctionError> {
    let state = TauriFlowLikeState::construct(&handler).await?;
    let app = App::load(app_id, state).await?;
    let events = load_local_runtime_events(&app).await;

    let (event, route_miss) = if let Some(route) = route.as_deref() {
        let (index, route_miss) = resolve_local_route(&events, route);
        let event = index
            .and_then(|index| events.get(index).cloned())
            .ok_or_else(|| TauriFunctionError::new("No active Page route was found"))?;
        (event, route_miss)
    } else if let Some(event_id) = event_id
        .as_deref()
        .map(str::trim)
        .filter(|event_id| !event_id.is_empty())
    {
        let event = events
            .iter()
            .find(|event| event.id == event_id)
            .cloned()
            .ok_or_else(|| TauriFunctionError::new("No active Event was found"))?;
        (event, false)
    } else {
        let (index, route_miss) = resolve_local_route(&events, "/");
        let event = index
            .and_then(|index| events.get(index).cloned())
            .ok_or_else(|| TauriFunctionError::new("No active default route was found"))?;
        (event, route_miss)
    };

    let canonical_route = canonical_event_route(&event);
    let Some(page_id) = event.default_page_id.as_deref() else {
        return Ok(LocalPageBootstrap {
            event,
            page: None,
            revision: None,
            execution_revision: None,
            canonical_route,
            route_miss,
        });
    };

    let board = app
        .open_board(event.board_id.clone(), None, event.board_version)
        .await
        .map_err(|error| {
            TauriFunctionError::new(&format!(
                "Failed to open Event board '{}' locally: {}",
                event.board_id, error
            ))
        })?;
    let board = board.lock().await;
    if board.id != event.board_id
        || event
            .board_version
            .is_some_and(|expected| board.version != expected)
        || (!board.page_ids.is_empty()
            && !board.page_ids.iter().any(|candidate| candidate == page_id))
    {
        return Err(TauriFunctionError::new(
            "The local Page Event configuration is invalid",
        ));
    }
    let page = match event.board_version {
        Some(version) => board.load_versioned_page(page_id, version, None).await,
        None => board.load_page(page_id, None).await,
    }
    .map_err(|error| {
        TauriFunctionError::new(&format!(
            "Failed to load Event Page '{}' locally: {}",
            page_id, error
        ))
    })?;
    if page.id != page_id
        || page
            .board_id
            .as_deref()
            .is_some_and(|page_board_id| page_board_id != board.id)
    {
        return Err(TauriFunctionError::new(
            "The local Page Event configuration is invalid",
        ));
    }

    let execution = PrerunPageExecution::from_page(&board, &page)?;
    let execution_revision = page_execution_revision(&board, &execution)?;
    let page = decorate_page_actions(&page, &execution, &execution_revision)?;
    let projected = flow_like_types::json::to_vec(&page)?;
    let revision = blake3::hash(&projected).to_hex().to_string();

    Ok(LocalPageBootstrap {
        event,
        page: Some(page),
        revision: Some(revision),
        execution_revision: Some(execution_revision),
        canonical_route,
        route_miss,
    })
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
        match app.open_board(bid.clone(), None, version).await {
            Ok(board) => {
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
            Err(error) => {
                // The caller named the board, so there is nothing to disambiguate and the
                // membership check has no board to run against. Read the payload directly
                // rather than failing an interface whose page is right there on disk.
                tracing::warn!(
                    "Board {} is unavailable locally; reading page {} without it: {}",
                    bid,
                    page_id,
                    error
                );
                let detached = detached_board(&app.id, &bid, flow_like_state);
                return load_page_from_board(&detached, &page_id, &bid, version).await;
            }
        }
    }

    // Scanning cannot consult page ids for a board this device never downloaded, so each
    // unreadable board is probed at the page's canonical location instead of aborting the
    // search — one un-downloaded board must not hide every page in the app.
    let mut unreadable: Vec<String> = Vec::new();

    for bid in app.boards.iter() {
        let board = match app.open_board(bid.clone(), None, version).await {
            Ok(board) => board,
            Err(error) => {
                tracing::warn!(
                    "Board {} is unavailable locally while looking up page {}: {}",
                    bid,
                    page_id,
                    error
                );
                let detached = detached_board(&app.id, bid, flow_like_state.clone());
                if let Ok(page) = load_page_from_board(&detached, &page_id, bid, version).await {
                    return Ok(page);
                }
                unreadable.push(bid.clone());
                continue;
            }
        };
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

    // A bare "not found" is the caller's signal that the page authoritatively does not exist.
    // It may only be given when every board could be consulted.
    if let Some(bid) = unreadable.first() {
        return Err(TauriFunctionError::new(&format!(
            "Failed to open board '{}' while looking up page '{}': board unavailable on this device",
            bid, page_id
        )));
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

/// Returns the page as it was actually stored. It can differ from what the caller sent —
/// workflow targets belonging to another app are rewritten below — and a caller that keeps its
/// own copy would otherwise re-send the foreign ids on every save.
#[tauri::command(async)]
pub async fn update_page(
    handler: AppHandle,
    app_id: String,
    mut page: Page,
) -> Result<Page, TauriFunctionError> {
    let flow_like_state = TauriFlowLikeState::construct(&handler).await?;
    let app = App::load(app_id, flow_like_state.clone()).await?;

    // A page copied out of another app keeps that app's `workflow_event` targets, and the runtime
    // prefers the action's context over the surface it renders on. Same rule as the API, so a page
    // does not flip depending on which side saved it last.
    let retargeted = retarget_page_workflow_actions(&mut page, &app.id);
    if !retargeted.is_empty() {
        tracing::warn!(
            app_id = %app.id,
            page_id = %page.id,
            retargeted = retargeted.len(),
            changes = ?retargeted,
            "page carried workflow targets from another app; rewrote them to the owning app"
        );
    }

    if flow_like_state.page_registry.contains_key(&page.id) {
        flow_like_state
            .page_registry
            .insert(page.id.clone(), page.clone());
    }

    let board_id = page
        .board_id
        .clone()
        .ok_or_else(|| TauriFunctionError::new("Page must have a board_id"))?;

    match app.open_board(board_id.clone(), None, None).await {
        Ok(board) => {
            let mut board_guard = board.lock().await;
            board_guard.save_page(&page, None).await?;
            board_guard.save(None).await?;
        }
        Err(error) => {
            // Caching a page fetched from the server must work on a device that does not have
            // the board yet, or the device can never leave that degraded state. Only the page
            // payload is written: the board's page id list is the server's to send, and
            // inventing a board file here would suppress the real download for good.
            tracing::warn!(
                "Board {} is unavailable locally; caching page {} without it: {}",
                board_id,
                page.id,
                error
            );
            let mut detached = detached_board(&app.id, &board_id, flow_like_state);
            detached.save_page(&page, None).await?;
        }
    }

    Ok(page)
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
