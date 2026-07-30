//! Reusable server-side LLM call telemetry.
//!
//! PRIVACY INVARIANT: an LLM call record is an aggregate measurement only. It
//! carries the provider, the model name, the kind of call, timings and token
//! counts — never a prompt, a completion, tool arguments, file paths or any
//! user identity. The ingest endpoint and this helper share the same record
//! type so both paths are held to that shape.

use sea_orm::{EntityTrait, Set};

use crate::{entity::telemetry_llm_call, state::AppState, telemetry::sink_from_env};

pub const LLM_OPERATION_CHAT: &str = "chat";
pub const LLM_OPERATION_EMBED: &str = "embed";
pub const LLM_OPERATION_TOOL: &str = "tool";
/// Kind of call being measured.
pub const LLM_OPERATIONS: [&str; 3] = [LLM_OPERATION_CHAT, LLM_OPERATION_EMBED, LLM_OPERATION_TOOL];

pub const LLM_STATUS_OK: &str = "ok";
pub const LLM_STATUS_ERROR: &str = "error";
/// Outcome vocabulary.
pub const LLM_STATUSES: [&str; 2] = [LLM_STATUS_OK, LLM_STATUS_ERROR];

/// `source` recorded for calls the backend makes itself.
pub const LLM_SOURCE_BACKEND: &str = "backend";

pub(crate) const MAX_LLM_FIELD_LEN: usize = 128;

/// One measured LLM invocation.
///
/// Construct with `..Default::default()` — the defaults are a successful chat
/// call with no tokens reported.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LlmCallRecord {
    pub provider: String,
    pub model: String,
    /// One of [`LLM_OPERATIONS`].
    pub operation: String,
    pub duration_ms: i32,
    pub prompt_tokens: Option<i32>,
    pub completion_tokens: Option<i32>,
    pub total_tokens: Option<i32>,
    /// One of [`LLM_STATUSES`].
    pub status: String,
    /// Classified failure label, never a raw error message.
    pub error_kind: Option<String>,
    pub tool_calls: i32,
    pub streamed: bool,
}

impl Default for LlmCallRecord {
    fn default() -> Self {
        Self {
            provider: String::new(),
            model: String::new(),
            operation: LLM_OPERATION_CHAT.to_string(),
            duration_ms: 0,
            prompt_tokens: None,
            completion_tokens: None,
            total_tokens: None,
            status: LLM_STATUS_OK.to_string(),
            error_kind: None,
            tool_calls: 0,
            streamed: false,
        }
    }
}

fn normalized_field(value: &str) -> String {
    value.trim().chars().take(MAX_LLM_FIELD_LEN).collect()
}

fn normalized_lowercase(value: &str) -> String {
    normalized_field(value).to_ascii_lowercase()
}

fn non_negative(value: Option<i32>) -> Option<i32> {
    value.filter(|count| *count >= 0)
}

/// Normalizes a record and enforces the vocabulary.
///
/// Returns `None` when the provider or model is empty or the operation or
/// status is outside the vocabulary, so a malformed call can never poison the
/// dashboards. `total_tokens` is derived from the prompt and completion counts
/// when the caller did not report it, and `error_kind` is dropped on success
/// so the error breakdown only ever counts real failures.
pub(crate) fn normalize_record(mut record: LlmCallRecord) -> Option<LlmCallRecord> {
    record.provider = normalized_lowercase(&record.provider);
    record.model = normalized_field(&record.model);
    record.operation = normalized_lowercase(&record.operation);
    record.status = normalized_lowercase(&record.status);

    if record.provider.is_empty() || record.model.is_empty() {
        return None;
    }
    if !LLM_OPERATIONS.contains(&record.operation.as_str()) {
        return None;
    }
    if !LLM_STATUSES.contains(&record.status.as_str()) {
        return None;
    }

    record.duration_ms = record.duration_ms.max(0);
    record.tool_calls = record.tool_calls.max(0);
    record.prompt_tokens = non_negative(record.prompt_tokens);
    record.completion_tokens = non_negative(record.completion_tokens);
    record.total_tokens = non_negative(record.total_tokens).or_else(|| {
        match (record.prompt_tokens, record.completion_tokens) {
            (None, None) => None,
            (prompt, completion) => {
                Some(prompt.unwrap_or(0).saturating_add(completion.unwrap_or(0)))
            }
        }
    });

    record.error_kind = if record.status == LLM_STATUS_ERROR {
        record
            .error_kind
            .as_deref()
            .map(normalized_lowercase)
            .filter(|kind| !kind.is_empty())
    } else {
        None
    };

    Some(record)
}

pub(crate) fn active_model(
    record: LlmCallRecord,
    source: &str,
    anon_id: Option<&str>,
    release: Option<&str>,
    now: chrono::NaiveDateTime,
) -> telemetry_llm_call::ActiveModel {
    telemetry_llm_call::ActiveModel {
        id: Set(flow_like_types::create_id()),
        anon_id: Set(anon_id.map(str::to_string)),
        source: Set(source.to_string()),
        release: Set(release.map(str::to_string)),
        provider: Set(record.provider),
        model: Set(record.model),
        operation: Set(record.operation),
        duration_ms: Set(record.duration_ms),
        prompt_tokens: Set(record.prompt_tokens),
        completion_tokens: Set(record.completion_tokens),
        total_tokens: Set(record.total_tokens),
        status: Set(record.status),
        error_kind: Set(record.error_kind),
        tool_calls: Set(record.tool_calls),
        streamed: Set(record.streamed),
        created_at: Set(now),
    }
}

/// Records an LLM invocation made by the backend itself.
///
/// Best-effort and fire-and-forget: respects the `telemetry` feature flag and
/// the `FLOW_LIKE_TELEMETRY_SINK` env var, then spawns a background insert with
/// `source` fixed to "backend" and `anon_id` left null.
pub fn record_llm_call(state: &AppState, record: LlmCallRecord) {
    if !state.platform_config.features.telemetry {
        return;
    }

    let sink = sink_from_env();
    if sink == "none" {
        return;
    }

    let Some(record) = normalize_record(record) else {
        tracing::warn!(
            "Dropped a backend LLM telemetry record with an empty provider or model, or an unknown operation or status"
        );
        return;
    };

    if sink == "log" {
        tracing::info!(
            source = LLM_SOURCE_BACKEND,
            provider = %record.provider,
            model = %record.model,
            operation = %record.operation,
            status = %record.status,
            duration_ms = record.duration_ms,
            "llm call"
        );
        return;
    }

    let db = state.db.clone();
    flow_like_types::tokio::spawn(async move {
        let model = active_model(
            record,
            LLM_SOURCE_BACKEND,
            None,
            None,
            chrono::Utc::now().naive_utc(),
        );
        if let Err(e) = telemetry_llm_call::Entity::insert(model).exec(&db).await {
            tracing::error!("Failed to persist backend LLM telemetry call: {}", e);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record() -> LlmCallRecord {
        LlmCallRecord {
            provider: "OpenAI".to_string(),
            model: "gpt-5-mini".to_string(),
            ..Default::default()
        }
    }

    #[test]
    fn defaults_describe_a_successful_chat_call() {
        let default = LlmCallRecord::default();
        assert_eq!(default.operation, LLM_OPERATION_CHAT);
        assert_eq!(default.status, LLM_STATUS_OK);
        assert_eq!(default.tool_calls, 0);
        assert!(!default.streamed);
    }

    #[test]
    fn accepts_every_documented_operation_and_status() {
        for operation in LLM_OPERATIONS {
            let normalized = normalize_record(LlmCallRecord {
                operation: operation.to_string(),
                ..record()
            })
            .unwrap();
            assert_eq!(normalized.operation, operation);
        }
        for status in LLM_STATUSES {
            let normalized = normalize_record(LlmCallRecord {
                status: status.to_string(),
                ..record()
            })
            .unwrap();
            assert_eq!(normalized.status, status);
        }
    }

    #[test]
    fn rejects_vocabulary_outside_the_contract() {
        for operation in ["completion", "", "chat ing", "rerank"] {
            assert!(
                normalize_record(LlmCallRecord {
                    operation: operation.to_string(),
                    ..record()
                })
                .is_none(),
                "expected operation '{operation}' to be rejected"
            );
        }
        for status in ["failed", "", "success", "500"] {
            assert!(
                normalize_record(LlmCallRecord {
                    status: status.to_string(),
                    ..record()
                })
                .is_none(),
                "expected status '{status}' to be rejected"
            );
        }
    }

    #[test]
    fn rejects_records_without_a_provider_or_model() {
        assert!(
            normalize_record(LlmCallRecord {
                provider: "   ".to_string(),
                ..record()
            })
            .is_none()
        );
        assert!(
            normalize_record(LlmCallRecord {
                model: String::new(),
                ..record()
            })
            .is_none()
        );
    }

    #[test]
    fn normalizes_case_whitespace_and_length() {
        let normalized = normalize_record(LlmCallRecord {
            provider: "  OpenAI  ".to_string(),
            model: format!("  {}  ", "m".repeat(MAX_LLM_FIELD_LEN + 10)),
            operation: " CHAT ".to_string(),
            status: " OK ".to_string(),
            ..record()
        })
        .unwrap();

        assert_eq!(normalized.provider, "openai");
        assert_eq!(normalized.model.len(), MAX_LLM_FIELD_LEN);
        assert_eq!(normalized.operation, LLM_OPERATION_CHAT);
        assert_eq!(normalized.status, LLM_STATUS_OK);
    }

    #[test]
    fn derives_total_tokens_when_the_caller_omits_them() {
        let normalized = normalize_record(LlmCallRecord {
            prompt_tokens: Some(120),
            completion_tokens: Some(30),
            ..record()
        })
        .unwrap();
        assert_eq!(normalized.total_tokens, Some(150));

        let partial = normalize_record(LlmCallRecord {
            prompt_tokens: Some(120),
            ..record()
        })
        .unwrap();
        assert_eq!(partial.total_tokens, Some(120));

        let none = normalize_record(record()).unwrap();
        assert_eq!(none.total_tokens, None);

        let explicit = normalize_record(LlmCallRecord {
            prompt_tokens: Some(1),
            completion_tokens: Some(1),
            total_tokens: Some(99),
            ..record()
        })
        .unwrap();
        assert_eq!(explicit.total_tokens, Some(99));
    }

    #[test]
    fn clamps_negative_measurements() {
        let normalized = normalize_record(LlmCallRecord {
            duration_ms: -5,
            tool_calls: -2,
            prompt_tokens: Some(-1),
            completion_tokens: Some(-1),
            total_tokens: Some(-1),
            ..record()
        })
        .unwrap();

        assert_eq!(normalized.duration_ms, 0);
        assert_eq!(normalized.tool_calls, 0);
        assert_eq!(normalized.prompt_tokens, None);
        assert_eq!(normalized.completion_tokens, None);
        assert_eq!(normalized.total_tokens, None);
    }

    #[test]
    fn error_kind_is_kept_only_for_failures() {
        let failed = normalize_record(LlmCallRecord {
            status: LLM_STATUS_ERROR.to_string(),
            error_kind: Some(" Rate_Limit ".to_string()),
            ..record()
        })
        .unwrap();
        assert_eq!(failed.error_kind.as_deref(), Some("rate_limit"));

        let succeeded = normalize_record(LlmCallRecord {
            error_kind: Some("rate_limit".to_string()),
            ..record()
        })
        .unwrap();
        assert_eq!(succeeded.error_kind, None);

        let blank = normalize_record(LlmCallRecord {
            status: LLM_STATUS_ERROR.to_string(),
            error_kind: Some("   ".to_string()),
            ..record()
        })
        .unwrap();
        assert_eq!(blank.error_kind, None);
    }
}
