//! Bounded text, correlated stream frames, and tool previews.

use super::external_continuation::MAX_TERMINAL_REPORT_DIAGNOSTICS;
use super::mcp::McpToolCompletion;
use super::workflow_diagnostics::{
    workflow_result_diagnostics, workflow_result_structured_diagnostics,
};
use super::workflow_state::WorkflowToolLoopSnapshot;
use crate::functions::ai::frontend_tool_bridge::FrontendToolContext;
use flow_like::flow::copilot::{BoardCommand, flowscript_workspace_envelope};
use std::collections::{HashMap, HashSet};
use tauri::ipc::Channel;

pub(super) fn utf8_prefix(value: &str, max_bytes: usize) -> &str {
    if value.len() <= max_bytes {
        return value;
    }
    let mut boundary = max_bytes;
    while boundary > 0 && !value.is_char_boundary(boundary) {
        boundary -= 1;
    }
    &value[..boundary]
}

/// Append text while keeping the retained copy bounded. Streaming still forwards each delta to
/// the frontend; this cap only prevents a long-running agent from retaining an unbounded duplicate
/// in the native process.
pub(super) fn append_bounded_text(target: &mut String, value: &str, max_bytes: usize) -> bool {
    const TRUNCATED: &str = "\n[FlowPilot output truncated in native retention]";
    if value.is_empty() {
        return true;
    }
    if target.len().saturating_add(value.len()) <= max_bytes {
        target.push_str(value);
        return true;
    }
    if target.len() >= max_bytes {
        return false;
    }

    let available = max_bytes - target.len();
    let content_bytes = available.saturating_sub(TRUNCATED.len());
    target.push_str(utf8_prefix(value, content_bytes));
    target.push_str(utf8_prefix(TRUNCATED, max_bytes - target.len()));
    false
}

pub(super) fn append_bounded_tail(target: &mut String, value: &str, max_bytes: usize) {
    if value.is_empty() || max_bytes == 0 {
        return;
    }
    target.push_str(value);
    if target.len() <= max_bytes {
        return;
    }
    let mut keep_from = target.len() - max_bytes;
    while keep_from < target.len() && !target.is_char_boundary(keep_from) {
        keep_from += 1;
    }
    target.drain(..keep_from);
}

pub(super) fn flowscript_response_workspace_envelope(
    source: &str,
    status: &str,
    snapshot: Option<&WorkflowToolLoopSnapshot>,
) -> String {
    let Some(snapshot) = snapshot else {
        return flowscript_workspace_envelope(source, status);
    };

    let mut payload = serde_json::json!({
        "source": source,
        "status": status,
    });
    if let Some(object) = payload.as_object_mut() {
        if matches!(
            status,
            "validation_error" | "validation_errors" | "draft_needs_repair" | "edit_interrupted"
        ) {
            let diagnostic_count = snapshot.last_errors.len();
            if diagnostic_count > 0 {
                object.insert(
                    "diagnostic_count".to_string(),
                    serde_json::json!(diagnostic_count),
                );
                object.insert(
                    "diagnostics".to_string(),
                    serde_json::json!(
                        snapshot
                            .last_errors
                            .iter()
                            .take(MAX_TERMINAL_REPORT_DIAGNOSTICS)
                            .collect::<Vec<_>>()
                    ),
                );
            }
            if !snapshot.last_structured_diagnostics.is_empty() {
                object.insert(
                    "structured_diagnostics".to_string(),
                    serde_json::Value::Array(snapshot.last_structured_diagnostics.clone()),
                );
            }
        }
        if let Some(regression) = snapshot.modular_fallback.as_ref() {
            object.insert(
                "completion".to_string(),
                serde_json::Value::String("partial_working_slice".to_string()),
            );
            if let Some(retained) = snapshot.retained_full_source.as_deref() {
                object.insert(
                    "retained_full_source".to_string(),
                    serde_json::Value::String(retained.to_string()),
                );
            }
            object.insert(
                "regression".to_string(),
                serde_json::json!({
                    "previous_call_sites": regression.previous_call_sites,
                    "candidate_call_sites": regression.candidate_call_sites,
                    "previous_statements": regression.previous_statements,
                    "candidate_statements": regression.candidate_statements,
                    "previous_scope_symbols": regression.previous_scope_symbols,
                    "retained_scope_symbols": regression.retained_scope_symbols,
                }),
            );
        }
    }
    serde_json::to_string(&payload)
        .unwrap_or_else(|_| flowscript_workspace_envelope(source, status))
}

pub(super) fn send_stream_json_event(
    channel: &Channel<String>,
    tag: &str,
    payload: &serde_json::Value,
) {
    send_correlated_stream_json_event(channel, tag, payload, None);
}

pub(super) fn scoped_parent_request_id(context: Option<&FrontendToolContext>) -> Option<String> {
    context
        .and_then(|context| context.parent_request_id.as_deref())
        .map(str::trim)
        .filter(|request_id| !request_id.is_empty())
        .map(str::to_string)
}

/// Derive the immutable request identity that owns retained drafts and the acceptance contract.
///
/// Nested runs spawned from one user turn bind to the outer chat's source prompt instead of their
/// per-run specialist instruction, so a follow-up repair run can resume the retained draft. The
/// prompt text alone is not a safe identity: two conversations can send identical short prompts
/// ("yes, build it") against the same board inside the draft-store lease window, so the owning
/// conversation id is folded in whenever the host supplies one. Runs without a tool context (the
/// board panel copilot) keep their raw-prompt identity unchanged.
pub(super) fn request_identity_prompt_for(
    tool_context: Option<&FrontendToolContext>,
    raw_user_prompt: &str,
) -> String {
    let source_prompt = tool_context
        .and_then(|context| context.source_user_prompt.as_deref())
        .filter(|prompt| !prompt.trim().is_empty())
        .unwrap_or(raw_user_prompt);
    let conversation_id = tool_context
        .and_then(|context| context.conversation_id.as_deref())
        .map(str::trim)
        .filter(|conversation_id| !conversation_id.is_empty());
    match conversation_id {
        Some(conversation_id) => format!("{conversation_id}\n{source_prompt}"),
        None => source_prompt.to_string(),
    }
}

pub(super) fn correlated_stream_payload(
    payload: &serde_json::Value,
    parent_request_id: Option<&str>,
) -> serde_json::Value {
    let mut payload = payload.clone();
    if let (Some(parent_request_id), Some(object)) = (parent_request_id, payload.as_object_mut()) {
        object
            .entry("parent_request_id".to_string())
            .or_insert_with(|| serde_json::Value::String(parent_request_id.to_string()));
    }
    payload
}

pub(super) fn send_correlated_stream_json_event(
    channel: &Channel<String>,
    tag: &str,
    payload: &serde_json::Value,
    parent_request_id: Option<&str>,
) {
    let payload = correlated_stream_payload(payload, parent_request_id);
    // stream_frame escapes a literal closing tag inside the payload — tool
    // results carry untrusted text that must not truncate the frame.
    let event = flow_like::flow::copilot::stream::stream_frame(tag, &payload);
    let _ = channel.send(event);
}

/// Add nested-run correlation to a core/provider frame without touching plain assistant text.
/// Core emits fully framed strings, whereas the desktop SDK adapters emit structured payloads.
pub(super) fn correlate_stream_frame(frame: &str, parent_request_id: Option<&str>) -> String {
    let Some(parent_request_id) = parent_request_id else {
        return frame.to_string();
    };
    let Some(tag_end) = frame.find('>') else {
        return frame.to_string();
    };
    if !frame.starts_with('<') {
        return frame.to_string();
    }
    let tag = &frame[1..tag_end];
    if !matches!(
        tag,
        "tool_start"
            | "tool_progress"
            | "tool_end"
            | "plan_step"
            | "usage_stat"
            | "flowscript_workspace"
    ) {
        return frame.to_string();
    }
    let close_tag = format!("</{tag}>");
    let Some(payload_text) = frame
        .strip_prefix(&frame[..=tag_end])
        .and_then(|rest| rest.strip_suffix(&close_tag))
    else {
        return frame.to_string();
    };
    let Ok(payload) = serde_json::from_str::<serde_json::Value>(payload_text) else {
        return frame.to_string();
    };
    let payload = correlated_stream_payload(&payload, Some(parent_request_id));
    // Re-frame through stream_frame: plain serialization would undo the closing-tag escape.
    flow_like::flow::copilot::stream::stream_frame(tag, &payload)
}

/// Build the workspace status frame emitted when a FlowScript lifecycle tool finishes. Retained
/// source tools return the exact model-authored `source`; the legacy one-shot edit falls back to
/// the source captured at tool start. Typed-draft results remain readable for old sessions, but
/// are no longer the advertised authoring path. The UI applies only `queued` workspaces and keeps
/// every other status visible for repair.
pub(super) fn flowscript_workspace_result_payload(
    tool_name: &str,
    result: &serde_json::Value,
    latest_submitted: Option<&str>,
) -> Option<serde_json::Value> {
    let explicit_workspace = result
        .get("flowscript_workspace")
        .and_then(serde_json::Value::as_str);
    let retained_workspace = matches!(
        tool_name,
        "write_flowscript"
            | "patch_flowscript"
            | "check_flowscript"
            | "commit_flowscript"
            | "begin_flow_ir_draft"
            | "update_flow_ir_draft"
            | "upsert_flow_ir_module"
            | "validate_flow_ir_draft"
            | "commit_flow_ir_draft"
    )
    .then(|| {
        result
            .get("source")
            .or_else(|| result.get("flowscript"))
            .and_then(serde_json::Value::as_str)
    })
    .flatten();
    let workspace = explicit_workspace.or(retained_workspace).or_else(|| {
        (tool_name == "edit_flowscript")
            .then_some(latest_submitted)
            .flatten()
    })?;
    let status = result
        .get("status")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("unknown");

    let mut payload = serde_json::json!({
        "source": workspace,
        "status": status,
    });
    if let Some(object) = payload.as_object_mut() {
        for key in ["draft_id", "revision", "base_fingerprint"] {
            if let Some(value) = result.get(key) {
                object.insert(key.to_string(), value.clone());
            }
        }
        let diagnostics = workflow_result_diagnostics(Some(result));
        if !diagnostics.is_empty() {
            object.insert(
                "diagnostic_count".to_string(),
                serde_json::json!(diagnostics.len()),
            );
            object.insert(
                "diagnostics".to_string(),
                serde_json::json!(
                    diagnostics
                        .iter()
                        .take(MAX_TERMINAL_REPORT_DIAGNOSTICS)
                        .collect::<Vec<_>>()
                ),
            );
        }
        let structured_diagnostics = workflow_result_structured_diagnostics(Some(result));
        if !structured_diagnostics.is_empty() {
            object.insert(
                "structured_diagnostics".to_string(),
                serde_json::Value::Array(structured_diagnostics),
            );
        }
    }
    Some(payload)
}

/// Emit the tool_start frame (plus a workspace preview for full-source authoring tools) for a tool
/// call announced either via tool.execution_start or the protocol v3 external_tool.requested
/// broadcast.
pub(super) fn announce_tool_start(
    channel: &Channel<String>,
    tool_call_id: &str,
    tool_name: &str,
    arguments: Option<&serde_json::Value>,
    extracted_flowscript_workspace: &mut Option<String>,
    parent_request_id: Option<&str>,
) {
    if flow_like::flow::copilot::stream::is_flowscript_authoring_tool(tool_name)
        && let Some(workspace) =
            arguments.and_then(flow_like::flow::copilot::stream::source_argument)
    {
        *extracted_flowscript_workspace = Some(workspace.to_string());
        send_stream_json_event(
            channel,
            "flowscript_workspace",
            &serde_json::json!({
                "source": workspace,
                "status": "submitted",
                "tool_call_id": tool_call_id,
            }),
        );
    }

    send_correlated_stream_json_event(
        channel,
        "tool_start",
        &serde_json::json!({
            "tool_call_id": tool_call_id,
            "tool": tool_name,
            "status": "running",
            "summary": flow_like::flow::copilot::stream::safe_text_preview(
                &summarize_tool_arguments(tool_name, arguments),
                600,
            ),
            "arguments_preview": preview_tool_arguments(tool_name, arguments),
        }),
        parent_request_id,
    );
}

/// Close every tool step that got a tool_start but never a completion event, so the frontend does
/// not keep spinners alive after the session ends (idle, error, or stream loss).
pub(super) fn close_pending_tool_steps(
    channel: &Channel<String>,
    open_tool_call_ids: &mut HashSet<String>,
    tool_names_by_call_id: &HashMap<String, String>,
    status: &str,
    error: Option<&str>,
    parent_request_id: Option<&str>,
) {
    let safe_error =
        error.map(|error| flow_like::flow::copilot::stream::safe_text_preview(error, 600));
    for tool_call_id in open_tool_call_ids.drain() {
        let tool = tool_names_by_call_id
            .get(&tool_call_id)
            .cloned()
            .unwrap_or_else(|| "tool".to_string());
        send_correlated_stream_json_event(
            channel,
            "tool_end",
            &serde_json::json!({
                "tool_call_id": tool_call_id,
                "tool": tool,
                "status": status,
                "result_summary": safe_error.as_deref().unwrap_or("completed"),
                "error": safe_error.as_deref(),
            }),
            parent_request_id,
        );
    }
}

pub(super) fn truncate_for_preview(value: &str, max_chars: usize) -> String {
    let mut result = String::new();
    for (index, ch) in value.chars().enumerate() {
        if index >= max_chars {
            result.push_str("...");
            break;
        }
        result.push(ch);
    }
    result
}

fn line_count(value: &str) -> usize {
    value.lines().count().max(usize::from(!value.is_empty()))
}

fn summarize_tool_arguments(tool_name: &str, arguments: Option<&serde_json::Value>) -> String {
    let Some(arguments) = arguments else {
        return "No arguments".to_string();
    };

    match tool_name {
        "get_declarations" | "catalog_search" | "search_by_pin" => arguments
            .get("query")
            .and_then(|value| value.as_str())
            .map(|query| format!("query: {query}"))
            .unwrap_or_else(|| "Searching".to_string()),
        "internet_search" => arguments
            .get("query")
            .and_then(|value| value.as_str())
            .map(|query| format!("query: {query}"))
            .unwrap_or_else(|| "Searching web".to_string()),
        "database_tool" | "storage_tool" => arguments
            .get("operation")
            .and_then(|value| value.as_str())
            .map(|operation| {
                let target = arguments
                    .get("table_name")
                    .or_else(|| arguments.get("tableName"))
                    .or_else(|| arguments.get("path"))
                    .or_else(|| arguments.get("prefix"))
                    .and_then(|value| value.as_str())
                    .unwrap_or("");
                if target.is_empty() {
                    operation.to_string()
                } else {
                    format!("{operation}: {target}")
                }
            })
            .unwrap_or_else(|| "Preparing frontend operation".to_string()),
        "execute_event" => arguments
            .get("event_id")
            .or_else(|| arguments.get("eventId"))
            .and_then(|value| value.as_str())
            .map(|event_id| format!("event: {event_id}"))
            .unwrap_or_else(|| "Executing event".to_string()),
        "run_board_tests" => arguments
            .get("board_id")
            .or_else(|| arguments.get("boardId"))
            .and_then(|value| value.as_str())
            .map(|board_id| format!("board tests: {board_id}"))
            .unwrap_or_else(|| "Running board tests".to_string()),
        "ask_user" => arguments
            .get("questions")
            .and_then(|value| value.as_array())
            .filter(|questions| !questions.is_empty())
            .map(|questions| {
                let first = questions
                    .first()
                    .and_then(|question| question.get("question"))
                    .and_then(|value| value.as_str())
                    .unwrap_or("Requesting user input");
                match questions.len() {
                    1 => truncate_for_preview(first, 180),
                    count => format!("{} (+{} more)", truncate_for_preview(first, 140), count - 1),
                }
            })
            .or_else(|| {
                arguments
                    .get("question")
                    .and_then(|value| value.as_str())
                    .map(|question| truncate_for_preview(question, 180))
            })
            .unwrap_or_else(|| "Requesting user input".to_string()),
        "edit_flowscript" | "write_flowscript" => arguments
            .get("source")
            .or_else(|| arguments.get("flowscript"))
            .or_else(|| arguments.get("script"))
            .or_else(|| arguments.get("content"))
            .and_then(|value| value.as_str())
            .map(|flowscript| {
                format!(
                    "{} lines, {} chars",
                    line_count(flowscript),
                    flowscript.chars().count()
                )
            })
            .unwrap_or_else(|| "Submitting FlowScript".to_string()),
        "patch_flowscript" => {
            let old_chars = arguments
                .get("old_text")
                .or_else(|| arguments.get("search"))
                .and_then(serde_json::Value::as_str)
                .map(|value| value.chars().count())
                .unwrap_or_default();
            let new_chars = arguments
                .get("new_text")
                .or_else(|| arguments.get("replacement"))
                .and_then(serde_json::Value::as_str)
                .map(|value| value.chars().count())
                .unwrap_or_default();
            format!("replace {old_chars} chars with {new_chars} chars")
        }
        "check_flowscript" | "test_flowscript" | "commit_flowscript" => {
            let draft_id = arguments
                .get("draft_id")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("<draft>");
            let revision = arguments
                .get("expected_revision")
                .and_then(serde_json::Value::as_u64)
                .map(|revision| revision.to_string())
                .unwrap_or_else(|| "?".to_string());
            format!("draft {draft_id}, revision {revision}")
        }
        "emit_commands" | "validate_commands" => arguments
            .get("commands")
            .and_then(|value| value.as_array())
            .map(|commands| format!("{} command(s)", commands.len()))
            .unwrap_or_else(|| "Preparing commands".to_string()),
        "emit_ui" | "validate_ui" => arguments
            .get("components")
            .and_then(|value| value.as_array())
            .map(|components| format!("{} component(s)", components.len()))
            .unwrap_or_else(|| "Preparing UI".to_string()),
        _ => preview_tool_arguments(tool_name, Some(arguments)),
    }
}

pub(super) fn preview_tool_arguments(
    _tool_name: &str,
    arguments: Option<&serde_json::Value>,
) -> String {
    let Some(arguments) = arguments else {
        return "{}".to_string();
    };

    flow_like::flow::copilot::stream::safe_json_preview(
        arguments,
        flow_like::flow::copilot::stream::TOOL_ARGUMENT_PREVIEW_CHARS,
    )
}

pub(super) fn extract_json_status(content: &str) -> Option<String> {
    serde_json::from_str::<serde_json::Value>(content)
        .ok()
        .and_then(|value| {
            value
                .get("status")
                .and_then(|status| status.as_str().map(str::to_string))
        })
}

pub(super) fn direct_sdk_tool_result_stream_status(content: &str) -> &'static str {
    flow_like::flow::copilot::stream::tool_result_stream_status(content)
}

pub(super) fn summarize_tool_result(content: Option<&str>, error: Option<&str>) -> String {
    if let Some(error) = error {
        return error.to_string();
    }

    let Some(content) = content else {
        return "Completed".to_string();
    };

    if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(content) {
        let status = parsed
            .get("status")
            .and_then(|value| value.as_str())
            .unwrap_or("done");
        let command_count = parsed
            .get("commands")
            .and_then(|value| value.as_array())
            .map(Vec::len);
        let component_count = parsed
            .get("components")
            .and_then(|value| value.as_array())
            .map(Vec::len);
        let error_count = parsed
            .get("errors")
            .and_then(|value| value.as_array())
            .map(Vec::len);
        let diagnostic_count = parsed
            .get("diagnostics")
            .and_then(|value| value.as_array())
            .map(Vec::len);

        let mut parts = vec![status.replace('_', " ")];
        if let Some(count) = command_count {
            parts.push(format!("{count} command(s)"));
        }
        if let Some(count) = component_count {
            parts.push(format!("{count} component(s)"));
        }
        if let Some(count) = error_count.filter(|count| *count > 0) {
            parts.push(format!("{count} error(s)"));
        }
        if let Some(count) = diagnostic_count.filter(|count| *count > 0) {
            parts.push(format!("{count} diagnostic(s)"));
        }
        return parts.join(" · ");
    }

    truncate_for_preview(content.trim(), 240)
}

pub(super) fn render_recovered_mutation_message(completion: &McpToolCompletion) -> String {
    let summary = summarize_tool_result(Some(&completion.result_text), None);
    let preview = preview_tool_result(&completion.result_text);
    format!(
        "`{}` completed successfully before the provider process exited ({summary}). The completed tool result was preserved:\n\n{preview}",
        completion.tool_name
    )
}

pub(super) fn preview_tool_result(content: &str) -> String {
    flow_like::flow::copilot::stream::safe_tool_activity_result_preview(content)
}

pub(super) fn send_commands_event(channel: &Channel<String>, commands: &[BoardCommand]) {
    if commands.is_empty() {
        return;
    }

    let cmd_event = format!(
        "<commands>{}</commands>",
        serde_json::to_string(commands).unwrap_or_default()
    );
    let _ = channel.send(cmd_event);
}
