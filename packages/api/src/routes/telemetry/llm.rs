//! Anonymous LLM call telemetry ingest.
//!
//! PRIVACY INVARIANT: like the event ingest, this handler is anonymous by
//! construction. It must never extract `Extension(AppUser)` and never store
//! user identity or IP addresses — only the random, client-generated `anon_id`.
//!
//! CONTENT INVARIANT: this endpoint measures LLM calls, it does not observe
//! them. A batch carrying a key that could hold model input or output is
//! rejected outright instead of being sanitized, so a client that has not been
//! updated cannot quietly leak conversation content.

use axum::{Json, extract::State};
use flow_like_types::Value;
use sea_orm::{ConnectionTrait, DbErr, EntityTrait};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use utoipa::ToSchema;

use super::errors::{MAX_RELEASE_LEN, optional_string, validate_source};
use super::validate_anon_id;
use crate::{
    entity::telemetry_llm_call,
    error::ApiError,
    state::AppState,
    telemetry::{
        llm::{LlmCallRecord, active_model, normalize_record},
        sink_from_env,
    },
};

const MAX_CALLS_PER_BATCH: usize = 50;

/// Keys that could carry a prompt, a completion or tool payloads. Their mere
/// presence anywhere in a call object fails the whole batch.
const FORBIDDEN_CONTENT_KEYS: [&str; 6] = [
    "prompt", "response", "messages", "input", "output", "content",
];

#[derive(Debug, Deserialize, ToSchema)]
pub struct TelemetryLlmCallPayload {
    /// Vendor that served the call, e.g. "openai" or "bedrock".
    pub provider: String,
    /// Model identifier, e.g. "gpt-5-mini".
    pub model: String,
    /// One of "chat", "embed" or "tool".
    pub operation: String,
    pub duration_ms: i64,
    #[serde(default)]
    pub prompt_tokens: Option<i64>,
    #[serde(default)]
    pub completion_tokens: Option<i64>,
    /// Derived from the prompt and completion counts when omitted.
    #[serde(default)]
    pub total_tokens: Option<i64>,
    /// One of "ok" or "error".
    pub status: String,
    /// Classified failure label. Never an error message.
    #[serde(default)]
    pub error_kind: Option<String>,
    #[serde(default)]
    pub tool_calls: Option<i64>,
    #[serde(default)]
    pub streamed: Option<bool>,
    /// Accepted for client compatibility; the server-side ingest timestamp is
    /// authoritative, so the client value is deliberately never stored.
    #[serde(default)]
    #[allow(dead_code)]
    pub client_ts: Option<String>,
    /// Any other key the client sends. Content-bearing keys fail the batch.
    #[serde(flatten)]
    #[schema(ignore)]
    pub extra: BTreeMap<String, Value>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct TelemetryLlmIngestPayload {
    /// Random client-generated identifier, 1-64 characters. Never a user id.
    #[serde(default)]
    pub anon_id: Option<String>,
    /// Origin of the batch: "desktop", "desktop_core", "desktop_native", "web" or "backend".
    pub source: String,
    #[serde(default)]
    pub release: Option<String>,
    /// Up to 50 calls per batch.
    pub calls: Vec<TelemetryLlmCallPayload>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct TelemetryLlmIngestResponse {
    pub accepted: usize,
}

fn is_forbidden_key(key: &str) -> bool {
    FORBIDDEN_CONTENT_KEYS.contains(&key.trim().to_ascii_lowercase().as_str())
}

/// Finds the first content-bearing key at any depth of an extra value.
fn forbidden_content_key(value: &Value) -> Option<String> {
    match value {
        Value::Object(map) => map.iter().find_map(|(key, entry)| {
            if is_forbidden_key(key) {
                Some(key.clone())
            } else {
                forbidden_content_key(entry)
            }
        }),
        Value::Array(items) => items.iter().find_map(forbidden_content_key),
        _ => None,
    }
}

fn content_key_in(call: &TelemetryLlmCallPayload) -> Option<String> {
    call.extra.iter().find_map(|(key, value)| {
        if is_forbidden_key(key) {
            Some(key.clone())
        } else {
            forbidden_content_key(value)
        }
    })
}

fn ensure_no_content(calls: &[TelemetryLlmCallPayload]) -> Result<(), ApiError> {
    match calls.iter().find_map(content_key_in) {
        Some(key) => Err(ApiError::bad_request(format!(
            "LLM telemetry never accepts model input or output; remove the '{}' key",
            key
        ))),
        None => Ok(()),
    }
}

fn ensure_batch_size(len: usize) -> Result<(), ApiError> {
    if len > MAX_CALLS_PER_BATCH {
        return Err(ApiError::bad_request(format!(
            "A telemetry LLM batch may contain at most {} calls",
            MAX_CALLS_PER_BATCH
        )));
    }
    Ok(())
}

/// Client counters are 64 bit; the columns are 32 bit and never negative.
fn clamp_count(value: i64) -> i32 {
    value.clamp(0, i64::from(i32::MAX)) as i32
}

/// A negative count is meaningless rather than zero, so it is dropped instead
/// of being clamped into a real measurement.
fn clamp_optional_count(value: Option<i64>) -> Option<i32> {
    value.filter(|count| *count >= 0).map(clamp_count)
}

fn validate_call(call: TelemetryLlmCallPayload) -> Option<LlmCallRecord> {
    normalize_record(LlmCallRecord {
        provider: call.provider,
        model: call.model,
        operation: call.operation,
        duration_ms: clamp_count(call.duration_ms),
        prompt_tokens: clamp_optional_count(call.prompt_tokens),
        completion_tokens: clamp_optional_count(call.completion_tokens),
        total_tokens: clamp_optional_count(call.total_tokens),
        status: call.status,
        error_kind: call.error_kind,
        tool_calls: clamp_optional_count(call.tool_calls).unwrap_or(0),
        streamed: call.streamed.unwrap_or(false),
    })
}

fn validate_calls(calls: Vec<TelemetryLlmCallPayload>) -> Vec<LlmCallRecord> {
    calls.into_iter().filter_map(validate_call).collect()
}

async fn persist_calls<C: ConnectionTrait>(
    db: &C,
    payload: &TelemetryLlmIngestPayload,
    records: Vec<LlmCallRecord>,
) -> Result<usize, DbErr> {
    let now = chrono::Utc::now().fixed_offset();
    let accepted = records.len();
    let models: Vec<telemetry_llm_call::ActiveModel> = records
        .into_iter()
        .map(|record| {
            active_model(
                record,
                &payload.source,
                payload.anon_id.as_deref(),
                payload.release.as_deref(),
                now,
            )
        })
        .collect();

    telemetry_llm_call::Entity::insert_many(models)
        .exec_without_returning(db)
        .await?;

    Ok(accepted)
}

/// Anonymous by construction: this handler intentionally never extracts
/// `Extension(AppUser)` and never persists user identity, IP addresses or any
/// prompt or completion content.
#[utoipa::path(
    post,
    path = "/telemetry/llm",
    tag = "telemetry",
    request_body = TelemetryLlmIngestPayload,
    responses(
        (status = 200, description = "Number of LLM calls that were accepted", body = TelemetryLlmIngestResponse),
        (status = 400, description = "Invalid batch, or a call carried prompt or completion content"),
        (status = 404, description = "Telemetry is disabled on this platform")
    ),
    description = "Submit a batch of anonymous LLM call measurements: provider, model, kind of call, duration, token counts and outcome. Prompts, completions and tool payloads are never accepted, and no account, user identity or IP address is ever stored."
)]
#[tracing::instrument(name = "POST /telemetry/llm", skip(state, payload))]
pub async fn ingest_llm_calls(
    State(state): State<AppState>,
    Json(mut payload): Json<TelemetryLlmIngestPayload>,
) -> Result<Json<TelemetryLlmIngestResponse>, ApiError> {
    if !state.platform_config.features.telemetry {
        return Err(ApiError::NOT_FOUND);
    }

    payload.anon_id = payload
        .anon_id
        .take()
        .map(|anon_id| anon_id.trim().to_string())
        .filter(|anon_id| !anon_id.is_empty());
    if let Some(anon_id) = &payload.anon_id {
        validate_anon_id(anon_id)?;
    }
    validate_source(&payload.source)?;
    ensure_batch_size(payload.calls.len())?;
    ensure_no_content(&payload.calls)?;

    payload.release = optional_string(payload.release.take(), MAX_RELEASE_LEN);

    let records = validate_calls(std::mem::take(&mut payload.calls));
    if records.is_empty() {
        return Ok(Json(TelemetryLlmIngestResponse { accepted: 0 }));
    }

    let sink = sink_from_env();

    if sink == "none" {
        return Ok(Json(TelemetryLlmIngestResponse {
            accepted: records.len(),
        }));
    }

    if sink == "log" {
        tracing::info!(
            source = %payload.source,
            anon_id = payload.anon_id.as_deref().unwrap_or(""),
            release = payload.release.as_deref().unwrap_or(""),
            calls = records.len(),
            models = ?records.iter().map(|record| record.model.as_str()).collect::<Vec<_>>(),
            "telemetry llm batch"
        );
        return Ok(Json(TelemetryLlmIngestResponse {
            accepted: records.len(),
        }));
    }

    match persist_calls(&state.db, &payload, records).await {
        Ok(accepted) => Ok(Json(TelemetryLlmIngestResponse { accepted })),
        Err(e) => {
            tracing::error!("Failed to persist telemetry LLM batch: {}", e);
            Ok(Json(TelemetryLlmIngestResponse { accepted: 0 }))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::telemetry::llm::{LLM_STATUS_ERROR, LLM_STATUS_OK};
    use serde_json::json;

    fn payload(source: &str, calls: Vec<TelemetryLlmCallPayload>) -> TelemetryLlmIngestPayload {
        TelemetryLlmIngestPayload {
            anon_id: Some("a1b2c3".to_string()),
            source: source.to_string(),
            release: None,
            calls,
        }
    }

    fn call() -> TelemetryLlmCallPayload {
        TelemetryLlmCallPayload {
            provider: "openai".to_string(),
            model: "gpt-5-mini".to_string(),
            operation: "chat".to_string(),
            duration_ms: 1200,
            prompt_tokens: Some(400),
            completion_tokens: Some(120),
            total_tokens: None,
            status: LLM_STATUS_OK.to_string(),
            error_kind: None,
            tool_calls: None,
            streamed: None,
            client_ts: None,
            extra: BTreeMap::new(),
        }
    }

    fn parse(value: serde_json::Value) -> TelemetryLlmIngestPayload {
        serde_json::from_value(value).expect("payload should deserialize")
    }

    #[test]
    fn every_forbidden_content_key_fails_the_batch() {
        for key in FORBIDDEN_CONTENT_KEYS {
            let parsed = parse(json!({
                "anon_id": "a1b2c3",
                "source": "desktop",
                "calls": [{
                    "provider": "openai",
                    "model": "gpt-5-mini",
                    "operation": "chat",
                    "duration_ms": 10,
                    "status": "ok",
                    key: "leaked"
                }]
            }));
            assert_eq!(parsed.calls[0].extra.len(), 1, "'{key}' should be captured");
            let error = ensure_no_content(&parsed.calls)
                .expect_err(&format!("expected '{key}' to fail the batch"));
            let reported = format!("{error:?}");
            assert!(reported.contains(key), "{reported}");
        }
    }

    #[test]
    fn content_keys_are_caught_case_insensitively_and_when_nested() {
        let parsed = parse(json!({
            "source": "web",
            "calls": [{
                "provider": "openai",
                "model": "gpt-5-mini",
                "operation": "chat",
                "duration_ms": 10,
                "status": "ok",
                "debug": { "trace": [{ "Messages": ["hi"] }] }
            }]
        }));
        assert!(ensure_no_content(&parsed.calls).is_err());

        let uppercase = parse(json!({
            "source": "web",
            "calls": [{
                "provider": "openai",
                "model": "gpt-5-mini",
                "operation": "chat",
                "duration_ms": 10,
                "status": "ok",
                "PROMPT": "hello"
            }]
        }));
        assert!(ensure_no_content(&uppercase.calls).is_err());
    }

    #[test]
    fn a_batch_without_content_keys_is_accepted() {
        let parsed = parse(json!({
            "anon_id": "a1b2c3",
            "source": "desktop",
            "release": "1.2.3",
            "calls": [{
                "provider": "openai",
                "model": "gpt-5-mini",
                "operation": "chat",
                "duration_ms": 1200,
                "prompt_tokens": 400,
                "completion_tokens": 120,
                "status": "ok",
                "tool_calls": 2,
                "streamed": true,
                "client_ts": "2026-07-26T10:00:00Z"
            }]
        }));
        assert!(parsed.calls[0].extra.is_empty());
        assert!(ensure_no_content(&parsed.calls).is_ok());

        let records = validate_calls(parsed.calls);
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].total_tokens, Some(520));
        assert_eq!(records[0].tool_calls, 2);
        assert!(records[0].streamed);
    }

    #[test]
    fn known_fields_never_land_in_the_extra_map() {
        let parsed = parse(json!({
            "source": "backend",
            "calls": [{
                "provider": "bedrock",
                "model": "claude",
                "operation": "embed",
                "duration_ms": 5,
                "total_tokens": 9,
                "status": "error",
                "error_kind": "timeout"
            }]
        }));
        assert!(parsed.calls[0].extra.is_empty());
        assert_eq!(parsed.calls[0].error_kind.as_deref(), Some("timeout"));
    }

    #[test]
    fn invalid_calls_are_dropped_without_failing_the_batch() {
        let records = validate_calls(vec![
            call(),
            TelemetryLlmCallPayload {
                operation: "rerank".to_string(),
                ..call()
            },
            TelemetryLlmCallPayload {
                status: "boom".to_string(),
                ..call()
            },
            TelemetryLlmCallPayload {
                provider: "  ".to_string(),
                ..call()
            },
        ]);
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].provider, "openai");
    }

    #[test]
    fn oversized_client_counters_are_clamped_into_the_column() {
        let records = validate_calls(vec![TelemetryLlmCallPayload {
            duration_ms: i64::MAX,
            prompt_tokens: Some(i64::MIN),
            total_tokens: Some(i64::MAX),
            tool_calls: Some(-4),
            ..call()
        }]);
        assert_eq!(records[0].duration_ms, i32::MAX);
        assert_eq!(records[0].prompt_tokens, None);
        assert_eq!(records[0].total_tokens, Some(i32::MAX));
        assert_eq!(records[0].tool_calls, 0);
    }

    #[test]
    fn enforces_the_batch_cap() {
        assert!(ensure_batch_size(0).is_ok());
        assert!(ensure_batch_size(MAX_CALLS_PER_BATCH).is_ok());
        assert!(ensure_batch_size(MAX_CALLS_PER_BATCH + 1).is_err());
    }

    #[test]
    fn the_batch_envelope_keeps_the_anonymous_contract() {
        let batch = payload(
            "desktop",
            vec![TelemetryLlmCallPayload {
                status: LLM_STATUS_ERROR.to_string(),
                error_kind: Some("auth_required".to_string()),
                ..call()
            }],
        );
        assert!(validate_source(&batch.source).is_ok());
        assert!(validate_source("mainframe").is_err());
        assert!(validate_anon_id(batch.anon_id.as_deref().unwrap()).is_ok());
        assert!(validate_anon_id("backend").is_err());

        let records = validate_calls(batch.calls);
        assert_eq!(records[0].error_kind.as_deref(), Some("auth_required"));
    }

    #[test]
    fn anon_id_is_optional() {
        let parsed = parse(json!({
            "source": "backend",
            "calls": []
        }));
        assert_eq!(parsed.anon_id, None);
        assert!(parsed.calls.is_empty());
    }
}
