pub mod capture;
pub mod fingerprint;
pub mod generator;
pub mod screenshot;
pub mod state;

pub use state::{RecordedAction, RecordingSettings, RecordingStatus};

use flow_like::flow_like_storage::files::store::FlowLikeStore;
use flow_like::hub::Hub;
use std::sync::Arc;
use tauri::AppHandle;

use crate::{
    functions::TauriFunctionError,
    state::{TauriFlowLikeState, TauriSettingsState},
};

use self::capture::EventCapture;
use self::state::RecordingState;

/// Get the storage store for recording screenshots
/// For online projects with a token, uses shared credentials from the hub
/// For offline projects, uses the local app_storage_store
async fn get_recording_store(
    handler: &AppHandle,
    app_id: Option<&str>,
    token: Option<&str>,
) -> Result<Option<FlowLikeStore>, TauriFunctionError> {
    let flow_state = TauriFlowLikeState::construct(handler).await?;

    // If we have a token and app_id, try to get shared credentials for online storage
    if let (Some(token), Some(app_id)) = (token, app_id) {
        // Get the hub URL from the current profile
        let profile = TauriSettingsState::current_profile(handler).await?;
        let hub_url = &profile.hub_profile.hub;

        if !hub_url.is_empty() {
            let http_client = TauriFlowLikeState::http_client(handler).await?;
            match Hub::new(hub_url, http_client).await {
                Ok(hub) => {
                    match hub.shared_credentials(token, app_id).await {
                        Ok(credentials) => {
                            // Use the content store for screenshots (StoreType::Content)
                            match credentials
                                .to_store_type(flow_like::credentials::StoreType::Content)
                                .await
                            {
                                Ok(store) => {
                                    tracing::info!(
                                        "[Recording] Using online storage for screenshots"
                                    );
                                    return Ok(Some(store));
                                }
                                Err(e) => {
                                    tracing::warn!(
                                        "[Recording] Failed to create online store: {}, falling back to local",
                                        e
                                    );
                                }
                            }
                        }
                        Err(e) => {
                            tracing::warn!(
                                "[Recording] Failed to get shared credentials: {}, falling back to local",
                                e
                            );
                        }
                    }
                }
                Err(e) => {
                    tracing::warn!(
                        "[Recording] Failed to create hub: {}, falling back to local",
                        e
                    );
                }
            }
        }
    }

    // Fallback to local storage
    let config = flow_state.config.read().await;
    let store = config.stores.app_storage_store.clone();
    if store.is_some() {
        tracing::info!("[Recording] Using local storage for screenshots");
    }
    Ok(store)
}

#[tauri::command(async)]
pub async fn start_recording(
    handler: AppHandle,
    app_id: Option<String>,
    board_id: Option<String>,
    settings: Option<RecordingSettings>,
    token: Option<String>,
) -> Result<String, TauriFunctionError> {
    tracing::info!(
        "[Recording] start_recording called with app_id: {:?}, board_id: {:?}, has_token: {}",
        app_id,
        board_id,
        token.is_some()
    );

    let recording_state = RecordingState::construct(&handler).await?;
    tracing::debug!("[Recording] Got recording state");

    // Get the appropriate store for screenshots (online or local)
    let store = get_recording_store(&handler, app_id.as_deref(), token.as_deref()).await?;

    // Start the session (window focus is now tracked dynamically during recording)
    let session_id = {
        let mut state = recording_state.inner.write().await;
        let id = state
            .start_session(app_id, board_id, settings.unwrap_or_default())
            .await?;
        tracing::info!("[Recording] Session started with id: {}", id);
        id
    };

    // Create and start the event capture with the store
    tracing::debug!("[Recording] Creating EventCapture...");
    let capture = EventCapture::new(
        recording_state.inner.clone(),
        handler.clone(),
        store.map(Arc::new),
    );
    capture.set_active(true);
    tracing::info!("[Recording] EventCapture created and set active");

    // Store the capture
    {
        let mut capture_guard = recording_state.capture.write().await;
        *capture_guard = Some(capture);
        tracing::debug!("[Recording] EventCapture stored in state");
    }

    #[cfg(desktop)]
    crate::tray::set_recording_tray_icon(&handler).await;

    Ok(session_id)
}

#[tauri::command(async)]
pub async fn pause_recording(handler: AppHandle) -> Result<(), TauriFunctionError> {
    let recording_state = RecordingState::construct(&handler).await?;

    // Pause event capture
    {
        let capture_guard = recording_state.capture.read().await;
        if let Some(capture) = capture_guard.as_ref() {
            capture.set_active(false);
        }
    }

    let mut state = recording_state.inner.write().await;
    state.pause().await?;
    Ok(())
}

#[tauri::command(async)]
pub async fn resume_recording(handler: AppHandle) -> Result<(), TauriFunctionError> {
    let recording_state = RecordingState::construct(&handler).await?;

    // Resume event capture
    {
        let capture_guard = recording_state.capture.read().await;
        if let Some(capture) = capture_guard.as_ref() {
            capture.set_active(true);
        }
    }

    let mut state = recording_state.inner.write().await;
    state.resume().await?;
    Ok(())
}

#[tauri::command(async)]
pub async fn stop_recording(handler: AppHandle) -> Result<Vec<RecordedAction>, TauriFunctionError> {
    let result = stop_recording_inner(&handler).await;

    // Restore the tray even when stopping fails — otherwise the recording
    // icon sticks and left-click stays detached from the menu.
    #[cfg(desktop)]
    crate::tray::restore_tray_icon(&handler).await;

    result
}

async fn stop_recording_inner(
    handler: &AppHandle,
) -> Result<Vec<RecordedAction>, TauriFunctionError> {
    tracing::debug!(" ========== STOP RECORDING CALLED ==========");
    tracing::info!("[Recording] stop_recording called");

    let recording_state = RecordingState::construct(handler).await?;

    // First, deactivate the capture (stops recording new events)
    {
        let capture_guard = recording_state.capture.read().await;
        if let Some(capture) = capture_guard.as_ref() {
            capture.set_active(false);
            tracing::debug!(" EventCapture deactivated");
            tracing::debug!("[Recording] EventCapture deactivated");
        } else {
            tracing::warn!(" No EventCapture found when stopping!");
            tracing::warn!("[Recording] No EventCapture found when stopping!");
        }
    }

    // Wait for pending events to drain by polling action count stability
    {
        let mut last_count = 0usize;
        let mut stable_ticks = 0u32;
        for _ in 0..20 {
            flow_like_types::tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            let state = recording_state.inner.read().await;
            let current = state.session.as_ref().map(|s| s.actions.len()).unwrap_or(0);
            if current == last_count {
                stable_ticks += 1;
                if stable_ticks >= 3 {
                    break;
                }
            } else {
                stable_ticks = 0;
            }
            last_count = current;
        }
    }

    // Get current state info before stopping
    {
        let state = recording_state.inner.read().await;
        tracing::debug!(" Current status: {:?}", state.status);
        if let Some(session) = &state.session {
            tracing::debug!(" Session has {} actions", session.actions.len());
            tracing::debug!(" Target board ID: {:?}", session.target_board_id);
        } else {
            tracing::warn!(" No active session!");
        }
    }

    // Get the recorded actions
    tracing::debug!(" Calling state.stop() to collect actions...");
    let actions = {
        let mut state = recording_state.inner.write().await;
        let result = state.stop().await?;
        tracing::debug!(" state.stop() returned {} actions", result.len());
        result
    };

    tracing::debug!(" Final action count: {}", actions.len());
    for (i, action) in actions.iter().enumerate() {
        let coords = action
            .coordinates
            .map(|(x, y)| format!("({}, {})", x, y))
            .unwrap_or_else(|| "N/A".to_string());
        tracing::debug!("  Action {}: {:?} at {}", i, action.action_type, coords);
    }
    tracing::info!(
        "[Recording] Stopped with {} actions recorded",
        actions.len()
    );

    // Now drop the capture (closes the channel and stops the processor)
    tracing::debug!(" Dropping EventCapture...");
    {
        let mut capture_guard = recording_state.capture.write().await;
        *capture_guard = None;
    }
    tracing::debug!(" ========== STOP RECORDING COMPLETE ==========");

    Ok(actions)
}

#[tauri::command(async)]
pub async fn get_recording_status(
    handler: AppHandle,
) -> Result<RecordingStatus, TauriFunctionError> {
    let recording_state = RecordingState::construct(&handler).await?;
    let state = recording_state.inner.read().await;
    Ok(state.status.clone())
}

#[tauri::command(async)]
pub async fn get_recorded_actions(
    handler: AppHandle,
) -> Result<Vec<RecordedAction>, TauriFunctionError> {
    let recording_state = RecordingState::construct(&handler).await?;
    let state = recording_state.inner.read().await;
    Ok(state
        .session
        .as_ref()
        .map(|s| s.actions.clone())
        .unwrap_or_default())
}

#[tauri::command(async)]
pub async fn insert_recording_to_board(
    handler: AppHandle,
    board_id: String,
    actions: Vec<RecordedAction>,
    position: (f64, f64),
    version: Option<(u32, u32, u32)>,
    app_id: Option<String>,
    use_pattern_matching: Option<bool>,
    template_confidence: Option<f64>,
    use_fingerprints: Option<bool>,
) -> Result<Vec<flow_like::flow::board::commands::GenericCommand>, TauriFunctionError> {
    tracing::info!(
        "insert_recording_to_board called with {} actions",
        actions.len()
    );
    let flow_state = TauriFlowLikeState::construct(&handler).await?;
    let board = flow_state
        .get_board(&board_id, version)
        .map_err(|e| TauriFunctionError::new(&format!("Board not found: {}", e)))?;

    let generator_opts = generator::GeneratorOptions {
        use_pattern_matching: use_pattern_matching.unwrap_or(false),
        template_confidence: template_confidence.unwrap_or(0.8),
        app_id,
        board_id: Some(board_id.clone()),
        bot_detection_evasion: false,
        use_fingerprints: use_fingerprints.unwrap_or(true),
    };

    let commands = generator::generate_add_node_commands(
        &actions,
        position,
        &flow_state,
        Some(generator_opts),
    )
    .await?;
    tracing::info!("Generated {} commands", commands.len());

    let mut board = board.lock().await;
    for (i, cmd) in commands.iter().enumerate() {
        tracing::debug!("Executing command {}/{}", i + 1, commands.len());
        board
            .execute_command(cmd.clone(), flow_state.clone())
            .await?;
    }

    tracing::info!("Successfully inserted {} nodes to board", commands.len());
    Ok(commands)
}
