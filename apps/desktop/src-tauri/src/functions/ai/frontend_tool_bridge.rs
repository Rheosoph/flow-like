use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    sync::{
        Mutex,
        atomic::{AtomicU64, Ordering},
        mpsc::{self, Sender},
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tauri::{AppHandle, Emitter};

pub const FLOWPILOT_FRONTEND_TOOL_EVENT: &str = "flowpilot://frontend-tool-request";
pub const GLOBAL_FRONTEND_TOOL_EVENT: &str = "flowpilot://global-tool-request";

static REQUEST_COUNTER: AtomicU64 = AtomicU64::new(1);
static PENDING_RESPONSES: Lazy<Mutex<HashMap<String, Sender<FrontendToolResponse>>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

#[derive(Debug, Clone)]
pub struct FrontendToolBridge {
    app_handle: AppHandle,
    timeout: Duration,
    event: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FrontendToolRequest {
    pub request_id: String,
    pub tool_name: String,
    pub arguments: Value,
    pub approval: FrontendToolApproval,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FrontendToolApproval {
    pub kind: String,
    pub title: String,
    pub description: String,
    pub session_key: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FrontendToolResponse {
    pub request_id: String,
    pub approved: bool,
    pub result: Option<Value>,
    pub error: Option<String>,
}

impl FrontendToolApproval {
    pub fn none() -> Self {
        Self {
            kind: "none".to_string(),
            title: String::new(),
            description: String::new(),
            session_key: String::new(),
        }
    }

    pub fn mutating(
        title: impl Into<String>,
        description: impl Into<String>,
        session_key: impl Into<String>,
    ) -> Self {
        Self {
            kind: "mutating".to_string(),
            title: title.into(),
            description: description.into(),
            session_key: session_key.into(),
        }
    }

    pub fn execute(
        title: impl Into<String>,
        description: impl Into<String>,
        session_key: impl Into<String>,
    ) -> Self {
        Self {
            kind: "execute".to_string(),
            title: title.into(),
            description: description.into(),
            session_key: session_key.into(),
        }
    }
}

impl FrontendToolBridge {
    pub fn new(app_handle: AppHandle) -> Self {
        Self::new_with_event(app_handle, FLOWPILOT_FRONTEND_TOOL_EVENT)
    }

    /// Build a bridge that emits its requests on a dedicated event channel. Used by the global
    /// FlowPilot assistant so its tool requests are handled by its own listener instead of the
    /// board copilot's, while sharing the single `flowpilot_frontend_tool_result` response command.
    pub fn new_with_event(app_handle: AppHandle, event: impl Into<String>) -> Self {
        Self {
            app_handle,
            timeout: Duration::from_secs(600),
            event: event.into(),
        }
    }

    pub fn call(
        &self,
        tool_name: impl Into<String>,
        arguments: Value,
        approval: FrontendToolApproval,
    ) -> Value {
        self.call_with_timeout(tool_name, arguments, approval, self.timeout)
    }

    pub fn call_with_timeout(
        &self,
        tool_name: impl Into<String>,
        arguments: Value,
        approval: FrontendToolApproval,
        timeout: Duration,
    ) -> Value {
        let tool_name = tool_name.into();
        let request_id = next_request_id();
        let (tx, rx) = mpsc::channel();

        if let Ok(mut pending) = PENDING_RESPONSES.lock() {
            pending.insert(request_id.clone(), tx);
        } else {
            return json!({
                "status": "error",
                "error": "FlowPilot frontend tool bridge is unavailable."
            });
        }

        let request = FrontendToolRequest {
            request_id: request_id.clone(),
            tool_name: tool_name.clone(),
            arguments,
            approval,
        };

        let (event_tx, event_rx) = mpsc::channel();
        let emit_handle = self.app_handle.clone();
        let emit_request = request.clone();
        let event_name = self.event.clone();

        if let Err(error) = self.app_handle.run_on_main_thread(move || {
            let result = emit_handle
                .emit(&event_name, &emit_request)
                .map_err(|error| error.to_string());
            let _ = event_tx.send(result);
        }) {
            remove_pending_response(&request_id);
            return json!({
                "status": "error",
                "tool": tool_name,
                "error": format!("Failed to dispatch frontend tool request: {error}")
            });
        }

        match event_rx.recv_timeout(Duration::from_secs(30)) {
            Ok(Ok(())) => {}
            Ok(Err(error)) => {
                remove_pending_response(&request_id);
                return json!({
                    "status": "error",
                    "tool": tool_name,
                    "error": format!("Failed to request frontend tool execution: {error}")
                });
            }
            Err(_) => {
                remove_pending_response(&request_id);
                return json!({
                    "status": "timeout",
                    "tool": tool_name,
                    "message": "Timed out dispatching the FlowPilot frontend tool request."
                });
            }
        }

        match rx.recv_timeout(timeout) {
            Ok(response) => {
                if !response.approved {
                    return json!({
                        "status": "denied",
                        "tool": tool_name,
                        "message": response.error.unwrap_or_else(|| "User denied the frontend tool request.".to_string())
                    });
                }

                if let Some(error) = response.error {
                    return json!({
                        "status": "error",
                        "tool": tool_name,
                        "error": error
                    });
                }

                normalize_tool_result(response.result)
            }
            Err(_) => {
                remove_pending_response(&request_id);
                json!({
                    "status": "timeout",
                    "tool": tool_name,
                    "message": "Timed out waiting for the FlowPilot frontend tool response."
                })
            }
        }
    }
}

#[tauri::command]
pub fn flowpilot_frontend_tool_result(response: FrontendToolResponse) -> Result<(), String> {
    let request_id = response.request_id.clone();
    let sender = remove_pending_response(&request_id)
        .ok_or_else(|| format!("No pending FlowPilot frontend tool request '{request_id}'"))?;
    sender
        .send(response)
        .map_err(|_| "FlowPilot frontend tool requester is no longer waiting.".to_string())
}

fn normalize_tool_result(result: Option<Value>) -> Value {
    match result {
        Some(Value::Object(mut object)) => {
            object
                .entry("status".to_string())
                .or_insert_with(|| Value::String("ok".to_string()));
            Value::Object(object)
        }
        Some(value) => json!({
            "status": "ok",
            "result": value
        }),
        None => json!({ "status": "ok" }),
    }
}

fn remove_pending_response(request_id: &str) -> Option<Sender<FrontendToolResponse>> {
    PENDING_RESPONSES
        .lock()
        .ok()
        .and_then(|mut pending| pending.remove(request_id))
}

fn next_request_id() -> String {
    let counter = REQUEST_COUNTER.fetch_add(1, Ordering::Relaxed);
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or_default();
    format!("flowpilot-tool-{millis}-{counter}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_object_adds_status_without_overwriting() {
        let result = normalize_tool_result(Some(json!({ "value": 1 })));
        assert_eq!(result.get("status").and_then(Value::as_str), Some("ok"));

        let result = normalize_tool_result(Some(json!({ "status": "custom" })));
        assert_eq!(result.get("status").and_then(Value::as_str), Some("custom"));
    }

    #[test]
    fn normalize_scalar_wraps_result() {
        let result = normalize_tool_result(Some(json!("hello")));
        assert_eq!(result.get("status").and_then(Value::as_str), Some("ok"));
        assert_eq!(result.get("result").and_then(Value::as_str), Some("hello"));
    }
}
