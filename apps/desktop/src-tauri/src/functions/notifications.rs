use serde_json::Value;

/// A pending tap older than this is considered stale and discarded. The window
/// must be long enough to cover a slow cold-start (Tauri init + Next.js boot
/// on a budget device) but short enough that a stray tap from a previous app
/// session never replays on a future launch.
#[cfg(target_os = "ios")]
const PENDING_TAP_MAX_AGE_SECS: f64 = 300.0;

#[cfg(target_os = "ios")]
fn read_and_clear_pending_tap_ios() -> Option<Value> {
    use objc2::msg_send;
    use objc2::runtime::{AnyClass, AnyObject};
    use std::ffi::CStr;
    use std::os::raw::c_char;
    use std::time::{SystemTime, UNIX_EPOCH};

    let pending_key: &CStr = c"FlowLike.PendingNotificationTap";
    let pending_ts_key: &CStr = c"FlowLike.PendingNotificationTap.Timestamp";

    unsafe {
        let user_defaults_cls = AnyClass::get(c"NSUserDefaults")?;
        let nsstring_cls = AnyClass::get(c"NSString")?;

        let defaults: *mut AnyObject = msg_send![user_defaults_cls, standardUserDefaults];
        if defaults.is_null() {
            return None;
        }

        let key: *mut AnyObject =
            msg_send![nsstring_cls, stringWithUTF8String: pending_key.as_ptr()];
        let ts_key: *mut AnyObject =
            msg_send![nsstring_cls, stringWithUTF8String: pending_ts_key.as_ptr()];
        if key.is_null() || ts_key.is_null() {
            return None;
        }

        let value: *mut AnyObject = msg_send![defaults, stringForKey: key];
        if value.is_null() {
            return None;
        }

        let stored_ts: f64 = msg_send![defaults, doubleForKey: ts_key];

        let utf8_ptr: *const c_char = msg_send![value, UTF8String];
        let json_str = if utf8_ptr.is_null() {
            None
        } else {
            Some(CStr::from_ptr(utf8_ptr).to_string_lossy().into_owned())
        };

        let _: () = msg_send![defaults, removeObjectForKey: key];
        let _: () = msg_send![defaults, removeObjectForKey: ts_key];

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs_f64())
            .unwrap_or(0.0);
        if stored_ts <= 0.0 || (now - stored_ts).abs() > PENDING_TAP_MAX_AGE_SECS {
            return None;
        }

        json_str.and_then(|s| serde_json::from_str::<Value>(&s).ok())
    }
}

#[cfg(not(target_os = "ios"))]
fn read_and_clear_pending_tap_ios() -> Option<Value> {
    None
}

#[tauri::command(async)]
pub fn get_pending_notification_tap() -> Option<Value> {
    let result = read_and_clear_pending_tap_ios();
    tracing::info!(
        target: "flow_like_push",
        "get_pending_notification_tap: present={}",
        result.is_some()
    );
    result
}
