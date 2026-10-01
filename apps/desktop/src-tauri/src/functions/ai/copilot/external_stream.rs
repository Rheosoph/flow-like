//! Codex and Claude stream parsing and progress translation.

use super::backend_types::FlowPilotAgentBackendKind;
use super::runtime::{EXTERNAL_AGENT_MESSAGE_STATE_MAX_ENTRIES, EXTERNAL_AGENT_TEXT_MAX_BYTES};
use super::stream_events::{
    append_bounded_text, extract_json_status, flowscript_workspace_result_payload,
    send_correlated_stream_json_event,
};
use super::workflow_sdk::{full_redacted_tool_result, is_flowscript_draft_operation_tool};
use std::collections::HashMap;
use tauri::ipc::Channel;

#[derive(Default)]
pub(super) struct ExternalAgentStreamState {
    pub(super) session_id: Option<String>,
    pub(super) agent_message_text_by_id: HashMap<String, String>,
    pub(super) last_agent_message_id: Option<String>,
    pub(super) has_streamed_assistant_text: bool,
    // Claude reports a tool's name only on the `tool_use` block; the matching
    // `tool_result` carries just the id, so remember id -> display name here.
    pub(super) claude_tool_names: HashMap<String, String>,
    // Accumulated extended-thinking text; re-framed as one upserted plan step.
    pub(super) claude_thinking: String,
    // Claude streams tool JSON by content-block index before it emits the complete assistant
    // message. Keep that transient index -> call-id mapping so FlowScript can appear while the
    // model is still writing the `source` JSON string.
    pub(super) claude_tool_call_ids_by_index: HashMap<u64, String>,
    pub(super) claude_flowscript_preview:
        flow_like::flow::copilot::stream::FlowScriptToolCallPreviewTracker,
}

impl ExternalAgentStreamState {
    pub(super) fn observe_session(
        &mut self,
        backend: FlowPilotAgentBackendKind,
        value: &serde_json::Value,
    ) {
        let session = match backend {
            FlowPilotAgentBackendKind::Codex
                if value.get("type").and_then(serde_json::Value::as_str)
                    == Some("thread.started") =>
            {
                value.get("thread_id")
            }
            FlowPilotAgentBackendKind::ClaudeCode => value.get("session_id"),
            _ => None,
        };
        if let Some(session) = session
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|session| !session.is_empty())
        {
            self.session_id = Some(session.to_string());
        }
    }

    pub(super) fn decorate_agent_delta(&mut self, item_id: &str, delta: &str) -> String {
        if delta.is_empty() {
            return String::new();
        }

        let mut out = String::new();
        if self.has_streamed_assistant_text
            && self.last_agent_message_id.as_deref() != Some(item_id)
            && !delta.starts_with('\n')
        {
            out.push_str("\n\n");
        }
        self.last_agent_message_id = Some(item_id.to_string());
        self.has_streamed_assistant_text = true;
        out.push_str(delta);
        out
    }
}

fn external_debug_value_preview(value: &serde_json::Value, max_chars: usize) -> String {
    match value {
        serde_json::Value::String(text) => match serde_json::from_str::<serde_json::Value>(text) {
            Ok(parsed) => flow_like::flow::copilot::stream::safe_json_preview(&parsed, max_chars),
            Err(_) => flow_like::flow::copilot::stream::safe_text_preview(text, max_chars),
        },
        value => flow_like::flow::copilot::stream::safe_json_preview(value, max_chars),
    }
}

fn external_tool_result_text(value: &serde_json::Value) -> Option<String> {
    if let Some(text) = value.as_str() {
        return Some(text.to_string());
    }
    if let Some(text) = value.get("text").and_then(serde_json::Value::as_str) {
        return Some(text.to_string());
    }
    if let Some(content) = value.get("content") {
        if let Some(text) = content.as_str() {
            return Some(text.to_string());
        }
        if let Some(entries) = content.as_array() {
            let text = entries
                .iter()
                .filter_map(|entry| {
                    entry
                        .get("text")
                        .or_else(|| entry.get("content"))
                        .and_then(serde_json::Value::as_str)
                })
                .collect::<Vec<_>>()
                .join("\n");
            if !text.is_empty() {
                return Some(text);
            }
        }
    }
    None
}

pub(super) fn external_result_details(
    result: Option<&serde_json::Value>,
    fallback_status: Option<&str>,
    error: Option<&str>,
) -> (String, String, String, Option<String>) {
    let result_text = result.and_then(external_tool_result_text);
    let terminal_status = result_text
        .as_deref()
        .and_then(extract_json_status)
        .or_else(|| fallback_status.map(str::to_string))
        .unwrap_or_else(|| {
            if error.is_some() {
                "error"
            } else {
                "completed"
            }
            .to_string()
        });
    // Provider event envelopes and direct SDK results carry different status vocabularies. Route
    // both through the core classifier instead of maintaining another success allowlist here:
    // accepted plans and advisory redirects are completed tool calls, while explicit error flags
    // and known-negative terminal statuses remain errors.
    let status = if error.is_some() {
        "error".to_string()
    } else if let Some(result_text) = result_text
        .as_deref()
        .filter(|text| serde_json::from_str::<serde_json::Value>(text).is_ok())
    {
        flow_like::flow::copilot::stream::tool_result_stream_status(result_text).to_string()
    } else {
        flow_like::flow::copilot::stream::tool_result_stream_status(
            &serde_json::json!({ "status": &terminal_status }).to_string(),
        )
        .to_string()
    };
    let result_summary = error
        .map(|error| flow_like::flow::copilot::stream::safe_text_preview(error, 600))
        .unwrap_or_else(|| {
            result_text
                .as_deref()
                .filter(|text| serde_json::from_str::<serde_json::Value>(text).is_ok())
                .map(flow_like::flow::copilot::stream::tool_result_summary)
                .unwrap_or_else(|| terminal_status.replace('_', " "))
        });
    let result_preview = result_text
        .as_deref()
        .map(|text| {
            flow_like::flow::copilot::stream::safe_tool_result_preview(
                text,
                flow_like::flow::copilot::stream::TOOL_RESULT_PREVIEW_CHARS,
            )
        })
        .or_else(|| {
            result.map(|value| {
                external_debug_value_preview(
                    value,
                    flow_like::flow::copilot::stream::TOOL_RESULT_PREVIEW_CHARS,
                )
            })
        });
    (status, terminal_status, result_summary, result_preview)
}

pub(super) fn external_agent_process_event(value: &serde_json::Value) -> Option<String> {
    let event_type = value
        .get("type")
        .or_else(|| value.get("event"))
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();

    if !matches!(event_type, "item.started" | "item.completed") {
        return None;
    }

    let item = value.get("item")?;
    let item_type = item
        .get("type")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();
    if item_type != "mcp_tool_call" {
        return None;
    }

    let tool_name = item
        .get("tool")
        .or_else(|| item.get("name"))
        .and_then(serde_json::Value::as_str)
        .unwrap_or("tool");
    let server_name = item
        .get("server")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("flowpilot");
    let tool_call_id = item
        .get("id")
        .or_else(|| value.get("id"))
        .and_then(serde_json::Value::as_str)
        .map(str::to_string)
        .unwrap_or_else(|| format!("external-{server_name}-{tool_name}"));

    if event_type == "item.started" {
        let arguments_preview =
            item.get("arguments")
                .or_else(|| item.get("input"))
                .map(|arguments| {
                    external_debug_value_preview(
                        arguments,
                        flow_like::flow::copilot::stream::TOOL_ARGUMENT_PREVIEW_CHARS,
                    )
                });
        return Some(flowpilot_stream_tag(
            "tool_start",
            &serde_json::json!({
                "tool_call_id": tool_call_id,
                "tool": tool_name,
                "status": "running",
                "summary": format!("{server_name}/{tool_name}"),
                "arguments_preview": arguments_preview,
            }),
        ));
    }

    let error = item
        .pointer("/error/message")
        .and_then(serde_json::Value::as_str)
        .map(str::to_string);
    let fallback_status = item.get("status").and_then(serde_json::Value::as_str);
    let result_value = item.get("result").or_else(|| item.get("output"));
    let (status, terminal_status, result_summary, result_preview) =
        external_result_details(result_value, fallback_status, error.as_deref());
    // Compiler-receipt evidence needs the full authored source: an 8KB preview truncates large
    // commit results (source + derived commands) mid-document and the captured "authored
    // FlowScript" fails validation over an ellipsis the model never wrote. Redaction still
    // applies — only truncation is lifted.
    let full_result = is_flowscript_draft_operation_tool(tool_name)
        .then(|| {
            result_value
                .and_then(external_tool_result_text)
                .map(|text| full_redacted_tool_result(&text))
        })
        .flatten();
    Some(flowpilot_stream_tag(
        "tool_end",
        &serde_json::json!({
            "tool_call_id": tool_call_id,
            "tool": tool_name,
            "status": status,
            "terminal_status": terminal_status,
            "result_summary": result_summary,
            "result_preview": result_preview,
            "result": full_result,
            "error": error.map(|error| flow_like::flow::copilot::stream::safe_text_preview(&error, 600)),
        }),
    ))
}

/// Detect a failed FlowPilot MCP server connection in Claude Code's `system`/`init` frame
/// (`{"type":"system","subtype":"init","mcp_servers":[{"name":…,"status":…}]}`).
pub(super) fn external_agent_mcp_connect_failure(value: &serde_json::Value) -> Option<String> {
    if value.get("type").and_then(serde_json::Value::as_str) != Some("system")
        || value.get("subtype").and_then(serde_json::Value::as_str) != Some("init")
    {
        return None;
    }
    let servers = value.get("mcp_servers")?.as_array()?;
    let failed: Vec<String> = servers
        .iter()
        .filter_map(|server| {
            let name = server.get("name").and_then(serde_json::Value::as_str)?;
            let status = server
                .get("status")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("unknown");
            matches!(status, "failed" | "error" | "disconnected")
                .then(|| format!("`{name}` (status: {status})"))
        })
        .collect();
    if failed.is_empty() {
        return None;
    }
    Some(format!(
        "MCP server connection failed: {} — the FlowPilot tools are unavailable, so this run cannot edit the board or UI",
        failed.join(", ")
    ))
}

/// Translate a Codex full-source authoring `mcp_tool_call` item into a workspace preview frame.
/// The frontend treats this `submitted` preview as non-authoritative; only the later `queued`
/// commit workspace owns application and command suppression.
#[cfg_attr(not(test), allow(dead_code))]
pub(super) fn external_agent_flowscript_workspace_event(
    value: &serde_json::Value,
) -> Option<String> {
    let event_type = value
        .get("type")
        .or_else(|| value.get("event"))
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();
    if !matches!(
        event_type,
        "item.started" | "item.updated" | "item.completed"
    ) {
        return None;
    }

    let item = value.get("item")?;
    let item_type = item
        .get("type")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();
    if item_type != "mcp_tool_call" {
        return None;
    }

    let tool_name = item
        .get("tool")
        .or_else(|| item.get("name"))
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();
    let display_tool_name = claude_display_tool_name(tool_name);

    // Completion is authoritative: source lifecycle results contain the exact retained document,
    // revision and compiler status. Prefer it over the repeated call arguments on item.completed.
    if event_type == "item.completed" && is_flowscript_draft_operation_tool(display_tool_name) {
        let result = item.get("result").or_else(|| item.get("output"));
        if let Some(result_text) = result.and_then(external_tool_result_text)
            && let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&result_text)
            && let Some(mut payload) =
                flowscript_workspace_result_payload(display_tool_name, &parsed, None)
        {
            if let Some(id) = item.get("id").and_then(serde_json::Value::as_str)
                && let Some(object) = payload.as_object_mut()
            {
                object.insert(
                    "tool_call_id".to_string(),
                    serde_json::Value::String(id.to_string()),
                );
            }
            return Some(flowpilot_stream_tag("flowscript_workspace", &payload));
        }
    }

    if !is_flowscript_authoring_tool_name(display_tool_name) {
        return None;
    }

    let arguments = external_agent_tool_arguments(item)?;
    let flowscript = extract_flowscript_source_from_tool_arguments(&arguments)?;
    if flowscript.trim().is_empty() {
        return None;
    }

    Some(flowpilot_stream_tag(
        "flowscript_workspace",
        &serde_json::json!({
            "source": flowscript,
            "status": "submitted",
        }),
    ))
}

fn is_flowscript_authoring_tool_name(tool_name: &str) -> bool {
    flow_like::flow::copilot::stream::is_flowscript_authoring_tool(tool_name)
}

fn external_agent_tool_arguments(item: &serde_json::Value) -> Option<serde_json::Value> {
    for key in ["arguments", "args", "input", "params", "parameters"] {
        if let Some(value) = item.get(key)
            && let Some(arguments) = normalize_external_tool_arguments(value)
        {
            return Some(arguments);
        }
    }

    for pointer in [
        "/call/arguments",
        "/function/arguments",
        "/request/arguments",
        "/tool_call/arguments",
    ] {
        if let Some(value) = item.pointer(pointer)
            && let Some(arguments) = normalize_external_tool_arguments(value)
        {
            return Some(arguments);
        }
    }

    None
}

fn normalize_external_tool_arguments(value: &serde_json::Value) -> Option<serde_json::Value> {
    match value {
        serde_json::Value::Null => None,
        serde_json::Value::String(text) => {
            let trimmed = text.trim();
            if trimmed.is_empty() {
                return None;
            }
            serde_json::from_str::<serde_json::Value>(trimmed)
                .ok()
                .or_else(|| Some(serde_json::Value::String(text.clone())))
        }
        _ => Some(value.clone()),
    }
}

fn extract_flowscript_source_from_tool_arguments(value: &serde_json::Value) -> Option<String> {
    extract_flowscript_source_from_tool_arguments_inner(value, 0)
}

fn extract_flowscript_source_from_tool_arguments_inner(
    value: &serde_json::Value,
    depth: u8,
) -> Option<String> {
    if depth > 4 {
        return None;
    }

    match value {
        serde_json::Value::Object(map) => {
            for key in ["flowscript", "script", "source", "content"] {
                if let Some(source) = map.get(key).and_then(serde_json::Value::as_str)
                    && !source.trim().is_empty()
                {
                    return Some(source.to_string());
                }
            }

            for key in ["arguments", "args", "input", "params", "parameters"] {
                if let Some(nested) = map.get(key)
                    && let Some(source) =
                        extract_flowscript_source_from_tool_arguments_inner(nested, depth + 1)
                {
                    return Some(source);
                }
            }

            None
        }
        serde_json::Value::String(text) => {
            let trimmed = text.trim();
            if trimmed.is_empty() {
                return None;
            }
            if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(trimmed) {
                return extract_flowscript_source_from_tool_arguments_inner(&parsed, depth + 1);
            }
            Some(text.clone())
        }
        _ => None,
    }
}

pub(super) fn external_agent_stream_delta(
    backend: FlowPilotAgentBackendKind,
    value: &serde_json::Value,
    state: &mut ExternalAgentStreamState,
) -> Option<String> {
    match backend {
        FlowPilotAgentBackendKind::Codex => codex_agent_message_delta(value, state),
        FlowPilotAgentBackendKind::ClaudeCode => claude_agent_message_delta(value, state),
        FlowPilotAgentBackendKind::GithubCopilot => None,
    }
}

/// Surfaces Claude Code extended thinking as an upserted `<plan_step>` frame so
/// the reasoning box streams live instead of being dropped. Text deltas keep
/// riding `external_agent_stream_delta`; this only handles `thinking_delta`.
pub(super) fn external_agent_reasoning_frame(
    backend: FlowPilotAgentBackendKind,
    value: &serde_json::Value,
    state: &mut ExternalAgentStreamState,
) -> Option<String> {
    if backend != FlowPilotAgentBackendKind::ClaudeCode {
        return None;
    }
    if value.get("type").and_then(serde_json::Value::as_str) != Some("stream_event") {
        return None;
    }
    let event = value.get("event")?;
    if event.get("type").and_then(serde_json::Value::as_str) != Some("content_block_delta") {
        return None;
    }
    let delta = event.get("delta")?;
    if delta.get("type").and_then(serde_json::Value::as_str) != Some("thinking_delta") {
        return None;
    }
    let thinking = delta.get("thinking").and_then(serde_json::Value::as_str)?;
    if thinking.is_empty() {
        return None;
    }
    append_bounded_text(
        &mut state.claude_thinking,
        thinking,
        EXTERNAL_AGENT_TEXT_MAX_BYTES,
    );
    Some(flow_like::flow::copilot::stream::plan_step_frame(
        "claude-thinking".to_string(),
        state.claude_thinking.trim().to_string(),
        flow_like::flow::copilot::PlanStepStatus::InProgress,
        "think",
    ))
}

pub(super) fn codex_agent_message_delta(
    value: &serde_json::Value,
    state: &mut ExternalAgentStreamState,
) -> Option<String> {
    let event_type = value
        .get("type")
        .or_else(|| value.get("event"))
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();

    if matches!(
        event_type,
        "agent_message_delta" | "assistant_message_delta"
    ) {
        let item_id = value
            .get("item_id")
            .or_else(|| value.get("itemId"))
            .or_else(|| value.get("id"))
            .and_then(serde_json::Value::as_str)
            .unwrap_or("codex-agent-message");
        let delta = value
            .get("delta")
            .or_else(|| value.pointer("/item/delta"))
            .or_else(|| value.get("text"))
            .and_then(serde_json::Value::as_str)?;
        // Record what was streamed so the terminal item.completed (which
        // carries the full text) diffs against it instead of re-emitting the
        // whole message a second time.
        if state.agent_message_text_by_id.len() >= EXTERNAL_AGENT_MESSAGE_STATE_MAX_ENTRIES
            && !state.agent_message_text_by_id.contains_key(item_id)
        {
            state.agent_message_text_by_id.clear();
        }
        {
            let previous = state
                .agent_message_text_by_id
                .entry(item_id.to_string())
                .or_default();
            append_bounded_text(previous, delta, EXTERNAL_AGENT_TEXT_MAX_BYTES);
        }
        return Some(state.decorate_agent_delta(item_id, delta));
    }

    if !matches!(
        event_type,
        "item.started" | "item.updated" | "item.completed"
    ) {
        return None;
    }

    let item = value.get("item")?;
    let item_type = item
        .get("type")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();
    if !matches!(item_type, "agent_message" | "assistant_message") {
        return None;
    }

    let item_id = item
        .get("id")
        .or_else(|| value.get("id"))
        .and_then(serde_json::Value::as_str)
        .unwrap_or("codex-agent-message");

    if let Some(delta) = item.get("delta").and_then(serde_json::Value::as_str) {
        if state.agent_message_text_by_id.len() >= EXTERNAL_AGENT_MESSAGE_STATE_MAX_ENTRIES
            && !state.agent_message_text_by_id.contains_key(item_id)
        {
            state.agent_message_text_by_id.clear();
        }
        {
            let previous = state
                .agent_message_text_by_id
                .entry(item_id.to_string())
                .or_default();
            append_bounded_text(previous, delta, EXTERNAL_AGENT_TEXT_MAX_BYTES);
        }
        return Some(state.decorate_agent_delta(item_id, delta));
    }

    let full_text = item.get("text").and_then(serde_json::Value::as_str)?;
    if state.agent_message_text_by_id.len() >= EXTERNAL_AGENT_MESSAGE_STATE_MAX_ENTRIES
        && !state.agent_message_text_by_id.contains_key(item_id)
    {
        state.agent_message_text_by_id.clear();
    }
    let delta = {
        let previous = state
            .agent_message_text_by_id
            .entry(item_id.to_string())
            .or_default();
        let delta = if full_text.starts_with(previous.as_str()) {
            full_text[previous.len()..].to_string()
        } else if previous.is_empty() {
            full_text.to_string()
        } else {
            String::new()
        };
        previous.clear();
        append_bounded_text(previous, full_text, EXTERNAL_AGENT_TEXT_MAX_BYTES);
        delta
    };

    if event_type == "item.completed" {
        state.agent_message_text_by_id.remove(item_id);
    }

    Some(state.decorate_agent_delta(item_id, &delta))
}

pub(super) fn claude_agent_message_delta(
    value: &serde_json::Value,
    state: &mut ExternalAgentStreamState,
) -> Option<String> {
    // Claude Code (with --include-partial-messages) streams assistant tokens as
    // `stream_event` frames wrapping a content_block_delta / text_delta. The full
    // `assistant` message and the final `result` event are handled elsewhere, so
    // only the incremental text deltas are emitted here to avoid duplication.
    if value.get("type").and_then(serde_json::Value::as_str) != Some("stream_event") {
        return None;
    }
    let event = value.get("event")?;
    if event.get("type").and_then(serde_json::Value::as_str) != Some("content_block_delta") {
        return None;
    }
    let delta = event.get("delta")?;
    if delta.get("type").and_then(serde_json::Value::as_str) != Some("text_delta") {
        return None;
    }
    let text = delta.get("text").and_then(serde_json::Value::as_str)?;
    if text.is_empty() {
        return None;
    }

    Some(state.decorate_agent_delta("claude-agent-message", text))
}

/// Strip the `mcp__<server>__` prefix Claude uses for MCP tools so the frontend
/// tool labeller recognizes the bare FlowPilot tool name (e.g. `edit_flowscript`).
fn claude_display_tool_name(name: &str) -> &str {
    name.strip_prefix("mcp__")
        .and_then(|rest| rest.split_once("__"))
        .map(|(_, tool)| tool)
        .unwrap_or(name)
}

/// Decode Claude's streamed `input_json_delta` fragments into live FlowScript workspace prefixes.
/// The later complete `assistant/tool_use` block remains authoritative and emits `submitted`.
fn claude_partial_tool_input_events(
    value: &serde_json::Value,
    state: &mut ExternalAgentStreamState,
) -> Vec<String> {
    let Some(event) = value.get("event") else {
        return Vec::new();
    };
    let event_type = event
        .get("type")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();
    let index = event.get("index").and_then(serde_json::Value::as_u64);

    match event_type {
        "content_block_start" => {
            let Some(block) = event.get("content_block") else {
                return Vec::new();
            };
            if block.get("type").and_then(serde_json::Value::as_str) != Some("tool_use") {
                return Vec::new();
            }
            let Some(id) = block.get("id").and_then(serde_json::Value::as_str) else {
                return Vec::new();
            };
            let name = block
                .get("name")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("tool");
            if let Some(index) = index {
                if state.claude_tool_call_ids_by_index.len()
                    >= EXTERNAL_AGENT_MESSAGE_STATE_MAX_ENTRIES
                    && !state.claude_tool_call_ids_by_index.contains_key(&index)
                {
                    state.claude_tool_call_ids_by_index.clear();
                }
                state
                    .claude_tool_call_ids_by_index
                    .insert(index, id.to_string());
            }
            state.claude_flowscript_preview.observe_name(id, name);
            Vec::new()
        }
        "content_block_delta" => {
            let Some(index) = index else {
                return Vec::new();
            };
            let Some(id) = state.claude_tool_call_ids_by_index.get(&index).cloned() else {
                return Vec::new();
            };
            let Some(delta) = event.get("delta") else {
                return Vec::new();
            };
            if delta.get("type").and_then(serde_json::Value::as_str) != Some("input_json_delta") {
                return Vec::new();
            }
            delta
                .get("partial_json")
                .and_then(serde_json::Value::as_str)
                .and_then(|partial| {
                    state
                        .claude_flowscript_preview
                        .observe_arguments_delta(&id, partial)
                })
                .into_iter()
                .collect()
        }
        "content_block_stop" => {
            if let Some(index) = index {
                state.claude_tool_call_ids_by_index.remove(&index);
            }
            Vec::new()
        }
        _ => Vec::new(),
    }
}

/// Surface Claude Code's tool activity as FlowPilot `tool_start`/`tool_end`
/// frames. Claude reports tool calls as `tool_use` blocks inside an `assistant`
/// message and their outcomes as `tool_result` blocks in the following `user`
/// message — unlike Codex's `item.*` events, so it needs its own extractor.
pub(super) fn claude_agent_tool_events(
    value: &serde_json::Value,
    state: &mut ExternalAgentStreamState,
) -> Vec<String> {
    let event_type = value
        .get("type")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();
    if event_type == "stream_event" {
        return claude_partial_tool_input_events(value, state);
    }
    let Some(blocks) = value
        .pointer("/message/content")
        .and_then(serde_json::Value::as_array)
    else {
        return Vec::new();
    };

    let mut events = Vec::new();
    match event_type {
        "assistant" => {
            for block in blocks {
                if block.get("type").and_then(serde_json::Value::as_str) != Some("tool_use") {
                    continue;
                }
                let Some(id) = block.get("id").and_then(serde_json::Value::as_str) else {
                    continue;
                };
                let name = claude_display_tool_name(
                    block
                        .get("name")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or("tool"),
                )
                .to_string();
                if state.claude_tool_names.len() >= EXTERNAL_AGENT_MESSAGE_STATE_MAX_ENTRIES
                    && !state.claude_tool_names.contains_key(id)
                {
                    state.claude_tool_names.clear();
                }
                state.claude_tool_names.insert(id.to_string(), name.clone());
                if let Some(arguments) = block.get("input").or_else(|| block.get("arguments"))
                    && let Some(frame) = state
                        .claude_flowscript_preview
                        .complete(id, &name, arguments)
                {
                    events.push(frame);
                }
                let arguments_preview =
                    block
                        .get("input")
                        .or_else(|| block.get("arguments"))
                        .map(|arguments| {
                            external_debug_value_preview(
                                arguments,
                                flow_like::flow::copilot::stream::TOOL_ARGUMENT_PREVIEW_CHARS,
                            )
                        });
                events.push(flowpilot_stream_tag(
                    "tool_start",
                    &serde_json::json!({
                        "tool_call_id": id,
                        "tool": name,
                        "status": "running",
                        "summary": format!("flowpilot/{name}"),
                        "arguments_preview": arguments_preview,
                    }),
                ));
            }
        }
        "user" => {
            for block in blocks {
                if block.get("type").and_then(serde_json::Value::as_str) != Some("tool_result") {
                    continue;
                }
                let Some(id) = block.get("tool_use_id").and_then(serde_json::Value::as_str) else {
                    continue;
                };
                let is_error = block
                    .get("is_error")
                    .and_then(serde_json::Value::as_bool)
                    .unwrap_or(false);
                let name = state
                    .claude_tool_names
                    .remove(id)
                    .unwrap_or_else(|| "tool".to_string());
                if is_flowscript_draft_operation_tool(&name)
                    && let Some(result_text) =
                        block.get("content").and_then(external_tool_result_text)
                    && let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&result_text)
                    && let Some(mut payload) =
                        flowscript_workspace_result_payload(&name, &parsed, None)
                {
                    if let Some(object) = payload.as_object_mut() {
                        object.insert(
                            "tool_call_id".to_string(),
                            serde_json::Value::String(id.to_string()),
                        );
                    }
                    events.push(flowpilot_stream_tag("flowscript_workspace", &payload));
                }
                let fallback_status = if is_error { "error" } else { "completed" };
                let error = is_error.then_some("Claude tool result reported an error");
                let (status, terminal_status, result_summary, result_preview) =
                    external_result_details(block.get("content"), Some(fallback_status), error);
                let full_result = is_flowscript_draft_operation_tool(&name)
                    .then(|| {
                        block
                            .get("content")
                            .and_then(external_tool_result_text)
                            .map(|text| full_redacted_tool_result(&text))
                    })
                    .flatten();
                events.push(flowpilot_stream_tag(
                    "tool_end",
                    &serde_json::json!({
                        "tool_call_id": id,
                        "tool": name,
                        "status": status,
                        "terminal_status": terminal_status,
                        "result_summary": result_summary,
                        "result_preview": result_preview,
                        "result": full_result,
                        "error": error,
                    }),
                ));
            }
        }
        _ => {}
    }
    events
}

pub(super) fn send_external_progress_event(
    channel: &Channel<String>,
    event_id: &str,
    message: &str,
    parent_request_id: Option<&str>,
) {
    let message = flow_like::flow::copilot::stream::safe_text_preview(message, 1_200);
    send_correlated_stream_json_event(
        channel,
        "tool_progress",
        &serde_json::json!({
            "tool_call_id": event_id,
            "message": message,
        }),
        parent_request_id,
    );
}

pub(super) fn flowpilot_stream_tag(tag: &str, value: &serde_json::Value) -> String {
    // stream_frame escapes a literal closing tag inside the payload — tool
    // results carry untrusted text that must not truncate the frame.
    flow_like::flow::copilot::stream::stream_frame(tag, value)
}

pub(super) fn external_agent_progress_label(value: &serde_json::Value) -> Option<String> {
    let event_type = value
        .get("type")
        .or_else(|| value.get("event"))
        .and_then(serde_json::Value::as_str)?;

    if matches!(
        event_type,
        "item.started" | "item.updated" | "item.completed"
    ) && let Some(item) = value.get("item")
    {
        let item_type = item
            .get("type")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default();
        let status = item
            .get("status")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default();

        match item_type {
            "mcp_tool_call" => {
                let tool = item
                    .get("tool")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("tool");
                if event_type == "item.completed" || status == "completed" {
                    return Some(format!("Completed {tool}"));
                }
                return Some(format!("Using {tool}..."));
            }
            "command_execution" => {
                let command = item
                    .get("command")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("command");
                if event_type == "item.completed" || status == "completed" {
                    return Some(format!("Command completed: {command}"));
                }
                return Some(format!("Running command: {command}"));
            }
            "file_change" => {
                if event_type == "item.completed" || status == "completed" {
                    return Some("File changes completed".to_string());
                }
                return Some("Applying file changes...".to_string());
            }
            "web_search" => {
                let query = item
                    .get("query")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("web");
                return Some(format!("Searching {query}..."));
            }
            "error" => {
                if let Some(message) = item.get("message").and_then(serde_json::Value::as_str) {
                    return Some(format!("Error: {message}"));
                }
            }
            _ => {}
        }
    }

    if event_type.contains("tool") {
        let name = value
            .get("name")
            .or_else(|| value.pointer("/tool/name"))
            .or_else(|| value.pointer("/item/name"))
            .and_then(serde_json::Value::as_str)
            .unwrap_or("tool");
        return Some(format!("Using {name}..."));
    }

    if event_type.contains("error") {
        return Some(format!("{}...", event_type.replace('_', " ")));
    }

    None
}

pub(super) fn external_agent_error_text(value: &serde_json::Value) -> Option<String> {
    let event_type = value
        .get("type")
        .or_else(|| value.get("event"))
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();

    // Claude Code's stream-json protocol reports terminal failures as a `result` frame with
    // `is_error: true` and an error subtype. The process itself may still exit successfully, so
    // failing to inspect this frame turns an authentication/model error into a normal answer.
    if event_type == "result" {
        let subtype = value
            .get("subtype")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default();
        let failed = value
            .get("is_error")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false)
            || subtype.starts_with("error")
            || matches!(subtype, "failed" | "failure");
        if failed {
            let direct = value
                .get("error")
                .and_then(|error| {
                    error
                        .as_str()
                        .or_else(|| error.get("message").and_then(serde_json::Value::as_str))
                })
                .or_else(|| value.get("message").and_then(serde_json::Value::as_str))
                .or_else(|| value.get("result").and_then(serde_json::Value::as_str))
                .map(str::trim)
                .filter(|message| !message.is_empty())
                .map(str::to_string);
            if direct.is_some() {
                return direct;
            }

            let errors = value
                .get("errors")
                .and_then(serde_json::Value::as_array)
                .map(|errors| {
                    errors
                        .iter()
                        .filter_map(|error| {
                            error.as_str().or_else(|| {
                                error.get("message").and_then(serde_json::Value::as_str)
                            })
                        })
                        .map(str::trim)
                        .filter(|message| !message.is_empty())
                        .collect::<Vec<_>>()
                        .join("\n")
                })
                .filter(|errors| !errors.is_empty());
            return Some(errors.unwrap_or_else(|| {
                if subtype.is_empty() {
                    "Claude Code reported an unknown execution error".to_string()
                } else {
                    format!("Claude Code reported {subtype}")
                }
            }));
        }
    }

    if event_type == "turn.failed" {
        return value
            .pointer("/error/message")
            .or_else(|| value.get("message"))
            .and_then(serde_json::Value::as_str)
            .map(str::to_string);
    }

    if event_type == "error" {
        return value
            .get("message")
            .and_then(serde_json::Value::as_str)
            .map(str::to_string);
    }

    // Note: mcp_tool_call items with an error are deliberately NOT fatal — a single failed tool
    // call is surfaced as a tool_end error frame (external_agent_process_event) and the agent can
    // recover and continue the turn.
    if matches!(
        event_type,
        "item.started" | "item.updated" | "item.completed"
    ) && let Some(item) = value.get("item")
    {
        let item_type = item
            .get("type")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default();
        if item_type == "error" {
            return item
                .get("message")
                .and_then(serde_json::Value::as_str)
                .map(str::to_string);
        }
    }

    None
}

pub(super) fn external_agent_result_text(
    _backend: FlowPilotAgentBackendKind,
    value: &serde_json::Value,
) -> Option<String> {
    // Codex item.completed agent messages match first; every backend then falls back to the
    // generic result/final extraction (`{"type":"result","message":…}` frames). mcp_tool_call
    // outputs never reach the fallback — their event type carries neither "result" nor "final".
    if let Some(text) = codex_agent_result_text(value) {
        return Some(text);
    }

    let event_type = value
        .get("type")
        .or_else(|| value.get("event"))
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();

    if !event_type.contains("result") && !event_type.contains("final") {
        return None;
    }

    let text = extract_external_agent_text(value);
    (!text.trim().is_empty()).then_some(text)
}

fn codex_agent_result_text(value: &serde_json::Value) -> Option<String> {
    let event_type = value
        .get("type")
        .or_else(|| value.get("event"))
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();

    if event_type == "item.completed" {
        let item = value.get("item")?;
        let item_type = item
            .get("type")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default();
        if matches!(item_type, "agent_message" | "assistant_message") {
            return item
                .get("text")
                .and_then(serde_json::Value::as_str)
                .filter(|text| !text.trim().is_empty())
                .map(str::to_string);
        }
    }

    None
}

fn extract_external_agent_text(value: &serde_json::Value) -> String {
    fn collect(value: &serde_json::Value, out: &mut Vec<String>) {
        match value {
            serde_json::Value::Object(map) => {
                for (key, child) in map {
                    if matches!(
                        key.as_str(),
                        "text" | "content" | "delta" | "message" | "result" | "summary"
                    ) {
                        match child {
                            serde_json::Value::String(text) => {
                                if !looks_like_machine_status(text) {
                                    out.push(text.clone());
                                }
                            }
                            serde_json::Value::Array(_) | serde_json::Value::Object(_) => {
                                collect(child, out);
                            }
                            _ => {}
                        }
                    } else if matches!(
                        child,
                        serde_json::Value::Array(_) | serde_json::Value::Object(_)
                    ) {
                        collect(child, out);
                    }
                }
            }
            serde_json::Value::Array(items) => {
                for item in items {
                    collect(item, out);
                }
            }
            _ => {}
        }
    }

    let mut parts = Vec::new();
    collect(value, &mut parts);
    parts.join("")
}

fn looks_like_machine_status(text: &str) -> bool {
    let trimmed = text.trim();
    trimmed.is_empty()
        || trimmed.starts_with('{')
        || trimmed.starts_with('[')
        || trimmed == "started"
        || trimmed == "completed"
}
