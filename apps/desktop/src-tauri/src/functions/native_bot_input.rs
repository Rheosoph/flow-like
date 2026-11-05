use serde_json::{Value, json};
use tauri::AppHandle;

fn validate(request_id: &str, options: &Value) -> Result<(), String> {
    if request_id.is_empty()
        || request_id.len() > 128
        || !request_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        return Err("invalid_input: Invalid native request identifier".into());
    }
    let operation = options
        .get("operation")
        .and_then(Value::as_str)
        .unwrap_or("");
    if !matches!(
        operation,
        "capabilities" | "voice" | "document" | "photo" | "summarize" | "extract"
    ) || options.to_string().len() > 128 * 1024
    {
        return Err("invalid_input: Invalid native bot operation".into());
    }
    if operation != "capabilities"
        && !options
            .get("scope")
            .and_then(Value::as_str)
            .is_some_and(|scope| !scope.is_empty() && scope.len() <= 8192)
    {
        return Err("invalid_input: A workspace is required".into());
    }
    if matches!(operation, "summarize" | "extract")
        && !options
            .get("text")
            .and_then(Value::as_str)
            .is_some_and(|text| !text.trim().is_empty() && text.len() <= 65_536)
    {
        return Err("invalid_input: Provide text no larger than 64 KiB".into());
    }
    Ok(())
}

#[cfg(any(target_os = "ios", target_os = "macos"))]
mod apple {
    use super::*;
    use std::{
        collections::HashMap,
        ffi::{CStr, CString, c_char},
        sync::{Mutex, OnceLock},
    };
    use tokio::sync::oneshot;

    struct Pending {
        client_id: String,
        sender: oneshot::Sender<Result<Value, String>>,
    }
    static REQUESTS: OnceLock<Mutex<HashMap<String, Pending>>> = OnceLock::new();
    unsafe extern "C" {
        fn flow_like_native_bot_request(
            id: *const c_char,
            options: *const c_char,
            callback: extern "C" fn(*const c_char, *const c_char),
        );
        fn flow_like_native_bot_cancel(id: *const c_char);
        fn flow_like_native_bot_activity_tokens() -> *mut c_char;
        fn flow_like_native_free_string(value: *mut c_char);
    }
    fn requests() -> std::sync::MutexGuard<'static, HashMap<String, Pending>> {
        REQUESTS
            .get_or_init(|| Mutex::new(HashMap::new()))
            .lock()
            .unwrap_or_else(|error| error.into_inner())
    }
    extern "C" fn ready(id: *const c_char, response: *const c_char) {
        if id.is_null() || response.is_null() {
            return;
        }
        let id = unsafe { CStr::from_ptr(id) }.to_string_lossy().into_owned();
        let response = unsafe { CStr::from_ptr(response) }.to_string_lossy();
        let result = serde_json::from_str::<Value>(&response)
            .map_err(|_| "unavailable: Invalid native response".to_owned())
            .and_then(|reply| {
                if reply["ok"] == true {
                    reply
                        .get("value")
                        .cloned()
                        .ok_or_else(|| "unavailable: Missing native result".into())
                } else {
                    Err(format!(
                        "{}: {}",
                        reply["error"]["code"].as_str().unwrap_or("unavailable"),
                        reply["error"]["message"]
                            .as_str()
                            .unwrap_or("Native operation failed")
                    ))
                }
            });
        if let Some(pending) = requests().remove(&id) {
            let _ = pending.sender.send(result);
        }
    }
    fn cancel_native(id: &str) {
        if let Some(pending) = requests().remove(id) {
            let _ = pending
                .sender
                .send(Err("cancelled: Native operation cancelled".into()));
        }
        if let Ok(id) = CString::new(id) {
            unsafe { flow_like_native_bot_cancel(id.as_ptr()) };
        }
    }
    pub fn cancel(client_id: &str) {
        let id = requests()
            .iter()
            .find(|(_, pending)| pending.client_id == client_id)
            .map(|(id, _)| id.clone());
        if let Some(id) = id {
            cancel_native(&id);
        }
    }
    struct CancelOnDrop(String);
    impl Drop for CancelOnDrop {
        fn drop(&mut self) {
            cancel_native(&self.0);
        }
    }

    pub async fn request(
        app: AppHandle,
        client_id: String,
        options: Value,
    ) -> Result<Value, String> {
        let id = uuid::Uuid::new_v4().to_string();
        let c_id = CString::new(id.as_str()).map_err(|_| "Invalid request identifier")?;
        let payload = CString::new(options.to_string()).map_err(|_| "Invalid native options")?;
        let (sender, receiver) = oneshot::channel();
        {
            let mut pending = requests();
            if pending.len() >= 8 || pending.values().any(|item| item.client_id == client_id) {
                return Err("busy: A native request is already running".into());
            }
            pending.insert(id.clone(), Pending { client_id, sender });
        }
        let _cancel = CancelOnDrop(id);
        app.run_on_main_thread(move || unsafe {
            flow_like_native_bot_request(c_id.as_ptr(), payload.as_ptr(), ready)
        })
        .map_err(|error| error.to_string())?;
        tokio::time::timeout(std::time::Duration::from_secs(180), receiver)
            .await
            .map_err(|_| "timeout: Native operation timed out".to_owned())?
            .map_err(|_| "cancelled: Native operation ended".to_owned())?
    }

    pub fn activity_tokens() -> Result<Value, String> {
        let pointer = unsafe { flow_like_native_bot_activity_tokens() };
        if pointer.is_null() {
            return Ok(json!([]));
        }
        let text = unsafe { CStr::from_ptr(pointer) }
            .to_string_lossy()
            .into_owned();
        unsafe { flow_like_native_free_string(pointer) };
        serde_json::from_str(&text).map_err(|_| "Invalid activity tokens".into())
    }
}

#[tauri::command]
pub async fn native_bot_request(
    app: AppHandle,
    request_id: String,
    options: Value,
) -> Result<Value, String> {
    validate(&request_id, &options)?;
    #[cfg(any(target_os = "ios", target_os = "macos"))]
    return apple::request(app, request_id, options).await;
    #[cfg(target_os = "android")]
    {
        let _ = app;
        return crate::android_mobile::bot_request(request_id, options).await;
    }
    #[cfg(not(any(target_os = "ios", target_os = "macos", target_os = "android")))]
    {
        let _ = app;
        Err("unavailable: Native capture is unavailable on this platform".into())
    }
}

#[tauri::command]
pub async fn native_bot_capabilities(app: AppHandle) -> Result<Value, String> {
    #[cfg(any(target_os = "ios", target_os = "macos"))]
    return apple::request(
        app,
        uuid::Uuid::new_v4().to_string(),
        json!({"operation":"capabilities"}),
    )
    .await;
    #[cfg(target_os = "android")]
    {
        let _ = app;
        return crate::android_mobile::capabilities();
    }
    #[cfg(not(any(target_os = "ios", target_os = "macos", target_os = "android")))]
    {
        let _ = app;
        Ok(json!({"voice":false,"document":false,"photo":false,"localAiAvailable":false}))
    }
}

#[tauri::command]
pub async fn native_bot_cancel_request(request_id: String) -> Result<(), String> {
    #[cfg(any(target_os = "ios", target_os = "macos"))]
    apple::cancel(&request_id);
    #[cfg(target_os = "android")]
    return crate::android_mobile::bot_cancel_request(&request_id);
    let _ = request_id;
    Ok(())
}

#[tauri::command]
pub async fn native_bot_activity_tokens() -> Result<Value, String> {
    #[cfg(any(target_os = "ios", target_os = "macos"))]
    return apple::activity_tokens();
    #[cfg(not(any(target_os = "ios", target_os = "macos")))]
    Ok(json!([]))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bounds_native_input_and_requires_workspace() {
        assert!(validate("a", &json!({"operation":"capabilities"})).is_ok());
        assert!(validate("a", &json!({"operation":"voice"})).is_err());
        assert!(
            validate(
                "a",
                &json!({"operation":"summarize","scope":"workspace","text":"hello"})
            )
            .is_ok()
        );
        assert!(
            validate(
                "a",
                &json!({"operation":"summarize","scope":"workspace","text":"x".repeat(65_537)})
            )
            .is_err()
        );
        assert!(validate("../a", &json!({"operation":"capabilities"})).is_err());
        assert!(validate("a", &json!({"operation":"shell","scope":"workspace"})).is_err());
    }
}
