//! SSE Proxy utilities for streaming execution responses
//!
//! Provides robust SSE parsing using `eventsource-stream` to properly handle
//! SSE protocol edge cases like multi-line data, reconnection, and buffering.

use crate::entity::sea_orm_active_enums::RunStatus;
use crate::entity::{execution_run, prelude::*};
use axum::response::sse::{Event, KeepAlive, Sse};
use eventsource_stream::Eventsource;
use futures_util::{Stream, StreamExt};
use sea_orm::{ActiveModelTrait, ActiveValue::Set, DatabaseConnection, EntityTrait};
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

                                    let db = db.clone();
                                    let run_id_clone = run_id.clone();
                                    flow_like_types::tokio::spawn(async move {
                                        if let Err(e) = update_run_on_completion(&db, &run_id_clone, run_status, log_level).await {
                                            tracing::error!(run_id = %run_id_clone, error = %e, "Failed to update run on completion");
                                        }
                                    });
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
        let mut model: execution_run::ActiveModel = existing.into();
        model.status = Set(status);
        model.log_level = Set(log_level);
        if started_at.is_none() {
            model.started_at = Set(Some(created_at));
        }
        model.completed_at = Set(Some(now));
        model.updated_at = Set(now);
        model.update(db).await?;
        tracing::info!(run_id = %run_id, log_level = log_level, "Updated run status on completion");
    }
    Ok(())
}
