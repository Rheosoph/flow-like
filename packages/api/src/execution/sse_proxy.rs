//! SSE Proxy utilities for streaming execution responses
//!
//! Provides robust SSE parsing using `eventsource-stream` to properly handle
//! SSE protocol edge cases like multi-line data, reconnection, and buffering.

use crate::entity::sea_orm_active_enums::{ExecutionStatus, RunStatus};
use crate::entity::{execution_run, execution_usage_tracking, prelude::*};
use crate::execution::dispatch::ByteStream;
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
    let stream = create_sse_stream(response, run_id, db);

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
) -> Pin<Box<dyn Stream<Item = Result<Event, Infallible>> + Send>> {
    let byte_stream = response.bytes_stream();
    let event_stream = byte_stream.eventsource();

    let stream = async_stream::stream! {
        let mut es = event_stream;

        while let Some(result) = es.next().await {
            match result {
                Ok(sse_event) => {
                    // Check if this is a completed event and update the database
                    if let Some(db) = &db
                        && let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&sse_event.data)
                            && let Some(event_type) = parsed.get("event_type").and_then(|v| v.as_str())
                                && event_type == "completed" {
                                    let log_level = parsed.get("payload")
                                        .and_then(|p| p.get("log_level"))
                                        .and_then(|l| l.as_i64())
                                        .unwrap_or(0) as i32;
                                    let status = parsed.get("payload")
                                        .and_then(|p| p.get("status"))
                                        .and_then(|s| s.as_str())
                                        .unwrap_or("Completed");

                                    let run_status = match status {
                                        "Failed" => RunStatus::Failed,
                                        "Cancelled" => RunStatus::Cancelled,
                                        "Timeout" => RunStatus::Timeout,
                                        _ => RunStatus::Completed,
                                    };

                                    if let Err(e) = update_run_on_completion(db.as_ref(), &run_id, run_status, log_level).await {
                                        tracing::error!(run_id = %run_id, error = %e, "Failed to update run on completion");
                                    }
                                }

                    let event = Event::default()
                        .event(&sse_event.event)
                        .data(sse_event.data);

                    yield Ok(event);
                }
                Err(err) => {
                    tracing::warn!(run_id = %run_id, error = %err, "SSE parse error");
                    let error_event = Event::default()
                        .event("error")
                        .data(format!(r#"{{"error":"{}"}}"#, err));
                    yield Ok(error_event);
                    break;
                }
            }
        }

        tracing::debug!(run_id = %run_id, "SSE stream ended");
    };

    Box::pin(stream)
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
                        .and_then(|s| s.as_str())
                        .unwrap_or("Completed");
                    let run_status = match status {
                        "Failed" => RunStatus::Failed,
                        "Cancelled" => RunStatus::Cancelled,
                        "Timeout" => RunStatus::Timeout,
                        _ => RunStatus::Completed,
                    };
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
                        .and_then(|s| s.as_str())
                        .unwrap_or("Completed");
                    let run_status = match status {
                        "Failed" => RunStatus::Failed,
                        "Cancelled" => RunStatus::Cancelled,
                        "Timeout" => RunStatus::Timeout,
                        _ => RunStatus::Completed,
                    };
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
            &tracking_app_id,
            now,
        )
        .await?;
        tracing::info!(run_id = %run_id, log_level = log_level, "Updated run status on completion");
    }
    Ok(())
}

async fn track_execution_usage_from_run(
    db: &DatabaseConnection,
    run_id: &str,
    board_id: &str,
    node_id: &str,
    microseconds: i64,
    status: ExecutionStatus,
    user_id: Option<&str>,
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
        app_id: Set(Some(app_id.to_string())),
        created_at: Set(now),
        updated_at: Set(now),
    }
    .insert(db)
    .await?;

    Ok(())
}
