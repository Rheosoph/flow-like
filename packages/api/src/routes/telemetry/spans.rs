//! Anonymous distributed tracing span ingest.
//!
//! PRIVACY INVARIANT: like the event ingest, this handler is anonymous by
//! construction. It must never extract `Extension(AppUser)` and never store
//! user identity or IP addresses — only the optional, random client-generated
//! `anon_id` and a sanitized span payload.

use axum::{Json, extract::State};
use flow_like_types::Value;
use sea_orm::{ConnectionTrait, DbErr, EntityTrait, Set};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use super::errors::{MAX_RELEASE_LEN, MAX_SHORT_STRING_LEN, optional_string, validate_source};
use super::{parse_client_ts, sanitize_props, validate_anon_id};
use crate::{entity::telemetry_span, error::ApiError, state::AppState, telemetry::sink_from_env};

const MAX_SPANS_PER_BATCH: usize = 200;
const MAX_SPAN_NAME_LEN: usize = 256;
const MAX_ATTRIBUTES_BYTES: usize = 8192;
/// Fits a W3C trace id (32 hex chars) and any reasonable custom id.
const MAX_TRACE_ID_LEN: usize = 64;

/// Span kinds accepted by the ingest, mirroring the `OpenTelemetry` vocabulary.
const SPAN_KINDS: [&str; 5] = ["server", "client", "internal", "producer", "consumer"];
const SPAN_STATUSES: [&str; 2] = ["ok", "error"];

#[derive(Debug, Deserialize, ToSchema)]
pub struct TelemetrySpanPayload {
    /// Trace the span belongs to, 1-64 characters.
    pub trace_id: String,
    /// Span identifier, unique within the trace, 1-64 characters.
    pub span_id: String,
    /// Parent span within the same trace. Null for the trace root.
    #[serde(default)]
    pub parent_span_id: Option<String>,
    /// Operation name, truncated to 256 characters.
    pub name: String,
    /// One of "server", "client", "internal", "producer", "consumer".
    pub kind: String,
    /// Span start (RFC 3339).
    pub started_at: String,
    pub duration_ms: i64,
    /// One of "ok", "error".
    pub status: String,
    /// Free-form anonymous attribute object. Secret-looking keys are redacted
    /// and oversized objects are dropped while the span itself is kept.
    #[serde(default)]
    pub attributes: Option<Value>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct TelemetrySpanIngestPayload {
    /// Random client-generated identifier, 1-64 characters. Never a user id.
    #[serde(default)]
    pub anon_id: Option<String>,
    /// Origin of the batch: "desktop", "desktop_core", "desktop_native", "web" or "backend".
    pub source: String,
    #[serde(default)]
    pub release: Option<String>,
    #[serde(default)]
    pub platform: Option<String>,
    /// Up to 200 spans per batch.
    pub spans: Vec<TelemetrySpanPayload>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct TelemetrySpanIngestResponse {
    pub accepted: usize,
}

#[derive(Debug, Clone, PartialEq)]
struct ValidatedSpan {
    trace_id: String,
    span_id: String,
    parent_span_id: Option<String>,
    name: String,
    kind: String,
    started_at: chrono::DateTime<chrono::FixedOffset>,
    duration_ms: i32,
    status: String,
    attributes: Option<Value>,
}

fn span_id(value: Option<String>) -> Option<String> {
    optional_string(value, MAX_TRACE_ID_LEN)
}

fn normalize_vocab(value: &str, vocab: &[&str]) -> Option<String> {
    let normalized = value.trim().to_ascii_lowercase();
    vocab.contains(&normalized.as_str()).then_some(normalized)
}

/// Redacts secret-looking keys and drops attribute objects that exceed the
/// size cap. An oversized or malformed object costs the attributes, never the
/// span — dropping the span would tear a hole into the trace waterfall.
fn validate_attributes(attributes: Option<Value>) -> Option<Value> {
    let mut attributes = attributes?;
    if !attributes.is_object() {
        return None;
    }
    sanitize_props(&mut attributes);
    let bytes = serde_json::to_vec(&attributes)
        .map(|bytes| bytes.len())
        .unwrap_or(usize::MAX);
    if bytes > MAX_ATTRIBUTES_BYTES {
        return None;
    }
    Some(attributes)
}

fn validate_span(span: TelemetrySpanPayload) -> Option<ValidatedSpan> {
    Some(ValidatedSpan {
        trace_id: span_id(Some(span.trace_id))?,
        span_id: span_id(Some(span.span_id))?,
        parent_span_id: span_id(span.parent_span_id),
        name: optional_string(Some(span.name), MAX_SPAN_NAME_LEN)?,
        kind: normalize_vocab(&span.kind, &SPAN_KINDS)?,
        started_at: parse_client_ts(Some(&span.started_at))?,
        duration_ms: span.duration_ms.clamp(0, i32::MAX as i64) as i32,
        status: normalize_vocab(&span.status, &SPAN_STATUSES)?,
        attributes: validate_attributes(span.attributes),
    })
}

fn validate_spans(spans: Vec<TelemetrySpanPayload>) -> Vec<ValidatedSpan> {
    spans.into_iter().filter_map(validate_span).collect()
}

fn ensure_batch_size(len: usize) -> Result<(), ApiError> {
    if len > MAX_SPANS_PER_BATCH {
        return Err(ApiError::bad_request(format!(
            "A telemetry span batch may contain at most {} spans",
            MAX_SPANS_PER_BATCH
        )));
    }
    Ok(())
}

async fn persist_spans<C: ConnectionTrait>(
    db: &C,
    payload: &TelemetrySpanIngestPayload,
    spans: Vec<ValidatedSpan>,
) -> Result<usize, DbErr> {
    let now = chrono::Utc::now().fixed_offset();
    let accepted = spans.len();
    let models: Vec<telemetry_span::ActiveModel> = spans
        .into_iter()
        .map(|span| telemetry_span::ActiveModel {
            id: Set(flow_like_types::create_id()),
            trace_id: Set(span.trace_id),
            span_id: Set(span.span_id),
            parent_span_id: Set(span.parent_span_id),
            name: Set(span.name),
            kind: Set(span.kind),
            source: Set(payload.source.clone()),
            anon_id: Set(payload.anon_id.clone()),
            release: Set(payload.release.clone()),
            platform: Set(payload.platform.clone()),
            started_at: Set(span.started_at),
            duration_ms: Set(span.duration_ms),
            status: Set(span.status),
            attributes: Set(span.attributes),
            created_at: Set(now),
        })
        .collect();

    telemetry_span::Entity::insert_many(models)
        .exec_without_returning(db)
        .await?;

    Ok(accepted)
}

/// Anonymous by construction: this handler intentionally never extracts
/// `Extension(AppUser)` and never persists user identity or IP addresses.
/// Sampling is a client-side decision — whatever reaches this endpoint is
/// stored, subject only to the batch cap.
#[utoipa::path(
    post,
    path = "/telemetry/spans",
    tag = "telemetry",
    request_body = TelemetrySpanIngestPayload,
    responses(
        (status = 200, description = "Number of spans that were accepted", body = TelemetrySpanIngestResponse),
        (status = 400, description = "Invalid batch"),
        (status = 404, description = "Telemetry is disabled on this platform")
    ),
    description = "Submit a batch of anonymous tracing spans used to render trace waterfalls and performance flamegraphs. No account, user identity or IP address is ever stored."
)]
#[tracing::instrument(name = "POST /telemetry/spans", skip(state, payload))]
pub async fn ingest_spans(
    State(state): State<AppState>,
    Json(mut payload): Json<TelemetrySpanIngestPayload>,
) -> Result<Json<TelemetrySpanIngestResponse>, ApiError> {
    if !state.platform_config.features.telemetry {
        return Err(ApiError::NOT_FOUND);
    }

    if let Some(anon_id) = payload.anon_id.as_deref() {
        validate_anon_id(anon_id)?;
    }
    validate_source(&payload.source)?;
    ensure_batch_size(payload.spans.len())?;

    payload.release = optional_string(payload.release.take(), MAX_RELEASE_LEN);
    payload.platform = optional_string(payload.platform.take(), MAX_SHORT_STRING_LEN);

    let validated = validate_spans(std::mem::take(&mut payload.spans));
    if validated.is_empty() {
        return Ok(Json(TelemetrySpanIngestResponse { accepted: 0 }));
    }

    let sink = sink_from_env();

    if sink == "none" {
        return Ok(Json(TelemetrySpanIngestResponse {
            accepted: validated.len(),
        }));
    }

    if sink == "log" {
        tracing::info!(
            source = %payload.source,
            anon_id = payload.anon_id.as_deref().unwrap_or(""),
            release = payload.release.as_deref().unwrap_or(""),
            platform = payload.platform.as_deref().unwrap_or(""),
            spans = validated.len(),
            traces = ?validated.iter().map(|span| span.trace_id.as_str()).collect::<Vec<_>>(),
            "telemetry span batch"
        );
        return Ok(Json(TelemetrySpanIngestResponse {
            accepted: validated.len(),
        }));
    }

    match persist_spans(&state.db, &payload, validated).await {
        Ok(accepted) => Ok(Json(TelemetrySpanIngestResponse { accepted })),
        Err(e) => {
            tracing::error!("Failed to persist telemetry span batch: {}", e);
            Ok(Json(TelemetrySpanIngestResponse { accepted: 0 }))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const START: &str = "2026-07-26T10:00:00Z";

    fn span(name: &str, kind: &str, status: &str) -> TelemetrySpanPayload {
        TelemetrySpanPayload {
            trace_id: "trace-1".to_string(),
            span_id: "span-1".to_string(),
            parent_span_id: None,
            name: name.to_string(),
            kind: kind.to_string(),
            started_at: START.to_string(),
            duration_ms: 12,
            status: status.to_string(),
            attributes: None,
        }
    }

    #[test]
    fn accepts_every_documented_kind_and_status() {
        for kind in SPAN_KINDS {
            for status in SPAN_STATUSES {
                let validated = validate_span(span("db.query", kind, status)).unwrap();
                assert_eq!(validated.kind, kind);
                assert_eq!(validated.status, status);
            }
        }
    }

    #[test]
    fn normalizes_kind_and_status_case_and_whitespace() {
        let validated = validate_span(span("db.query", " SERVER ", " Error ")).unwrap();
        assert_eq!(validated.kind, "server");
        assert_eq!(validated.status, "error");
    }

    #[test]
    fn drops_spans_with_an_unknown_kind_or_status() {
        assert!(validate_span(span("db.query", "database", "ok")).is_none());
        assert!(validate_span(span("db.query", "server", "cancelled")).is_none());
        assert!(validate_span(span("db.query", "", "ok")).is_none());
    }

    #[test]
    fn drops_spans_with_invalid_ids_or_start() {
        let long_id = "t".repeat(MAX_TRACE_ID_LEN + 1);
        assert!(
            validate_span(TelemetrySpanPayload {
                trace_id: String::new(),
                ..span("db.query", "server", "ok")
            })
            .is_none()
        );
        assert!(
            validate_span(TelemetrySpanPayload {
                span_id: "   ".to_string(),
                ..span("db.query", "server", "ok")
            })
            .is_none()
        );
        assert!(
            validate_span(TelemetrySpanPayload {
                started_at: "yesterday".to_string(),
                ..span("db.query", "server", "ok")
            })
            .is_none()
        );
        let truncated = validate_span(TelemetrySpanPayload {
            trace_id: long_id.clone(),
            parent_span_id: Some(long_id),
            ..span("db.query", "server", "ok")
        })
        .unwrap();
        assert_eq!(truncated.trace_id.len(), MAX_TRACE_ID_LEN);
        assert_eq!(
            truncated.parent_span_id.as_deref().map(str::len),
            Some(MAX_TRACE_ID_LEN)
        );
    }

    #[test]
    fn caps_the_span_name_and_drops_empty_names() {
        let long =
            validate_span(span(&"n".repeat(MAX_SPAN_NAME_LEN + 10), "server", "ok")).unwrap();
        assert_eq!(long.name.len(), MAX_SPAN_NAME_LEN);
        assert!(validate_span(span("   ", "server", "ok")).is_none());
    }

    #[test]
    fn clamps_durations_into_the_stored_range() {
        let negative = validate_span(TelemetrySpanPayload {
            duration_ms: -5,
            ..span("db.query", "server", "ok")
        })
        .unwrap();
        assert_eq!(negative.duration_ms, 0);

        let huge = validate_span(TelemetrySpanPayload {
            duration_ms: i64::MAX,
            ..span("db.query", "server", "ok")
        })
        .unwrap();
        assert_eq!(huge.duration_ms, i32::MAX);
    }

    #[test]
    fn redacts_secret_attributes_at_any_depth() {
        let validated = validate_span(TelemetrySpanPayload {
            attributes: Some(json!({
                "http.route": "/api/v1/apps",
                "token": "abc",
                "nested": { "API_KEY": "x", "list": [{ "refresh_token": "y", "keep": 1 }] }
            })),
            ..span("http.request", "server", "ok")
        })
        .unwrap();
        let attributes = validated.attributes.unwrap();
        assert_eq!(attributes["token"], "[REDACTED]");
        assert_eq!(attributes["nested"]["API_KEY"], "[REDACTED]");
        assert_eq!(
            attributes["nested"]["list"][0]["refresh_token"],
            "[REDACTED]"
        );
        assert_eq!(attributes["http.route"], "/api/v1/apps");
        assert_eq!(attributes["nested"]["list"][0]["keep"], 1);
    }

    #[test]
    fn oversized_or_non_object_attributes_are_dropped_but_the_span_survives() {
        let oversized = validate_span(TelemetrySpanPayload {
            attributes: Some(json!({ "blob": "x".repeat(MAX_ATTRIBUTES_BYTES + 1) })),
            ..span("http.request", "server", "ok")
        })
        .unwrap();
        assert_eq!(oversized.attributes, None);

        let scalar = validate_span(TelemetrySpanPayload {
            attributes: Some(json!("not-an-object")),
            ..span("http.request", "server", "ok")
        })
        .unwrap();
        assert_eq!(scalar.attributes, None);

        let fits = validate_span(TelemetrySpanPayload {
            attributes: Some(json!({ "blob": "x".repeat(MAX_ATTRIBUTES_BYTES / 2) })),
            ..span("http.request", "server", "ok")
        })
        .unwrap();
        assert!(fits.attributes.is_some());
    }

    #[test]
    fn enforces_the_batch_cap() {
        assert!(ensure_batch_size(0).is_ok());
        assert!(ensure_batch_size(MAX_SPANS_PER_BATCH).is_ok());
        assert!(ensure_batch_size(MAX_SPANS_PER_BATCH + 1).is_err());
    }

    #[test]
    fn invalid_spans_do_not_drop_the_rest_of_the_batch() {
        let validated = validate_spans(vec![
            span("root", "server", "ok"),
            span("bad", "weird", "ok"),
            span("child", "client", "error"),
        ]);
        assert_eq!(validated.len(), 2);
        assert_eq!(validated[0].name, "root");
        assert_eq!(validated[1].name, "child");
    }

    #[test]
    fn rejects_the_reserved_backend_anon_id() {
        assert!(validate_anon_id("backend").is_err());
        assert!(validate_anon_id("a1b2c3").is_ok());
    }
}
