use serde_json::{Value, json};
use tauri::AppHandle;

#[path = "native_location.rs"]
mod location;

#[cfg(any(target_os = "ios", target_os = "macos"))]
#[path = "native_shared_files.rs"]
mod shared_files;

#[cfg(any(target_os = "ios", target_os = "macos"))]
mod apple {
    use super::*;
    use std::ffi::{CStr, CString, c_char};
    use std::sync::OnceLock;
    static APP: OnceLock<AppHandle> = OnceLock::new();
    unsafe extern "C" {
        fn flow_like_native_publish_snapshot(snapshot: *const c_char) -> i32;
        fn flow_like_native_clear_snapshot() -> i32;
        fn flow_like_native_take_actions() -> *mut c_char;
        fn flow_like_native_pending_actions() -> *mut c_char;
        fn flow_like_native_ack_action(id: *const c_char, scope: *const c_char) -> i32;
        fn flow_like_native_publish_app_icons(payload: *const c_char) -> i32;
        fn flow_like_native_complete_action(result: *const c_char) -> i32;
        fn flow_like_native_shared_files_directory() -> *mut c_char;
        fn flow_like_native_write_clipboard(payload: *const c_char) -> *mut c_char;
        fn flow_like_native_free_string(value: *mut c_char);
        fn flow_like_native_set_action_callback(callback: extern "C" fn());
        fn flow_like_native_set_inactive_callback(callback: extern "C" fn());
        fn flow_like_native_set_location_background_callback(callback: extern "C" fn());
        fn flow_like_native_geofence_permission(mode: *const c_char) -> *mut c_char;
    }
    extern "C" fn action_ready() {
        if let Some(app) = APP.get() {
            crate::utils::emit_to_ui(app, "native-action", json!({}));
        }
    }
    extern "C" fn inactive() {
        if let Some(app) = APP.get() {
            crate::utils::emit_to_ui(app, "native-inactive", json!({}));
        }
    }
    extern "C" fn location_background() {
        if let Some(app) = APP.get() {
            crate::utils::emit_to_ui(app, "native-location-background", json!({}));
        }
    }
    pub fn setup(app: &AppHandle) {
        let _ = APP.set(app.clone());
        unsafe {
            flow_like_native_set_action_callback(action_ready);
            flow_like_native_set_inactive_callback(inactive);
            flow_like_native_set_location_background_callback(location_background);
        }
    }
    pub fn publish(snapshot: Value) -> Result<(), String> {
        let value = CString::new(snapshot.to_string()).map_err(|e| e.to_string())?;
        if unsafe { flow_like_native_publish_snapshot(value.as_ptr()) } != 0 {
            return Err("Could not publish native widget data".into());
        }
        Ok(())
    }
    pub fn clear() -> Result<(), String> {
        if unsafe { flow_like_native_clear_snapshot() } != 0 {
            return Err("Could not clear native widget data".into());
        }
        Ok(())
    }
    unsafe fn take_json(value: *mut c_char) -> Result<Value, String> {
        if value.is_null() {
            return Err("Native integration returned no response".into());
        }
        let text = unsafe { CStr::from_ptr(value) }
            .to_string_lossy()
            .into_owned();
        unsafe {
            flow_like_native_free_string(value);
        }
        serde_json::from_str(&text).map_err(|e| e.to_string())
    }
    pub fn take_actions() -> Result<Value, String> {
        unsafe { take_json(flow_like_native_take_actions()) }
    }
    pub fn pending_actions() -> Result<Value, String> {
        unsafe { take_json(flow_like_native_pending_actions()) }
    }
    pub fn acknowledge_action(id: String, scope: String) -> Result<(), String> {
        let id = CString::new(id).map_err(|e| e.to_string())?;
        let scope = CString::new(scope).map_err(|e| e.to_string())?;
        if unsafe { flow_like_native_ack_action(id.as_ptr(), scope.as_ptr()) } != 0 {
            return Err("Could not acknowledge native action".into());
        }
        Ok(())
    }
    pub fn publish_app_icons(payload: Value) -> Result<(), String> {
        let payload = CString::new(payload.to_string()).map_err(|e| e.to_string())?;
        if unsafe { flow_like_native_publish_app_icons(payload.as_ptr()) } != 0 {
            return Err("Could not publish native app icons".into());
        }
        Ok(())
    }
    pub fn complete_action(result: Value) -> Result<(), String> {
        let result = CString::new(result.to_string()).map_err(|e| e.to_string())?;
        if unsafe { flow_like_native_complete_action(result.as_ptr()) } != 0 {
            return Err("Could not return the native action result".into());
        }
        Ok(())
    }
    pub fn shared_files_directory() -> Result<std::path::PathBuf, String> {
        let value = unsafe { flow_like_native_shared_files_directory() };
        if value.is_null() {
            return Err("Shared files are unavailable".into());
        }
        let path = unsafe { CStr::from_ptr(value) }
            .to_string_lossy()
            .into_owned();
        unsafe {
            flow_like_native_free_string(value);
        }
        if path.is_empty() {
            return Err("Shared files are unavailable".into());
        }
        Ok(path.into())
    }
    pub fn write_clipboard(payload: Value) -> Result<Value, String> {
        if let Some(deadline) = payload.get("requestDeadline").and_then(Value::as_u64) {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_err(|e| e.to_string())?
                .as_millis() as u64;
            if now >= deadline {
                return Ok(
                    json!({"ok":false,"error":{"code":"expired","message":"Clipboard request has expired"}}),
                );
            }
        }
        let value = CString::new(payload.to_string()).map_err(|e| e.to_string())?;
        unsafe { take_json(flow_like_native_write_clipboard(value.as_ptr())) }
    }
    pub fn geofence_permission(mode: String) -> Result<Value, String> {
        let mode = CString::new(mode).map_err(|_| "invalid_input: Invalid permission mode")?;
        let reply = unsafe { take_json(flow_like_native_geofence_permission(mode.as_ptr())) }?;
        if reply["ok"] == true {
            return reply
                .get("value")
                .cloned()
                .ok_or_else(|| "unavailable: Permission status is missing".into());
        }
        Err(format!(
            "{}: {}",
            reply["error"]["code"].as_str().unwrap_or("unavailable"),
            reply["error"]["message"]
                .as_str()
                .unwrap_or("Location permission is unavailable")
        ))
    }
}

pub fn setup(app: &AppHandle) {
    #[cfg(any(target_os = "ios", target_os = "macos"))]
    apple::setup(app);
    let app = app.clone();
    flow_like::flow::execution::device::set_local_device_handler(std::sync::Arc::new(
        move |command, args| {
            let app = app.clone();
            Box::pin(async move {
                if command == "location.current" {
                    return Ok(location::device_request(app, args).await);
                }
                if command != "clipboard.write" {
                    return Ok(
                        json!({"ok":false,"error":{"code":"unsupported","message":"Unsupported local device operation"}}),
                    );
                }
                native_write_clipboard(app, args)
                    .await
                    .map_err(|error| flow_like_types::anyhow!(error))
            })
        },
    ));
}

#[tauri::command]
pub async fn native_get_location(
    app: AppHandle,
    request_id: String,
    options: Value,
) -> Result<Value, String> {
    location::get(app, request_id, options).await
}

#[tauri::command]
pub async fn native_cancel_location(request_id: String) -> Result<(), String> {
    location::cancel(&request_id);
    Ok(())
}

#[tauri::command]
pub async fn native_geofence_permission(mode: String) -> Result<Value, String> {
    if !["status", "foreground", "background"].contains(&mode.as_str()) {
        return Err("invalid_input: Invalid permission mode".into());
    }
    #[cfg(any(target_os = "ios", target_os = "macos"))]
    return apple::geofence_permission(mode);
    #[cfg(not(any(target_os = "ios", target_os = "macos")))]
    Ok(
        json!({"authorization":"not_determined","accuracyAuthorization":"reduced","backgroundSupported":false,
        "monitoredRegionCount":0,"maxMonitoredRegions":20,"error":{"code":"unsupported","message":"Native geofencing is unavailable on this platform"}}),
    )
}

#[tauri::command]
pub async fn native_publish_snapshot(snapshot: Value) -> Result<(), String> {
    if snapshot.to_string().len() > 2 * 1024 * 1024 {
        return Err("Native snapshot exceeds 2 MiB".into());
    }
    #[cfg(any(target_os = "ios", target_os = "macos"))]
    return apple::publish(snapshot);
    #[cfg(not(any(target_os = "ios", target_os = "macos")))]
    {
        let _ = snapshot;
        Ok(())
    }
}

#[tauri::command]
pub async fn native_clear_snapshot() -> Result<(), String> {
    #[cfg(any(target_os = "ios", target_os = "macos"))]
    return apple::clear();
    #[cfg(not(any(target_os = "ios", target_os = "macos")))]
    Ok(())
}

#[tauri::command]
pub async fn native_take_actions() -> Result<Value, String> {
    #[cfg(any(target_os = "ios", target_os = "macos"))]
    return apple::take_actions();
    #[cfg(not(any(target_os = "ios", target_os = "macos")))]
    Ok(json!([]))
}

#[tauri::command]
pub async fn native_pending_actions() -> Result<Value, String> {
    #[cfg(any(target_os = "ios", target_os = "macos"))]
    return apple::pending_actions();
    #[cfg(not(any(target_os = "ios", target_os = "macos")))]
    Ok(json!([]))
}

#[tauri::command]
pub async fn native_ack_action(id: String, scope: String) -> Result<(), String> {
    #[cfg(any(target_os = "ios", target_os = "macos"))]
    return apple::acknowledge_action(id, scope);
    #[cfg(not(any(target_os = "ios", target_os = "macos")))]
    {
        let _ = (id, scope);
        Ok(())
    }
}

#[tauri::command]
pub async fn native_publish_app_icons(scope: String, icons: Value) -> Result<(), String> {
    let payload = json!({ "scope": scope, "icons": icons });
    if payload.to_string().len() > 1024 * 1024 {
        return Err("Native app icons exceed 1 MiB".into());
    }
    #[cfg(any(target_os = "ios", target_os = "macos"))]
    return apple::publish_app_icons(payload);
    #[cfg(not(any(target_os = "ios", target_os = "macos")))]
    Ok(())
}

#[tauri::command]
pub async fn native_complete_action(result: Value) -> Result<(), String> {
    if result.to_string().len() > 262_144 {
        return Err("Native action result exceeds 256 KiB".into());
    }
    #[cfg(any(target_os = "ios", target_os = "macos"))]
    return apple::complete_action(result);
    #[cfg(not(any(target_os = "ios", target_os = "macos")))]
    Ok(())
}

#[tauri::command]
pub async fn native_read_shared_file(path: String) -> Result<Value, String> {
    #[cfg(any(target_os = "ios", target_os = "macos"))]
    {
        let root = apple::shared_files_directory()?;
        return tokio::task::spawn_blocking(move || {
            let (bytes, name) = shared_files::read(&root, std::path::Path::new(&path))?;
            let mime_type = flow_like_types::mime_guess::from_path(&name)
                .first_or_octet_stream()
                .to_string();
            Ok(json!({"bytes":bytes,"name":name,"mimeType":mime_type}))
        })
        .await
        .map_err(|e| e.to_string())?;
    }
    #[cfg(not(any(target_os = "ios", target_os = "macos")))]
    {
        let _ = path;
        Err("Native shared files are unavailable on this platform".into())
    }
}

#[tauri::command]
pub async fn native_write_clipboard(app: AppHandle, payload: Value) -> Result<Value, String> {
    if payload.to_string().len() > 24 * 1024 * 1024 {
        return Err("Clipboard payload exceeds 24 MiB".into());
    }
    #[cfg(any(target_os = "ios", target_os = "macos"))]
    {
        let (tx, rx) = tokio::sync::oneshot::channel();
        app.run_on_main_thread(move || {
            if tx.is_closed() {
                return;
            }
            let _ = tx.send(apple::write_clipboard(payload));
        })
        .map_err(|e| e.to_string())?;
        return rx.await.map_err(|e| e.to_string())?;
    }
    #[cfg(not(any(target_os = "ios", target_os = "macos")))]
    {
        if let Some(deadline) = payload.get("requestDeadline").and_then(Value::as_u64) {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_err(|e| e.to_string())?
                .as_millis() as u64;
            if now >= deadline {
                return Ok(
                    json!({"ok":false,"error":{"code":"expired","message":"Clipboard request has expired"}}),
                );
            }
        }
        use flow_like_types::base64::{Engine, engine::general_purpose::STANDARD};
        use tauri_plugin_clipboard_manager::ClipboardExt;
        if payload["localOnly"] == true || payload.get("expiresAt").is_some() {
            return Ok(
                json!({"ok":false,"error":{"code":"unsupported_options","message":"Clipboard expiration and local-only sharing are unavailable on this device"}}),
            );
        }
        let format = payload["format"].as_str().unwrap_or_default();
        let result = match format {
            "text" => app
                .clipboard()
                .write_text(payload["text"].as_str().ok_or("Text is required")?)
                .map_err(|e| e.to_string()),
            "html" => app
                .clipboard()
                .write_html(
                    payload["html"].as_str().ok_or("HTML is required")?,
                    payload["text"].as_str(),
                )
                .map_err(|e| e.to_string()),
            "image" => {
                let data = STANDARD
                    .decode(payload["imageBase64"].as_str().ok_or("Image is required")?)
                    .map_err(|e| e.to_string())?;
                let rgba = image::load_from_memory_with_format(&data, image::ImageFormat::Png)
                    .map_err(|e| e.to_string())?
                    .to_rgba8();
                let image = tauri::image::Image::new_owned(
                    rgba.as_raw().clone(),
                    rgba.width(),
                    rgba.height(),
                );
                app.clipboard()
                    .write_image(&image)
                    .map_err(|e| e.to_string())
            }
            _ => Err("Unsupported clipboard format".into()),
        };
        Ok(match result {
            Ok(()) => json!({"ok":true,"value":{"format":format}}),
            Err(message) => {
                json!({"ok":false,"error":{"code":"clipboard_failed","message":message}})
            }
        })
    }
}
