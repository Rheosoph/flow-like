//! Headless Android login renewal shares the foreground native session authority.

use jni::{
    JNIEnv, JavaVM,
    objects::{GlobalRef, JByteArray, JClass, JString, JValue},
    sys::jstring,
};
use serde_json::Value;
use std::{ptr, sync::OnceLock, time::Duration};
use zeroize::Zeroizing;

struct Bridge {
    vm: JavaVM,
    class: GlobalRef,
}
static BRIDGE: OnceLock<Bridge> = OnceLock::new();
static RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();

fn java<T>(
    callback: impl FnOnce(&mut JNIEnv<'_>, &GlobalRef) -> Result<T, String>,
) -> Result<T, String> {
    let bridge = BRIDGE
        .get()
        .ok_or("Android credential bridge is unavailable")?;
    let mut env = bridge
        .vm
        .attach_current_thread()
        .map_err(|_| "Cannot attach credential worker")?;
    let result = callback(&mut env, &bridge.class);
    if env.exception_check().unwrap_or(true) {
        let _ = env.exception_clear();
        return Err("Android credential storage is unavailable".into());
    }
    result
}

pub(crate) fn secure_read(key: &str) -> Result<Option<Zeroizing<Vec<u8>>>, String> {
    java(|env, class| {
        let key = env.new_string(key).map_err(|_| "Invalid credential key")?;
        let value = env
            .call_static_method(
                class,
                "secureRecordRead",
                "(Ljava/lang/String;)[B",
                &[JValue::Object(&key)],
            )
            .and_then(|v| v.l())
            .map_err(|_| "Cannot read Android credential")?;
        if value.is_null() {
            return Ok(None);
        }
        let value = env
            .convert_byte_array(JByteArray::from(value))
            .map_err(|_| "Invalid Android credential")?;
        if value.len() > 512 * 1024 {
            return Err("Android credential is too large".into());
        }
        Ok(Some(Zeroizing::new(value)))
    })
}

pub(crate) fn secure_write(key: &str, bytes: &[u8]) -> Result<(), String> {
    if bytes.len() > 512 * 1024 {
        return Err("Android credential is too large".into());
    }
    java(|env, class| {
        let key = env.new_string(key).map_err(|_| "Invalid credential key")?;
        let bytes = env
            .byte_array_from_slice(bytes)
            .map_err(|_| "Invalid credential bytes")?;
        env.call_static_method(
            class,
            "secureRecordWrite",
            "(Ljava/lang/String;[B)V",
            &[JValue::Object(&key), JValue::Object(&bytes)],
        )
        .map_err(|_| "Cannot save Android credential")?;
        Ok(())
    })
}

pub(crate) fn secure_delete(key: &str) -> Result<(), String> {
    java(|env, class| {
        let key = env.new_string(key).map_err(|_| "Invalid credential key")?;
        env.call_static_method(
            class,
            "secureRecordDelete",
            "(Ljava/lang/String;)V",
            &[JValue::Object(&key)],
        )
        .map_err(|_| "Cannot remove Android credential")?;
        Ok(())
    })
}

fn read_string(method: &str) -> Result<String, String> {
    java(|env, class| {
        let value = env
            .call_static_method(class, method, "()Ljava/lang/String;", &[])
            .and_then(|v| v.l())
            .map_err(|_| "Cannot read Android workspace")?;
        if value.is_null() {
            return Err("Android workspace is unavailable".into());
        }
        env.get_string(&JString::from(value))
            .map(|value| value.into())
            .map_err(|_| "Invalid Android workspace".into())
    })
}

fn scope(raw: &str) -> Result<[String; 3], String> {
    if raw.len() > 8192 {
        return Err("Invalid workspace".into());
    }
    let parts: [String; 3] = serde_json::from_str(raw).map_err(|_| "Invalid workspace")?;
    let url = reqwest::Url::parse(&parts[0]).map_err(|_| "Invalid hub")?;
    if url.scheme() != "https"
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || parts[0].contains(['%', '\\'])
        || parts
            .iter()
            .any(|part| part.is_empty() || part.chars().any(char::is_control))
        || parts[1].len() > 256
        || parts[2].len() > 512
        || parts[2] == "local"
    {
        return Err("Invalid account workspace".into());
    }
    Ok(parts)
}

fn profile_matches(settings: &Value, parts: &[String; 3]) -> bool {
    let Some(profile) = settings
        .get("profiles")
        .and_then(|profiles| profiles.get(&parts[1]))
        .and_then(|profile| profile.get("hub_profile"))
    else {
        return false;
    };
    settings.get("current_profile").and_then(Value::as_str) == Some(parts[1].as_str())
        && profile
            .get("hub")
            .and_then(Value::as_str)
            .and_then(|hub| {
                flow_like::hub::hub_origin(
                    hub,
                    profile
                        .get("secure")
                        .and_then(Value::as_bool)
                        .unwrap_or(true),
                )
            })
            .as_deref()
            == Some(parts[0].trim_end_matches('/'))
}

fn current(raw: &str, parts: &[String; 3]) -> Result<(), String> {
    if read_string("currentScope")? != raw {
        return Err("The Android workspace changed".into());
    }
    let settings: Value = serde_json::from_str(&read_string("settingsJson")?)
        .map_err(|_| "Invalid profile settings")?;
    if !profile_matches(&settings, parts) {
        return Err("The Android profile changed".into());
    }
    Ok(())
}

async fn access_token(raw: String, rejected: Option<String>) -> Result<String, String> {
    let parts = scope(&raw)?;
    current(&raw, &parts)?;
    let binding = crate::native_auth::SessionBinding::capture(&parts[1], &parts[0])
        .await?
        .ok_or("Sign in to continue")?;
    if binding.subject != parts[2] {
        return Err("The Android account changed".into());
    }
    let token = binding.access_token(rejected.as_deref()).await?;
    current(&raw, &parts)?;
    if !binding.is_current() {
        return Err("The Android login changed".into());
    }
    Ok(token)
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_com_flow_1like_app_BotCredentialBroker_nativeAccessToken(
    mut env: JNIEnv,
    class: JClass,
    raw: JString,
    rejected: JString,
) -> jstring {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        if BRIDGE.get().is_none() {
            let bridge = Bridge {
                vm: env.get_java_vm().ok()?,
                class: env.new_global_ref(class).ok()?,
            };
            let _ = BRIDGE.set(bridge);
        }
        let raw: String = env.get_string(&raw).ok()?.into();
        let rejected = if rejected.is_null() {
            None
        } else {
            Some(String::from(env.get_string(&rejected).ok()?))
        };
        if rejected.as_ref().is_some_and(|value| value.len() > 65536) {
            return None;
        }
        if RUNTIME.get().is_none() {
            let runtime = tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .enable_all()
                .build()
                .ok()?;
            let _ = RUNTIME.set(runtime);
        }
        let token = RUNTIME.get()?.block_on(async {
            tokio::time::timeout(Duration::from_secs(15), access_token(raw, rejected))
                .await
                .ok()?
                .ok()
        })?;
        env.new_string(token).ok().map(JString::into_raw)
    }))
    .ok()
    .flatten()
    .unwrap_or(ptr::null_mut())
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_com_flow_1like_app_BotCredentialBroker_nativeGeofenceReady(
    _env: JNIEnv,
    _class: JClass,
) -> jni::sys::jboolean {
    std::panic::catch_unwind(crate::event_sink::geolocation::notify_if_ready).unwrap_or(false)
        as jni::sys::jboolean
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn credential_requests_require_a_remote_unambiguous_account_scope() {
        for origin in [
            "http://hub.example",
            "https://a@hub.example",
            "https://hub.example?x=1",
            "https://hub.example/%2e",
        ] {
            assert!(scope(&serde_json::json!([origin, "profile", "alice"]).to_string()).is_err());
        }
        assert!(
            scope(&serde_json::json!(["https://hub.example", "profile", "local"]).to_string())
                .is_err()
        );
        assert!(
            scope(&serde_json::json!(["https://hub.example", "profile", "alice"]).to_string())
                .is_ok()
        );
    }
    #[test]
    fn persisted_profile_hub_and_selection_must_match() {
        let parts = [
            "https://hub.example".into(),
            "profile".into(),
            "alice".into(),
        ];
        let mut settings = serde_json::json!({"current_profile":"profile","profiles":{"profile":{"hub_profile":{"hub":"hub.example","secure":true}}}});
        assert!(profile_matches(&settings, &parts));
        settings["current_profile"] = serde_json::json!("other");
        assert!(!profile_matches(&settings, &parts));
        settings["current_profile"] = serde_json::json!("profile");
        settings["profiles"]["profile"]["hub_profile"]["hub"] =
            serde_json::json!("foreign.example");
        assert!(!profile_matches(&settings, &parts));
    }
}
