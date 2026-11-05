use flow_like_types::bot_message::WorkspaceHandle;
use flow_like_types::data_classification::DataClassification;
use serde::Serialize;
use serde_json::{Map, Value, json};

use crate::error::ToolError;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Ok,
    Error,
    NeedsApproval,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Usage {
    pub cpu_ms: u64,
    pub model_in: u64,
    pub model_out: u64,
}

/// What a tool returns before the envelope is built.
/// An image the model should see next to the result (core attaches it to the message).
#[derive(Debug, Clone)]
pub struct ToolImage {
    pub label: String,
    pub mime: String,
    pub data: flow_like_types::Bytes,
}

#[derive(Debug, Clone, Default)]
pub struct ToolOutput {
    pub result: Value,
    pub saved: Vec<WorkspaceHandle>,
    pub warnings: Vec<String>,
    pub usage: Usage,
    pub classification: Option<DataClassification>,
    pub truncated: bool,
    pub images: Vec<ToolImage>,
}

impl ToolOutput {
    pub fn new(result: Value) -> Self {
        Self {
            result,
            ..Self::default()
        }
    }

    pub fn saved(mut self, handle: WorkspaceHandle) -> Self {
        self.saved.push(handle);
        self
    }

    pub fn warn(mut self, warning: impl Into<String>) -> Self {
        self.warnings.push(warning.into());
        self
    }

    pub fn truncated(mut self, truncated: bool) -> Self {
        self.truncated = truncated;
        self
    }

    pub fn classified(mut self, level: Option<DataClassification>) -> Self {
        self.classification = level;
        self
    }
}

#[derive(Debug, Clone)]
pub struct ToolOutcome {
    pub status: Status,
    pub tool: String,
    pub result: Value,
    pub error: Option<ToolError>,
    pub truncated: bool,
    pub saved: Vec<WorkspaceHandle>,
    pub warnings: Vec<String>,
    pub usage: Usage,
    pub classification: Option<DataClassification>,
    pub images: Vec<ToolImage>,
}

impl ToolOutcome {
    pub fn from_output(tool: &str, mut output: ToolOutput, cap: usize) -> Self {
        let capped = cap_value(&mut output.result, cap);
        if capped {
            output.warnings.push(format!(
                "The result exceeded {cap} bytes and was shortened; ask for less or page through it"
            ));
        }
        Self {
            status: Status::Ok,
            tool: tool.to_string(),
            result: output.result,
            error: None,
            truncated: output.truncated || capped,
            saved: output.saved,
            warnings: output.warnings,
            usage: output.usage,
            classification: output.classification,
            images: output.images,
        }
    }

    pub fn from_error(tool: &str, error: ToolError) -> Self {
        let status = if error.code == crate::error::code::NEEDS_APPROVAL {
            Status::NeedsApproval
        } else {
            Status::Error
        };
        Self {
            status,
            tool: tool.to_string(),
            result: Value::Null,
            error: Some(error),
            truncated: false,
            saved: Vec::new(),
            warnings: Vec::new(),
            usage: Usage::default(),
            classification: None,
            images: Vec::new(),
        }
    }

    pub fn is_ok(&self) -> bool {
        self.status == Status::Ok
    }

    /// `{status, tool, …result fields, truncated, saved, warnings, usage, classification}`.
    pub fn envelope(&self) -> Value {
        let mut envelope = Map::new();
        envelope.insert("status".into(), json!(self.status));
        envelope.insert("tool".into(), json!(self.tool));
        match &self.result {
            Value::Object(fields) => envelope.extend(fields.clone()),
            Value::Null => {}
            other => {
                envelope.insert("result".into(), other.clone());
            }
        }
        if let Some(Value::Object(details)) = self.error.as_ref().and_then(|e| e.details.clone()) {
            envelope.extend(details);
        }
        if let Some(error) = &self.error {
            envelope.insert(
                "error".into(),
                json!({ "code": error.code, "message": error.message }),
            );
        }
        envelope.insert("truncated".into(), json!(self.truncated));
        envelope.insert("saved".into(), json!(self.saved));
        envelope.insert("warnings".into(), json!(self.warnings));
        envelope.insert("usage".into(), json!(self.usage));
        envelope.insert(
            "classification".into(),
            json!(self.classification.map(DataClassification::as_str)),
        );
        Value::Object(envelope)
    }
}

const MIN_SHRINK: usize = 200;

fn serialized_len(value: &Value) -> usize {
    value.to_string().len()
}

/// Shortens the value until its JSON fits `cap` bytes: the longest string loses its middle
/// first, then the longest array its tail. Returns whether anything was cut.
pub fn cap_value(value: &mut Value, cap: usize) -> bool {
    let mut cut = false;
    while serialized_len(value) > cap {
        let longest_text = max_len(value, &|v| v.as_str().map(str::len));
        let longest_list = max_len(value, &|v| v.as_array().map(Vec::len));
        if longest_text > MIN_SHRINK {
            let target = (longest_text / 2).max(MIN_SHRINK / 2);
            rewrite_first(value, &|v| match v {
                Value::String(text) if text.len() == longest_text => {
                    *text = shrink_string(text, target);
                    true
                }
                _ => false,
            });
        } else if longest_list > 1 {
            rewrite_first(value, &|v| match v {
                Value::Array(items) if items.len() == longest_list => {
                    items.truncate(longest_list / 2);
                    true
                }
                _ => false,
            });
        } else {
            break;
        }
        cut = true;
    }
    cut
}

fn max_len(value: &Value, measure: &dyn Fn(&Value) -> Option<usize>) -> usize {
    let own = measure(value).unwrap_or(0);
    let inner = match value {
        Value::Array(items) => items.iter().map(|item| max_len(item, measure)).max(),
        Value::Object(fields) => fields.values().map(|item| max_len(item, measure)).max(),
        _ => None,
    };
    own.max(inner.unwrap_or(0))
}

fn rewrite_first(value: &mut Value, rewrite: &dyn Fn(&mut Value) -> bool) -> bool {
    if rewrite(value) {
        return true;
    }
    match value {
        Value::Array(items) => items.iter_mut().any(|item| rewrite_first(item, rewrite)),
        Value::Object(fields) => fields.values_mut().any(|item| rewrite_first(item, rewrite)),
        _ => false,
    }
}

fn floor_boundary(text: &str, mut index: usize) -> usize {
    index = index.min(text.len());
    while !text.is_char_boundary(index) {
        index -= 1;
    }
    index
}

/// Keeps the head and the tail of `text` within about `target` bytes around a marker.
pub fn shrink_string(text: &str, target: usize) -> String {
    let head_end = floor_boundary(text, target * 2 / 3);
    let mut tail_start = floor_boundary(text, text.len().saturating_sub(target / 3));
    while !text.is_char_boundary(tail_start) {
        tail_start += 1;
    }
    let tail_start = tail_start.max(head_end);
    let removed = tail_start - head_end;
    format!(
        "{}\n…[{removed} bytes truncated]…\n{}",
        &text[..head_end],
        &text[tail_start..]
    )
}
