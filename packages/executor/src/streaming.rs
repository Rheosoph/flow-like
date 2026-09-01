//! Streaming execution support
//!
//! Provides streaming execution that yields events as they occur,
//! suitable for Lambda streaming responses or SSE endpoints.

use crate::config::ExecutorConfig;
use crate::error::ExecutorError;
use crate::execute::validate_executor_request_claims;
use crate::jwt::verify_jwt_async;
use crate::types::{ExecutionRequest, ExecutionStatus};
use flow_like::flow::event::Event;
use flow_like::flow::execution::rejection::RejectionStage;
use flow_like::flow::execution::{InternalRun, RunPayload};
use flow_like::flow::oauth::OAuthToken;
use flow_like::profile::Profile;
use flow_like_storage::Path;
use flow_like_types::intercom::{BufferedInterComHandler, InterComEvent};
use futures_util::Stream;
use std::collections::{BTreeSet, HashMap};
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Instant;
use tokio::sync::mpsc;

/// All events are sent as InterComEvent for consistent frontend handling
pub type StreamEvent = InterComEvent;

pub fn event_to_ndjson(event: &StreamEvent) -> String {
    serde_json::to_string(event).unwrap_or_default() + "\n"
}

pub fn event_to_sse(event: &StreamEvent) -> String {
    let data = serde_json::to_string(event).unwrap_or_default();
    format!("data: {}\n\n", data)
}

pub fn run_initiated_event(run_id: &str) -> StreamEvent {
    InterComEvent::with_type("run_initiated", serde_json::json!({ "run_id": run_id }))
}

pub fn completed_event(
    run_id: &str,
    status: ExecutionStatus,
    duration_ms: u64,
    log_level: Option<u8>,
) -> StreamEvent {
    InterComEvent::with_type(
        "completed",
        serde_json::json!({
            "run_id": run_id,
            "status": status,
            "duration_ms": duration_ms,
            "log_level": log_level.unwrap_or(0)
        }),
    )
}

pub fn error_event(message: &str) -> StreamEvent {
    InterComEvent::with_type("error", serde_json::json!({ "message": message }))
}

const PAGE_EXECUTION_FAILURE_MESSAGE: &str = "Page execution failed";

fn fallback_error_message(error: &ExecutorError, is_page_execution: bool) -> String {
    if is_page_execution {
        PAGE_EXECUTION_FAILURE_MESSAGE.to_string()
    } else {
        error.to_string()
    }
}

fn send_fallback_failure(
    tx: &mpsc::UnboundedSender<StreamEvent>,
    run_id: &str,
    duration_ms: u64,
    error: &ExecutorError,
    is_page_execution: bool,
) {
    tracing::error!(
        run_id,
        error = %error,
        is_page_execution,
        "Streaming execution failed before the workflow could report a result"
    );
    let message = fallback_error_message(error, is_page_execution);
    let _ = tx.send(error_event(&message));
    let _ = tx.send(completed_event(
        run_id,
        ExecutionStatus::Failed,
        duration_ms,
        Some(4),
    ));
}

/// Stream of execution events
pub struct ExecutionStream {
    rx: mpsc::UnboundedReceiver<StreamEvent>,
}

impl Stream for ExecutionStream {
    type Item = StreamEvent;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        Pin::new(&mut self.rx).poll_recv(cx)
    }
}

/// Execute a flow and stream events back
pub async fn execute_streaming(
    request: ExecutionRequest,
    config: ExecutorConfig,
) -> Result<ExecutionStream, ExecutorError> {
    let claims = verify_jwt_async(&request.executor_jwt).await?;
    if let Err(error) = validate_executor_request_claims(&claims, &request) {
        crate::execute::record_claims_rejection(&request, &claims.run_id, &error).await;
        return Err(error);
    }

    let (tx, rx) = mpsc::unbounded_channel::<StreamEvent>();

    // Send started event immediately
    let _ = tx.send(run_initiated_event(&claims.run_id));

    // Spawn execution task
    tokio::spawn(run_execution(
        request,
        config,
        claims.run_id,
        claims.callback_url,
        claims.sub,
        claims.page_execution.is_some(),
        tx,
    ));

    Ok(ExecutionStream { rx })
}

async fn run_execution(
    request: ExecutionRequest,
    config: ExecutorConfig,
    run_id: String,
    callback_url: String,
    executor_subject: String,
    is_page_execution: bool,
    tx: mpsc::UnboundedSender<StreamEvent>,
) {
    let start = Instant::now();

    let result = execute_inner(
        &request,
        &config,
        &run_id,
        &callback_url,
        &executor_subject,
        &tx,
    )
    .await;

    let duration_ms = start.elapsed().as_millis() as u64;

    match result {
        Ok((status, log_level, _output, _error)) => {
            let _ = tx.send(completed_event(&run_id, status, duration_ms, log_level));
        }
        Err(e) => {
            send_fallback_failure(&tx, &run_id, duration_ms, &e, is_page_execution);
        }
    }
}

async fn execute_inner(
    request: &ExecutionRequest,
    config: &ExecutorConfig,
    run_id: &str,
    callback_url: &str,
    executor_subject: &str,
    tx: &mpsc::UnboundedSender<StreamEvent>,
) -> Result<
    (
        ExecutionStatus,
        Option<u8>,
        Option<serde_json::Value>,
        Option<String>,
    ),
    ExecutorError,
> {
    let state = crate::execute::build_flow_state(&request.credentials).await?;
    let execution_environment = state.execution_environment;

    let mut wasm_nodes = Vec::new();
    let mut failed_wasm_package_ids = BTreeSet::new();

    // Load WASM packages from presigned URLs if any are specified
    if let Some(ref wasm_packages) = request.wasm_packages {
        if !wasm_packages.is_empty() {
            match crate::wasm_loader::load_wasm_packages(
                &request.app_id,
                &request.board_id,
                request.board_version,
                wasm_packages,
            )
            .await
            {
                Ok(report) => {
                    tracing::info!(
                        count = report.nodes.len(),
                        "Loaded WASM nodes for streaming execution"
                    );
                    failed_wasm_package_ids = report.failed_package_ids;
                    wasm_nodes = report.nodes;
                }
                Err(e) => {
                    return Err(e);
                }
            }
        }
    }

    state.node_registry.write().await.node_registry = crate::execute::request_registry(wasm_nodes);

    let state = Arc::new(state);

    let board_id = &request.board_id;
    let template = match crate::execute::resolve_run_template(&state, request)
        .await
        .map_err(|e| match e {
            ExecutorError::BoardLoad(msg) if !failed_wasm_package_ids.is_empty() => {
                let failed: Vec<&str> =
                    failed_wasm_package_ids.iter().map(String::as_str).collect();
                ExecutorError::BoardLoad(format!(
                    "{} (WASM packages failed to load: {})",
                    msg,
                    failed.join(", ")
                ))
            }
            other => other,
        }) {
        Ok(template) => template,
        Err(error) => {
            crate::execute::record_executor_rejection(
                &state,
                request,
                run_id,
                RejectionStage::Resolution,
                error.to_string(),
            )
            .await;
            return Err(error);
        }
    };
    let unavailable_wasm_packages = crate::wasm_loader::unavailable_board_wasm_packages(
        template.board.as_ref(),
        request.wasm_packages.as_ref(),
        &failed_wasm_package_ids,
    );
    if !unavailable_wasm_packages.is_empty() {
        let error = ExecutorError::Execution(format!(
            "Missing WASM package artifacts for board {}: {}",
            board_id,
            unavailable_wasm_packages.join(", ")
        ));
        crate::execute::record_executor_rejection(
            &state,
            request,
            run_id,
            RejectionStage::Setup,
            error.to_string(),
        )
        .await;
        return Err(error);
    }

    emit_event(
        tx,
        "log",
        serde_json::json!({ "message": "Execution started" }),
    );

    // Parse event from JSON if provided
    let event: Option<Event> = request
        .event_json
        .as_ref()
        .and_then(|json| serde_json::from_str(json).ok());

    // Convert OAuth tokens from input format to core format
    let oauth_tokens: HashMap<String, OAuthToken> = request
        .oauth_tokens
        .as_ref()
        .map(|tokens| {
            tokens
                .iter()
                .map(|(k, v)| {
                    let token = OAuthToken {
                        access_token: v.access_token.clone(),
                        refresh_token: v.refresh_token.clone(),
                        expires_at: v.expires_at.map(|e| e as u64),
                        token_type: v.token_type.clone(),
                    };
                    (k.clone(), token)
                })
                .collect()
        })
        .unwrap_or_default();

    // Use profile from request if provided, otherwise use default (empty profile)
    let mut profile: Profile = request
        .profile
        .as_ref()
        .and_then(|p| serde_json::from_value(p.clone()).ok())
        .unwrap_or_default();

    // Always use the API's callback URL as hub for remote interactions
    profile.hub = callback_url.to_string();

    let run_payload = RunPayload {
        id: request.node_id.clone(),
        payload: request.payload.clone(),
        runtime_variables: request.runtime_variables.clone(),
        filter_secrets: Some(true),
    };

    // Create BufferedInterComHandler to stream events back to client
    let tx_clone = tx.clone();
    let intercom_handler = BufferedInterComHandler::new(
        Arc::new(move |events| {
            let tx = tx_clone.clone();
            Box::pin(async move {
                tracing::debug!(
                    event_count = events.len(),
                    "Forwarding intercom events batch"
                );
                for intercom_event in events {
                    tracing::debug!(event_type = %intercom_event.event_type, "Forwarding intercom event");
                    let _ = tx.send(intercom_event);
                }
                Ok(())
            })
        }),
        Some(50),
        Some(100),
        Some(true),
    );
    let callback = intercom_handler.into_callback();

    tracing::info!(
        stream_state = request.stream_state,
        app_id = %request.app_id,
        board_id = %request.board_id,
        node_id = %request.node_id,
        run_id = %run_id,
        "Creating InternalRun with predetermined run_id"
    );

    let context_token = request
        .token
        .clone()
        .or_else(|| Some(request.executor_jwt.clone()));

    let channel = crate::channel::build_run_channel(
        request.channel.as_ref(),
        run_id,
        callback_url,
        context_token.as_deref(),
    )
    .await
    .map_err(|e| ExecutorError::RunInit(e.to_string()))?;

    let mut run = InternalRun::from_template(
        &request.app_id,
        template.clone(),
        event,
        &state,
        &profile,
        &run_payload,
        request.stream_state,
        callback,
        Some(request.credentials.clone()),
        context_token,
        oauth_tokens,
        Some(run_id.to_string()),
        Some(channel.clone()),
    )
    .await
    .map_err(|e| ExecutorError::RunInit(e.to_string()))?;

    run.set_execution_environment(execution_environment);
    if let Some(mode) = request.execution_mode {
        run.set_execution_mode(mode);
    }

    run.set_execution_sub(executor_subject.to_string()).await;

    // Set user context if provided
    if let Some(user_context) = request.user_context.clone() {
        run.set_user_context(user_context);
    }

    let execution_result = tokio::time::timeout(config.execution_timeout(), async {
        run.execute(state.clone()).await
    })
    .await;
    channel.close().await;

    // Flush any remaining buffered events
    tracing::debug!("Flushing remaining buffered intercom events");
    let _ = intercom_handler.flush().await;
    tracing::debug!("Intercom flush completed");

    match execution_result {
        Ok(log_meta) => {
            let log_level = log_meta.as_ref().map(|m| m.log_level);

            // Flush logs to database if we have metadata
            if let Some(meta) = &log_meta {
                let (db_fn, write_options) = {
                    let guard = state.config.read().await;
                    (
                        guard.callbacks.build_logs_database.clone(),
                        guard.callbacks.lance_write_options.clone(),
                    )
                };
                if let Some(db_fn) = db_fn.as_ref() {
                    let base_path = Path::from("runs")
                        .child(request.app_id.as_str())
                        .child(request.board_id.as_str());
                    match state
                        .with_lance_session(db_fn(base_path.clone()))
                        .execute()
                        .await
                    {
                        Ok(db) => {
                            if let Err(e) = meta.flush(db, write_options.as_ref()).await {
                                tracing::error!(error = %e, "Failed to flush run logs");
                            } else {
                                tracing::info!("Successfully flushed run logs to {}", base_path);
                            }
                        }
                        Err(e) => {
                            tracing::error!(error = %e, path = %base_path, "Failed to open log database");
                        }
                    }
                }
            }

            let status = ExecutionStatus::from_final_run_status(&run.get_status().await);
            let (event_type, message, error) = match &status {
                ExecutionStatus::Completed => ("log", "Execution completed", None),
                ExecutionStatus::Cancelled => (
                    "error",
                    "Execution cancelled",
                    Some("Execution cancelled".to_string()),
                ),
                ExecutionStatus::Failed | ExecutionStatus::Running => (
                    "error",
                    "Execution failed",
                    Some("Execution failed".to_string()),
                ),
            };
            emit_event(tx, event_type, serde_json::json!({ "message": message }));
            Ok((status, log_level, None, error))
        }
        Err(_) => {
            emit_event(
                tx,
                "error",
                serde_json::json!({ "message": "Execution timeout" }),
            );
            Ok((
                ExecutionStatus::Failed,
                Some(4), // Fatal log level for timeout
                None,
                Some("Execution timeout".to_string()),
            ))
        }
    }
}

fn emit_event(
    tx: &mpsc::UnboundedSender<StreamEvent>,
    event_type: &str,
    payload: serde_json::Value,
) {
    let _ = tx.send(InterComEvent::with_type(event_type, payload));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn page_fallback_error_hides_board_resolution_details() {
        let error = ExecutorError::BoardLoad(
            "Failed to resolve board hidden-board at apps/app-1/boards/hidden-board".to_string(),
        );

        let message = fallback_error_message(&error, true);

        assert_eq!(message, PAGE_EXECUTION_FAILURE_MESSAGE);
        assert!(!message.contains("hidden-board"));
        assert!(!message.contains("apps/app-1"));
    }

    #[test]
    fn direct_board_fallback_error_keeps_existing_diagnostic() {
        let error = ExecutorError::BoardLoad(
            "Failed to resolve board visible-board at apps/app-1/boards/visible-board".to_string(),
        );

        let message = fallback_error_message(&error, false);

        assert!(message.contains("visible-board"));
        assert!(message.contains("apps/app-1"));
    }

    #[test]
    fn fallback_failure_always_finishes_the_stream_with_terminal_status() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let error = ExecutorError::BoardLoad("resolver failed".to_string());

        send_fallback_failure(&tx, "run-1", 42, &error, true);

        let error_event = rx.try_recv().expect("error event");
        assert_eq!(error_event.event_type, "error");
        assert_eq!(
            error_event
                .payload
                .get("message")
                .and_then(|value| value.as_str()),
            Some(PAGE_EXECUTION_FAILURE_MESSAGE)
        );

        let completed = rx.try_recv().expect("completed event");
        assert_eq!(completed.event_type, "completed");
        assert_eq!(
            completed
                .payload
                .get("status")
                .and_then(|value| value.as_str()),
            Some("failed")
        );
        assert!(rx.try_recv().is_err());
    }
}
