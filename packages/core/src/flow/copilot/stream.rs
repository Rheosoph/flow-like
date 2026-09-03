//! Frame builders for the copilot stream protocol.
//!
//! Every backend streams the same XML-tagged JSON frames over the token channel:
//! `<tool_start>`/`<tool_end>` for tool lifecycle, `<plan_step>` for reasoning/phase steps, plus
//! payload tags (`<commands>`, `<components>`, …) emitted by the callers. The frontend parser
//! (`copilot-stream-parser.ts`) consumes exactly this grammar.

use std::cmp::Reverse;
use std::collections::HashMap;

use serde_json::{Value, json};

use super::types::{PlanStep, PlanStepStatus, StreamEvent};

/// Provider-frame payload limits for persisted audit diagnostics. These are intentionally larger
/// than compact UI summaries so non-trivial FlowScripts and structured tool results remain useful,
/// while the frontend still enforces a per-run byte ceiling and redacts secrets before storage.
pub const TOOL_ARGUMENT_PREVIEW_CHARS: usize = 8 * 1024;
pub const TOOL_RESULT_PREVIEW_CHARS: usize = 8 * 1024;

const MAX_STREAMED_FLOWSCRIPT_ARGUMENT_BYTES: usize = 512 * 1024;
const FLOWSCRIPT_PREVIEW_EMIT_CHARS: usize = 160;

#[derive(Debug, Default)]
struct PartialFlowScriptToolCall {
    tool_name: String,
    arguments: String,
    last_emitted_source_bytes: usize,
    sequence: u64,
}

/// Tracks streamed tool-call JSON and emits bounded snapshots of a FlowScript source argument.
///
/// Models write FlowScript inside a JSON string argument. Waiting for the completed tool call makes
/// the workspace appear only after the model has finished authoring it, which defeats a live code
/// preview. This tracker incrementally decodes that one JSON string without treating incomplete
/// JSON as an error. The authoritative submitted/validation/queued frames still come from the
/// completed tool call and compiler result.
#[derive(Debug, Default)]
pub struct FlowScriptToolCallPreviewTracker {
    calls: HashMap<String, PartialFlowScriptToolCall>,
}

impl FlowScriptToolCallPreviewTracker {
    pub fn observe_name(&mut self, internal_call_id: &str, name: &str) {
        let call = self.calls.entry(internal_call_id.to_string()).or_default();
        if name.is_empty() || call.tool_name == name || call.tool_name.ends_with(name) {
            return;
        }
        if name.starts_with(&call.tool_name) {
            // Some providers send the cumulative name on every delta.
            call.tool_name.clear();
            call.tool_name.push_str(name);
        } else {
            // Others send disjoint fragments.
            call.tool_name.push_str(name);
        }
    }

    /// Append a provider argument delta and return a live workspace frame when enough new source
    /// is available. Frames contain the complete decoded prefix so consumers can replace their
    /// preview atomically and recover from a dropped individual delta.
    pub fn observe_arguments_delta(
        &mut self,
        internal_call_id: &str,
        delta: &str,
    ) -> Option<String> {
        let call = self.calls.entry(internal_call_id.to_string()).or_default();
        if call.arguments.len() >= MAX_STREAMED_FLOWSCRIPT_ARGUMENT_BYTES {
            return None;
        }
        let remaining = MAX_STREAMED_FLOWSCRIPT_ARGUMENT_BYTES - call.arguments.len();
        append_char_boundary_prefix(&mut call.arguments, delta, remaining);
        if !is_flowscript_authoring_tool(&call.tool_name) {
            return None;
        }
        let source = extract_partial_source_argument(&call.arguments)?;
        if source.is_empty() || source.len() <= call.last_emitted_source_bytes {
            return None;
        }
        let newly_decoded = &source[call.last_emitted_source_bytes..];
        let should_emit = call.last_emitted_source_bytes == 0
            || source.len() - call.last_emitted_source_bytes >= FLOWSCRIPT_PREVIEW_EMIT_CHARS
            || newly_decoded.contains('\n');
        if !should_emit {
            return None;
        }
        call.last_emitted_source_bytes = source.len();
        call.sequence = call.sequence.saturating_add(1);
        Some(flowscript_workspace_frame(
            &source,
            "drafting",
            Some(internal_call_id),
            Some(call.sequence),
        ))
    }

    /// Finish a streamed tool call and emit its authoritative, complete source before compiler
    /// execution begins. Providers without argument deltas still get this submitted snapshot.
    pub fn complete(
        &mut self,
        internal_call_id: &str,
        tool_name: &str,
        arguments: &Value,
    ) -> Option<String> {
        let mut call = self.calls.remove(internal_call_id).unwrap_or_default();
        if call.tool_name.is_empty() {
            call.tool_name = tool_name.to_string();
        }
        if !is_flowscript_authoring_tool(tool_name)
            && !is_flowscript_authoring_tool(&call.tool_name)
        {
            return None;
        }
        let source = source_argument(arguments)?;
        if source.trim().is_empty() {
            return None;
        }
        Some(flowscript_workspace_frame(
            source,
            "submitted",
            Some(internal_call_id),
            Some(call.sequence.saturating_add(1)),
        ))
    }
}

pub fn is_flowscript_authoring_tool(tool_name: &str) -> bool {
    let name = tool_name.rsplit("__").next().unwrap_or(tool_name);
    matches!(name, "edit_flowscript" | "write_flowscript")
}

pub fn source_argument(arguments: &Value) -> Option<&str> {
    ["source", "flowscript", "script", "content"]
        .into_iter()
        .find_map(|key| arguments.get(key).and_then(Value::as_str))
}

pub fn flowscript_workspace_frame(
    source: &str,
    status: &str,
    tool_call_id: Option<&str>,
    sequence: Option<u64>,
) -> String {
    let mut payload = json!({
        "source": source,
        "status": status,
    });
    if let Some(object) = payload.as_object_mut() {
        if let Some(tool_call_id) = tool_call_id {
            object.insert(
                "tool_call_id".to_string(),
                Value::String(tool_call_id.to_string()),
            );
        }
        if let Some(sequence) = sequence {
            object.insert("sequence".to_string(), Value::from(sequence));
        }
    }
    stream_frame("flowscript_workspace", &payload)
}

fn append_char_boundary_prefix(target: &mut String, value: &str, max_bytes: usize) {
    if value.len() <= max_bytes {
        target.push_str(value);
        return;
    }
    let mut boundary = max_bytes;
    while boundary > 0 && !value.is_char_boundary(boundary) {
        boundary -= 1;
    }
    target.push_str(&value[..boundary]);
}

fn extract_partial_source_argument(arguments: &str) -> Option<String> {
    ["source", "flowscript", "script", "content"]
        .into_iter()
        .filter_map(|key| find_json_string_value_start(arguments, key))
        .min()
        .and_then(|start| decode_partial_json_string(&arguments[start..]))
}

fn find_json_string_value_start(arguments: &str, key: &str) -> Option<usize> {
    let needle = format!("\"{key}\"");
    let mut search_from = 0usize;
    while let Some(relative) = arguments[search_from..].find(&needle) {
        let key_start = search_from + relative;
        let mut cursor = key_start + needle.len();
        cursor += arguments[cursor..]
            .chars()
            .take_while(|character| character.is_whitespace())
            .map(char::len_utf8)
            .sum::<usize>();
        if arguments.as_bytes().get(cursor) != Some(&b':') {
            search_from = key_start + needle.len();
            continue;
        }
        cursor += 1;
        cursor += arguments[cursor..]
            .chars()
            .take_while(|character| character.is_whitespace())
            .map(char::len_utf8)
            .sum::<usize>();
        if arguments.as_bytes().get(cursor) == Some(&b'"') {
            return Some(cursor + 1);
        }
        search_from = key_start + needle.len();
    }
    None
}

/// Decode the available prefix of a JSON string. A trailing incomplete escape is held until the
/// next provider delta, so snapshots never end with invented replacement characters.
fn decode_partial_json_string(encoded: &str) -> Option<String> {
    let mut output = String::new();
    let mut chars = encoded.chars();
    while let Some(character) = chars.next() {
        match character {
            '"' => return Some(output),
            '\\' => {
                let escaped = chars.next()?;
                match escaped {
                    '"' => output.push('"'),
                    '\\' => output.push('\\'),
                    '/' => output.push('/'),
                    'b' => output.push('\u{0008}'),
                    'f' => output.push('\u{000c}'),
                    'n' => output.push('\n'),
                    'r' => output.push('\r'),
                    't' => output.push('\t'),
                    'u' => {
                        let digits = chars.by_ref().take(4).collect::<String>();
                        if digits.len() != 4 {
                            return None;
                        }
                        let code = u32::from_str_radix(&digits, 16).ok()?;
                        if (0xd800..=0xdbff).contains(&code) {
                            if chars.next() != Some('\\') || chars.next() != Some('u') {
                                return None;
                            }
                            let low_digits = chars.by_ref().take(4).collect::<String>();
                            if low_digits.len() != 4 {
                                return None;
                            }
                            let low = u32::from_str_radix(&low_digits, 16).ok()?;
                            if !(0xdc00..=0xdfff).contains(&low) {
                                return None;
                            }
                            let scalar = 0x1_0000 + ((code - 0xd800) << 10) + (low - 0xdc00);
                            output.push(char::from_u32(scalar)?);
                        } else if (0xdc00..=0xdfff).contains(&code) {
                            return None;
                        } else {
                            output.push(char::from_u32(code)?);
                        }
                    }
                    _ => return None,
                }
            }
            _ => output.push(character),
        }
        if output.len() >= MAX_STREAMED_FLOWSCRIPT_ARGUMENT_BYTES {
            break;
        }
    }
    Some(output)
}

pub fn stream_frame(tag: &str, payload: &Value) -> String {
    // Tool results can contain arbitrary untrusted text (including fetched web pages). JSON does
    // not normally escape angle brackets, so a literal closing tag matching this frame could close
    // the outer protocol early. Escape only that sentinel (not every angle bracket, which would
    // needlessly inflate large FlowScript previews); JSON parsing reconstructs the original text.
    let closing_tag = format!("</{tag}>");
    let escaped_closing_tag = format!("\\u003c/{tag}\\u003e");
    let payload = serde_json::to_string(payload)
        .unwrap_or_default()
        .replace(&closing_tag, &escaped_closing_tag);
    format!("<{tag}>{payload}</{tag}>")
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

/// Detailed tool frames used by provider adapters that have structured arguments/results. Values
/// are recursively redacted and bounded before entering the persisted debug timeline.
pub fn detailed_tool_start_frame(
    tool_call_id: &str,
    tool: &str,
    summary: Option<&str>,
    arguments: Option<&Value>,
) -> String {
    let mut payload = json!({
        "tool_call_id": tool_call_id,
        "tool": tool,
        "status": "running",
    });
    if let Some(object) = payload.as_object_mut() {
        if let Some(summary) = summary {
            object.insert("summary".to_string(), Value::String(summary.to_string()));
        }
        if let Some(arguments) = arguments {
            object.insert(
                "arguments_preview".to_string(),
                Value::String(safe_json_preview(arguments, TOOL_ARGUMENT_PREVIEW_CHARS)),
            );
        }
    }
    stream_frame("tool_start", &payload)
}

pub fn detailed_tool_end_frame(
    tool_call_id: &str,
    tool: &str,
    status: &str,
    terminal_status: Option<&str>,
    result_summary: Option<&str>,
    result_preview: Option<&str>,
) -> String {
    let mut payload = json!({
        "tool_call_id": tool_call_id,
        "tool": tool,
        "status": status,
    });
    if let Some(object) = payload.as_object_mut() {
        if let Some(terminal_status) = terminal_status {
            object.insert(
                "terminal_status".to_string(),
                Value::String(terminal_status.to_string()),
            );
        }
        if let Some(result_summary) = result_summary {
            object.insert(
                "result_summary".to_string(),
                Value::String(result_summary.to_string()),
            );
        }
        if let Some(result_preview) = result_preview {
            object.insert(
                "result_preview".to_string(),
                Value::String(result_preview.to_string()),
            );
        }
    }
    stream_frame("tool_end", &payload)
}

fn sensitive_debug_key(key: &str) -> bool {
    let mut normalized = String::with_capacity(key.len());
    let mut previous_was_lower_or_digit = false;
    for character in key.chars() {
        if character.is_ascii_uppercase() && previous_was_lower_or_digit {
            normalized.push('_');
        }
        if character.is_ascii_alphanumeric() {
            normalized.push(character.to_ascii_lowercase());
            previous_was_lower_or_digit =
                character.is_ascii_lowercase() || character.is_ascii_digit();
        } else {
            normalized.push('_');
            previous_was_lower_or_digit = false;
        }
    }
    let segments = normalized
        .split('_')
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>();
    segments.iter().any(|segment| {
        matches!(
            *segment,
            "secret"
                | "password"
                | "passwd"
                | "passphrase"
                | "token"
                | "authorization"
                | "authentication"
                | "credential"
                | "credentials"
                | "cookie"
        )
    }) || normalized == "auth"
        || normalized == "key"
        || normalized.ends_with("_key")
        || normalized.starts_with("key_")
        || normalized == "apikey"
}

fn redact_bearer_tokens(text: &str) -> String {
    let lowercase = text.to_ascii_lowercase();
    let mut result = String::with_capacity(text.len());
    let mut cursor = 0usize;
    while let Some(relative) = lowercase[cursor..].find("bearer ") {
        let start = cursor + relative;
        let token_start = start + "bearer ".len();
        result.push_str(&text[cursor..token_start]);
        result.push_str("<redacted>");
        let token_len = text[token_start..]
            .find(|character: char| {
                character.is_ascii_whitespace() || matches!(character, ',' | ';' | '"' | '\'')
            })
            .unwrap_or(text.len() - token_start);
        cursor = token_start + token_len;
    }
    result.push_str(&text[cursor..]);
    result
}

fn redact_basic_tokens(text: &str) -> String {
    let lowercase = text.to_ascii_lowercase();
    let mut result = String::with_capacity(text.len());
    let mut cursor = 0usize;
    while let Some(relative) = lowercase[cursor..].find("basic ") {
        let start = cursor + relative;
        let token_start = start + "basic ".len();
        result.push_str(&text[cursor..token_start]);
        result.push_str("<redacted>");
        let token_len = text[token_start..]
            .find(|character: char| {
                character.is_ascii_whitespace() || matches!(character, ',' | ';' | '"' | '\'')
            })
            .unwrap_or(text.len() - token_start);
        cursor = token_start + token_len;
    }
    result.push_str(&text[cursor..]);
    result
}

fn redact_known_secret_tokens(text: &str) -> String {
    const PREFIXES: &[&str] = &[
        "sk-",
        "ghp_",
        "github_pat_",
        "xoxb-",
        "xoxp-",
        "AKIA",
        "AIza",
    ];
    let mut redacted = text.to_string();
    for prefix in PREFIXES {
        let mut result = String::with_capacity(redacted.len());
        let mut cursor = 0usize;
        while let Some(relative) = redacted[cursor..].find(prefix) {
            let start = cursor + relative;
            let before = start
                .checked_sub(1)
                .and_then(|index| redacted.as_bytes().get(index))
                .copied();
            if before.is_some_and(|byte| byte.is_ascii_alphanumeric()) {
                let end = start + prefix.len();
                result.push_str(&redacted[cursor..end]);
                cursor = end;
                continue;
            }
            result.push_str(&redacted[cursor..start]);
            result.push_str("<redacted>");
            let token_end = redacted[start..]
                .find(|character: char| {
                    character.is_ascii_whitespace()
                        || matches!(character, ',' | ';' | '"' | '\'' | ')' | '}' | ']')
                })
                .map(|offset| start + offset)
                .unwrap_or(redacted.len());
            cursor = token_end;
        }
        result.push_str(&redacted[cursor..]);
        redacted = result;
    }
    redacted
}

fn redact_url_query_values(text: &str) -> String {
    text.split_inclusive('\n')
        .map(|line| {
            if !line.contains("://") || !line.contains('?') {
                return line.to_string();
            }
            let Some((base, query_and_fragment)) = line.split_once('?') else {
                return line.to_string();
            };
            let (query, fragment) = query_and_fragment
                .split_once('#')
                .map(|(query, fragment)| (query, Some(fragment)))
                .unwrap_or((query_and_fragment, None));
            let redacted_query = query
                .split('&')
                .map(|part| {
                    part.split_once('=')
                        .map(|(key, value)| {
                            let suffix = value
                                .find(|character: char| {
                                    character.is_ascii_whitespace()
                                        || matches!(character, '"' | '\'' | ')' | '}' | ']')
                                })
                                .map(|index| &value[index..])
                                .unwrap_or_default();
                            format!("{key}=<redacted>{suffix}")
                        })
                        .unwrap_or_else(|| part.to_string())
                })
                .collect::<Vec<_>>()
                .join("&");
            match fragment {
                Some(fragment) => format!("{base}?{redacted_query}#{fragment}"),
                None => format!("{base}?{redacted_query}"),
            }
        })
        .collect()
}

fn redact_private_key_blocks(text: &str) -> String {
    const BEGIN: &str = "-----BEGIN ";
    const END: &str = "-----END ";
    const PRIVATE_KEY_SUFFIX: &str = "PRIVATE KEY-----";

    let mut result = String::with_capacity(text.len());
    let mut cursor = 0usize;
    while let Some(relative_begin) = text[cursor..].find(BEGIN) {
        let begin = cursor + relative_begin;
        let header_end = text[begin..]
            .find('\n')
            .map(|offset| begin + offset)
            .unwrap_or(text.len());
        let header = &text[begin..header_end];
        if !header.contains(PRIVATE_KEY_SUFFIX) {
            result.push_str(&text[cursor..header_end]);
            cursor = header_end;
            continue;
        }

        result.push_str(&text[cursor..begin]);
        result.push_str("<redacted private key>");
        let Some(relative_end) = text[header_end..].find(END) else {
            cursor = text.len();
            break;
        };
        let end_start = header_end + relative_end;
        let end_line_end = text[end_start..]
            .find('\n')
            .map(|offset| end_start + offset + 1)
            .unwrap_or(text.len());
        cursor = end_line_end;
    }
    result.push_str(&text[cursor..]);
    result
}

fn is_secret_marker_boundary(byte: Option<u8>) -> bool {
    byte.is_none_or(|byte| !byte.is_ascii_alphanumeric())
}

/// Redact assignment-style secrets embedded in otherwise useful free-form text (for example a
/// FlowScript submitted as one JSON string). Object-key redaction cannot see those values.
fn redact_inline_secret_values(text: &str) -> String {
    const MARKERS: &[&str] = &[
        "password",
        "passwd",
        "passphrase",
        "clientsecret",
        "client_secret",
        "client-secret",
        "privatekey",
        "private_key",
        "private-key",
        "accesstoken",
        "access_token",
        "access-token",
        "refreshtoken",
        "refresh_token",
        "refresh-token",
        "apikey",
        "api_key",
        "api-key",
        "authorization",
        "credential",
        "credentials",
        "cookie",
        "secret",
        "token",
    ];

    let lowercase = text.to_ascii_lowercase();
    let bytes = text.as_bytes();
    let mut result = String::with_capacity(text.len());
    let mut cursor = 0usize;

    while cursor < text.len() {
        let next = MARKERS
            .iter()
            .filter_map(|marker| {
                let mut search_from = cursor;
                while let Some(relative) = lowercase[search_from..].find(marker) {
                    let start = search_from + relative;
                    let end = start + marker.len();
                    if is_secret_marker_boundary(start.checked_sub(1).map(|index| bytes[index]))
                        && is_secret_marker_boundary(bytes.get(end).copied())
                    {
                        return Some((start, end));
                    }
                    search_from = end;
                }
                None
            })
            .min_by_key(|(start, _)| *start);

        let Some((marker_start, marker_end)) = next else {
            result.push_str(&text[cursor..]);
            break;
        };

        // Only redact a marker that behaves like a field/variable assignment. A mention such as
        // "password reset" stays visible because it has no nearby `=` or `:` delimiter.
        let search_end = text[marker_end..]
            .find(['\n', ',', ';'])
            .map(|offset| marker_end + offset)
            .unwrap_or(text.len());
        let assignment_window = &text[marker_end..search_end];
        let equals = assignment_window
            .find('=')
            .map(|offset| marker_end + offset);
        let colon = assignment_window
            .find(':')
            .map(|offset| marker_end + offset);
        let delimiter = match (equals, colon) {
            // FlowScript declarations commonly use `PASSWORD: string = "..."`; prefer the
            // assignment there. For JSON/text values such as `password: "a=b"`, the quote in
            // the type window identifies the colon as the value delimiter.
            (Some(equals), Some(colon)) if colon < equals => {
                let between = text[colon + 1..equals].trim();
                if !between.is_empty()
                    && between.chars().all(|character| {
                        character.is_ascii_alphanumeric()
                            || matches!(character, '_' | '<' | '>' | '[' | ']' | '?')
                    })
                {
                    Some(equals)
                } else {
                    Some(colon)
                }
            }
            (Some(equals), _) => Some(equals),
            (None, colon) => colon,
        };
        let Some(delimiter) = delimiter else {
            result.push_str(&text[cursor..marker_end]);
            cursor = marker_end;
            continue;
        };

        let mut value_start = delimiter + 1;
        while bytes
            .get(value_start)
            .is_some_and(|byte| byte.is_ascii_whitespace())
        {
            value_start += 1;
        }
        if value_start >= text.len() {
            result.push_str(&text[cursor..value_start]);
            break;
        }

        let mut value_end = value_start;
        let quoted = matches!(bytes[value_start], b'"' | b'\'' | b'`');
        if quoted {
            let quote = bytes[value_start];
            value_end += 1;
            let mut escaped = false;
            while value_end < bytes.len() {
                let current = bytes[value_end];
                value_end += 1;
                if current == quote && !escaped {
                    break;
                }
                escaped = current == b'\\' && !escaped;
                if current != b'\\' {
                    escaped = false;
                }
            }
        } else {
            value_end += text[value_start..]
                .find(['\n', ',', ';', ')', '}'])
                .unwrap_or(text.len() - value_start);
        }

        result.push_str(&text[cursor..value_start]);
        // An empty value carries no secret, and replacing it corrupts captured sources — the
        // FlowScript contract deliberately leaves credentials as "" for the user to fill.
        // Quoted replacements keep their quotes so redacted text stays syntactically valid.
        let raw_value = &text[value_start..value_end];
        let empty_value = if quoted {
            raw_value.len() <= 2
        } else {
            raw_value.trim().is_empty()
        };
        if empty_value {
            result.push_str(raw_value);
        } else if quoted {
            let quote = bytes[value_start] as char;
            result.push(quote);
            result.push_str("<redacted>");
            result.push(quote);
        } else {
            result.push_str("<redacted>");
        }
        cursor = value_end;

        // Avoid repeatedly matching a marker inside the replacement boundary.
        if cursor <= marker_start {
            cursor = marker_end;
        }
    }

    result
}

fn truncate_preview(value: &str, max_chars: usize) -> String {
    let mut preview = value.chars().take(max_chars).collect::<String>();
    if value.chars().count() > max_chars {
        preview.push_str("...");
    }
    preview
}

/// Per-string bound applied while redacting a payload, before the caller's own preview budget
/// shrinks the result. It exists only to stop an unbounded intermediate allocation, so it is set
/// far above any real artifact: a compiler receipt captured in full needs its authored FlowScript
/// byte-for-byte, and the previous 16KB bound silently truncated exactly that.
const DEBUG_STRING_CAP: usize = 10 * 1024 * 1024;

fn safe_debug_string(text: &str) -> String {
    safe_debug_string_with_cap(text, DEBUG_STRING_CAP)
}

/// Like [`safe_debug_string`] with a caller-chosen intermediate bound. Full-result capture
/// (`max_chars == usize::MAX`) must not silently truncate at the debug default: a compiler
/// receipt needs the byte-for-byte source, only redacted.
fn safe_debug_string_with_cap(text: &str, max_chars: usize) -> String {
    // Bound each string before serializing a potentially huge tool payload. The outer preview has
    // its own tighter limit; this cap merely prevents an unbounded intermediate allocation.
    let bounded = truncate_preview(text, max_chars);
    redact_inline_secret_values(&redact_private_key_blocks(&redact_known_secret_tokens(
        &redact_url_query_values(&redact_basic_tokens(&redact_bearer_tokens(&bounded))),
    )))
}

fn redact_debug_value(value: &Value, field_name: Option<&str>) -> Value {
    if field_name.is_some_and(sensitive_debug_key) {
        return Value::String("<redacted>".to_string());
    }
    match value {
        Value::Object(object) => {
            let is_secret = object.get("secret").and_then(Value::as_bool) == Some(true);
            Value::Object(
                object
                    .iter()
                    .map(|(key, value)| {
                        let value = if is_secret
                            && matches!(key.as_str(), "default" | "default_value" | "value")
                        {
                            Value::String("<redacted>".to_string())
                        } else {
                            redact_debug_value(value, Some(key))
                        };
                        (key.clone(), value)
                    })
                    .collect(),
            )
        }
        Value::Array(values) => Value::Array(
            values
                .iter()
                .take(32)
                .map(|value| redact_debug_value(value, None))
                .collect(),
        ),
        Value::String(text) => Value::String(safe_debug_string(text)),
        _ => value.clone(),
    }
}

/// Top-level fields that carry the diagnostic meaning of a tool result. Structure-aware bounding
/// shortens every other string (for example a re-embedded `source`) before touching these.
const PRESERVED_PREVIEW_FIELDS: &[&str] = &[
    "status",
    "code",
    "message",
    "revision",
    "diagnostics",
    "review_notes",
];

/// Strings at or below this length are never shortened, and a shortened string always ends up
/// below it, so a shrink can only reduce the serialized size.
const PREVIEW_SHRINK_MIN_CHARS: usize = 160;
const PREVIEW_SHRINK_KEEP_CHARS: usize = 80;

#[derive(Debug, Clone)]
enum PreviewPathSegment {
    Key(String),
    Index(usize),
}

struct PreviewShrinkCandidate {
    savings: usize,
    path: Vec<PreviewPathSegment>,
}

fn shrunk_preview_string(text: &str) -> String {
    let total = text.chars().count();
    let prefix = text
        .chars()
        .take(PREVIEW_SHRINK_KEEP_CHARS)
        .collect::<String>();
    format!(
        "{prefix}… [{} chars omitted]",
        total - PREVIEW_SHRINK_KEEP_CHARS
    )
}

/// Exact character count this string contributes to the serialized document, quotes and escapes
/// included. Pretty-printing never alters string tokens, so the measurement is exact for both
/// compact and pretty output.
fn serialized_string_chars(text: &str) -> usize {
    serde_json::to_string(text).map_or_else(|_| text.chars().count(), |token| token.chars().count())
}

fn preview_shrink_savings(text: &str) -> Option<usize> {
    if text.chars().count() <= PREVIEW_SHRINK_MIN_CHARS {
        return None;
    }
    let savings = serialized_string_chars(text)
        .saturating_sub(serialized_string_chars(&shrunk_preview_string(text)));
    (savings > 0).then_some(savings)
}

fn collect_shrinkable_strings(
    value: &Value,
    path: &mut Vec<PreviewPathSegment>,
    protected: bool,
    shrink_protected: bool,
    candidates: &mut Vec<PreviewShrinkCandidate>,
) {
    match value {
        Value::Object(object) => {
            for (key, child) in object {
                let child_protected = protected
                    || (path.is_empty() && PRESERVED_PREVIEW_FIELDS.contains(&key.as_str()));
                path.push(PreviewPathSegment::Key(key.clone()));
                collect_shrinkable_strings(
                    child,
                    path,
                    child_protected,
                    shrink_protected,
                    candidates,
                );
                path.pop();
            }
        }
        Value::Array(values) => {
            for (index, child) in values.iter().enumerate() {
                path.push(PreviewPathSegment::Index(index));
                collect_shrinkable_strings(child, path, protected, shrink_protected, candidates);
                path.pop();
            }
        }
        Value::String(text) if protected == shrink_protected => {
            if let Some(savings) = preview_shrink_savings(text) {
                candidates.push(PreviewShrinkCandidate {
                    savings,
                    path: path.clone(),
                });
            }
        }
        _ => {}
    }
}

fn shorten_string_at(value: &mut Value, path: &[PreviewPathSegment]) {
    let mut current = value;
    for segment in path {
        current = match segment {
            PreviewPathSegment::Key(key) => match current.get_mut(key) {
                Some(next) => next,
                None => return,
            },
            PreviewPathSegment::Index(index) => match current.get_mut(index) {
                Some(next) => next,
                None => return,
            },
        };
    }
    if let Value::String(text) = current
        && text.chars().count() > PREVIEW_SHRINK_MIN_CHARS
    {
        *text = shrunk_preview_string(text);
    }
}

/// Decide every truncation for one protection tier in a single budgeting pass: measure each
/// shrinkable string once, shrink the largest savings first, and stop as soon as the overshoot is
/// covered. Shrinking never restructures the document, so the collected paths stay valid and no
/// intermediate re-serialization is needed. Returns the overshoot this tier could not absorb.
fn apply_preview_shrink_budget(
    value: &mut Value,
    shrink_protected: bool,
    overshoot: usize,
) -> usize {
    if overshoot == 0 {
        return 0;
    }
    let mut candidates = Vec::new();
    collect_shrinkable_strings(
        value,
        &mut Vec::new(),
        false,
        shrink_protected,
        &mut candidates,
    );
    candidates.sort_by_key(|candidate| Reverse(candidate.savings));
    let mut remaining = overshoot;
    for candidate in candidates {
        if remaining == 0 {
            break;
        }
        shorten_string_at(value, &candidate.path);
        remaining = remaining.saturating_sub(candidate.savings);
    }
    remaining
}

/// Smallest still-valid preview: the protected diagnostic fields alone, shrunk in place and with
/// oversized arrays collapsed to counts as the very last step. Used when even shrinking every
/// string cannot reach the budget, for example a wide payload whose keys alone exceed it. For
/// pathologically small budgets the envelope may still exceed `max_chars`; validity wins there.
fn minimal_preview_envelope(redacted: &Value, max_chars: usize) -> String {
    let mut fields = serde_json::Map::new();
    fields.insert("preview_truncated".to_string(), Value::Bool(true));
    if let Value::Object(object) = redacted {
        for field in PRESERVED_PREVIEW_FIELDS {
            if let Some(value) = object.get(*field) {
                fields.insert((*field).to_string(), value.clone());
            }
        }
    }
    let mut envelope = Value::Object(fields);
    if let Ok(serialized) = serde_json::to_string_pretty(&envelope) {
        let overshoot = serialized.chars().count().saturating_sub(max_chars);
        if overshoot == 0 {
            return serialized;
        }
        let remaining = apply_preview_shrink_budget(&mut envelope, false, overshoot);
        apply_preview_shrink_budget(&mut envelope, true, remaining);
    }
    if let Ok(serialized) = serde_json::to_string_pretty(&envelope)
        && serialized.chars().count() <= max_chars
    {
        return serialized;
    }
    if let Value::Object(fields) = &mut envelope {
        for field in ["diagnostics", "review_notes"] {
            if let Some(count) = fields.get(field).and_then(Value::as_array).map(Vec::len) {
                fields.insert(
                    field.to_string(),
                    Value::String(format!("{count} item(s) omitted")),
                );
            }
        }
    }
    serde_json::to_string_pretty(&envelope).unwrap_or_else(|_| "<unavailable>".to_string())
}

/// Bounded JSON safe for a user-visible debug report. It never includes values stored under
/// credential-like fields. Oversized payloads are bounded structure-first in one budgeting pass:
/// every shrinkable string is measured exactly once, the largest are truncated until the budget
/// is covered, and the document is serialized at most twice, so `status`, `diagnostics`,
/// `review_notes`, `message`, and `revision` survive intact and the preview stays valid JSON.
/// When protected content itself exceeds the budget it is truncated inside its string values,
/// and a payload whose structure alone overflows the budget collapses to a minimal envelope of
/// the protected fields — the preview never degrades to invalid JSON.
pub fn safe_json_preview(value: &Value, max_chars: usize) -> String {
    let mut redacted = redact_debug_value(value, None);
    let Ok(serialized) = serde_json::to_string_pretty(&redacted) else {
        return "<unavailable>".to_string();
    };
    let overshoot = serialized.chars().count().saturating_sub(max_chars);
    if overshoot == 0 {
        return serialized;
    }
    let remaining = apply_preview_shrink_budget(&mut redacted, false, overshoot);
    apply_preview_shrink_budget(&mut redacted, true, remaining);
    match serde_json::to_string_pretty(&redacted) {
        Ok(serialized) if serialized.chars().count() <= max_chars => serialized,
        _ => minimal_preview_envelope(&redacted, max_chars),
    }
}

/// Bounded, redacted text safe for a user-visible debug report.
pub fn safe_text_preview(text: &str, max_chars: usize) -> String {
    // The intermediate redaction bound must never undercut the caller's requested size, or a
    // "full" capture still comes back truncated at the debug default.
    truncate_preview(
        &safe_debug_string_with_cap(text, max_chars.max(DEBUG_STRING_CAP)),
        max_chars,
    )
}

pub fn safe_tool_result_preview(output: &str, max_chars: usize) -> String {
    match serde_json::from_str::<Value>(output) {
        Ok(value) => safe_json_preview(&value, max_chars),
        Err(_) => safe_text_preview(output, max_chars),
    }
}

pub fn tool_result_terminal_status(output: &str) -> Option<String> {
    serde_json::from_str::<Value>(output)
        .ok()
        .and_then(|value| {
            value
                .get("status")
                .and_then(Value::as_str)
                .map(str::to_owned)
        })
}

fn explicitly_failing_tool_status(status: &str) -> bool {
    matches!(
        status,
        "error"
            | "failed"
            | "failure"
            | "timeout"
            | "timed_out"
            | "denied"
            | "cancelled"
            | "canceled"
            | "unresolved"
            | "infeasible"
            | "zero_progress_circuit_open"
    ) || [
        "_error",
        "_errors",
        "_failed",
        "_failure",
        "_timeout",
        "_rejected",
        "_unavailable",
        "_violation",
        "_mismatch",
        "_conflict",
        "_exhausted",
        "_stalled",
        "_blocked",
        "_refused",
        "_needs_repair",
    ]
    .iter()
    .any(|suffix| status.ends_with(suffix))
}

fn explicitly_advisory_tool_status(status: &str) -> bool {
    matches!(
        status,
        "validation_error"
            | "validation_errors"
            | "draft_needs_repair"
            | "module_needs_repair"
            | "scope_plan_accepted"
            | "scope_plan_required"
            | "declaration_lookup_required"
            | "declaration_lookup_in_flight"
            | "declaration_batch_required"
            | "declaration_follow_up_unrelated"
            | "diagnostic_lookup_required"
            | "duplicate_declaration_lookup"
            | "retained_revision_required"
            | "flowscript_draft_required"
            | "commit_validated_prefix"
            | "predraft_inspection_budget_exhausted"
            | "discovery_budget_exhausted"
            | "time_budget_extended"
            | "time_budget_unavailable"
            | "deferred"
    )
}

/// The plan-step UI has only running/done/error states. Preserve the more specific backend status
/// separately as `terminal_status`. Tool payloads may carry an explicit `is_error` bit; trust it
/// ahead of provider-specific status vocabulary. Otherwise only known-negative statuses fail.
/// Advisory and newly introduced success/progress statuses must not turn red merely because an
/// older client does not recognise their name.
pub fn tool_result_stream_status(output: &str) -> &'static str {
    let parsed = serde_json::from_str::<Value>(output).ok();
    if let Some(is_error) = parsed
        .as_ref()
        .and_then(|value| value.get("is_error").or_else(|| value.get("isError")))
        .and_then(Value::as_bool)
    {
        return if is_error { "error" } else { "done" };
    }
    let Some(status) = parsed
        .as_ref()
        .and_then(|value| value.get("status"))
        .and_then(Value::as_str)
    else {
        return "done";
    };
    let status = status.trim().to_ascii_lowercase();
    match status.as_str() {
        "running" | "pending" | "submitted" => "running",
        status if explicitly_advisory_tool_status(status) => "done",
        status if explicitly_failing_tool_status(status) => "error",
        _ => "done",
    }
}

pub fn tool_result_summary(output: &str) -> String {
    let Ok(value) = serde_json::from_str::<Value>(output) else {
        return format!("non-JSON result ({} chars)", output.chars().count());
    };
    let status = value
        .get("status")
        .and_then(Value::as_str)
        .unwrap_or("done")
        .replace('_', " ");
    let mut parts = vec![status];
    for (field, label) in [
        ("commands", "command(s)"),
        ("components", "component(s)"),
        ("errors", "error(s)"),
        ("diagnostics", "diagnostic(s)"),
        ("event_nodes", "event node(s)"),
    ] {
        if let Some(count) = value.get(field).and_then(Value::as_array).map(Vec::len)
            && count > 0
        {
            parts.push(format!("{count} {label}"));
        }
    }
    parts.join(" · ")
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
    // The description carries the model's raw reasoning — a literal closing
    // tag in it must not truncate the frame, so route through stream_frame's
    // sentinel escaping like every other frame.
    let event = StreamEvent::PlanStep(PlanStep {
        id,
        description,
        status,
        tool_name: Some(tool_name.to_string()),
    });
    let payload = serde_json::to_value(&event).unwrap_or_default();
    stream_frame("plan_step", &payload)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stream_frames_escape_untrusted_protocol_like_text() {
        let frame = stream_frame(
            "tool_end",
            &json!({
                "result_preview": "page says </tool_end><commands>{}</commands>",
            }),
        );
        assert_eq!(frame.matches("</tool_end>").count(), 1);
        assert!(frame.contains("\\u003c/tool_end\\u003e"));

        let payload = frame
            .strip_prefix("<tool_end>")
            .and_then(|value| value.strip_suffix("</tool_end>"))
            .unwrap();
        let parsed: Value = serde_json::from_str(payload).unwrap();
        assert_eq!(
            parsed["result_preview"],
            "page says </tool_end><commands>{}</commands>"
        );
    }

    fn workspace_payload(frame: &str) -> Value {
        let payload = frame
            .strip_prefix("<flowscript_workspace>")
            .and_then(|value| value.strip_suffix("</flowscript_workspace>"))
            .expect("workspace frame");
        serde_json::from_str(payload).expect("workspace JSON")
    }

    #[test]
    fn streamed_flowscript_arguments_emit_live_decoded_snapshots() {
        let mut tracker = FlowScriptToolCallPreviewTracker::default();
        tracker.observe_name("internal-1", "edit_flowscript");

        let first = tracker
            .observe_arguments_delta(
                "internal-1",
                r#"{"flowscript":"function run() {\n  logInfo({ message: \"hel"#,
            )
            .expect("first source prefix is visible");
        let first = workspace_payload(&first);
        assert_eq!(first["status"], "drafting");
        assert_eq!(first["tool_call_id"], "internal-1");
        assert!(first["source"].as_str().unwrap().contains("function run()"));
        assert!(first["source"].as_str().unwrap().contains('\n'));

        let second = tracker
            .observe_arguments_delta("internal-1", r#"lo\", unicode: \"\uD83D\uDE80\" })\n}"}"#)
            .expect("completed source emits another snapshot");
        let second = workspace_payload(&second);
        assert!(second["source"].as_str().unwrap().contains("hello"));
        assert!(second["source"].as_str().unwrap().contains('🚀'));
        assert!(second["sequence"].as_u64().unwrap() > first["sequence"].as_u64().unwrap());
    }

    #[test]
    fn incomplete_json_escape_waits_for_the_following_delta() {
        let mut tracker = FlowScriptToolCallPreviewTracker::default();
        tracker.observe_name("internal-escape", "write_flowscript");
        assert!(
            tracker
                .observe_arguments_delta("internal-escape", r#"{"source":"line one\"#)
                .is_none()
        );
        let frame = tracker
            .observe_arguments_delta("internal-escape", r#"nline two"}"#)
            .expect("completed newline escape becomes visible");
        assert_eq!(workspace_payload(&frame)["source"], "line one\nline two");
    }

    #[test]
    fn interleaved_tool_call_deltas_do_not_cross_contaminate_sources() {
        let mut tracker = FlowScriptToolCallPreviewTracker::default();
        tracker.observe_name("flow", "write_flowscript");
        tracker.observe_name("search", "catalog_search");
        assert!(
            tracker
                .observe_arguments_delta("search", r#"{"query":"email"}"#)
                .is_none()
        );
        let frame = tracker
            .observe_arguments_delta("flow", r#"{"source":"eventsSimple() {\n}"}"#)
            .expect("FlowScript call is tracked independently");
        assert_eq!(workspace_payload(&frame)["source"], "eventsSimple() {\n}");
    }

    #[test]
    fn repeated_or_cumulative_tool_name_deltas_keep_live_preview_enabled() {
        let mut cumulative = FlowScriptToolCallPreviewTracker::default();
        cumulative.observe_name("cumulative", "write_");
        cumulative.observe_name("cumulative", "write_flowscript");
        cumulative.observe_name("cumulative", "write_flowscript");
        let frame = cumulative
            .observe_arguments_delta("cumulative", r#"{"source":"eventsSimple() {\n}"}"#)
            .expect("cumulative provider name remains recognizable");
        assert_eq!(workspace_payload(&frame)["status"], "drafting");

        let mut fragmented = FlowScriptToolCallPreviewTracker::default();
        fragmented.observe_name("fragmented", "write_");
        fragmented.observe_name("fragmented", "flow");
        fragmented.observe_name("fragmented", "script");
        let frame = fragmented
            .observe_arguments_delta("fragmented", r#"{"source":"eventsSimple() {\n}"}"#)
            .expect("fragmented provider name remains recognizable");
        assert_eq!(workspace_payload(&frame)["status"], "drafting");
    }

    #[test]
    fn completed_tool_call_supplies_preview_when_provider_has_no_deltas() {
        let mut tracker = FlowScriptToolCallPreviewTracker::default();
        let frame = tracker
            .complete(
                "internal-full",
                "edit_flowscript",
                &json!({ "flowscript": "eventsSimple() {\n  logInfo({ message: \"ok\" })\n}" }),
            )
            .expect("full tool call emits source");
        let payload = workspace_payload(&frame);
        assert_eq!(payload["status"], "submitted");
        assert_eq!(payload["tool_call_id"], "internal-full");
        assert!(payload["source"].as_str().unwrap().contains("logInfo"));
    }

    #[test]
    fn safe_preview_recursively_redacts_credentials_and_collapses_long_text() {
        let preview = safe_json_preview(
            &json!({
                "password": "pw-value",
                "nested": {
                    "access_token": "token-value",
                    "api-key": "key-value",
                    "accessToken": "camel-token-value",
                    "clientSecret": "camel-secret-value",
                    "privateKey": "camel-key-value",
                },
                "endpoint": "https://example.test/file?X-Amz-Signature=signed-value&token=query-token",
                "header": "Bearer bearer-token-value",
                "basic_header": "Basic YmFzaWMtc2VjcmV0",
                "pem": "-----BEGIN PRIVATE KEY-----\nprivate-key-material\n-----END PRIVATE KEY-----",
                "instruction": "x".repeat(600),
                "harmless": "visible",
            }),
            2_200,
        );

        assert!(!preview.contains("pw-value"));
        assert!(!preview.contains("token-value"));
        assert!(!preview.contains("key-value"));
        assert!(!preview.contains("camel-token-value"));
        assert!(!preview.contains("camel-secret-value"));
        assert!(!preview.contains("camel-key-value"));
        assert!(!preview.contains("signed-value"));
        assert!(!preview.contains("query-token"));
        assert!(!preview.contains("bearer-token-value"));
        assert!(!preview.contains("YmFzaWMtc2VjcmV0"));
        assert!(!preview.contains("private-key-material"));
        assert!(preview.contains("<redacted>"));
        assert!(preview.contains("<redacted private key>"));
        assert!(preview.contains("X-Amz-Signature=<redacted>"));
        assert!(preview.contains("Bearer <redacted>"));
        assert!(preview.contains(&"x".repeat(100)));
        assert!(preview.contains("visible"));
    }

    #[test]
    fn safe_preview_redacts_value_fields_on_secret_objects() {
        let preview = safe_json_preview(
            &json!({
                "variables": [
                    {
                        "name": "imap_password",
                        "secret": true,
                        "default": "must-not-leak-default",
                        "default_value": "must-not-leak-default-value",
                        "value": { "nested": "must-not-leak-value" },
                        "description": "visible secret variable metadata",
                    },
                    {
                        "name": "poll_interval",
                        "secret": false,
                        "default": 30,
                        "default_value": 60,
                        "value": 90,
                    },
                ],
            }),
            2_200,
        );
        let preview: Value = serde_json::from_str(&preview).expect("complete JSON preview");

        let secret = &preview["variables"][0];
        assert_eq!(secret["default"], "<redacted>");
        assert_eq!(secret["default_value"], "<redacted>");
        assert_eq!(secret["value"], "<redacted>");
        assert_eq!(secret["description"], "visible secret variable metadata");

        let public = &preview["variables"][1];
        assert_eq!(public["default"], 30);
        assert_eq!(public["default_value"], 60);
        assert_eq!(public["value"], 90);
    }

    #[test]
    fn long_flowscript_preview_stays_actionable_and_redacts_inline_secrets() {
        let source = r#"@secret
const IMAP_PASSWORD: string = "must-not-leak"

function pollSupportInbox() {
    const connection = emailImapConnect({
        host: "imap.example.test",
        password: "also-must-not-leak"
    })
    logInfo({ message: "poll complete" })
}
"#;
        let preview = safe_json_preview(&json!({ "flowscript": source }), 2_200);
        assert!(preview.contains("pollSupportInbox"));
        assert!(preview.contains("emailImapConnect"));
        assert!(preview.contains("poll complete"));
        assert!(preview.contains("<redacted>"));
        assert!(!preview.contains("must-not-leak"));
        assert!(!preview.contains("also-must-not-leak"));
    }

    #[test]
    fn full_capture_keeps_a_json_source_longer_than_the_debug_cap() {
        // A compiler receipt is captured through the JSON path; a board program well past the
        // 16KB per-string debug default must survive byte-for-byte, or every downstream check
        // (size bounds, lint, wired-id references) silently measures a partial program.
        let source = format!(
            "function longBoard() {{\n{}    logInfo({{ message: \"done\" }})\n}}\n",
            "    logInfo({ message: \"filler statement for a large generated board\" })\n"
                .repeat(400)
        );
        assert!(source.chars().count() > 16_384);

        let preview = safe_tool_result_preview(
            &json!({ "status": "valid", "flowscript": source }).to_string(),
            usize::MAX,
        );

        assert!(preview.contains("longBoard"));
        assert!(!preview.contains("..."));
        let captured: Value = serde_json::from_str(&preview).expect("preview stays valid JSON");
        assert_eq!(
            captured
                .get("flowscript")
                .and_then(Value::as_str)
                .map(|text| text.chars().count()),
            Some(source.chars().count()),
        );
    }

    #[test]
    fn non_json_results_keep_safe_bounded_diagnostics() {
        let preview = safe_tool_result_preview(
            "validation failed at pollSupportInbox: missing Done connection; password=must-not-leak; provider key sk-proj-also-must-not-leak",
            2_800,
        );
        assert!(preview.contains("validation failed at pollSupportInbox"));
        assert!(preview.contains("missing Done connection"));
        assert!(preview.contains("password=<redacted>"));
        assert!(!preview.contains("must-not-leak"));
        assert!(!preview.contains("sk-proj-"));
    }

    #[test]
    fn detailed_frames_include_safe_previews_and_raw_terminal_status() {
        let start = detailed_tool_start_frame(
            "call-1",
            "database_tool",
            Some("insert rows"),
            Some(&json!({ "password": "pw-value", "operation": "insert" })),
        );
        assert!(start.contains("arguments_preview"));
        assert!(start.contains("redacted"));
        assert!(!start.contains("pw-value"));

        let end = detailed_tool_end_frame(
            "call-1",
            "database_tool",
            "error",
            Some("timeout"),
            Some("timeout"),
            Some("{}"),
        );
        assert!(end.contains("terminal_status"));
        assert!(end.contains("timeout"));
        assert!(end.contains("\"status\":\"error\""));
    }

    #[test]
    fn provider_argument_limit_keeps_large_flowscript_tail_visible() {
        let flowscript = format!(
            "function buildSupportWorkflow() {{\n{}\nlogInfo({{ message: \"AUDIT_TAIL_MARKER\" }})\n}}",
            "    logDebug({ message: \"building\" })\n".repeat(90)
        );
        assert!(flowscript.chars().count() > 2_800);

        let frame = detailed_tool_start_frame(
            "call-large",
            "edit_flowscript",
            Some("submit workflow"),
            Some(&json!({ "flowscript": flowscript })),
        );

        assert!(frame.contains("buildSupportWorkflow"));
        assert!(frame.contains("AUDIT_TAIL_MARKER"));
    }

    #[test]
    fn oversized_tool_result_preview_stays_valid_json_and_keeps_diagnostics() {
        let source = "eventsSimple() {\n    logInfo({ message: \"poll\" })\n}\n".repeat(800);
        let response = json!({
            "status": "validation_errors",
            "message": "FlowScript check failed. Apply a unique text patch to this retained revision.",
            "revision": 16,
            "source": source,
            "draft_id": "support-mailbox",
            "diagnostics": [
                {
                    "id": "FSD-1",
                    "code": "FS_FUNCTION_RETURN_MISMATCH",
                    "message": "return value 1 of function pollSupportInbox is not a resolvable FlowScript value",
                },
                {
                    "id": "FSD-2",
                    "code": "FS_UNKNOWN_INPUT_PIN",
                    "message": "unknown input pin `body` on emailSmtpSend",
                },
            ],
            "review_notes": [
                {
                    "id": "FSD-3",
                    "code": "FS_REQUEST_ACCEPTANCE_INCOMPLETE",
                    "message": "The reachable FlowScript workflow does not implement required request scope \"send email\".",
                },
            ],
        });
        let raw = response.to_string();
        assert!(
            raw.len() > 40_000,
            "fixture must exceed 40KB: {}",
            raw.len()
        );

        let preview = safe_tool_result_preview(&raw, TOOL_RESULT_PREVIEW_CHARS);
        assert!(preview.chars().count() <= TOOL_RESULT_PREVIEW_CHARS);
        let parsed: Value =
            serde_json::from_str(&preview).expect("bounded preview must remain valid JSON");
        assert_eq!(parsed["status"], "validation_errors");
        assert_eq!(parsed["revision"], 16);
        assert!(
            parsed["message"]
                .as_str()
                .unwrap()
                .contains("Apply a unique text patch")
        );
        assert!(
            parsed["diagnostics"][0]["message"]
                .as_str()
                .unwrap()
                .contains("pollSupportInbox")
        );
        assert_eq!(parsed["diagnostics"][1]["code"], "FS_UNKNOWN_INPUT_PIN");
        assert_eq!(
            parsed["review_notes"][0]["code"],
            "FS_REQUEST_ACCEPTANCE_INCOMPLETE"
        );
        let bounded_source = parsed["source"].as_str().unwrap();
        assert!(bounded_source.contains("chars omitted"));
        assert!(bounded_source.chars().count() < 200);
    }

    #[test]
    fn multi_megabyte_wide_payloads_preview_to_valid_json_with_diagnostics_present() {
        let mut payload = serde_json::Map::new();
        payload.insert("status".to_string(), json!("validation_errors"));
        payload.insert("code".to_string(), json!("FLOWSCRIPT_CHECK_FAILED"));
        payload.insert(
            "message".to_string(),
            json!("Apply a unique text patch to this retained revision."),
        );
        payload.insert("revision".to_string(), json!(7));
        payload.insert(
            "diagnostics".to_string(),
            json!([{
                "id": "FSD-1",
                "code": "FS_UNKNOWN_INPUT_PIN",
                "message": "unknown input pin `body` on emailSmtpSend",
            }]),
        );
        for index in 0..320 {
            payload.insert(format!("field_{index:03}"), json!("y".repeat(16_000)));
        }
        let payload = Value::Object(payload);
        assert!(
            payload.to_string().len() > 5_000_000,
            "fixture must exceed 5MB"
        );

        let preview = safe_json_preview(&payload, 2_200);
        assert!(preview.chars().count() <= 2_200, "{}", preview.len());
        let parsed: Value =
            serde_json::from_str(&preview).expect("wide oversized preview must stay valid JSON");
        assert_eq!(parsed["status"], "validation_errors");
        assert_eq!(parsed["revision"], 7);
        assert_eq!(parsed["diagnostics"][0]["code"], "FS_UNKNOWN_INPUT_PIN");
    }

    #[test]
    fn oversized_unprotected_strings_shrink_in_one_pass_without_losing_structure() {
        let preview = safe_json_preview(
            &json!({
                "status": "valid",
                "message": "FlowScript is valid.",
                "source": "x".repeat(12_000),
                "harmless": "visible",
            }),
            2_200,
        );
        assert!(preview.chars().count() <= 2_200);
        let parsed: Value = serde_json::from_str(&preview).expect("preview must stay valid JSON");
        assert_eq!(parsed["status"], "valid");
        assert_eq!(parsed["harmless"], "visible");
        let source = parsed["source"].as_str().unwrap();
        assert!(source.starts_with(&"x".repeat(PREVIEW_SHRINK_KEEP_CHARS)));
        assert!(source.contains("chars omitted"));
    }

    #[test]
    fn protected_content_exceeding_the_budget_truncates_inside_strings_keeping_json_valid() {
        let preview = safe_json_preview(
            &json!({
                "status": "error",
                "message": "m".repeat(12_000),
            }),
            1_000,
        );
        assert!(preview.chars().count() <= 1_000);
        let parsed: Value = serde_json::from_str(&preview).expect("preview must stay valid JSON");
        assert_eq!(parsed["status"], "error");
        let message = parsed["message"].as_str().unwrap();
        assert!(message.starts_with(&"m".repeat(PREVIEW_SHRINK_KEEP_CHARS)));
        assert!(message.contains("chars omitted"));
    }

    #[test]
    fn terminal_failure_statuses_are_classified_as_errors() {
        for status in [
            "error",
            "failed",
            "timeout",
            "timed_out",
            "denied",
            "cancelled",
            "scope_plan_rejected",
            "request_identity_mismatch",
            "edit_budget_exhausted",
        ] {
            let output = json!({ "status": status }).to_string();
            assert_eq!(tool_result_stream_status(&output), "error");
            assert_eq!(
                tool_result_terminal_status(&output).as_deref(),
                Some(status)
            );
        }
        for status in [
            "ok",
            "done",
            "queued",
            "already_queued",
            "applied",
            "completed",
            "valid",
            "no_changes",
            "validation_errors",
            "draft_needs_repair",
            "module_needs_repair",
            "scope_plan_accepted",
            "scope_plan_required",
            "declaration_lookup_required",
            "predraft_inspection_budget_exhausted",
            "discovery_budget_exhausted",
            "time_budget_extended",
            "time_budget_unavailable",
            "provider_specific_advisory",
        ] {
            assert_eq!(
                tool_result_stream_status(&json!({ "status": status }).to_string()),
                "done"
            );
        }
    }

    #[test]
    fn explicit_tool_error_flag_overrides_provider_status_vocabulary() {
        assert_eq!(
            tool_result_stream_status(
                &json!({ "status": "validation_errors", "is_error": false }).to_string()
            ),
            "done"
        );
        assert_eq!(
            tool_result_stream_status(
                &json!({ "status": "validation_errors", "is_error": true }).to_string()
            ),
            "error"
        );
        assert_eq!(
            tool_result_stream_status(
                &json!({ "status": "scope_plan_accepted", "isError": false }).to_string()
            ),
            "done"
        );
    }
}
