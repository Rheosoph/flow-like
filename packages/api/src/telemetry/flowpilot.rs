//! Shared parsing for the FlowPilot failure trace.
//!
//! The client publishes each run's failure causes inside the
//! `flowpilot_generation_metrics` props, already secret-redacted, generalized
//! and bounded. Both the daily rollup and the admin read path fold those
//! signatures, so the normalization rules — which is a *known* failure kind,
//! how long a code or message may be, how many signatures one payload may
//! contribute — live here instead of drifting between the two.

use std::collections::BTreeMap;

/// The event the client publishes one aggregate payload into per FlowPilot run.
pub const FLOWPILOT_METRICS_EVENT: &str = "flowpilot_generation_metrics";

/// Mirrors `FlowPilotFailureKind` in `agent-debug-report.ts`. An unrecognized
/// kind is dropped rather than stored: the client controls this vocabulary, and
/// an open one would let a malformed payload create unbounded groups.
pub const FAILURE_KINDS: [&str; 7] = [
    "subagent_dispatch",
    "flowscript_apply",
    "widget_apply",
    "data_apply",
    "page_apply",
    "tool_error",
    "run_error",
];

pub const MAX_FAILURE_TOOL_CHARS: usize = 80;
pub const MAX_FAILURE_CODE_CHARS: usize = 80;
pub const MAX_FAILURE_MESSAGE_CHARS: usize = 200;
/// Matches the client's per-run cap; a payload claiming more is truncated.
pub const MAX_FAILURE_SIGNATURES_PER_EVENT: usize = 12;
/// Placeholder for an absent dimension. Keeping the group key non-null lets the
/// unique index carry all five dimensions.
pub const UNKNOWN_DIMENSION: &str = "unknown";

/// One deduplicated failure cause as reported by a single client payload.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FailureSignature {
    pub kind: String,
    pub tool: String,
    pub code: String,
    pub message: String,
    pub count: i64,
}

impl FailureSignature {
    /// Group identity, matching the daily table's unique index.
    pub fn key(&self) -> (String, String, String, String) {
        (
            self.kind.clone(),
            self.tool.clone(),
            self.code.clone(),
            self.message.clone(),
        )
    }
}

/// Trim, clamp to `limit` characters (never splitting a UTF-8 boundary) and
/// fall back to the unknown placeholder when nothing usable is left.
fn bounded_dimension(value: Option<&serde_json::Value>, limit: usize) -> String {
    let Some(text) = value.and_then(|v| v.as_str()) else {
        return UNKNOWN_DIMENSION.to_string();
    };
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return UNKNOWN_DIMENSION.to_string();
    }
    trimmed.chars().take(limit).collect()
}

fn signature_count(value: Option<&serde_json::Value>) -> i64 {
    value
        .and_then(|v| {
            v.as_i64()
                .or_else(|| v.as_u64().map(|raw| raw.min(i64::MAX as u64) as i64))
        })
        .unwrap_or(1)
        .max(1)
}

/// Read the `failures` array out of one metrics payload.
///
/// Entries with an unknown kind are skipped. Duplicate signatures inside a
/// single payload are merged so one event can never inflate a group beyond the
/// runs it actually represents.
pub fn parse_failure_signatures(props: Option<&serde_json::Value>) -> Vec<FailureSignature> {
    let Some(entries) = props
        .and_then(|p| p.get("failures"))
        .and_then(|v| v.as_array())
    else {
        return Vec::new();
    };

    let mut merged: BTreeMap<(String, String, String, String), FailureSignature> = BTreeMap::new();
    for entry in entries.iter().take(MAX_FAILURE_SIGNATURES_PER_EVENT) {
        let Some(kind) = entry.get("kind").and_then(|v| v.as_str()) else {
            continue;
        };
        if !FAILURE_KINDS.contains(&kind) {
            continue;
        }
        let signature = FailureSignature {
            kind: kind.to_string(),
            tool: bounded_dimension(entry.get("tool"), MAX_FAILURE_TOOL_CHARS),
            code: bounded_dimension(entry.get("code"), MAX_FAILURE_CODE_CHARS),
            message: bounded_dimension(entry.get("message"), MAX_FAILURE_MESSAGE_CHARS),
            count: signature_count(entry.get("count")),
        };
        merged
            .entry(signature.key())
            .and_modify(|existing| existing.count = existing.count.saturating_add(signature.count))
            .or_insert(signature);
    }
    merged.into_values().collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn missing_or_malformed_failures_yield_nothing() {
        assert!(parse_failure_signatures(None).is_empty());
        assert!(parse_failure_signatures(Some(&json!({}))).is_empty());
        assert!(parse_failure_signatures(Some(&json!({ "failures": "nope" }))).is_empty());
        assert!(
            parse_failure_signatures(Some(&json!({ "failures": [{ "message": "no kind" }] })))
                .is_empty()
        );
    }

    #[test]
    fn unknown_kinds_are_dropped_so_groups_stay_bounded() {
        let parsed = parse_failure_signatures(Some(&json!({
            "failures": [
                { "kind": "made_up_kind", "count": 4 },
                { "kind": "widget_apply", "count": 2 },
            ]
        })));
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].kind, "widget_apply");
        assert_eq!(parsed[0].count, 2);
    }

    #[test]
    fn absent_dimensions_become_the_unknown_placeholder() {
        let parsed = parse_failure_signatures(Some(&json!({
            "failures": [{ "kind": "run_error", "tool": "   " }]
        })));
        assert_eq!(parsed[0].tool, UNKNOWN_DIMENSION);
        assert_eq!(parsed[0].code, UNKNOWN_DIMENSION);
        assert_eq!(parsed[0].message, UNKNOWN_DIMENSION);
        assert_eq!(parsed[0].count, 1);
    }

    #[test]
    fn oversized_dimensions_are_clamped_on_character_boundaries() {
        let parsed = parse_failure_signatures(Some(&json!({
            "failures": [{
                "kind": "tool_error",
                "code": "C".repeat(MAX_FAILURE_CODE_CHARS + 40),
                "message": "ü".repeat(MAX_FAILURE_MESSAGE_CHARS + 40),
            }]
        })));
        assert_eq!(parsed[0].code.chars().count(), MAX_FAILURE_CODE_CHARS);
        assert_eq!(parsed[0].message.chars().count(), MAX_FAILURE_MESSAGE_CHARS);
    }

    #[test]
    fn duplicate_signatures_in_one_payload_merge() {
        let parsed = parse_failure_signatures(Some(&json!({
            "failures": [
                { "kind": "flowscript_apply", "code": "FS_PARSE_ERROR", "count": 2 },
                { "kind": "flowscript_apply", "code": "FS_PARSE_ERROR", "count": 3 },
            ]
        })));
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].count, 5);
    }

    #[test]
    fn payloads_cannot_exceed_the_per_event_signature_cap() {
        let failures: Vec<serde_json::Value> = (0..MAX_FAILURE_SIGNATURES_PER_EVENT + 8)
            .map(|index| json!({ "kind": "tool_error", "code": format!("C{index}") }))
            .collect();
        let parsed = parse_failure_signatures(Some(&json!({ "failures": failures })));
        assert_eq!(parsed.len(), MAX_FAILURE_SIGNATURES_PER_EVENT);
    }

    #[test]
    fn counts_are_clamped_to_at_least_one() {
        let parsed = parse_failure_signatures(Some(&json!({
            "failures": [{ "kind": "run_error", "count": -9 }]
        })));
        assert_eq!(parsed[0].count, 1);
    }
}
