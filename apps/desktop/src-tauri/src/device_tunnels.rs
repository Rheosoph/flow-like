//! The device area's local ports and transfers on the desktop, over the native device client: a
//! local port carries each connection as one encrypted service stream, and deploy uploads and
//! model pushes stream files from this computer straight into data streams, so no byte crosses the
//! webview. The keys the device area unlocks are mirrored here. Locking a device locks it here
//! too; when the area lets go on its own, keys kept for model access or used by a run stay.

#[cfg(desktop)]
mod forward;
#[cfg(desktop)]
mod keys;
#[cfg(desktop)]
mod transfer;

/// The tray's "Lock All Devices" ran; the device area locks its sessions too.
#[cfg(desktop)]
const LOCKED_ALL_EVENT: &str = "device-keys-locked-all";

/// A new page in `label`: its local ports close, and the area lets go of the keys its earlier
/// pages unlocked.
pub fn close_webview(label: &str) {
    #[cfg(desktop)]
    {
        forward::FORWARDS.close_owner(Some(label));
        tauri::async_runtime::spawn(keys::MIRRORS.page_loaded(label));
    }
    #[cfg(not(desktop))]
    let _ = label;
}

/// The main window is gone or the app exits.
pub fn close_all() {
    #[cfg(desktop)]
    {
        forward::FORWARDS.close_owner(None);
        tauri::async_runtime::spawn(keys::MIRRORS.close_all());
    }
}

/// Locks every device this app holds keys for, also those a run's prompt unlocked, and tells
/// the device area to lock its sessions.
#[cfg(desktop)]
pub async fn lock_all_devices(app: &tauri::AppHandle) {
    keys::MIRRORS.lock_all().await;
    crate::utils::emit_to_ui(app, LOCKED_ALL_EVENT, ());
}

#[cfg(desktop)]
fn main_webview(webview: &tauri::Webview) -> Result<(), String> {
    if webview.label() == "main" && webview.window().label() == "main" {
        Ok(())
    } else {
        Err("Native device connections are available only in the main desktop window.".into())
    }
}

#[cfg(desktop)]
fn checked_device(device_id: &str) -> Result<(), String> {
    flow_like_device_protocol::validate_management_id(device_id)
        .map_err(|error| format!("Device id {device_id:?} is invalid: {error}"))
}

/// Opens 127.0.0.1:`port` (0 picks one) for a deployed service listener, once the device accepted
/// a probe stream to it. Without a touch every 30 s the port closes.
#[cfg(desktop)]
#[tauri::command]
pub(crate) async fn device_tunnel_listen(
    webview: tauri::Webview,
    target: forward::ForwardTarget,
    port: Option<u16>,
) -> Result<forward::ListenerInfo, String> {
    main_webview(&webview)?;
    forward::listen(webview.label(), target, port.unwrap_or(0)).await
}

#[cfg(desktop)]
#[tauri::command]
pub(crate) fn device_tunnel_touch(
    webview: tauri::Webview,
    id: String,
    token: String,
) -> Result<(), String> {
    main_webview(&webview)?;
    forward::FORWARDS.touch(webview.label(), &id, &token)
}

#[cfg(desktop)]
#[tauri::command]
pub(crate) fn device_tunnel_close(
    webview: tauri::Webview,
    id: String,
    token: String,
) -> Result<(), String> {
    main_webview(&webview)?;
    forward::FORWARDS.close(webview.label(), &id, &token)
}

/// The device area unlocked a device: the same password and vault open its keys here. Errors read
/// `<code>: <message>` like `device_models_unlock`.
#[cfg(desktop)]
#[tauri::command]
pub(crate) async fn device_keys_unlock(
    webview: tauri::Webview,
    unlock: crate::device_models::VaultUnlock,
) -> Result<(), String> {
    main_webview(&webview)?;
    checked_device(&unlock.device_id).map_err(|error| format!("invalid: {error}"))?;
    keys::MIRRORS.unlock(webview.label(), unlock).await
}

/// The device area locked a device (`lock`: its keys go here too, kept or not) or let go of it
/// on its own (`release`: keys kept for model access or used by a run stay).
#[cfg(desktop)]
#[tauri::command]
pub(crate) async fn device_keys_lock(
    webview: tauri::Webview,
    device_id: String,
    mode: keys::LockMode,
) -> Result<(), String> {
    main_webview(&webview)?;
    checked_device(&device_id)?;
    keys::MIRRORS.lock(&device_id, mode).await;
    Ok(())
}

/// Lock all in the device area: every device this app holds keys for locks, also those a run's
/// prompt unlocked.
#[cfg(desktop)]
#[tauri::command]
pub(crate) async fn device_keys_lock_all(webview: tauri::Webview) -> Result<(), String> {
    main_webview(&webview)?;
    keys::MIRRORS.lock_all().await;
    Ok(())
}

#[cfg(desktop)]
#[tauri::command]
pub(crate) async fn device_keys_keep(
    webview: tauri::Webview,
    device_id: String,
    keep: bool,
) -> Result<(), String> {
    main_webview(&webview)?;
    checked_device(&device_id)?;
    keys::MIRRORS.keep(&device_id, keep).await;
    Ok(())
}

/// Streams one file of a prepared export from `offset` to its end and answers the device's
/// transfer status.
#[cfg(desktop)]
#[tauri::command]
pub(crate) async fn device_upload_artifact(
    app_handle: tauri::AppHandle,
    webview: tauri::Webview,
    transfer: String,
    upload: transfer::ArtifactUpload,
) -> Result<serde_json::Value, transfer::ArtifactUploadFailure> {
    main_webview(&webview)
        .map_err(|error| transfer::ArtifactUploadFailure::new("prepare", error))?;
    transfer::upload(&app_handle, &transfer, upload).await
}

/// Sends a model asset job's bytes from this computer's Bit store, or without a file only opens
/// the device's push session; answers the job's state. `progress` gets the bytes the device holds.
#[cfg(desktop)]
#[tauri::command]
pub(crate) async fn device_push_model_asset(
    app_handle: tauri::AppHandle,
    webview: tauri::Webview,
    transfer: String,
    push: transfer::ModelAssetPush,
    progress: tauri::ipc::Channel<u64>,
) -> Result<serde_json::Value, String> {
    main_webview(&webview)?;
    transfer::push(&app_handle, &transfer, push, move |bytes| {
        let _ = progress.send(bytes);
    })
    .await
}

#[cfg(desktop)]
#[tauri::command]
pub(crate) fn device_transfer_cancel(
    webview: tauri::Webview,
    transfer: String,
) -> Result<(), String> {
    main_webview(&webview)?;
    transfer::cancel(&transfer)
}
