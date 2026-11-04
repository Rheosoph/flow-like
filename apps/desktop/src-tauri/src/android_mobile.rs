//! Android bot surfaces and credentials. Secret operations are internal Rust calls.

use flow_like_types::base64::{Engine as _, engine::general_purpose::STANDARD};
use serde_json::{Value, json};
use std::{path::PathBuf, sync::OnceLock};
use tauri::{
    Wry,
    plugin::{Builder, PluginHandle, TauriPlugin},
};
use zeroize::Zeroizing;

static PLUGIN: OnceLock<PluginHandle<Wry>> = OnceLock::new();

pub fn init() -> TauriPlugin<Wry> {
    Builder::new("bot-mobile")
        .setup(|_app, api| {
            let handle = api.register_android_plugin("com.flow_like.app", "BotMobilePlugin")?;
            PLUGIN
                .set(handle)
                .map_err(|_| "Android bot plugin already registered")?;
            Ok(())
        })
        .build()
}

fn call(command: &str, payload: Value) -> Result<Value, String> {
    PLUGIN
        .get()
        .ok_or("Android bot integration is not ready")?
        .run_mobile_plugin(command, payload)
        .map_err(|error| error.to_string())
}

pub fn secure_read(key: &str) -> Result<Option<Zeroizing<Vec<u8>>>, String> {
    if PLUGIN.get().is_none() {
        return crate::android_bot_runtime::secure_read(key);
    }
    let reply = call("secureGet", json!({"key": key}))?;
    let Some(value) = reply.get("value").and_then(Value::as_str) else {
        return Ok(None);
    };
    STANDARD
        .decode(value)
        .map(Zeroizing::new)
        .map(Some)
        .map_err(|_| "Android credential store returned an invalid value".into())
}

pub fn secure_write(key: &str, bytes: &[u8]) -> Result<(), String> {
    if PLUGIN.get().is_none() {
        return crate::android_bot_runtime::secure_write(key, bytes);
    }
    call(
        "secureSet",
        json!({"key": key, "value": STANDARD.encode(bytes)}),
    )
    .map(drop)
}

pub fn secure_delete(key: &str) -> Result<(), String> {
    if PLUGIN.get().is_none() {
        return crate::android_bot_runtime::secure_delete(key);
    }
    call("secureDelete", json!({"key": key})).map(drop)
}

pub fn publish(snapshot: Value) -> Result<(), String> {
    call("publishSnapshot", json!({"snapshot": snapshot})).map(drop)
}

pub fn clear() -> Result<(), String> {
    call("clearSnapshot", json!({})).map(drop)
}

pub fn pending_actions() -> Result<Value, String> {
    let result = call("pendingActions", json!({}))?;
    Ok(result.get("actions").cloned().unwrap_or_else(|| json!([])))
}

pub fn take_actions() -> Result<Value, String> {
    pending_actions()
}

pub fn acknowledge_action(id: String, scope: String) -> Result<(), String> {
    call("acknowledgeAction", json!({"id": id, "scope": scope})).map(drop)
}

pub fn complete_action(result: Value) -> Result<(), String> {
    call("completeAction", json!({"result": result})).map(drop)
}

pub fn shared_files_directory() -> Result<PathBuf, String> {
    let reply = call("sharedFilesDirectory", json!({}))?;
    reply
        .get("path")
        .and_then(Value::as_str)
        .filter(|path| !path.is_empty())
        .map(PathBuf::from)
        .ok_or_else(|| "Android shared files are unavailable".into())
}

pub async fn bot_request(request_id: String, options: Value) -> Result<Value, String> {
    let handle = PLUGIN
        .get()
        .ok_or("unavailable: Android bot integration is not ready")?;
    handle
        .run_mobile_plugin_async(
            "botRequest",
            json!({"requestId": request_id, "options": options}),
        )
        .await
        .map_err(|error| error.to_string())
}

pub fn bot_cancel_request(request_id: &str) -> Result<(), String> {
    call("botCancelRequest", json!({"requestId": request_id})).map(drop)
}

pub fn capabilities() -> Result<Value, String> {
    call("capabilities", json!({}))
}

fn unwrap_reply(reply: Value) -> Result<Value, String> {
    if reply["ok"] == true {
        reply
            .get("value")
            .cloned()
            .ok_or("unavailable: Missing native result".into())
    } else {
        Err(format!(
            "{}: {}",
            reply["error"]["code"].as_str().unwrap_or("unavailable"),
            reply["error"]["message"]
                .as_str()
                .unwrap_or("Native operation failed")
        ))
    }
}

async fn call_async(command: &str, payload: Value) -> Result<Value, String> {
    PLUGIN
        .get()
        .ok_or("Android native integration is not ready")?
        .run_mobile_plugin_async(command, payload)
        .await
        .map_err(|error| error.to_string())
}

pub async fn location_request(request_id: String, options: Value) -> Result<Value, String> {
    unwrap_reply(
        call_async(
            "locationRequest",
            json!({"requestId":request_id,"options":options}),
        )
        .await?,
    )
}
pub fn location_cancel(request_id: &str) -> Result<(), String> {
    call("locationCancel", json!({"requestId":request_id})).map(drop)
}
pub async fn geofence_permission(mode: String) -> Result<Value, String> {
    unwrap_reply(call_async("geofencePermission", json!({"mode":mode})).await?)
}
pub fn geofence_scope() -> Result<String, String> {
    call("geofenceScope", json!({}))?["scope"]
        .as_str()
        .map(str::to_owned)
        .ok_or("No active geofence workspace".into())
}
pub fn geofence_sync(config: Value) -> Result<Value, String> {
    call("geofenceSync", json!({"config":config}))
}
pub fn geofence_events() -> Result<Value, String> {
    Ok(call("geofenceEvents", json!({}))?["events"].clone())
}
pub fn geofence_ack(ids: Vec<String>) -> Result<(), String> {
    call("geofenceAck", json!({"ids":ids})).map(drop)
}
pub fn geofence_runtime_state() -> Result<Value, String> {
    call("geofenceRuntimeState", json!({}))
}
pub async fn write_clipboard(payload: Value) -> Result<Value, String> {
    call_async("writeClipboard", json!({"payload":payload})).await
}
pub async fn read_private_clipboard() -> Result<Value, String> {
    call_async("readPrivateClipboard", json!({})).await
}
pub fn automation_current(id: &str, scope: &str) -> Result<bool, String> {
    Ok(call("automationCurrent", json!({"id":id,"scope":scope}))?["active"] == true)
}
