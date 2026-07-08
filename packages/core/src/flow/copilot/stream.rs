//! Frame builders for the copilot stream protocol.
//!
//! Every backend streams the same XML-tagged JSON frames over the token channel:
//! `<tool_start>`/`<tool_end>` for tool lifecycle, `<plan_step>` for reasoning/phase steps, plus
//! payload tags (`<commands>`, `<components>`, …) emitted by the callers. The frontend parser
//! (`copilot-stream-parser.ts`) consumes exactly this grammar.

use serde_json::{Value, json};

use super::types::{PlanStep, PlanStepStatus, StreamEvent};

pub fn stream_frame(tag: &str, payload: &Value) -> String {
    format!(
        "<{tag}>{}</{tag}>",
        serde_json::to_string(payload).unwrap_or_default()
    )
}

pub fn tool_start_frame(tool_call_id: &str, tool: &str, summary: Option<&str>) -> String {
    let mut payload = json!({
        "tool_call_id": tool_call_id,
        "tool": tool,
        "status": "running",
    });
    if let (Some(summary), Some(object)) = (summary, payload.as_object_mut()) {
        object.insert("summary".to_string(), Value::String(summary.to_string()));
    }
    stream_frame("tool_start", &payload)
}

pub fn tool_end_frame(tool_call_id: &str, tool: &str, status: &str) -> String {
    stream_frame(
        "tool_end",
        &json!({
            "tool_call_id": tool_call_id,
            "tool": tool,
            "status": status,
        }),
    )
}

/// Emit an LLM usage/stats frame (`<usage_stat>`), matching the `chat_usage_stat` shape the simple
/// chat's app events emit so the shared `<UsageStats>` renderer displays the agent's own token use.
/// `stats` is a serialized `LLMUsageStats`; the payload mirrors `IChatUsageStat` on the frontend.
pub fn usage_stat_frame(step_name: &str, stats: &Value) -> String {
    stream_frame(
        "usage_stat",
        &json!({ "step_name": step_name, "stats": stats }),
    )
}

pub fn plan_step_frame(
    id: String,
    description: String,
    status: PlanStepStatus,
    tool_name: &str,
) -> String {
    let event = StreamEvent::PlanStep(PlanStep {
        id,
        description,
        status,
        tool_name: Some(tool_name.to_string()),
    });
    format!(
        "<plan_step>{}</plan_step>",
        serde_json::to_string(&event).unwrap_or_default()
    )
}
