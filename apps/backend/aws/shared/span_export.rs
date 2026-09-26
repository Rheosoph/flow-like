//! Export policy shared by every AWS API target. Spans leave the process only
//! through [`sanitize`], so the Lambda and ECS APIs cannot drift apart on what
//! reaches the collector.

use opentelemetry::trace::{SpanId, TraceId};
use opentelemetry::{KeyValue, Value};
use opentelemetry_sdk::trace::{IdGenerator, RandomIdGenerator, SpanData};
use std::time::{SystemTime, UNIX_EPOCH};

pub fn sanitize(span: &mut SpanData) {
    // Only our explicit target is collected. This second boundary prevents a
    // future instrument(skip_all) omission from exporting handler arguments.
    span.attributes.retain(safe_attribute);
    span.events = Default::default();
    span.links = Default::default();
    if matches!(span.status, opentelemetry::trace::Status::Error { .. })
        || span
            .attributes
            .iter()
            .any(|attribute| attribute.key.as_str() == "error.type")
    {
        span.status = opentelemetry::trace::Status::error("");
    }
    if span.name.len() > 96
        || !span
            .name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
    {
        span.name = "operation".into();
    }
}

fn safe_attribute(attribute: &KeyValue) -> bool {
    let key = attribute.key.as_str();
    match (&attribute.value, key) {
        (Value::I64(_), "http.status_code" | "http.response.status_code" | "retry_count") => true,
        (Value::Bool(_), "faas.coldstart" | "http.cancelled") => true,
        (
            Value::F64(value),
            "http.response_ready_ms" | "http.first_byte_ms" | "http.duration_ms",
        ) => value.is_finite() && *value >= 0.0,
        (Value::String(value), "http.route") => {
            let value = value.as_str();
            value.len() <= 256
                && (value == "unmatched" || value.starts_with('/'))
                && value
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"/_-.{}:*".contains(&b))
        }
        (
            Value::String(value),
            "http.method"
            | "http.request.method"
            | "db.operation"
            | "db.system.name"
            | "db.table"
            | "rpc.service"
            | "rpc.method"
            | "cloud.service"
            | "error.type",
        ) => {
            value.as_str().len() <= 64
                && value
                    .as_str()
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
        }
        (Value::String(value), "faas.invocation_id") => {
            let value = value.as_str();
            value.len() == 36
                && value.bytes().enumerate().all(|(i, b)| {
                    if [8, 13, 18, 23].contains(&i) {
                        b == b'-'
                    } else {
                        b.is_ascii_hexdigit()
                    }
                })
        }
        _ => false,
    }
}

/// X-Ray time-prefixed trace ids for roots this process starts.
#[derive(Debug, Default)]
pub struct XrayIds(RandomIdGenerator);

impl IdGenerator for XrayIds {
    fn new_trace_id(&self) -> TraceId {
        let mut bytes = self.0.new_trace_id().to_bytes();
        let seconds = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as u32;
        bytes[..4].copy_from_slice(&seconds.to_be_bytes());
        TraceId::from_bytes(bytes)
    }

    fn new_span_id(&self) -> SpanId {
        self.0.new_span_id()
    }
}
