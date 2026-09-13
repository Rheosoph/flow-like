use serde_json::{Value, json};
use tauri::AppHandle;

fn location_timeout(options: &Value) -> Result<std::time::Duration, String> {
    if !options.is_object()
        || options
            .get("highAccuracy")
            .is_some_and(|value| !value.is_boolean())
    {
        return Err("invalid_input: Invalid location options".into());
    }
    for (key, minimum, maximum) in [("maximumAgeMs", 0, 300_000), ("timeoutMs", 100, 120_000)] {
        if let Some(value) = options.get(key) {
            if !value
                .as_u64()
                .is_some_and(|number| (minimum..=maximum).contains(&number))
            {
                return Err(format!(
                    "invalid_input: {key} is outside the supported range"
                ));
            }
        }
    }
    let mut timeout = options
        .get("timeoutMs")
        .and_then(Value::as_u64)
        .unwrap_or(10_000);
    if let Some(value) = options.get("requestDeadline") {
        let deadline = value
            .as_f64()
            .filter(|number| number.is_finite() && *number > 0.0)
            .ok_or("invalid_input: Invalid location deadline")?;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|_| "unavailable: Device clock is unavailable")?
            .as_secs_f64()
            * 1000.0;
        if deadline <= now {
            return Err("timeout: The location request has expired".into());
        }
        timeout = timeout.min((deadline - now).ceil() as u64);
    }
    Ok(std::time::Duration::from_millis(timeout))
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

    type LocationReply = Result<Value, String>;
    struct PendingRequest {
        client_id: String,
        sender: oneshot::Sender<LocationReply>,
    }
    type Pending = HashMap<String, PendingRequest>;
    static PENDING: OnceLock<Mutex<Pending>> = OnceLock::new();

    unsafe extern "C" {
        fn flow_like_native_get_location(
            request_id: *const c_char,
            options: *const c_char,
            callback: extern "C" fn(*const c_char, *const c_char),
        );
        fn flow_like_native_cancel_location(request_id: *const c_char);
    }

    fn pending() -> std::sync::MutexGuard<'static, Pending> {
        PENDING
            .get_or_init(|| Mutex::new(HashMap::new()))
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    extern "C" fn location_ready(request_id: *const c_char, response: *const c_char) {
        if request_id.is_null() || response.is_null() {
            return;
        }
        // Swift owns these callback strings; copy both before returning.
        let id = unsafe { CStr::from_ptr(request_id) }
            .to_string_lossy()
            .into_owned();
        let value = unsafe { CStr::from_ptr(response) }.to_string_lossy();
        let result = serde_json::from_str::<Value>(&value)
            .map_err(|_| "unavailable: Invalid native location response".to_owned())
            .and_then(|reply| {
                if reply["ok"] == true {
                    reply
                        .get("value")
                        .cloned()
                        .ok_or_else(|| "unavailable: Native location is missing".into())
                } else {
                    Err(format!(
                        "{}: {}",
                        reply["error"]["code"].as_str().unwrap_or("unavailable"),
                        reply["error"]["message"]
                            .as_str()
                            .unwrap_or("Location is unavailable")
                    ))
                }
            });
        let sender = pending().remove(&id);
        if let Some(request) = sender {
            let _ = request.sender.send(result);
        }
    }

    pub fn cancel(request_id: &str) {
        let native_id = pending()
            .iter()
            .find(|(_, request)| request.client_id == request_id)
            .map(|(id, _)| id.clone());
        if let Some(native_id) = native_id {
            cancel_native(&native_id);
        }
    }

    fn cancel_native(native_id: &str) {
        let request = pending().remove(native_id);
        if let Some(request) = request {
            let _ = request
                .sender
                .send(Err("cancelled: The location request was cancelled".into()));
        }
        if let Ok(id) = CString::new(native_id) {
            // The Swift entry point schedules sensor cleanup on the main thread.
            unsafe {
                flow_like_native_cancel_location(id.as_ptr());
            }
        }
    }

    struct CancelLocation(String);
    impl Drop for CancelLocation {
        fn drop(&mut self) {
            cancel_native(&self.0);
        }
    }

    pub async fn get(app: AppHandle, request_id: String, options: Value) -> LocationReply {
        let timeout = location_timeout(&options)?;
        if request_id.is_empty()
            || request_id.len() > 128
            || !request_id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
        {
            return Err("invalid_input: Invalid location request identifier".into());
        }
        let native_id = uuid::Uuid::new_v4().to_string();
        let id = CString::new(native_id.as_str())
            .map_err(|_| "invalid_input: Invalid request identifier")?;
        let payload = CString::new(options.to_string())
            .map_err(|_| "invalid_input: Invalid location options")?;
        let (sender, receiver) = oneshot::channel();
        {
            let mut requests = pending();
            if requests.len() >= 16
                || requests
                    .values()
                    .any(|request| request.client_id == request_id)
            {
                return Err("busy: Too many pending location requests".into());
            }
            requests.insert(
                native_id.clone(),
                PendingRequest {
                    client_id: request_id,
                    sender,
                },
            );
        }
        // Dropping a workflow future also stops the sensor request.
        let _cancel = CancelLocation(native_id.clone());
        app.run_on_main_thread(move || {
            let still_pending = pending().contains_key(&native_id);
            if still_pending {
                unsafe {
                    flow_like_native_get_location(id.as_ptr(), payload.as_ptr(), location_ready);
                }
            }
        })
        .map_err(|_| "unavailable: The location request could not reach the app")?;
        tokio::time::timeout(timeout, receiver)
            .await
            .map_err(|_| "timeout: The location request timed out".to_owned())?
            .map_err(|_| "cancelled: The location request was cancelled".to_owned())?
    }
}

pub async fn get(app: AppHandle, request_id: String, options: Value) -> Result<Value, String> {
    #[cfg(any(target_os = "ios", target_os = "macos"))]
    return apple::get(app, request_id, options).await;
    #[cfg(not(any(target_os = "ios", target_os = "macos")))]
    {
        let _ = (app, request_id);
        location_timeout(&options)?;
        Err("unsupported: Native location is unavailable on this platform".into())
    }
}

pub fn cancel(request_id: &str) {
    #[cfg(any(target_os = "ios", target_os = "macos"))]
    apple::cancel(request_id);
    #[cfg(not(any(target_os = "ios", target_os = "macos")))]
    let _ = request_id;
}

pub async fn device_request(app: AppHandle, options: Value) -> Value {
    match get(app, format!("local-{}", uuid::Uuid::new_v4()), options).await {
        Ok(value) => json!({"ok":true,"value":value}),
        Err(error) => {
            let (code, message) = error.split_once(": ").unwrap_or(("unavailable", &error));
            json!({"ok":false,"error":{"code":code,"message":message}})
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn options_reject_wrong_types_and_out_of_bounds_values() {
        for options in [
            json!({"highAccuracy":null}),
            json!({"highAccuracy":1}),
            json!({"maximumAgeMs":300001}),
            json!({"maximumAgeMs":1.5}),
            json!({"timeoutMs":99}),
            json!({"timeoutMs":120001}),
            json!({"requestDeadline":-1}),
        ] {
            assert!(location_timeout(&options).is_err(), "{options}");
        }
        assert_eq!(
            location_timeout(&json!({})).unwrap(),
            std::time::Duration::from_secs(10)
        );
    }

    #[test]
    fn expired_deadline_never_starts_a_sensor_request() {
        assert!(
            location_timeout(&json!({"requestDeadline":1}))
                .unwrap_err()
                .starts_with("timeout:")
        );
    }
}
