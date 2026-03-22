//! Streaming execution support
//!
//! Provides streaming execution that yields events as they occur,
//! suitable for Lambda streaming responses or SSE endpoints.

use crate::config::ExecutorConfig;
use crate::error::ExecutorError;
use crate::jwt::verify_jwt_async;
use crate::types::{ExecutionRequest, ExecutionStatus};
use flow_like::credentials::StoreType;
use flow_like::flow::board::Board;
use flow_like::flow::event::Event;
use flow_like::flow::execution::{InternalRun, RunPayload};
use flow_like::flow::oauth::OAuthToken;
use flow_like::flow_like_model_provider::provider::ModelProviderConfiguration;
use flow_like::profile::Profile;
use flow_like::state::{FlowLikeConfig, FlowLikeState};
use flow_like::utils::http::HTTPClient;
use flow_like_storage::Path;
use flow_like_types::intercom::{BufferedInterComHandler, InterComEvent};
use futures_util::Stream;
use std::collections::HashMap;
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

    let (tx, rx) = mpsc::unbounded_channel::<StreamEvent>();

    // Send started event immediately
    let _ = tx.send(run_initiated_event(&claims.run_id));

    // Spawn execution task
    tokio::spawn(run_execution(
        request,
        config,
        claims.run_id,
        claims.callback_url,
        tx,
    ));

    Ok(ExecutionStream { rx })
}

async fn run_execution(
    request: ExecutionRequest,
    config: ExecutorConfig,
    run_id: String,
    callback_url: String,
    tx: mpsc::UnboundedSender<StreamEvent>,
) {
    let start = Instant::now();

    let result = execute_inner(&request, &config, &run_id, &callback_url, &tx).await;

    let duration_ms = start.elapsed().as_millis() as u64;

    match result {
        Ok((status, log_level, _output, _error)) => {
            let _ = tx.send(completed_event(&run_id, status, duration_ms, log_level));
        }
        Err(e) => {
            let _ = tx.send(error_event(&e.to_string()));
        }
    }
}

async fn execute_inner(
    request: &ExecutionRequest,
    config: &ExecutorConfig,
    run_id: &str,
    callback_url: &str,
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
    let content_store = request
        .credentials
        .to_store_type(StoreType::Content)
        .await
        .map_err(|e| ExecutorError::Storage(e.to_string()))?;

    let meta_store = request
        .credentials
        .to_store_type(StoreType::Meta)
        .await
        .map_err(|e| ExecutorError::Storage(e.to_string()))?;

    let log_store = request
        .credentials
        .to_store_type(StoreType::Logs)
        .await
        .map_err(|e| ExecutorError::Storage(e.to_string()))?;

    let mut flow_config = FlowLikeConfig::with_default_store(content_store);
    flow_config.register_app_meta_store(meta_store.clone());
    flow_config.register_log_store(log_store);

    match request.credentials.to_logs_db_builder() {
        Ok(logs_db_builder) => {
            tracing::info!("Successfully created logs database builder");
            flow_config.register_build_logs_database(logs_db_builder);
        }
        Err(e) => {
            tracing::error!(error = %e, "Failed to create logs database builder - logs will not be persisted");
        }
    }

    // Load model provider configuration from environment
    let model_provider_config = ModelProviderConfiguration::default();

    let http_client = HTTPClient::new_without_refetch();
    let state =
        FlowLikeState::new_with_model_config(flow_config, http_client, model_provider_config);

    let mut registry = crate::execute::PREPARED_REGISTRY.clone();

    // Load WASM packages from presigned URLs if any are specified
    if let Some(ref wasm_packages) = request.wasm_packages {
        if !wasm_packages.is_empty() {
            match crate::wasm_loader::load_wasm_packages(wasm_packages).await {
                Ok(wasm_nodes) => {
                    tracing::info!(
                        count = wasm_nodes.len(),
                        "Loaded WASM nodes for streaming execution"
                    );
                    for logic in wasm_nodes {
                        let node = logic.get_node();
                        registry.insert(node, logic);
                    }
                }
                Err(e) => {
                    tracing::error!(error = %e, "Failed to load WASM packages for streaming");
                }
            }
        }
    }

    state.node_registry.write().await.node_registry = Arc::new(registry);

    let state = Arc::new(state);

    // Load board using pre-resolved board_id and version (cached)
    let board_id = &request.board_id;
    let storage_root = Path::from("apps").child(request.app_id.to_string());
    let cache_key = (
        request.app_id.clone(),
        board_id.clone(),
        request.board_version,
    );
    let state_clone = state.clone();
    let storage_root_clone = storage_root.clone();
    let board_id_clone = board_id.clone();
    let board_version = request.board_version;
    let board = crate::execute::BOARD_CACHE
        .try_get_with(cache_key, async move {
            let b = Board::load(
                storage_root_clone,
                &board_id_clone,
                state_clone,
                board_version,
            )
            .await
            .map_err(|e| {
                ExecutorError::BoardLoad(format!("Failed to load board {}: {}", board_id_clone, e))
            })?;
            Ok::<Arc<Board>, ExecutorError>(Arc::new(b))
        })
        .await
        .map_err(|e: Arc<ExecutorError>| {
            ExecutorError::BoardLoad(format!("Board cache error: {}", e))
        })?;

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

    let mut run = InternalRun::new_with_run_id(
        &request.app_id,
        board.clone(),
        event,
        &state,
        &profile,
        &run_payload,
        request.stream_state,
        callback,
        Some(request.credentials.clone()),
        Some(request.executor_jwt.clone()),
        oauth_tokens,
        Some(run_id.to_string()),
    )
    .await
    .map_err(|e| ExecutorError::RunInit(e.to_string()))?;

    // Set user context if provided
    if let Some(user_context) = request.user_context.clone() {
        run.set_user_context(user_context);
    }

    let execution_result = tokio::time::timeout(config.execution_timeout(), async {
        run.execute(state.clone()).await
    })
    .await;

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
                    match db_fn(base_path.clone()).execute().await {
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

            emit_event(
                tx,
                "log",
                serde_json::json!({ "message": "Execution completed" }),
            );
            Ok((ExecutionStatus::Completed, log_level, None, None))
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
