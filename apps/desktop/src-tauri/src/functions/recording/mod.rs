pub mod browser;
pub mod capture;
pub mod fingerprint;
pub mod generator;
pub mod screenshot;
pub mod state;

pub use state::{RecordedAction, RecordingSettings, RecordingStatus};

use flow_like::flow_like_storage::files::store::FlowLikeStore;
use flow_like::hub::Hub;
use flow_like_types::tokio::sync::OwnedRwLockWriteGuard;
use std::sync::Arc;
use tauri::AppHandle;

use crate::{
    functions::TauriFunctionError,
    state::{TauriFlowLikeState, TauriSettingsState},
};

use self::capture::EventCapture;
use self::state::RecordingState;

fn profile_identity_key(id: &str, hub: &str) -> Result<String, TauriFunctionError> {
    Ok(blake3::hash(&serde_json::to_vec(&(id, hub))?)
        .to_hex()
        .to_string())
}

async fn recording_identity(handler: &AppHandle) -> Result<String, TauriFunctionError> {
    let profile = TauriSettingsState::current_profile(handler).await?;
    profile_identity_key(&profile.hub_profile.id, &profile.hub_profile.hub)
}

/// The caller holds the recorder lifecycle lock and keeps the hub unchanged.
pub async fn remap_recording_profile(
    handler: &AppHandle,
    local_id: &str,
    server_id: &str,
    hub: &str,
) -> Result<(), TauriFunctionError> {
    let old = profile_identity_key(local_id, hub)?;
    let new = profile_identity_key(server_id, hub)?;
    RecordingState::construct(handler)
        .await?
        .inner
        .write()
        .await
        .remap_profile_identity(&old, &new)
}

/// Acquire before settings so profile changes serialize with recorder start and insertion.
pub async fn lock_recording_lifecycle(
    handler: &AppHandle,
) -> Result<OwnedRwLockWriteGuard<Option<EventCapture>>, TauriFunctionError> {
    let recording = RecordingState::construct(handler).await?;
    Ok(recording.capture.write_owned().await)
}

/// Drain the producer without consulting settings, which the caller may already hold.
pub async fn stop_for_profile_change(
    handler: &AppHandle,
    capture: &mut OwnedRwLockWriteGuard<Option<EventCapture>>,
) -> Result<(), TauriFunctionError> {
    if let Some(capture) = capture.take() {
        capture.finish().await;
    }
    let recording = RecordingState::construct(handler).await?;
    let mut state = recording.inner.write().await;
    if state.status != RecordingStatus::Idle {
        state.stop().await?;
    }
    drop(state);
    #[cfg(desktop)]
    crate::tray::restore_tray_icon(handler).await;
    crate::utils::emit_to_ui(handler, "recording:reset", ());
    Ok(())
}

/// Get the storage store for recording screenshots
/// For online projects with a token, uses shared credentials from the hub
/// For offline projects, uses the local app_storage_store
async fn get_recording_store(
    handler: &AppHandle,
    app_id: Option<&str>,
    token: Option<&str>,
) -> Result<Option<FlowLikeStore>, TauriFunctionError> {
    let flow_state = TauriFlowLikeState::construct(handler).await?;

    if let (Some(token), Some(app_id)) = (token, app_id) {
        let profile = TauriSettingsState::current_profile(handler).await?;
        let hub_url = &profile.hub_profile.hub;
        if !hub_url.is_empty() {
            let http_client = TauriFlowLikeState::http_client(handler).await?;
            let remote_store = async {
                Hub::new(hub_url, http_client)
                    .await?
                    .shared_credentials(token, app_id)
                    .await?
                    .to_store_type(flow_like::credentials::StoreType::Content)
                    .await
            };
            match remote_store.await {
                Ok(store) => return Ok(Some(store)),
                Err(error) => tracing::warn!(
                    %error,
                    "[Recording] Hub unavailable, using local storage for screenshots"
                ),
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
    let mut settings = settings.unwrap_or_default();
    let browser_recording = settings
        .browser_debugger_address
        .as_ref()
        .is_some_and(|address| !address.trim().is_empty());
    if browser_recording {
        settings.capture_screenshots = false;
        settings.capture_fingerprints = false;
    } else {
        if settings.capture_screenshots && (app_id.is_none() || board_id.is_none()) {
            return Err(TauriFunctionError::new(
                "Screenshot templates need an application and board. Disable screenshots to record coordinates only.",
            ));
        }
        super::permissions::ensure_recording_permissions(
            &handler,
            settings.capture_screenshots,
            settings.capture_fingerprints,
        )
        .await?;
    }
    let recording_state = RecordingState::construct(&handler).await?;
    let mut capture_guard = recording_state.capture.write().await;
    let identity = recording_identity(&handler).await?;
    if recording_state.inner.read().await.status != RecordingStatus::Idle {
        return Err(TauriFunctionError::new("Recording already in progress"));
    }
    let store = if settings.capture_screenshots {
        get_recording_store(&handler, app_id.as_deref(), token.as_deref()).await?
    } else {
        None
    };
    let session_id = recording_state
        .inner
        .write()
        .await
        .start_session(identity, app_id, board_id, settings.clone())
        .await?;
    let capture = match EventCapture::new(
        recording_state.inner.clone(),
        handler.clone(),
        store.map(Arc::new),
        settings,
    )
    .await
    {
        Ok(capture) => capture,
        Err(error) => {
            let _ = recording_state.inner.write().await.stop().await;
            return Err(error);
        }
    };
    capture.set_active(true);
    *capture_guard = Some(capture);
    #[cfg(desktop)]
    crate::tray::set_recording_tray_icon(&handler).await;
    Ok(session_id)
}

#[tauri::command(async)]
pub async fn pause_recording(handler: AppHandle) -> Result<(), TauriFunctionError> {
    let recording_state = RecordingState::construct(&handler).await?;
    let capture_guard = recording_state.capture.write().await;
    if recording_state.inner.read().await.status != RecordingStatus::Recording {
        return Err(TauriFunctionError::new("Not currently recording"));
    }
    if let Some(capture) = capture_guard.as_ref() {
        capture.set_active(false);
        capture.flush().await;
    }
    recording_state.inner.write().await.pause().await
}

#[tauri::command(async)]
pub async fn resume_recording(handler: AppHandle) -> Result<(), TauriFunctionError> {
    let recording_state = RecordingState::construct(&handler).await?;
    let capture_guard = recording_state.capture.write().await;
    recording_state.inner.write().await.resume().await?;
    if let Some(capture) = capture_guard.as_ref() {
        capture.set_active(true);
    }
    Ok(())
}

#[tauri::command(async)]
pub async fn stop_recording(handler: AppHandle) -> Result<Vec<RecordedAction>, TauriFunctionError> {
    let recording_state = RecordingState::construct(&handler).await?;
    let mut capture_guard = recording_state.capture.write().await;
    if let Some(capture) = capture_guard.take() {
        // Disconnect the producer, then process every event already accepted by the listener.
        capture.finish().await;
    }
    {
        let mut state = recording_state.inner.write().await;
        if state.status != RecordingStatus::Idle {
            state.stop().await?;
        }
    }
    #[cfg(desktop)]
    crate::tray::restore_tray_icon(&handler).await;
    let identity = recording_identity(&handler).await.ok();
    let state = recording_state.inner.read().await;
    let visible_actions = identity
        .as_deref()
        .map(|identity| state.last_completed_actions(identity))
        .unwrap_or_default();
    crate::utils::emit_to_ui(&handler, "recording:stopped", Vec::<RecordedAction>::new());
    Ok(visible_actions)
}

#[tauri::command(async)]
pub async fn get_recording_status(
    handler: AppHandle,
    app_id: Option<String>,
    board_id: String,
) -> Result<RecordingStatus, TauriFunctionError> {
    let recording_state = RecordingState::construct(&handler).await?;
    let _capture_guard = recording_state.capture.read().await;
    let identity = recording_identity(&handler).await?;
    let state = recording_state.inner.read().await;
    Ok(
        if state.owns_context(&identity, app_id.as_deref(), Some(&board_id)) {
            state.status.clone()
        } else {
            RecordingStatus::Idle
        },
    )
}

#[tauri::command(async)]
pub async fn get_recorded_actions(
    handler: AppHandle,
    app_id: Option<String>,
    board_id: String,
) -> Result<Vec<RecordedAction>, TauriFunctionError> {
    let recording_state = RecordingState::construct(&handler).await?;
    let _capture_guard = recording_state.capture.read().await;
    let identity = recording_identity(&handler).await?;
    let state = recording_state.inner.read().await;
    Ok(state.actions_for_context(&identity, app_id.as_deref(), Some(&board_id)))
}

#[tauri::command(async)]
pub async fn clear_recorded_actions(
    handler: AppHandle,
    app_id: Option<String>,
    board_id: String,
) -> Result<(), TauriFunctionError> {
    let state = RecordingState::construct(&handler).await?;
    let _capture_guard = state.capture.write().await;
    let identity = recording_identity(&handler).await?;
    let mut state = state.inner.write().await;
    if state.status != RecordingStatus::Idle {
        return Err(TauriFunctionError::new(
            "Stop recording before clearing actions",
        ));
    }
    state.clear_completed(&identity, app_id.as_deref(), Some(&board_id));
    Ok(())
}

#[tauri::command(async)]
#[allow(clippy::too_many_arguments)]
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
    bot_detection_evasion: Option<bool>,
) -> Result<Vec<flow_like::flow::board::commands::GenericCommand>, TauriFunctionError> {
    let recording = RecordingState::construct(&handler).await?;
    let _capture_guard = recording.capture.write().await;
    let identity = recording_identity(&handler).await?;
    recording.inner.read().await.validate_completed_actions(
        &identity,
        app_id.as_deref(),
        Some(&board_id),
        &actions,
    )?;
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
        app_id: app_id.clone(),
        board_id: Some(board_id.clone()),
        bot_detection_evasion: bot_detection_evasion.unwrap_or(false),
        use_fingerprints: use_fingerprints.unwrap_or(false),
    };
    if !generator_opts.template_confidence.is_finite()
        || !(0.0..=1.0).contains(&generator_opts.template_confidence)
    {
        return Err(TauriFunctionError::new(
            "Template confidence must be between 0 and 1",
        ));
    }

    let commands = generator::generate_add_node_commands(
        &actions,
        position,
        &flow_state,
        Some(generator_opts),
    )
    .await?;
    tracing::info!("Generated {} commands", commands.len());

    let mut board = board.lock().await;
    let commands = board.execute_commands(commands, flow_state.clone()).await?;

    tracing::info!("Successfully inserted {} nodes to board", commands.len());
    recording
        .inner
        .write()
        .await
        .clear_completed(&identity, app_id.as_deref(), Some(&board_id));
    Ok(commands)
}
