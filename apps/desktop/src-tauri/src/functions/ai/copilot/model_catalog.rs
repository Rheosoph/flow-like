//! Codex and Claude model discovery and catalog parsing.

use super::backend_types::{CopilotModelInfo, ReasoningEffortOption};
use super::cli_auth::{read_bounded_process_stderr, stop_external_discovery_process};
use super::cli_resolution::{CliResolution, augmented_path_with_dirs};
use std::{process::Stdio, time::Duration};

/// Discover the Codex models available for the current authentication mode by
/// driving the installed `codex` CLI's `app-server` JSON-RPC protocol.
///
/// Codex model availability is auth-, policy-, and version-dependent, so the set
/// is read from Codex itself rather than hard-coded. Any failure (missing
/// `app-server` subcommand, unauthenticated session, timeout) is returned as an
/// actionable backend error; the frontend keeps its static default only as a
/// visibly degraded fallback.
pub(super) async fn list_codex_models_via_app_server(
    cli: &CliResolution,
) -> Result<Vec<CopilotModelInfo>, String> {
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt};

    let mut child = tokio::process::Command::new(&cli.executable)
        .arg("app-server")
        .env("PATH", augmented_path_with_dirs(&cli.path_dirs))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| format!("Failed to start codex app-server: {e}"))?;

    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| "codex app-server did not expose stdin".to_string())?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "codex app-server did not expose stdout".to_string())?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| "codex app-server did not expose stderr".to_string())?;
    let stderr_handle = tokio::spawn(read_bounded_process_stderr(stderr));

    // Newline-delimited JSON-RPC 2.0 (without the "jsonrpc" field), matching the
    // codex app-server framing: initialize -> initialized -> model/list.
    const MODEL_LIST_ID: i64 = 1;
    let messages = [
        serde_json::json!({
            "method": "initialize",
            "id": 0,
            "params": {
                "clientInfo": {
                    "name": "flow-like",
                    "title": "Flow-Like FlowPilot",
                    "version": env!("CARGO_PKG_VERSION"),
                }
            }
        }),
        serde_json::json!({ "method": "initialized", "params": {} }),
        serde_json::json!({
            "method": "model/list",
            "id": MODEL_LIST_ID,
            "params": { "limit": 100, "includeHidden": false }
        }),
    ];
    let mut payload = String::new();
    for message in &messages {
        payload.push_str(&message.to_string());
        payload.push('\n');
    }
    stdin
        .write_all(payload.as_bytes())
        .await
        .map_err(|e| format!("Failed to send codex app-server request: {e}"))?;
    stdin
        .flush()
        .await
        .map_err(|e| format!("Failed to flush codex app-server request: {e}"))?;

    let read_models = async {
        let mut lines = tokio::io::BufReader::new(stdout).lines();
        while let Some(line) = lines
            .next_line()
            .await
            .map_err(|e| format!("Failed to read codex app-server output: {e}"))?
        {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            let Ok(value) = serde_json::from_str::<serde_json::Value>(trimmed) else {
                continue;
            };
            if value.get("id").and_then(serde_json::Value::as_i64) != Some(MODEL_LIST_ID) {
                continue;
            }
            if let Some(error) = value.get("error") {
                return Err(format!("codex app-server model/list failed: {error}"));
            }
            let entries = value
                .get("result")
                .and_then(|result| result.get("data"))
                .and_then(serde_json::Value::as_array)
                .cloned()
                .ok_or_else(|| {
                    "codex app-server protocol error: model/list response omitted the result.data array"
                        .to_string()
                })?;
            let models = parse_codex_model_catalog(&entries);
            if models.is_empty() {
                return Err(
                    "Codex model unavailable: model/list returned no usable model entries"
                        .to_string(),
                );
            }
            return Ok(models);
        }
        Err("codex app-server closed before returning models".to_string())
    };

    let outcome = tokio::time::timeout(Duration::from_secs(7), read_models).await;
    let stderr = stop_external_discovery_process(&mut child, stderr_handle).await;
    let result = match outcome {
        Ok(result) => result,
        Err(_) => Err("codex app-server model listing timed out".to_string()),
    };
    result.map_err(|error| {
        if stderr.is_empty() {
            error
        } else {
            format!("{error}: {stderr}")
        }
    })
}

pub(super) fn reasoning_effort_display_name(id: &str) -> String {
    match id.trim().to_ascii_lowercase().as_str() {
        "xhigh" => "Extra high".to_string(),
        value => value
            .split(['-', '_'])
            .filter(|part| !part.is_empty())
            .map(|part| {
                let mut chars = part.chars();
                match chars.next() {
                    Some(first) => first.to_uppercase().chain(chars).collect::<String>(),
                    None => String::new(),
                }
            })
            .collect::<Vec<_>>()
            .join(" "),
    }
}

fn parse_reasoning_effort_options(value: Option<&serde_json::Value>) -> Vec<ReasoningEffortOption> {
    let Some(entries) = value.and_then(serde_json::Value::as_array) else {
        return Vec::new();
    };

    let mut options = Vec::new();
    for entry in entries {
        let (id, name, description) = match entry {
            serde_json::Value::String(id) => {
                let id = id.trim();
                if id.is_empty() {
                    continue;
                }
                (id.to_string(), reasoning_effort_display_name(id), None)
            }
            serde_json::Value::Object(object) => {
                let Some(id) = object
                    .get("reasoningEffort")
                    .or_else(|| object.get("id"))
                    .or_else(|| object.get("value"))
                    .and_then(serde_json::Value::as_str)
                    .map(str::trim)
                    .filter(|id| !id.is_empty())
                else {
                    continue;
                };
                let name = object
                    .get("name")
                    .or_else(|| object.get("displayName"))
                    .and_then(serde_json::Value::as_str)
                    .map(str::trim)
                    .filter(|name| !name.is_empty())
                    .map(str::to_string)
                    .unwrap_or_else(|| reasoning_effort_display_name(id));
                let description = object
                    .get("description")
                    .and_then(serde_json::Value::as_str)
                    .map(str::trim)
                    .filter(|description| !description.is_empty())
                    .map(str::to_string);
                (id.to_string(), name, description)
            }
            _ => continue,
        };

        if options
            .iter()
            .any(|existing: &ReasoningEffortOption| existing.id == id)
        {
            continue;
        }
        options.push(ReasoningEffortOption {
            id,
            name,
            description,
        });
    }
    options
}

fn optional_non_empty_string(value: Option<&serde_json::Value>) -> Option<String> {
    value
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn model_context_window(entry: &serde_json::Value) -> Option<u64> {
    // Catalog protocols may omit limits. Do not infer one from the model name.
    entry
        .get("contextWindow")
        .and_then(serde_json::Value::as_u64)
        .filter(|limit| *limit > 0)
}

fn model_supports_vision(entry: &serde_json::Value) -> Option<bool> {
    entry
        .get("supportsVision")
        .and_then(serde_json::Value::as_bool)
        .or_else(|| {
            let modalities = entry.get("inputModalities")?.as_array()?;
            let modalities = modalities
                .iter()
                .map(serde_json::Value::as_str)
                .collect::<Option<Vec<_>>>()?;
            Some(modalities.contains(&"image"))
        })
}

/// Convert a `model/list` `data` array into FlowPilot model options, skipping
/// hidden entries and preserving Codex's ordering (recommended model first).
pub(super) fn parse_codex_model_catalog(entries: &[serde_json::Value]) -> Vec<CopilotModelInfo> {
    let mut models = Vec::new();
    for entry in entries {
        if entry
            .get("hidden")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false)
        {
            continue;
        }
        let Some(id) = entry
            .get("id")
            .or_else(|| entry.get("model"))
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|id| !id.is_empty())
            .map(str::to_string)
        else {
            continue;
        };
        let name = entry
            .get("displayName")
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .unwrap_or(id.as_str())
            .to_string();
        models.push(CopilotModelInfo {
            id,
            name,
            description: optional_non_empty_string(entry.get("description")),
            context_window: model_context_window(entry),
            supports_vision: model_supports_vision(entry),
            supported_reasoning_efforts: parse_reasoning_effort_options(
                entry.get("supportedReasoningEfforts"),
            ),
            default_reasoning_effort: optional_non_empty_string(
                entry.get("defaultReasoningEffort"),
            ),
            is_default: entry
                .get("isDefault")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false),
        });
    }
    models
}

pub(super) fn codex_models_with_configured_default(
    discovered: Vec<CopilotModelInfo>,
) -> Vec<CopilotModelInfo> {
    let mut configured_default = CopilotModelInfo::basic("default", "Codex configured default");
    configured_default.is_default = true;
    if let Some(runtime_default) = discovered.iter().find(|model| model.is_default) {
        configured_default.description = runtime_default.description.clone();
        configured_default.context_window = runtime_default.context_window;
        configured_default.supports_vision = runtime_default.supports_vision;
        configured_default.supported_reasoning_efforts =
            runtime_default.supported_reasoning_efforts.clone();
        configured_default.default_reasoning_effort =
            runtime_default.default_reasoning_effort.clone();
    }

    let mut models = vec![configured_default];
    for model in discovered {
        if model.id != "default" && !models.iter().any(|existing| existing.id == model.id) {
            models.push(model);
        }
    }
    models
}

/// Discover the Claude Code models available for the current authentication by
/// driving the CLI's stream-json control protocol — the same `initialize`
/// handshake the Agent SDK's `supportedModels()` reads. Claude Code has no
/// model-listing subcommand, so this is the only auth-aware, version-current
/// source; nothing about the model set is hard-coded. Any failure (CLI missing,
/// unauthenticated, protocol change, timeout) surfaces as an error and the
/// frontend keeps its static default only as a visibly degraded fallback.
pub(super) async fn list_claude_models_via_control_protocol(
    cli: &CliResolution,
) -> Result<Vec<CopilotModelInfo>, String> {
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt};

    // A neutral cwd keeps the handshake from triggering workspace-trust or
    // CLAUDE.md discovery for the user's project; the model set only depends on
    // account auth (read from the keychain), not the working directory.
    let mut child = tokio::process::Command::new(&cli.executable)
        .args([
            "-p",
            "--output-format",
            "stream-json",
            "--verbose",
            "--input-format",
            "stream-json",
        ])
        .current_dir(std::env::temp_dir())
        .env("PATH", augmented_path_with_dirs(&cli.path_dirs))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| format!("Failed to start claude control session: {e}"))?;

    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| "claude control session did not expose stdin".to_string())?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "claude control session did not expose stdout".to_string())?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| "claude control session did not expose stderr".to_string())?;
    let stderr_handle = tokio::spawn(read_bounded_process_stderr(stderr));

    // Newline-delimited control protocol: send one `initialize` control_request;
    // the success control_response carries the model catalog at
    // `response.response.models`.
    let request = serde_json::json!({
        "request_id": "flowpilot-model-list",
        "type": "control_request",
        "request": { "subtype": "initialize" }
    });
    stdin
        .write_all(format!("{request}\n").as_bytes())
        .await
        .map_err(|e| format!("Failed to send claude initialize request: {e}"))?;
    stdin
        .flush()
        .await
        .map_err(|e| format!("Failed to flush claude initialize request: {e}"))?;

    let read_models = async {
        let mut lines = tokio::io::BufReader::new(stdout).lines();
        while let Some(line) = lines
            .next_line()
            .await
            .map_err(|e| format!("Failed to read claude control output: {e}"))?
        {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            let Ok(value) = serde_json::from_str::<serde_json::Value>(trimmed) else {
                continue;
            };
            if value.get("type").and_then(serde_json::Value::as_str) != Some("control_response") {
                continue;
            }
            let response = value.get("response");
            if response
                .and_then(|response| response.get("subtype"))
                .and_then(serde_json::Value::as_str)
                == Some("error")
            {
                let message = response
                    .and_then(|response| response.get("error"))
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("unknown error");
                return Err(format!("claude initialize failed: {message}"));
            }
            let entries = response
                .and_then(|response| response.get("response"))
                .and_then(|inner| inner.get("models"))
                .and_then(serde_json::Value::as_array)
                .cloned()
                .ok_or_else(|| {
                    "claude control protocol error: initialize response omitted the models array"
                        .to_string()
                })?;
            let models = parse_claude_model_catalog(&entries);
            if models.is_empty() {
                return Err(
                    "Claude Code model unavailable: initialize returned no usable model entries"
                        .to_string(),
                );
            }
            return Ok(models);
        }
        Err("claude control session closed before returning models".to_string())
    };

    let outcome = tokio::time::timeout(Duration::from_secs(11), read_models).await;
    let stderr = stop_external_discovery_process(&mut child, stderr_handle).await;
    let result = match outcome {
        Ok(result) => result,
        Err(_) => Err("claude model listing timed out".to_string()),
    };
    result.map_err(|error| {
        if stderr.is_empty() {
            error
        } else {
            format!("{error}: {stderr}")
        }
    })
}

/// Convert the Claude Code `initialize` handshake's `models` array into FlowPilot
/// model options. `value` is the id passed to `--model`; `displayName` is shown
/// to the user (falling back to the value), preserving the CLI's ordering.
pub(super) fn parse_claude_model_catalog(entries: &[serde_json::Value]) -> Vec<CopilotModelInfo> {
    let mut models = Vec::new();
    for entry in entries {
        let Some(id) = entry
            .get("value")
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
        else {
            continue;
        };
        if models
            .iter()
            .any(|existing: &CopilotModelInfo| existing.id == id)
        {
            continue;
        }
        let name = entry
            .get("displayName")
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .unwrap_or(id.as_str())
            .to_string();
        let supports_effort = entry
            .get("supportsEffort")
            .and_then(serde_json::Value::as_bool);
        let supported_reasoning_efforts = if supports_effort == Some(false) {
            Vec::new()
        } else {
            parse_reasoning_effort_options(entry.get("supportedEffortLevels"))
        };
        models.push(CopilotModelInfo {
            is_default: entry
                .get("isDefault")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(id == "default"),
            id,
            name,
            description: optional_non_empty_string(entry.get("description")),
            context_window: model_context_window(entry),
            supports_vision: model_supports_vision(entry),
            supported_reasoning_efforts,
            default_reasoning_effort: optional_non_empty_string(
                entry
                    .get("defaultReasoningEffort")
                    .or_else(|| entry.get("defaultEffortLevel")),
            ),
        });
    }
    models
}
