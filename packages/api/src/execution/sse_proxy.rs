//! SSE Proxy utilities for streaming execution responses
//!
//! Provides robust SSE parsing using `eventsource-stream` to properly handle
//! SSE protocol edge cases like multi-line data, reconnection, and buffering.

use crate::audit::{ExecutionAuditContext, record_execution_result};
use crate::entity::sea_orm_active_enums::{AuditActorType, ExecutionStatus, RunStatus};
use crate::entity::{execution_run, execution_usage_tracking, prelude::*};
use crate::execution::dispatch::{ByteStream, DispatchError};
use crate::execution::page_action_sealer::{PageActionSealingContext, PageActionSealingReport};
use crate::execution::run_summary::{RunSummary, apply_run_summary, without_nodes};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::{http::HeaderMap, response::IntoResponse};
use eventsource_stream::Eventsource;
use flow_like::flow::execution::rejection::REJECTION_OPERATION_PREFIX;
use flow_like_types::create_id;
use flow_like_types::http_streaming::{HTTP_RESPONSE_CHUNK, accepts_event_stream};
use flow_like_types::tokio::task::JoinHandle;
use futures_util::{Stream, StreamExt};
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter,
};
use std::convert::Infallible;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;
use tracing::Instrument;

pub(crate) fn completed_run_status(status: Option<&str>) -> RunStatus {
    match status.map(str::to_ascii_lowercase).as_deref() {
        Some("completed" | "success" | "succeeded") => RunStatus::Completed,
        Some("cancelled" | "canceled") => RunStatus::Cancelled,
        Some("timeout" | "timed_out") => RunStatus::Timeout,
        Some("failed") => RunStatus::Failed,
        Some(_) | None => RunStatus::Failed,
    }
}

/// The bookkeeping an executor's `completed` event carries for the run row.
pub(crate) struct CompletedPayload {
    pub status: RunStatus,
    pub log_level: i32,
    pub summary: RunSummary,
    /// `rejected:<stage>` when the executor refused the run before it started.
    pub rejection_step: Option<String>,
}

/// Reads the `payload` of a parsed `completed` envelope
/// (`{"event_type": "completed", "payload": {…}}`). Every reader of an
/// executor stream goes through here, so the payload shape lives in one place.
pub(crate) fn parse_completed_payload(envelope: &serde_json::Value) -> CompletedPayload {
    let payload = envelope.get("payload");
    let log_level = payload
        .and_then(|p| p.get("log_level"))
        .and_then(|l| l.as_i64())
        .unwrap_or(0) as i32;
    let status = completed_run_status(
        payload
            .and_then(|p| p.get("status"))
            .and_then(|s| s.as_str()),
    );
    let summary = payload
        .and_then(|p| serde_json::from_value(p.clone()).ok())
        .unwrap_or_default();
    let rejection_step = payload
        .and_then(|p| p.get("current_step"))
        .and_then(|s| s.as_str())
        .filter(|step| step.starts_with(REJECTION_OPERATION_PREFIX))
        .map(str::to_string);
    CompletedPayload {
        status,
        log_level,
        summary,
        rejection_step,
    }
}

pub(crate) fn is_completed_event(envelope: &serde_json::Value) -> bool {
    envelope.get("event_type").and_then(|v| v.as_str()) == Some("completed")
}

/// Post-run writes for `completed` events. Each runs on its own task, so the
/// client receives `completed` without waiting for them and a dropped response
/// body cannot cancel them. The proxy awaits them before its stream ends: on
/// Lambda the invocation, and with it the unfrozen environment, lasts until
/// they commit. A run whose writes never finish is timed out by the run sweeper.
#[derive(Default)]
pub(crate) struct CompletionWrites(Vec<JoinHandle<()>>);

impl CompletionWrites {
    pub(crate) fn start(
        &mut self,
        context: &ExecutionAuditContext,
        run_id: &str,
        envelope: &serde_json::Value,
    ) {
        if !is_completed_event(envelope) {
            return;
        }
        let (context, run_id) = (context.clone(), run_id.to_string());
        let completed = parse_completed_payload(envelope);
        let span = tracing::info_span!(
            target: "flow_like::observability",
            "execution.completion_update",
            run_id = %run_id
        );
        self.0.push(flow_like_types::tokio::spawn(
            async move {
                if let Err(error) =
                    update_run_on_completed_event(&context, &run_id, completed).await
                {
                    tracing::error!(run_id = %run_id, %error, "Failed to update run on completion");
                }
            }
            .instrument(span),
        ));
    }

    pub(crate) async fn finish(&mut self) {
        for write in self.0.drain(..) {
            if let Err(error) = write.await {
                tracing::error!(%error, "Run completion update stopped before it finished");
            }
        }
    }
}

/// Records a `completed` executor event on the run row.
pub(crate) async fn update_run_on_completed_event(
    context: &ExecutionAuditContext,
    run_id: &str,
    completed: CompletedPayload,
) -> Result<(), sea_orm::DbErr> {
    update_run_on_completion_with_runtime(
        context,
        run_id,
        completed.status,
        completed.log_level,
        None,
        &completed.summary,
    )
    .await?;
    if let Some(step) = completed.rejection_step {
        ExecutionRun::update_many()
            .col_expr(
                execution_run::Column::CurrentStep,
                sea_orm::sea_query::Expr::value(step),
            )
            .filter(execution_run::Column::Id.eq(run_id))
            .exec(context.db.as_ref())
            .await?;
    }
    Ok(())
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
    db: Option<ExecutionAuditContext>,
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
    db: Option<ExecutionAuditContext>,
    page_actions: Option<Arc<PageActionSealingContext>>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let stream = create_sse_stream(response, run_id, db, page_actions);

    Sse::new(stream).keep_alive(
        KeepAlive::new()
            .text("keep-alive")
            .interval(Duration::from_secs(1)),
    )
}

pub(crate) fn wants_http_event_stream(headers: &HeaderMap) -> bool {
    let accept = headers
        .get_all(axum::http::header::ACCEPT)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .collect::<Vec<_>>()
        .join(",");
    accepts_event_stream(&accept)
}

pub(crate) fn executor_response_bytes(response: reqwest::Response) -> ByteStream {
    Box::pin(
        response
            .bytes_stream()
            .map(|result| result.map_err(|error| DispatchError::Network(error.to_string()))),
    )
}

fn public_http_event(name: &'static str, payload: serde_json::Value) -> Event {
    Event::default().event(name).data(payload.to_string())
}

fn public_http_error(message: &'static str) -> Event {
    public_http_event("error", serde_json::json!({ "error": message }))
}

fn finish_http_stream(
    output: &mut Option<flow_like_types::tokio::sync::mpsc::Sender<Event>>,
    terminal: &mut Option<flow_like_types::tokio::sync::oneshot::Sender<Event>>,
    event: Event,
) {
    output.take();
    if let Some(terminal) = terminal.take() {
        let _ = terminal.send(event);
    }
}

/// Stream only the workflow's explicit HTTP output. The executor's other
/// messages remain private, and its completion still updates the run record.
pub(crate) fn stream_http_response(
    bytes: ByteStream,
    run_id: String,
    db: Option<ExecutionAuditContext>,
    timeout: Duration,
    map_result: fn(serde_json::Value) -> serde_json::Value,
) -> axum::response::Response {
    // Reserve a separate terminal slot so a full queue cannot hide a timeout.
    // After a disconnect, drain until the original execution deadline to keep
    // authenticated completion events available for run bookkeeping.
    let (tx, mut rx) = flow_like_types::tokio::sync::mpsc::channel::<Event>(16);
    let (terminal_tx, terminal_rx) = flow_like_types::tokio::sync::oneshot::channel();
    let deadline = flow_like_types::tokio::time::Instant::now() + timeout;
    flow_like_types::tokio::spawn(async move {
        let mut es = bytes.eventsource();
        let mut output = Some(tx);
        let mut terminal = Some(terminal_tx);
        let mut has_result = false;
        let final_event = loop {
            let next = flow_like_types::tokio::time::timeout_at(deadline, es.next()).await;
            let sse_event = match next {
                Ok(Some(Ok(event))) => event,
                Ok(None) => {
                    break public_http_error("Response stream ended before completion");
                }
                Ok(Some(Err(error))) => {
                    tracing::warn!(run_id = %run_id, %error, "HTTP response stream failed");
                    record_executor_failure(db.as_ref(), &run_id, &error).await;
                    break public_http_error("Response stream failed");
                }
                Err(_) => {
                    break public_http_error("Response stream timed out");
                }
            };
            let parsed = match serde_json::from_str::<serde_json::Value>(&sse_event.data) {
                Ok(value) => value,
                Err(error) => {
                    tracing::warn!(run_id = %run_id, %error, "Invalid executor HTTP response event");
                    break public_http_error("Response stream failed");
                }
            };
            let event_type = parsed.get("event_type").and_then(|value| value.as_str());
            if event_type == Some("completed") {
                let completed = parse_completed_payload(&parsed);
                let successful = matches!(completed.status, RunStatus::Completed);
                if let Some(db) = &db
                    && let Err(error) = update_run_on_completed_event(db, &run_id, completed).await
                {
                    tracing::error!(run_id = %run_id, %error, "Failed to update HTTP run on completion");
                }
                break if successful {
                    public_http_event("done", serde_json::json!({}))
                } else {
                    public_http_error("Workflow execution failed")
                };
            }
            if event_type == Some("error") {
                // Close the public response immediately, then drain the
                // executor's completion event for its authenticated status.
                finish_http_stream(
                    &mut output,
                    &mut terminal,
                    public_http_error("Workflow execution failed"),
                );
                continue;
            }
            let event = match event_type {
                Some(HTTP_RESPONSE_CHUNK) => Some(public_http_event(
                    "chunk",
                    parsed.get("payload").cloned().unwrap_or_default(),
                )),
                Some("generic_result") if !has_result => {
                    has_result = true;
                    Some(public_http_event(
                        "result",
                        map_result(parsed.get("payload").cloned().unwrap_or_default()),
                    ))
                }
                _ => None,
            };
            if let (Some(tx), Some(event)) = (output.as_ref(), event) {
                match flow_like_types::tokio::time::timeout_at(deadline, tx.send(event)).await {
                    Ok(Ok(())) => {}
                    Ok(Err(_)) => output = None,
                    Err(_) => {
                        break public_http_error("Response stream timed out");
                    }
                }
            }
        };
        finish_http_stream(&mut output, &mut terminal, final_event);
    });

    let stream = async_stream::stream! {
        while let Some(event) = rx.recv().await {
            yield Ok::<_, Infallible>(event);
        }
        if let Ok(event) = terminal_rx.await {
            yield Ok(event);
        }
    };
    let mut response = Sse::new(stream)
        .keep_alive(
            KeepAlive::new()
                .text("keep-alive")
                .interval(Duration::from_secs(15)),
        )
        .into_response();
    response.headers_mut().insert(
        axum::http::header::CACHE_CONTROL,
        axum::http::HeaderValue::from_static("no-cache, no-transform"),
    );
    response.headers_mut().insert(
        "x-accel-buffering",
        axum::http::HeaderValue::from_static("no"),
    );
    response.headers_mut().append(
        axum::http::header::VARY,
        axum::http::HeaderValue::from_static("Accept"),
    );
    response
}

fn create_sse_stream(
    response: reqwest::Response,
    run_id: String,
    db: Option<ExecutionAuditContext>,
    page_actions: Option<Arc<PageActionSealingContext>>,
) -> Pin<Box<dyn Stream<Item = Result<Event, Infallible>> + Send>> {
    let byte_stream = response.bytes_stream();
    let event_stream = byte_stream.eventsource();

    let stream = async_stream::stream! {
        let mut es = event_stream;
        let mut event_ordinal = 0_u64;
        let mut completion = CompletionWrites::default();

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

                    if let Some(db) = &db
                        && let Ok(parsed) = serde_json::from_str::<serde_json::Value>(data)
                    {
                        completion.start(db, &run_id, &parsed);
                    }

                    let event = Event::default()
                        .event(&sse_event.event)
                        .data(data.to_owned());

                    yield Ok(event);
                }
                Err(err) => {
                    tracing::warn!(run_id = %run_id, error = %err, "SSE parse error");
                    // A transport failure says nothing about the worker outcome.
                    // Its authenticated terminal report owns the run status.
                    let payload = serde_json::json!({ "error": err.to_string() });
                    let error_event = Event::default()
                        .event("error")
                        .data(serde_json::to_string(&payload).unwrap_or_else(|_| "{\"error\":\"stream error\"}".to_string()));
                    yield Ok(error_event);
                    break;
                }
            }
        }

        completion.finish().await;
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
    db: Option<ExecutionAuditContext>,
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
                if let Some(db) = &db
                    && let Err(e) =
                        update_run_on_completed_event(db, &run_id, parse_completed_payload(&parsed))
                            .await
                {
                    tracing::error!(run_id = %run_id, error = %e, "Failed to update run on completion");
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
    db: Option<ExecutionAuditContext>,
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
                    record_executor_failure(db.as_ref(), &run_id, &err).await;
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
                if let Some(db) = &db
                    && let Err(e) =
                        update_run_on_completed_event(db, &run_id, parse_completed_payload(&parsed))
                            .await
                {
                    tracing::error!(run_id = %run_id, error = %e, "Failed to update run on completion");
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

fn completion_update(
    run_id: &str,
    model: execution_run::ActiveModel,
) -> sea_orm::UpdateMany<execution_run::Entity> {
    ExecutionRun::update_many()
        .set(model)
        .filter(execution_run::Column::Id.eq(run_id))
        .filter(execution_run::Column::Status.is_in([RunStatus::Pending, RunStatus::Running]))
}

pub async fn update_run_on_completion(
    context: &ExecutionAuditContext,
    run_id: &str,
    status: RunStatus,
    log_level: i32,
) -> Result<(), sea_orm::DbErr> {
    update_run_on_completion_with_runtime(
        context,
        run_id,
        status,
        log_level,
        None,
        &RunSummary::default(),
    )
    .await
}

/// `LogLevel::Fatal`: the executor died before it reported any outcome.
const EXECUTOR_FAILED_LOG_LEVEL: i32 = 4;

/// Closes a run whose executor stream broke off without a `completed` event.
/// The reason lands before the status flips so the audit entry carries it.
pub(crate) async fn fail_run_on_stream_error(
    context: &ExecutionAuditContext,
    run_id: &str,
    reason: &str,
) -> Result<(), sea_orm::DbErr> {
    failure_reason_update(run_id, reason)
        .exec(context.db.as_ref())
        .await?;
    update_run_on_completion(
        context,
        run_id,
        RunStatus::Failed,
        EXECUTOR_FAILED_LOG_LEVEL,
    )
    .await?;
    leave_failure_log(context, run_id, reason).await;
    Ok(())
}

/// Best effort: the run row already carries the reason. Only a run this call closed gets the
/// entry, so a run that finished on its own never shows a stray fatal line.
async fn leave_failure_log(context: &ExecutionAuditContext, run_id: &str, reason: &str) {
    let Some(state) = &context.state else {
        return;
    };
    let run = match without_nodes(ExecutionRun::find_by_id(run_id))
        .one(context.db.as_ref())
        .await
    {
        Ok(Some(run)) => run,
        Ok(None) => return,
        Err(error) => {
            tracing::warn!(run_id = %run_id, %error, "Failed to load the run to log its failure");
            return;
        }
    };
    if run.status != RunStatus::Failed || run.error_message.as_deref() != Some(reason) {
        return;
    }
    if let Err(error) = crate::execution::rejection::record_failure_log(state, &run, reason).await {
        tracing::warn!(run_id = %run_id, %error, "Failed to leave the failure reason in the run's log");
    }
}

fn failure_reason_update(run_id: &str, reason: &str) -> sea_orm::UpdateMany<execution_run::Entity> {
    ExecutionRun::update_many()
        .col_expr(
            execution_run::Column::ErrorMessage,
            sea_orm::sea_query::Expr::value(reason),
        )
        .filter(execution_run::Column::Id.eq(run_id))
        .filter(execution_run::Column::Status.is_in([RunStatus::Pending, RunStatus::Running]))
}

/// The executor's own `error` envelope, so every client that ends a run on
/// `event_type: "error"` also ends it when the API sees the stream fail.
pub(crate) fn stream_failure_event(message: String) -> Event {
    let envelope = flow_like_types::intercom::InterComEvent::with_type(
        "error",
        serde_json::json!({ "message": message }),
    );
    Event::default().event("error").data(
        serde_json::to_string(&envelope)
            .unwrap_or_else(|_| r#"{"event_type":"error","payload":{}}"#.to_string()),
    )
}

/// Only a platform-reported worker death settles the run here. Any other
/// transport error says nothing about the outcome: an HTTP executor still
/// sends its own terminal report.
async fn record_executor_failure(
    db: Option<&ExecutionAuditContext>,
    run_id: &str,
    error: &eventsource_stream::EventStreamError<DispatchError>,
) {
    let (
        Some(db),
        eventsource_stream::EventStreamError::Transport(DispatchError::ExecutorFailed(reason)),
    ) = (db, error)
    else {
        return;
    };
    if let Err(error) = fail_run_on_stream_error(db, run_id, reason).await {
        tracing::error!(run_id = %run_id, %error, "Failed to record executor failure");
    }
}

fn db_query_span(operation: &'static str, table: &'static str) -> tracing::Span {
    tracing::info_span!(
        target: "flow_like::observability",
        "db.query",
        db.operation = operation,
        db.table = table
    )
}

pub async fn update_run_on_completion_with_runtime(
    context: &ExecutionAuditContext,
    run_id: &str,
    status: RunStatus,
    log_level: i32,
    runtime_ms: Option<u64>,
    summary: &RunSummary,
) -> Result<(), sea_orm::DbErr> {
    let db = context.db.as_ref();
    crate::payments::node::cancel_run_database(db, context.dialect, run_id, "RUN_ENDED")
        .await
        .map_err(|error| sea_orm::DbErr::Custom(error.to_string()))?;

    if let Some(existing) = without_nodes(ExecutionRun::find_by_id(run_id))
        .one(db)
        .instrument(db_query_span("select", "ExecutionRun"))
        .await?
    {
        if !matches!(existing.status, RunStatus::Pending | RunStatus::Running) {
            apply_run_summary(db, run_id, &existing.app_id, summary).await?;
            record_execution_result(context, &existing, run_id, AuditActorType::Executor)
                .await
                .map_err(|error| sea_orm::DbErr::Custom(error.to_string()))?;
            return Ok(());
        }
        let now = chrono::Utc::now().fixed_offset();
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
        let tracking_duration_us = runtime_ms
            .map(|ms| i64::try_from(ms.saturating_mul(1000)).unwrap_or(i64::MAX))
            .unwrap_or_else(|| (now - tracking_started_at).num_microseconds().unwrap_or(0));
        let tracking_status = match status {
            RunStatus::Completed => ExecutionStatus::Info,
            RunStatus::Failed | RunStatus::Timeout => ExecutionStatus::Error,
            RunStatus::Cancelled => ExecutionStatus::Warn,
            _ => ExecutionStatus::Info,
        };

        let mut model = execution_run::ActiveModel {
            ..Default::default()
        };
        model.status = Set(status);
        model.log_level = Set(log_level);
        summary.apply_to(&mut model);
        if started_at.is_none() {
            model.started_at = Set(Some(created_at));
        }
        model.completed_at = Set(Some(now));
        model.updated_at = Set(now);
        let completed = completion_update(run_id, model)
            .exec_with_returning(db)
            .instrument(db_query_span("update", "ExecutionRun"))
            .await?
            .into_iter()
            .next();
        let changed = completed.is_some();
        let persisted = match completed {
            Some(persisted) => persisted,
            None => {
                apply_run_summary(db, run_id, &tracking_app_id, summary).await?;
                without_nodes(ExecutionRun::find_by_id(run_id))
                    .one(db)
                    .await?
                    .ok_or_else(|| {
                        sea_orm::DbErr::Custom(format!(
                            "Run disappeared after completion: {run_id}"
                        ))
                    })?
            }
        };
        record_execution_result(context, &persisted, run_id, AuditActorType::Executor)
            .await
            .map_err(|error| sea_orm::DbErr::Custom(error.to_string()))?;
        if !changed {
            return Ok(());
        }
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
        .instrument(db_query_span("insert", "ExecutionUsageTracking"))
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
    now: chrono::DateTime<chrono::FixedOffset>,
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
    use flow_like_types::tokio;
    use std::collections::HashSet;

    fn executor_event(event_type: &str, payload: serde_json::Value) -> bytes::Bytes {
        bytes::Bytes::from(format!(
            "event: message\ndata: {}\n\n",
            serde_json::json!({ "event_type": event_type, "payload": payload })
        ))
    }

    async fn http_stream_text(bytes: ByteStream) -> String {
        let response = stream_http_response(
            bytes,
            "run-http-test".into(),
            None,
            Duration::from_secs(1),
            std::convert::identity,
        );
        String::from_utf8(
            axum::body::to_bytes(response.into_body(), 1024 * 1024)
                .await
                .unwrap()
                .to_vec(),
        )
        .unwrap()
    }

    #[test]
    fn http_stream_negotiation_reads_all_accept_headers() {
        let mut headers = HeaderMap::new();
        assert!(!wants_http_event_stream(&headers));
        headers.append("accept", "application/json, */*".parse().unwrap());
        headers.append("accept", "text/event-stream;q=0".parse().unwrap());
        assert!(!wants_http_event_stream(&headers));
        headers.append("accept", "text/event-stream;q=0.5".parse().unwrap());
        assert!(wants_http_event_stream(&headers));
    }

    #[tokio::test]
    async fn http_stream_delivers_chunk_before_workflow_completion_and_filters_telemetry() {
        let (finish, completed) = tokio::sync::oneshot::channel();
        let bytes: ByteStream = Box::pin(async_stream::stream! {
            yield Ok(executor_event("log", serde_json::json!({"secret": "private"})));
            let chunk = executor_event(HTTP_RESPONSE_CHUNK, serde_json::json!({"text": "first\nline"}));
            let split = chunk.len() / 2;
            yield Ok(chunk.slice(..split));
            yield Ok(chunk.slice(split..));
            completed.await.unwrap();
            yield Ok(executor_event("generic_result", serde_json::json!({"body": "final"})));
            yield Ok(executor_event("generic_result", serde_json::json!("duplicate")));
            yield Ok(executor_event("completed", serde_json::json!({"status": "completed"})));
        });
        let response = stream_http_response(
            bytes,
            "run-http-test".into(),
            None,
            Duration::from_secs(2),
            std::convert::identity,
        );
        assert_eq!(response.headers()["content-type"], "text/event-stream");
        assert_eq!(response.headers()["x-accel-buffering"], "no");
        assert_eq!(response.headers()["vary"], "Accept");
        let mut events = response.into_body().into_data_stream().eventsource();
        let chunk = tokio::time::timeout(Duration::from_secs(1), events.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert_eq!(chunk.event, "chunk");
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&chunk.data).unwrap(),
            serde_json::json!({"text": "first\nline"})
        );
        // The workflow is still waiting on this signal when the first chunk arrives.
        finish.send(()).unwrap();
        let result = events.next().await.unwrap().unwrap();
        assert_eq!(result.event, "result");
        assert_eq!(result.data, r#"{"body":"final"}"#);
        let done = events.next().await.unwrap().unwrap();
        assert_eq!(done.event, "done");
        assert_eq!(done.data, "{}");
        assert!(events.next().await.is_none());
    }

    #[tokio::test]
    async fn http_stream_errors_are_terminal_and_sanitized() {
        for bytes in [
            Box::pin(futures_util::stream::empty()) as ByteStream,
            Box::pin(futures_util::stream::iter([Err(DispatchError::Network(
                "private executor URL and token".into(),
            ))])),
            Box::pin(futures_util::stream::iter([Err(
                DispatchError::ExecutorFailed("private worker details".into()),
            )])),
            Box::pin(futures_util::stream::iter([Ok(bytes::Bytes::from_static(
                b"data: private invalid JSON\n\n",
            ))])),
            Box::pin(futures_util::stream::iter([Ok(executor_event(
                "completed",
                serde_json::json!({"status": "failed", "error": "private trace"}),
            ))])),
        ] {
            let text = http_stream_text(bytes).await;
            assert!(text.contains("event: error"), "{text}");
            assert!(!text.contains("private"), "{text}");
            assert!(!text.contains("event: done"), "{text}");
        }
    }

    #[tokio::test]
    async fn http_stream_drains_completion_after_a_public_error() {
        let (observed, completion) = tokio::sync::oneshot::channel();
        let bytes: ByteStream = Box::pin(async_stream::stream! {
            yield Ok(executor_event("error", serde_json::json!({"error": "private trace"})));
            observed.send(()).unwrap();
            yield Ok(executor_event("completed", serde_json::json!({"status": "failed"})));
        });
        let text = http_stream_text(bytes).await;
        tokio::time::timeout(Duration::from_secs(1), completion)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(text.matches("event: error").count(), 1);
        assert!(!text.contains("private"));
        assert!(!text.contains("event: done"));
    }

    #[tokio::test]
    async fn http_stream_keeps_reading_completion_after_the_client_disconnects() {
        let (release, resume) = tokio::sync::oneshot::channel();
        let (observed, completion) = tokio::sync::oneshot::channel();
        let bytes: ByteStream = Box::pin(async_stream::stream! {
            resume.await.unwrap();
            yield Ok(executor_event(HTTP_RESPONSE_CHUNK, serde_json::json!("partial")));
            observed.send(()).unwrap();
            yield Ok(executor_event("completed", serde_json::json!({"status": "completed"})));
        });
        let response = stream_http_response(
            bytes,
            "run-http-test".into(),
            None,
            Duration::from_secs(2),
            std::convert::identity,
        );
        drop(response);
        release.send(()).unwrap();
        tokio::time::timeout(Duration::from_secs(1), completion)
            .await
            .unwrap()
            .unwrap();
    }

    #[tokio::test]
    async fn http_stream_timeout_terminates_even_when_the_output_queue_is_full() {
        struct OnDrop(Option<tokio::sync::oneshot::Sender<()>>);
        impl Drop for OnDrop {
            fn drop(&mut self) {
                if let Some(sender) = self.0.take() {
                    let _ = sender.send(());
                }
            }
        }
        let (dropped, stopped) = tokio::sync::oneshot::channel();
        let bytes: ByteStream = Box::pin(async_stream::stream! {
            let _on_drop = OnDrop(Some(dropped));
            for index in 0..100 {
                yield Ok(executor_event(HTTP_RESPONSE_CHUNK, serde_json::json!(index)));
            }
            futures_util::future::pending::<()>().await;
        });
        let response = stream_http_response(
            bytes,
            "run-http-test".into(),
            None,
            Duration::from_millis(20),
            std::convert::identity,
        );
        // Wait without reading the body, leaving the producer's queue full.
        tokio::time::timeout(Duration::from_secs(1), stopped)
            .await
            .unwrap()
            .unwrap();
        let text = String::from_utf8(
            axum::body::to_bytes(response.into_body(), 1024 * 1024)
                .await
                .unwrap()
                .to_vec(),
        )
        .unwrap();
        assert_eq!(text.matches("event: chunk").count(), 16);
        assert!(text.contains("Response stream timed out"));
        assert_eq!(text.matches("event: error").count(), 1);
    }

    #[tokio::test]
    async fn buffered_http_result_ignores_partial_responses() {
        let bytes = Box::pin(futures_util::stream::iter([
            Ok(executor_event(
                HTTP_RESPONSE_CHUNK,
                serde_json::json!("partial"),
            )),
            Ok(executor_event(
                "generic_result",
                serde_json::json!({"answer": 42}),
            )),
            Ok(executor_event(
                "completed",
                serde_json::json!({"status": "completed"}),
            )),
        ]));
        let result = collect_generic_result_bytes(
            bytes,
            "run-buffered-test".into(),
            None,
            Duration::from_secs(1),
        )
        .await;
        assert_eq!(result, Some(serde_json::json!({"answer": 42})));
    }

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
    fn completed_payload_keeps_only_a_rejection_step() {
        let completed = parse_completed_payload(&serde_json::json!({
            "event_type": "completed",
            "payload": {
                "status": "failed",
                "log_level": 4,
                "nodes": [],
                "logs": 1,
                "current_step": "rejected:setup"
            }
        }));
        assert_eq!(completed.rejection_step.as_deref(), Some("rejected:setup"));
        assert_eq!(completed.summary.nodes, Some(Vec::new()));

        let completed = parse_completed_payload(&serde_json::json!({
            "event_type": "completed",
            "payload": { "status": "failed", "current_step": "node:abc" }
        }));
        assert_eq!(completed.rejection_step, None);
    }

    #[test]
    fn completion_cannot_overwrite_an_existing_terminal_result() {
        use sea_orm::QueryTrait;
        let sql = completion_update(
            "run-1",
            execution_run::ActiveModel {
                status: Set(RunStatus::Completed),
                ..Default::default()
            },
        )
        .build(sea_orm::DatabaseBackend::Postgres)
        .to_string();
        assert!(sql.contains("\"ExecutionRun\".\"id\" = 'run-1'"));
        assert!(sql.contains("\"ExecutionRun\".\"status\" IN ('PENDING', 'RUNNING')"));
    }

    #[tokio::test]
    async fn stream_failure_uses_the_executor_error_envelope() {
        let stream = futures_util::stream::iter([Ok::<_, Infallible>(stream_failure_event(
            "Executor failed: out of memory".into(),
        ))]);
        let body = Sse::new(stream).into_response().into_body();
        let mut events = body.into_data_stream().eventsource();
        let event = events.next().await.unwrap().unwrap();
        assert_eq!(event.event, "error");
        let envelope: serde_json::Value = serde_json::from_str(&event.data).unwrap();
        assert_eq!(envelope["event_type"], "error");
        assert_eq!(
            envelope["payload"]["message"],
            "Executor failed: out of memory"
        );
    }

    #[test]
    fn failure_reason_cannot_overwrite_a_finished_run() {
        use sea_orm::QueryTrait;
        let sql = failure_reason_update("run-1", "out of memory")
            .build(sea_orm::DatabaseBackend::Postgres)
            .to_string();
        assert!(sql.contains("\"errorMessage\" = 'out of memory'"), "{sql}");
        assert!(sql.contains("\"ExecutionRun\".\"id\" = 'run-1'"), "{sql}");
        assert!(
            sql.contains("\"ExecutionRun\".\"status\" IN ('PENDING', 'RUNNING')"),
            "{sql}"
        );
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
