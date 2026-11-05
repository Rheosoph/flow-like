use super::model::{NativeRegistration, Transition};
use anyhow::{Result, anyhow};

#[cfg(any(target_os = "ios", target_os = "macos"))]
mod apple {
    use super::*;
    use std::ffi::{CStr, CString, c_char};
    unsafe extern "C" {
        fn flow_like_native_sync_geofences(value: *const c_char) -> *mut c_char;
        fn flow_like_native_scope() -> *mut c_char;
        fn flow_like_native_take_geofence_events() -> *mut c_char;
        fn flow_like_native_ack_geofence_events(ids: *const c_char) -> i32;
        fn flow_like_native_set_geofence_callback(callback: extern "C" fn());
        fn flow_like_native_set_geofence_expiration_callback(callback: extern "C" fn());
        fn flow_like_native_geofence_background_time_remaining() -> f64;
        fn flow_like_native_geofence_foreground() -> i32;
        fn flow_like_native_free_string(value: *mut c_char);
    }
    extern "C" fn ready() {
        super::super::notify();
    }
    extern "C" fn expired() {
        super::super::expire();
    }
    fn take(value: *mut c_char) -> Result<String> {
        if value.is_null() {
            return Err(anyhow!("Native geofencing returned no value"));
        }
        let text = unsafe { CStr::from_ptr(value) }
            .to_string_lossy()
            .into_owned();
        unsafe {
            flow_like_native_free_string(value);
        }
        Ok(text)
    }
    pub fn setup() {
        unsafe {
            flow_like_native_set_geofence_callback(ready);
            flow_like_native_set_geofence_expiration_callback(expired);
        }
    }
    pub fn scope() -> Result<String> {
        take(unsafe { flow_like_native_scope() })
    }
    pub fn sync(scope: &str, registrations: &[NativeRegistration]) -> Result<()> {
        let value = CString::new(
            serde_json::json!({"scope":scope,"registrations":registrations}).to_string(),
        )?;
        let reply: serde_json::Value = serde_json::from_str(&take(unsafe {
            flow_like_native_sync_geofences(value.as_ptr())
        })?)?;
        if reply["ok"] != true {
            return Err(anyhow!(
                "{}",
                reply["error"]["message"]
                    .as_str()
                    .unwrap_or("Could not register native geofences")
            ));
        }
        Ok(())
    }
    pub fn peek() -> Result<Vec<Transition>> {
        let values: Vec<serde_json::Value> =
            serde_json::from_str(&take(unsafe { flow_like_native_take_geofence_events() })?)?;
        Ok(values
            .into_iter()
            .filter_map(|value| serde_json::from_value(value).ok())
            .collect())
    }
    pub fn ack(ids: &[String]) -> Result<()> {
        if ids.is_empty() {
            return Ok(());
        }
        let value = CString::new(serde_json::to_string(ids)?)?;
        if unsafe { flow_like_native_ack_geofence_events(value.as_ptr()) } != 0 {
            return Err(anyhow!("Could not acknowledge geofence transitions"));
        }
        Ok(())
    }
    pub fn remaining() -> f64 {
        unsafe { flow_like_native_geofence_background_time_remaining() }
    }
    pub fn foreground() -> bool {
        unsafe { flow_like_native_geofence_foreground() != 0 }
    }
}
#[cfg(any(target_os = "ios", target_os = "macos"))]
pub use apple::*;
#[cfg(target_os = "android")]
mod android {
    use super::*;
    use crate::android_mobile;
    pub fn setup() {
        super::super::notify();
    }
    pub fn scope() -> Result<String> {
        android_mobile::geofence_scope().map_err(anyhow::Error::msg)
    }
    pub fn sync(scope: &str, registrations: &[NativeRegistration]) -> Result<()> {
        let reply = android_mobile::geofence_sync(
            serde_json::json!({"scope":scope,"registrations":registrations}),
        )
        .map_err(anyhow::Error::msg)?;
        if reply["ok"] != true {
            return Err(anyhow!(
                "{}",
                reply["error"]["message"]
                    .as_str()
                    .unwrap_or("Android could not register location Events")
            ));
        }
        Ok(())
    }
    pub fn peek() -> Result<Vec<Transition>> {
        let values = android_mobile::geofence_events().map_err(anyhow::Error::msg)?;
        let values = values
            .as_array()
            .ok_or_else(|| anyhow!("Android returned invalid geofence transitions"))?;
        Ok(values
            .iter()
            .filter_map(|value| serde_json::from_value(value.clone()).ok())
            .collect())
    }
    pub fn ack(ids: &[String]) -> Result<()> {
        android_mobile::geofence_ack(ids.to_vec()).map_err(anyhow::Error::msg)
    }
    pub fn remaining() -> f64 {
        android_mobile::geofence_runtime_state()
            .ok()
            .and_then(|value| value["remaining"].as_f64())
            .unwrap_or(0.0)
    }
    pub fn foreground() -> bool {
        android_mobile::geofence_runtime_state()
            .ok()
            .is_some_and(|value| value["foreground"] == true)
    }
}
#[cfg(target_os = "android")]
pub use android::*;
#[cfg(not(any(target_os = "ios", target_os = "macos", target_os = "android")))]
mod unavailable {
    use super::*;
    pub fn setup() {}
    pub fn scope() -> Result<String> {
        Err(anyhow!("Native geofences require iOS, Android, or macOS"))
    }
    pub fn sync(_: &str, _: &[NativeRegistration]) -> Result<()> {
        Err(anyhow!("Native geofences require iOS, Android, or macOS"))
    }
    pub fn peek() -> Result<Vec<Transition>> {
        Ok(vec![])
    }
    pub fn ack(_: &[String]) -> Result<()> {
        Ok(())
    }
    pub fn remaining() -> f64 {
        0.0
    }
    pub fn foreground() -> bool {
        false
    }
}
#[cfg(not(any(target_os = "ios", target_os = "macos", target_os = "android")))]
pub use unavailable::*;
