use serde::{Deserialize, Serialize};
use tauri::AppHandle;
#[cfg(target_os = "macos")]
use tauri_plugin_opener::OpenerExt;

use crate::functions::TauriFunctionError;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PermissionStatus {
    pub accessibility: bool,
    pub screen_recording: bool,
    pub executable_path: Option<String>,
}

fn current_executable_path() -> Option<String> {
    std::env::current_exe()
        .ok()
        .map(|path| path.display().to_string())
}

#[cfg(target_os = "macos")]
fn open_macos_privacy_pane(handler: &AppHandle, anchor: &str) -> Result<(), TauriFunctionError> {
    let url = format!("x-apple.systempreferences:com.apple.preference.security?{anchor}");

    if handler
        .opener()
        .open_url(url.as_str(), None::<&str>)
        .is_ok()
    {
        return Ok(());
    }

    std::process::Command::new("open")
        .arg(&url)
        .spawn()
        .map(|_| ())
        .map_err(|error| {
            TauriFunctionError::new(&format!(
                "Failed to open macOS System Settings privacy pane: {error}"
            ))
        })
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
                &kCFTypeDictionaryKeyCallBacks as *const c_void,
                &kCFTypeDictionaryValueCallBacks as *const c_void,
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
            executable_path: super::current_executable_path(),
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
            executable_path: super::current_executable_path(),
        }
    }

    pub fn request_accessibility() -> bool {
        true
    }

    pub fn request_screen_recording() -> bool {
        true
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
    handler: AppHandle,
    permission_type: String,
) -> Result<bool, TauriFunctionError> {
    #[cfg(not(target_os = "macos"))]
    let _ = handler;

    #[cfg(target_os = "macos")]
    {
        match permission_type.as_str() {
            "accessibility" => {
                let granted = macos::request_accessibility();
                if !granted {
                    open_macos_privacy_pane(&handler, "Privacy_Accessibility")?;
                }
                Ok(granted || macos::check_accessibility())
            }
            "screen_recording" => {
                let granted = macos::request_screen_recording();
                if !granted {
                    open_macos_privacy_pane(&handler, "Privacy_ScreenCapture")?;
                }
                Ok(granted || macos::check_screen_recording())
            }
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
