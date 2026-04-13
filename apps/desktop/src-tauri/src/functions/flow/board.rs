use crate::{
    functions::TauriFunctionError,
    state::{TauriFlowLikeState, TauriSettingsState},
};
use flow_like::{
    app::App,
    flow::board::{Board, VersionType, commands::GenericCommand},
};
use std::collections::HashMap;
use tauri::AppHandle;
use tauri_plugin_dialog::DialogExt;

#[tauri::command(async)]
pub async fn save_board(handler: AppHandle, board_id: String) -> Result<(), TauriFunctionError> {
    let file_path = handler.dialog().file().blocking_save_file();
    if let Some(file_path) = file_path {
        let board_state = TauriFlowLikeState::construct(&handler).await?;
        let board = board_state.get_board(&board_id, None)?;
        let board = board.lock().await.clone();
        let board_string = serde_json::to_string(&board)
            .map_err(|e| TauriFunctionError::from(anyhow::Error::new(e)))?;
        let file_path = file_path
            .as_path()
            .ok_or(TauriFunctionError::new("Invalid file path"))?;
        std::fs::write(file_path, board_string)
            .map_err(|e| TauriFunctionError::from(anyhow::Error::new(e)))?;
    }
    Err(TauriFunctionError::new("Board not found"))
}

#[tauri::command(async)]
pub async fn create_board_version(
    handler: AppHandle,
    app_id: String,
    board_id: String,
    version_type: VersionType,
) -> Result<(u32, u32, u32), TauriFunctionError> {
    let board_state = TauriFlowLikeState::construct(&handler).await?;
    let board = board_state.get_board(&board_id, None);
    if let Ok(board) = board {
        let mut board = board.lock().await;
        let version = board.create_version(version_type, None).await?;
        return Ok(version);
    }

    let flow_like_state = TauriFlowLikeState::construct(&handler).await?;

    if let Ok(app) = App::load(app_id, flow_like_state).await {
        let board = app.open_board(board_id, Some(true), None).await?;
        let version = board
            .lock()
            .await
            .create_version(version_type, None)
            .await?;
        return Ok(version);
    }

    Err(TauriFunctionError::new("Board not found"))
}

#[tauri::command(async)]
pub async fn get_board_versions(
    handler: AppHandle,
    app_id: String,
    board_id: String,
) -> Result<Vec<(u32, u32, u32)>, TauriFunctionError> {
    let board_state = TauriFlowLikeState::construct(&handler).await?;
    let board = board_state.get_board(&board_id, None);
    if let Ok(board) = board {
        let board = board.lock().await;
        let versions = board.get_versions(None).await?;
        return Ok(versions);
    }

    let flow_like_state = TauriFlowLikeState::construct(&handler).await?;

    if let Ok(app) = App::load(app_id, flow_like_state).await {
        let board = app.open_board(board_id, Some(true), None).await?;
        let versions = board.lock().await.get_versions(None).await?;
        return Ok(versions);
    }

    Err(TauriFunctionError::new("Board not found"))
}

#[tauri::command(async)]
pub async fn get_board(
    handler: AppHandle,
    app_id: String,
    board_id: String,
    version: Option<(u32, u32, u32)>,
) -> Result<Board, TauriFunctionError> {
    let board_state = TauriFlowLikeState::construct(&handler).await?;
    let board = board_state.get_board(&board_id, version);
    if let Ok(board) = board {
        let board = board.lock().await.clone();
        return Ok(board);
    }

    let flow_like_state = TauriFlowLikeState::construct(&handler).await?;

    if let Ok(app) = App::load(app_id, flow_like_state).await {
        let board = app.open_board(board_id, Some(true), version).await?;
        return Ok(board.lock().await.clone());
    }

    Err(TauriFunctionError::new("Board not found"))
}

#[tauri::command(async)]
pub async fn close_board(handler: AppHandle, board_id: String) -> Result<(), TauriFunctionError> {
    let board_state = TauriFlowLikeState::construct(&handler).await?;
    let store = TauriFlowLikeState::get_project_meta_store(&handler).await?;

    let board = { board_state.remove_board(&board_id)? };

    if let Some(board) = board {
        let board = board.lock().await;
        board.save(Some(store.clone())).await?;
        return Ok(());
    }

    Err(TauriFunctionError::new("Board not found"))
}

#[tauri::command(async)]
pub async fn get_open_boards(
    app_handle: AppHandle,
) -> Result<Vec<(String, String, String)>, TauriFunctionError> {
    let profile = TauriSettingsState::current_profile(&app_handle).await?;
    let flow_like_state = TauriFlowLikeState::construct(&app_handle).await?;

    let mut board_app_lookup = HashMap::new();

    for app in profile.hub_profile.apps.unwrap_or_default().iter() {
        if let Ok(app) = App::load(app.app_id.clone(), flow_like_state.clone()).await {
            let app = app;
            for board_id in app.boards.iter() {
                board_app_lookup.insert(board_id.clone(), app.id.clone());
            }
        }
    }

    let board_state = flow_like_state.board_registry.clone();
    let mut boards = Vec::with_capacity(board_state.len());
    for entry in board_state.iter() {
        let value = entry.value();
        let board_id = entry.key().clone();
        let board = value.lock().await;
        if let Some(app_id) = board_app_lookup.get(&board_id) {
            boards.push((app_id.clone(), board_id, board.name.clone()));
        }
    }

    Ok(boards)
}

#[tauri::command(async)]
pub async fn undo_board(
    handler: AppHandle,
    _app_id: String,
    board_id: String,
    commands: Vec<GenericCommand>,
) -> Result<Board, TauriFunctionError> {
    let flow_like_state = TauriFlowLikeState::construct(&handler).await?;
    let board = flow_like_state.get_board(&board_id, None)?;
    let store = TauriFlowLikeState::get_project_meta_store(&handler).await?;
    let mut board = board.lock().await;
    board.undo(commands, flow_like_state).await?;
    board.save(Some(store.clone())).await?;
    Ok(board.clone())
}

#[tauri::command(async)]
pub async fn redo_board(
    handler: AppHandle,
    _app_id: String,
    board_id: String,
    commands: Vec<GenericCommand>,
) -> Result<Board, TauriFunctionError> {
    let flow_like_state = TauriFlowLikeState::construct(&handler).await?;
    let store = TauriFlowLikeState::get_project_meta_store(&handler).await?;
    let board = flow_like_state.get_board(&board_id, None)?;
    let mut board = board.lock().await;
    board.redo(commands, flow_like_state).await?;
    board.save(Some(store.clone())).await?;
    Ok(board.clone())
}

#[tauri::command(async)]
pub async fn execute_command(
    handler: AppHandle,
    _app_id: String,
    board_id: String,
    command: GenericCommand,
) -> Result<GenericCommand, TauriFunctionError> {
    let flow_like_state = TauriFlowLikeState::construct(&handler).await?;
    let store = TauriFlowLikeState::get_project_meta_store(&handler).await?;

    let board = flow_like_state.get_board(&board_id, None)?;

    let mut board = board.lock().await;
    let command = board.execute_command(command, flow_like_state).await?;

    board.save(Some(store)).await?;
    Ok(command)
}

#[tauri::command(async)]
pub async fn execute_commands(
    handler: AppHandle,
    _app_id: String,
    board_id: String,
    commands: Vec<GenericCommand>,
) -> Result<Vec<GenericCommand>, TauriFunctionError> {
    let flow_like_state = TauriFlowLikeState::construct(&handler).await?;
    let store = TauriFlowLikeState::get_project_meta_store(&handler).await?;

    let board = flow_like_state.get_board(&board_id, None)?;

    let mut board = board.lock().await;
    let commands = board.execute_commands(commands, flow_like_state).await?;

    board.save(Some(store)).await?;
    Ok(commands)
}

/// Gets the elements required for executing a workflow on a specific page.
///
/// This returns only the elements that are referenced by nodes in the board,
/// along with their children. Use `wildcard: true` to get all elements.
#[tauri::command(async)]
pub async fn get_execution_elements(
    handler: AppHandle,
    board_id: String,
    page_id: String,
    wildcard: bool,
) -> Result<std::collections::HashMap<String, flow_like_types::Value>, TauriFunctionError> {
    let flow_like_state = TauriFlowLikeState::construct(&handler).await?;
    let board = flow_like_state.get_board(&board_id, None)?;
    let board = board.lock().await;

    let elements = board
        .get_execution_elements(&page_id, wildcard, None)
        .await?;
    Ok(elements)
}
