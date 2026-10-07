#[cfg(target_os = "ios")]
mod ios {
    use std::{
        collections::HashMap,
        ffi::{CStr, CString, c_char},
        sync::{Mutex, OnceLock},
        time::Duration,
    };
    use tokio::sync::oneshot;

    type TokenReply = Result<String, String>;
    type Pending = HashMap<String, oneshot::Sender<TokenReply>>;
    static PENDING: OnceLock<Mutex<Pending>> = OnceLock::new();

    unsafe extern "C" {
        fn flow_like_request_remote_push_token(
            request_id: *const c_char,
            callback: extern "C" fn(*const c_char, *const c_char, *const c_char),
        );
        fn flow_like_cancel_remote_push_token(request_id: *const c_char);
    }

    fn pending() -> std::sync::MutexGuard<'static, Pending> {
        PENDING
            .get_or_init(|| Mutex::new(HashMap::new()))
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    extern "C" fn token_ready(
        request_id: *const c_char,
        token: *const c_char,
        error: *const c_char,
    ) {
        if request_id.is_null() {
            return;
        }
        // Swift owns the callback strings; copy them before returning.
        let id = unsafe { CStr::from_ptr(request_id) }.to_string_lossy();
        let result = if !error.is_null() {
            Err(unsafe { CStr::from_ptr(error) }
                .to_string_lossy()
                .into_owned())
        } else if !token.is_null() {
            let token = unsafe { CStr::from_ptr(token) }
                .to_string_lossy()
                .into_owned();
            if token.is_empty() {
                Err("Firebase returned an empty push token.".into())
            } else {
                Ok(token)
            }
        } else {
            Err("Push registration returned no result.".into())
        };
        let sender = pending().remove(id.as_ref());
        if let Some(sender) = sender {
            let _ = sender.send(result);
        }
    }

    struct CancelRequest(String);

    impl Drop for CancelRequest {
        fn drop(&mut self) {
            pending().remove(&self.0);
            if let Ok(id) = CString::new(self.0.as_str()) {
                // The Swift entry point performs cleanup on the main queue.
                unsafe { flow_like_cancel_remote_push_token(id.as_ptr()) };
            }
        }
    }

    pub async fn get(app: tauri::AppHandle) -> TokenReply {
        let request_id = uuid::Uuid::new_v4().to_string();
        let id =
            CString::new(request_id.as_str()).map_err(|_| "Invalid push request identifier")?;
        let (sender, receiver) = oneshot::channel();
        {
            let mut requests = pending();
            if requests.len() >= 16 {
                return Err("Too many pending push registration requests.".into());
            }
            requests.insert(request_id.clone(), sender);
        }
        let _cancel = CancelRequest(request_id.clone());
        app.run_on_main_thread(move || {
            if pending().contains_key(&request_id) {
                unsafe { flow_like_request_remote_push_token(id.as_ptr(), token_ready) };
            }
        })
        .map_err(|_| "Push registration could not reach the app")?;
        // Swift reports its own 20-second deadline; this also covers a stalled main queue.
        tokio::time::timeout(Duration::from_secs(22), receiver)
            .await
            .map_err(|_| "Push registration timed out. Check your connection and try again.")?
            .map_err(|_| "Push registration was cancelled.")?
    }
}

pub async fn get(app: tauri::AppHandle) -> Result<String, String> {
    #[cfg(target_os = "ios")]
    return ios::get(app).await;
    #[cfg(not(target_os = "ios"))]
    {
        let _ = app;
        Err("Native Firebase token registration is only available on iOS.".into())
    }
}
