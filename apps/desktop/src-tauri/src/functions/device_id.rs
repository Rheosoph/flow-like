//! Stable per-install device id for push-target deduplication.
//!
//! The server uses `(user_id, device_id, provider)` as the upsert key for
//! push targets. If `device_id` changes on every reinstall, every old row
//! gets superseded and `pushEnabled` flips to false on the previous one.
//!
//! Sources of stability per platform:
//!   - iOS: a UUID stored in the Keychain. Keychain items are scoped to the
//!     bundle id + Team ID and survive uninstall/reinstall.
//!   - Android: `Settings.Secure.ANDROID_ID`, scoped to (signing key, user,
//!     device) since Android 8.0. Survives reinstall as long as the signing
//!     key is unchanged. Read in `MainActivity.kt` and exported as the
//!     `FL_STABLE_DEVICE_ID` env var before the Rust runtime boots.
//!   - Desktop: the JS layer keeps a UUID in the AppData FS, which already
//!     survives uninstall/reinstall on macOS/Windows/Linux. We return `None`
//!     here and let the client fall through to that path.
use flow_like_types::create_id;

#[cfg(target_os = "ios")]
const KEYCHAIN_SERVICE: &str = "com.flow_like.app.device_id";
#[cfg(target_os = "ios")]
const KEYCHAIN_ACCOUNT: &str = "stable_device_id";

#[cfg(target_os = "ios")]
fn ios_read_or_create() -> Option<String> {
    use security_framework::passwords::{get_generic_password, set_generic_password};

    if let Ok(bytes) = get_generic_password(KEYCHAIN_SERVICE, KEYCHAIN_ACCOUNT)
        && let Ok(value) = String::from_utf8(bytes)
        && !value.is_empty()
    {
        return Some(value);
    }

    let new_id = create_id();
    match set_generic_password(KEYCHAIN_SERVICE, KEYCHAIN_ACCOUNT, new_id.as_bytes()) {
        Ok(()) => Some(new_id),
        Err(error) => {
            tracing::warn!(
                target: "flow_like_push",
                "Failed to persist stable device id in iOS Keychain: {}",
                error
            );
            None
        }
    }
}

#[cfg(target_os = "android")]
fn android_read() -> Option<String> {
    std::env::var("FL_STABLE_DEVICE_ID")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

#[tauri::command(async)]
pub fn get_stable_device_id() -> Option<String> {
    #[cfg(target_os = "ios")]
    {
        return ios_read_or_create();
    }
    #[cfg(target_os = "android")]
    {
        return android_read();
    }
    #[cfg(not(any(target_os = "ios", target_os = "android")))]
    {
        None
    }
}
