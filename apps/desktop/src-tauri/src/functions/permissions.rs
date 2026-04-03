use serde::{Deserialize, Serialize};
use tauri::AppHandle;

use crate::functions::TauriFunctionError;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PermissionStatus {
    pub accessibility: bool,
    pub screen_recording: bool,
}

#[cfg(target_os = "macos")]
mod macos {
    use super::PermissionStatus;
    use std::ffi::c_void;
    use std::ptr;

    #[link(name = "ApplicationServices", kind = "framework")]
    unsafe extern "C" {
        fn AXIsProcessTrusted() -> bool;
        fn AXIsProcessTrustedWithOptions(options: *const c_void) -> bool;
    }

    #[link(name = "CoreGraphics", kind = "framework")]
    unsafe extern "C" {
        fn CGPreflightScreenCaptureAccess() -> bool;
        fn CGRequestScreenCaptureAccess() -> bool;
    }

    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        fn CFStringCreateWithCString(
            allocator: *const c_void,
            c_str: *const i8,
            encoding: u32,
        ) -> *const c_void;
        fn CFDictionaryCreate(
            allocator: *const c_void,
            keys: *const *const c_void,
            values: *const *const c_void,
            num_values: isize,
            key_callbacks: *const c_void,
            value_callbacks: *const c_void,
        ) -> *const c_void;
        fn CFRelease(cf: *const c_void);
        static kCFBooleanTrue: *const c_void;
        static kCFTypeDictionaryKeyCallBacks: c_void;
        static kCFTypeDictionaryValueCallBacks: c_void;
    }

    const K_CF_STRING_ENCODING_UTF8: u32 = 0x08000100;

    pub fn check_accessibility() -> bool {
        unsafe { AXIsProcessTrusted() }
    }

    pub fn check_screen_recording() -> bool {
        unsafe { CGPreflightScreenCaptureAccess() }
    }

    pub fn request_accessibility() -> bool {
        unsafe {
            let key_str = b"AXTrustedCheckOptionPrompt\0";
            let key = CFStringCreateWithCString(
                ptr::null(),
                key_str.as_ptr() as *const i8,
                K_CF_STRING_ENCODING_UTF8,
            );

            if key.is_null() {
                return AXIsProcessTrustedWithOptions(ptr::null());
            }

            let keys = [key];
            let values = [kCFBooleanTrue];

            let options = CFDictionaryCreate(
                ptr::null(),
                keys.as_ptr(),
                values.as_ptr(),
                1,
                &kCFTypeDictionaryKeyCallBacks as *const _ as *const c_void,
                &kCFTypeDictionaryValueCallBacks as *const _ as *const c_void,
            );

            let trusted = AXIsProcessTrustedWithOptions(options);

            if !options.is_null() {
                CFRelease(options);
            }
            CFRelease(key);

            trusted
        }
    }

    pub fn request_screen_recording() -> bool {
        unsafe {
            CGRequestScreenCaptureAccess();
            CGPreflightScreenCaptureAccess()
        }
    }

    pub fn get_permission_status() -> PermissionStatus {
        PermissionStatus {
            accessibility: check_accessibility(),
            screen_recording: check_screen_recording(),
        }
    }
}

#[cfg(not(target_os = "macos"))]
mod other {
    use super::PermissionStatus;

    pub fn get_permission_status() -> PermissionStatus {
        PermissionStatus {
            accessibility: true,
            screen_recording: true,
        }
    }

    pub fn request_accessibility() -> bool {
        true
    }

    pub fn request_screen_recording() -> bool {
        true
    }
}

/// Returns the ATT authorization status string on iOS.
/// Possible values: "not_determined", "restricted", "denied", "authorized".
/// On non-iOS platforms, always returns "authorized".
#[tauri::command(async)]
pub async fn get_tracking_authorization_status(
    _handler: AppHandle,
) -> Result<String, TauriFunctionError> {
    #[cfg(target_os = "ios")]
    {
        let status: u32 = unsafe {
            let cls = objc2::runtime::AnyClass::get(c"ATTrackingManager")
                .ok_or_else(|| TauriFunctionError::new("ATTrackingManager class not found"))?;
            objc2::msg_send![cls, trackingAuthorizationStatus]
        };
        let status_str = match status {
            0 => "not_determined",
            1 => "restricted",
            2 => "denied",
            3 => "authorized",
            _ => "not_determined",
        };
        Ok(status_str.to_string())
    }
    #[cfg(not(target_os = "ios"))]
    {
        Ok("authorized".to_string())
    }
}

#[tauri::command(async)]
pub async fn check_rpa_permissions(
    _handler: AppHandle,
) -> Result<PermissionStatus, TauriFunctionError> {
    #[cfg(target_os = "macos")]
    {
        Ok(macos::get_permission_status())
    }
    #[cfg(not(target_os = "macos"))]
    {
        Ok(other::get_permission_status())
    }
}

#[tauri::command(async)]
pub async fn request_rpa_permission(
    _handler: AppHandle,
    permission_type: String,
) -> Result<bool, TauriFunctionError> {
    #[cfg(target_os = "macos")]
    {
        match permission_type.as_str() {
            "accessibility" => Ok(macos::request_accessibility()),
            "screen_recording" => Ok(macos::request_screen_recording()),
            _ => Err(TauriFunctionError::new(&format!(
                "Unknown permission type: {}",
                permission_type
            ))),
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        match permission_type.as_str() {
            "accessibility" | "screen_recording" => Ok(true),
            _ => Err(TauriFunctionError::new(&format!(
                "Unknown permission type: {}",
                permission_type
            ))),
        }
    }
}
