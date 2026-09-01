//! SSE Proxy utilities for streaming execution responses
//!
//! Provides robust SSE parsing using `eventsource-stream` to properly handle
//! SSE protocol edge cases like multi-line data, reconnection, and buffering.

use crate::entity::sea_orm_active_enums::{ExecutionStatus, RunStatus};
use crate::entity::{execution_run, execution_usage_tracking, prelude::*};
use crate::execution::dispatch::ByteStream;
use crate::execution::page_action_sealer::{PageActionSealingContext, PageActionSealingReport};
use axum::response::sse::{Event, KeepAlive, Sse};
use eventsource_stream::Eventsource;
use flow_like_types::create_id;
use futures_util::{Stream, StreamExt};
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter,
};
use std::convert::Infallible;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

pub(crate) fn completed_run_status(status: Option<&str>) -> RunStatus {
    match status.map(str::to_ascii_lowercase).as_deref() {
        Some("completed" | "success" | "succeeded") => RunStatus::Completed,
        Some("cancelled" | "canceled") => RunStatus::Cancelled,
        Some("timeout" | "timed_out") => RunStatus::Timeout,
        Some("failed") => RunStatus::Failed,
        Some(_) | None => RunStatus::Failed,
    }
}

/// Create an SSE stream from an executor HTTP response
///
/// Uses `eventsource-stream` for proper SSE protocol handling instead of
/// manual byte parsing. This correctly handles:
/// - Multi-line data fields
/// - Event ID tracking
/// - Retry directives
/// - Proper message boundaries
pub fn proxy_sse_response(
    response: reqwest::Response,
    run_id: String,
    db: Option<Arc<DatabaseConnection>>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    proxy_sse_response_with_page_actions(response, run_id, db, None)
}

/// Create an SSE stream and govern executable actions emitted by a Page run.
///
/// The Page capability is inserted only into the JSON envelope's `payload`.
/// The caller's normal authentication remains separate, and streams without a
/// sealing context retain the existing byte-for-byte data behavior.
pub fn proxy_sse_response_with_page_actions(
    response: reqwest::Response,
    run_id: String,
    db: Option<Arc<DatabaseConnection>>,
    page_actions: Option<Arc<PageActionSealingContext>>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let stream = create_sse_stream(response, run_id, db, page_actions);

    Sse::new(stream).keep_alive(
        KeepAlive::new()
            .text("keep-alive")
            .interval(Duration::from_secs(1)),
    )
}

fn create_sse_stream(
    response: reqwest::Response,
    run_id: String,
    db: Option<Arc<DatabaseConnection>>,
    page_actions: Option<Arc<PageActionSealingContext>>,
) -> Pin<Box<dyn Stream<Item = Result<Event, Infallible>> + Send>> {
    let byte_stream = response.bytes_stream();
    let event_stream = byte_stream.eventsource();

    let stream = async_stream::stream! {
        let mut es = event_stream;
        let mut event_ordinal = 0_u64;

        while let Some(result) = es.next().await {
            match result {
                Ok(sse_event) => {
                    let ordinal = event_ordinal;
                    event_ordinal = event_ordinal.saturating_add(1);
                    let transformed = page_actions.as_deref().map(|context| {
                        seal_page_action_sse_envelope(
                            &sse_event.data,
                            &sse_event.id,
                            &run_id,
                            ordinal,
                            context,
                        )
                    });
                    let data = transformed
                        .as_ref()
                        .map(|result| result.data.as_str())
                        .unwrap_or(sse_event.data.as_str());

                    if let Some(transformed) = &transformed {
                        if transformed.report.rejected > 0 {
                            tracing::warn!(
                                run_id = %run_id,
                                message_id = %transformed.message_id,
                                rejected = transformed.report.rejected,
                                "stripped rejected dynamic Page action targets from executor output"
                            );
                        }
                        if transformed.report.sealed > 0 {
                            tracing::debug!(
                                run_id = %run_id,
                                message_id = %transformed.message_id,
                                sealed = transformed.report.sealed,
                                "sealed dynamic Page actions in executor output"
                            );
                        }
                    }

                    // Check if this is a completed event and update the database
                    if let Some(db) = &db
                        && let Ok(parsed) = serde_json::from_str::<serde_json::Value>(data)
                            && let Some(event_type) = parsed.get("event_type").and_then(|v| v.as_str())
                                && event_type == "completed" {
                                    let log_level = parsed.get("payload")
                                        .and_then(|p| p.get("log_level"))
                                        .and_then(|l| l.as_i64())
                                        .unwrap_or(0) as i32;
                                    let status = parsed.get("payload")
                                        .and_then(|p| p.get("status"))
                                        .and_then(|s| s.as_str());

                                    let run_status = completed_run_status(status);

                                    if let Err(e) = update_run_on_completion(db.as_ref(), &run_id, run_status, log_level).await {
                                        tracing::error!(run_id = %run_id, error = %e, "Failed to update run on completion");
                                    }
                                }

                    let event = Event::default()
                        .event(&sse_event.event)
                        .data(data.to_owned());

                    yield Ok(event);
                }
                Err(err) => {
                    tracing::warn!(run_id = %run_id, error = %err, "SSE parse error");
                    if let Some(db) = &db
                        && let Err(e) = update_run_on_completion(db.as_ref(), &run_id, RunStatus::Failed, 0).await {
                            tracing::error!(run_id = %run_id, error = %e, "Failed to mark run failed after SSE parse error");
                        }
                    let payload = serde_json::json!({ "error": err.to_string() });
                    let error_event = Event::default()
                        .event("error")
                        .data(serde_json::to_string(&payload).unwrap_or_else(|_| "{\"error\":\"stream error\"}".to_string()));
                    yield Ok(error_event);
                    break;
                }
            }
        }

        tracing::debug!(run_id = %run_id, "SSE stream ended");
    };

    Box::pin(stream)
}

#[derive(Debug)]
struct SealedSseEnvelope {
    data: String,
    message_id: String,
    report: PageActionSealingReport,
}

/// Transform one executor SSE envelope without depending on HTTP state.
///
/// Executor event ids are stable across delivery retries. Older executors may
/// omit them, so the SSE id is scoped to the run, followed by the run-local
/// ordinal as the final deterministic fallback.
fn seal_page_action_sse_envelope(
    data: &str,
    sse_id: &str,
    run_id: &str,
    event_ordinal: u64,
    context: &PageActionSealingContext,
) -> SealedSseEnvelope {
    let fallback_message_id = if sse_id.trim().is_empty() {
        format!("{run_id}:stream:{event_ordinal}")
    } else {
        format!("{run_id}:sse:{sse_id}")
    };
    let Ok(mut envelope) = serde_json::from_str::<serde_json::Value>(data) else {
        return SealedSseEnvelope {
            data: data.to_owned(),
            message_id: fallback_message_id,
            report: PageActionSealingReport::default(),
        };
    };
    let Some(envelope_object) = envelope.as_object_mut() else {
        return SealedSseEnvelope {
            data: data.to_owned(),
            message_id: fallback_message_id,
            report: PageActionSealingReport::default(),
        };
    };

    let message_id = ["event_id", "eventId", "id"]
        .iter()
        .find_map(|key| {
            envelope_object
                .get(*key)
                .and_then(serde_json::Value::as_str)
                .filter(|value| !value.trim().is_empty())
                .map(ToOwned::to_owned)
        })
        .unwrap_or(fallback_message_id);
    let event_type = envelope_object
        .get("event_type")
        .or_else(|| envelope_object.get("eventType"))
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
        .to_string();
    let report = envelope_object
        .get_mut("payload")
        .map(|payload| context.seal_payload(&event_type, &message_id, payload))
        .unwrap_or_default();
    let data = serde_json::to_string(&envelope).unwrap_or_else(|_| data.to_owned());

    SealedSseEnvelope {
        data,
        message_id,
        report,
    }
}

/// Consume an executor SSE response and return a single JSON body built from
/// the first `generic_result` event. Mirrors the desktop HTTP-sink behavior
/// so a synchronous `curl` against the same endpoint returns a single JSON
/// payload regardless of which transport (desktop/server) is serving it.
///
/// Also updates the run record on the `completed` event so the usual
/// bookkeeping stays in place.
///
/// Returns `None` if the stream ends without emitting a `generic_result`
/// inside `timeout`.
pub async fn collect_generic_result(
    response: reqwest::Response,
    run_id: String,
    db: Option<Arc<DatabaseConnection>>,
    timeout: Duration,
) -> Option<serde_json::Value> {
    let byte_stream = response.bytes_stream();
    let mut es = byte_stream.eventsource();

    let collect = async {
        let mut generic_result: Option<serde_json::Value> = None;
        while let Some(result) = es.next().await {
            let sse_event = match result {
                Ok(evt) => evt,
                Err(err) => {
                    tracing::warn!(run_id = %run_id, error = %err, "SSE parse error while collecting result");
                    break;
                }
            };

            let parsed: serde_json::Value = match serde_json::from_str(&sse_event.data) {
                Ok(v) => v,
                Err(_) => continue,
            };

            let event_type = parsed
                .get("event_type")
                .and_then(|v| v.as_str())
                .unwrap_or("");

            if event_type == "generic_result" && generic_result.is_none() {
                generic_result = parsed.get("payload").cloned();
            }

            if event_type == "completed" {
                if let Some(db) = &db {
                    let log_level = parsed
                        .get("payload")
                        .and_then(|p| p.get("log_level"))
                        .and_then(|l| l.as_i64())
                        .unwrap_or(0) as i32;
                    let status = parsed
                        .get("payload")
                        .and_then(|p| p.get("status"))
                        .and_then(|s| s.as_str());
                    let run_status = completed_run_status(status);
                    if let Err(e) =
                        update_run_on_completion(db.as_ref(), &run_id, run_status, log_level).await
                    {
                        tracing::error!(run_id = %run_id, error = %e, "Failed to update run on completion");
                    }
                }
                // `completed` is the terminator — we can stop reading.
                break;
            }
        }
        generic_result
    };

    match flow_like_types::tokio::time::timeout(timeout, collect).await {
        Ok(result) => result,
        Err(_) => {
            tracing::warn!(run_id = %run_id, "Timed out collecting generic_result");
            None
        }
    }
}

/// ByteStream variant of `collect_generic_result` for the Lambda streaming
/// path. Same semantics: drains the SSE stream until the first
/// `generic_result` event (and `completed` for run bookkeeping), returns the
/// payload, and gives up after `timeout`.
///
/// The Lambda `ByteStream` is already a `Stream<Item = Result<Bytes, _>>` —
/// `eventsource-stream` parses it the same way as a reqwest response body.
pub async fn collect_generic_result_bytes(
    stream: ByteStream,
    run_id: String,
    db: Option<Arc<DatabaseConnection>>,
    timeout: Duration,
) -> Option<serde_json::Value> {
    let mut es = stream.eventsource();

    let collect = async {
        let mut generic_result: Option<serde_json::Value> = None;
        while let Some(result) = es.next().await {
            let sse_event = match result {
                Ok(evt) => evt,
                Err(err) => {
                    tracing::warn!(run_id = %run_id, error = %err, "Lambda SSE parse error while collecting result");
                    break;
                }
            };

            let parsed: serde_json::Value = match serde_json::from_str(&sse_event.data) {
                Ok(v) => v,
                Err(_) => continue,
            };

            let event_type = parsed
                .get("event_type")
                .and_then(|v| v.as_str())
                .unwrap_or("");

            if event_type == "generic_result" && generic_result.is_none() {
                generic_result = parsed.get("payload").cloned();
            }

            if event_type == "completed" {
                if let Some(db) = &db {
                    let log_level = parsed
                        .get("payload")
                        .and_then(|p| p.get("log_level"))
                        .and_then(|l| l.as_i64())
                        .unwrap_or(0) as i32;
                    let status = parsed
                        .get("payload")
                        .and_then(|p| p.get("status"))
                        .and_then(|s| s.as_str());
                    let run_status = completed_run_status(status);
                    if let Err(e) =
                        update_run_on_completion(db.as_ref(), &run_id, run_status, log_level).await
                    {
                        tracing::error!(run_id = %run_id, error = %e, "Failed to update run on completion");
                    }
                }
                break;
            }
        }
        generic_result
    };

    match flow_like_types::tokio::time::timeout(timeout, collect).await {
        Ok(result) => result,
        Err(_) => {
            tracing::warn!(run_id = %run_id, "Timed out collecting generic_result from Lambda stream");
            None
        }
    }
}

pub async fn update_run_on_completion(
    db: &DatabaseConnection,
    run_id: &str,
    status: RunStatus,
    log_level: i32,
) -> Result<(), sea_orm::DbErr> {
    if let Some(existing) = ExecutionRun::find_by_id(run_id).one(db).await? {
        let now = chrono::Utc::now().naive_utc();
        let started_at = existing.started_at;
        let created_at = existing.created_at;
        let tracking_board_id = existing.board_id.clone();
        let tracking_node_id = existing
            .node_id
            .clone()
            .or_else(|| existing.event_id.clone())
            .unwrap_or_default();
        let tracking_user_id = existing.user_id.clone();
        let tracking_technical_user_id = existing.technical_user_id.clone();
        let tracking_app_id = existing.app_id.clone();
        let tracking_started_at = started_at.unwrap_or(created_at);
        let tracking_duration_us = (now - tracking_started_at).num_microseconds().unwrap_or(0);
        let tracking_status = match status {
            RunStatus::Completed => ExecutionStatus::Info,
            RunStatus::Failed | RunStatus::Timeout => ExecutionStatus::Error,
            RunStatus::Cancelled => ExecutionStatus::Warn,
            _ => ExecutionStatus::Info,
        };

        let mut model: execution_run::ActiveModel = existing.into();
        model.status = Set(status);
        model.log_level = Set(log_level);
        if started_at.is_none() {
            model.started_at = Set(Some(created_at));
        }
        model.completed_at = Set(Some(now));
        model.updated_at = Set(now);
        model.update(db).await?;
        track_execution_usage_from_run(
            db,
            run_id,
            &tracking_board_id,
            &tracking_node_id,
            tracking_duration_us,
            tracking_status,
            tracking_user_id.as_deref(),
            tracking_technical_user_id.as_deref(),
            &tracking_app_id,
            now,
        )
        .await?;
        tracing::info!(run_id = %run_id, log_level = log_level, "Updated run status on completion");
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn track_execution_usage_from_run(
    db: &DatabaseConnection,
    run_id: &str,
    board_id: &str,
    node_id: &str,
    microseconds: i64,
    status: ExecutionStatus,
    user_id: Option<&str>,
    technical_user_id: Option<&str>,
    app_id: &str,
    now: chrono::NaiveDateTime,
) -> Result<(), sea_orm::DbErr> {
    let existing = execution_usage_tracking::Entity::find()
        .filter(execution_usage_tracking::Column::Version.eq(run_id))
        .one(db)
        .await?;
    if existing.is_some() {
        return Ok(());
    }

    let instance = std::env::var("INSTANCE_ID").ok();
    execution_usage_tracking::ActiveModel {
        id: Set(create_id()),
        instance: Set(instance),
        board_id: Set(board_id.to_string()),
        node_id: Set(node_id.to_string()),
        version: Set(run_id.to_string()),
        microseconds: Set(microseconds.max(0)),
        status: Set(status),
        user_id: Set(user_id.map(ToOwned::to_owned)),
        technical_user_id: Set(technical_user_id.map(ToOwned::to_owned)),
        app_id: Set(Some(app_id.to_string())),
        created_at: Set(now),
        updated_at: Set(now),
    }
    .insert(db)
    .await?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    fn page_action_context() -> PageActionSealingContext {
        crate::backend_jwt::init_for_tests();
        PageActionSealingContext {
            sub: "user-1".into(),
            technical_user_id: None,
            source_app_id: "app-1".into(),
            source_event_id: "page-event-1".into(),
            source_page_id: "page-1".into(),
            source_manifest_revision: "manifest-1".into(),
            target_app_id: "app-1".into(),
            target_board_id: "board-1".into(),
            target_board_version: Some((2, 1, 0)),
            target_board_etag: None,
            wasm_authority_revision: Some("wasm-revision-1".into()),
            origin_run_id: "run-1".into(),
            allowed_entry_nodes: HashSet::from(["entry-1".into()]),
        }
    }

    #[test]
    fn completion_status_parsing_matches_executor_serialization() {
        assert!(matches!(
            completed_run_status(Some("completed")),
            RunStatus::Completed
        ));
        assert!(matches!(
            completed_run_status(Some("failed")),
            RunStatus::Failed
        ));
        assert!(matches!(
            completed_run_status(Some("Cancelled")),
            RunStatus::Cancelled
        ));
        assert!(matches!(
            completed_run_status(Some("timeout")),
            RunStatus::Timeout
        ));
        assert!(matches!(
            completed_run_status(Some("unexpected")),
            RunStatus::Failed
        ));
        assert!(matches!(completed_run_status(None), RunStatus::Failed));
    }

    #[test]
    fn page_action_transform_uses_executor_event_id_and_strips_raw_routing() {
        let input = serde_json::json!({
            "event_id": "executor-message-1",
            "event_type": "a2ui",
            "payload": {
                "type": "surfaceUpdate",
                "components": [{"id": "button", "component": {
                    "actions": [{
                        "name": "workflow_event",
                        "context": {
                            "nodeId": "entry-1",
                            "appId": "app-1",
                            "boardId": "board-1",
                            "input": "kept"
                        }
                    }]
                }}]
            }
        });

        let transformed = seal_page_action_sse_envelope(
            &serde_json::to_string(&input).unwrap(),
            "transport-message-1",
            "run-1",
            9,
            &page_action_context(),
        );
        let output: serde_json::Value = serde_json::from_str(&transformed.data).unwrap();
        let action = &output["payload"]["components"][0]["component"]["actions"][0];

        assert_eq!(transformed.message_id, "executor-message-1");
        assert_eq!(transformed.report.sealed, 1);
        assert_eq!(transformed.report.rejected, 0);
        assert_eq!(action["context"]["input"], "kept");
        assert!(action["context"].get("nodeId").is_none());
        assert!(action["context"].get("appId").is_none());
        assert!(action["context"].get("boardId").is_none());
        assert!(action["pageAction"]["capabilityJwt"].is_string());
    }

    #[test]
    fn page_action_transform_falls_back_to_sse_id_then_run_ordinal() {
        let input = serde_json::json!({
            "event_type": "a2ui",
            "payload": {"value": true}
        });
        let encoded = serde_json::to_string(&input).unwrap();

        let with_sse_id = seal_page_action_sse_envelope(
            &encoded,
            "transport-2",
            "run-4",
            5,
            &page_action_context(),
        );
        let with_ordinal =
            seal_page_action_sse_envelope(&encoded, "", "run-4", 5, &page_action_context());

        assert_eq!(with_sse_id.message_id, "run-4:sse:transport-2");
        assert_eq!(with_ordinal.message_id, "run-4:stream:5");
    }

    #[test]
    fn page_action_transform_preserves_non_json_and_removes_rejected_targets() {
        let context = page_action_context();
        let plain = "executor sent a diagnostic line";
        let unchanged = seal_page_action_sse_envelope(plain, "", "run-1", 0, &context);
        assert_eq!(unchanged.data, plain);

        let input = serde_json::json!({
            "event_type": "a2ui",
            "payload": {
                "type": "surfaceUpdate",
                "components": [{"id": "button", "component": {
                    "actions": [{
                        "name": "workflow_event",
                        "context": {"nodeId": "foreign-entry", "other": 42}
                    }]
                }}]
            }
        });
        let rejected = seal_page_action_sse_envelope(
            &serde_json::to_string(&input).unwrap(),
            "",
            "run-1",
            1,
            &context,
        );
        let output: serde_json::Value = serde_json::from_str(&rejected.data).unwrap();
        let action = &output["payload"]["components"][0]["component"]["actions"][0];

        assert_eq!(rejected.report.rejected, 1);
        assert!(action.get("pageAction").is_none());
        assert!(action["context"].get("nodeId").is_none());
        assert_eq!(action["context"]["other"], 42);
    }
}
