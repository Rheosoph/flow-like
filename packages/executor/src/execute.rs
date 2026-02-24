//! Core execution logic
//!
//! Environment-agnostic flow execution with batched callback reporting

use crate::config::{ExecutorConfig};
use crate::error::ExecutorError;
use crate::jwt::{verify_jwt_async, ExecutorClaims};
use crate::types::{EventType, ExecutionEvent, ExecutionRequest, ExecutionResult, ExecutionStatus};
use flow_like::credentials::StoreType;
use flow_like::flow::board::Board;
use flow_like::flow::event::Event;
use flow_like::flow::execution::{InternalRun, RunPayload};
use flow_like::flow::oauth::OAuthToken;
use flow_like::flow_like_model_provider::provider::ModelProviderConfiguration;
use flow_like::profile::Profile;
use flow_like::state::{FlowLikeConfig, FlowLikeState, FlowNodeRegistryInner};
use flow_like::utils::http::HTTPClient;
use flow_like_catalog::get_catalog;
use flow_like_storage::Path;
use flow_like_types::create_id;
use flow_like_types::intercom::BufferedInterComHandler;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::mpsc;

/// API-compatible event input format
#[derive(Debug, Clone, Serialize, Deserialize)]
struct ApiEventInput {
    event_type: String,
    payload: serde_json::Value,
}

/// API-compatible events push request
#[derive(Debug, Clone, Serialize)]
struct PushEventsRequest {
    events: Vec<ApiEventInput>,
}

/// API-compatible progress update request
#[derive(Debug, Clone, Serialize)]
struct ProgressUpdateRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    progress: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    current_step: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    output_len: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

/// Execute a flow with batched callback reporting
pub async fn execute(
    request: ExecutionRequest,
    config: ExecutorConfig,
) -> Result<ExecutionResult, ExecutorError> {
    let start = Instant::now();

    // Verify JWT and extract claims
    let claims = verify_jwt_async(&request.executor_jwt).await?;

    // Create stores from credentials
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

    // Set up event channel for API callback batching
    let (event_tx, event_rx) = mpsc::unbounded_channel::<ExecutionEvent>();
    let sequence = Arc::new(AtomicI32::new(0));

    // Start callback batcher for sending events to API
    let executor_jwt = request.executor_jwt.clone();
    let callback_handle = tokio::spawn(run_callback_batcher(
        event_rx,
        claims.clone(),
        executor_jwt.clone(),
        config.clone(),
    ));

    // Build FlowLike state
    let catalog = get_catalog();
    let mut flow_config = FlowLikeConfig::with_default_store(content_store);
    flow_config.register_app_meta_store(meta_store.clone());
    flow_config.register_log_store(log_store);

    // Register logs database builder for LanceDB log storage
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

    let catalog_arc = Arc::new(catalog);
    let mut registry = FlowNodeRegistryInner::prepare(&catalog_arc);

    // Load WASM packages from presigned URLs if any are specified
    if let Some(ref wasm_packages) = request.wasm_packages {
        if !wasm_packages.is_empty() {
            match crate::wasm_loader::load_wasm_packages(wasm_packages).await {
                Ok(wasm_nodes) => {
                    tracing::info!(count = wasm_nodes.len(), "Loaded WASM nodes for execution");
                    for logic in wasm_nodes {
                        let node = logic.get_node();
                        registry.insert(node, logic);
                    }
                }
                Err(e) => {
                    tracing::error!(error = %e, "Failed to load WASM packages");
                }
            }
        }
    }

    state.node_registry.write().await.node_registry = Arc::new(registry);

    let state = Arc::new(state);

    // Load board using pre-resolved board_id and version
    let board_id = &request.board_id;
    let storage_root = Path::from("apps").child(request.app_id.to_string());
    let board = Board::load(storage_root, board_id, state.clone(), request.board_version)
        .await
        .map_err(|e| {
            ExecutorError::BoardLoad(format!("Failed to load board {}: {}", board_id, e))
        })?;
    let board = Arc::new(board);

    // Send start event to API
    send_event(
        &event_tx,
        &sequence,
        &claims.run_id,
        EventType::Log,
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

    // Create run payload with the node_id to execute
    let mut profile: Profile = request
        .profile
        .as_ref()
        .and_then(|p| serde_json::from_value(p.clone()).ok())
        .unwrap_or_default();

    // Always use the API's callback URL as hub for remote interactions
    profile.hub = claims.callback_url.clone();

    let run_payload = RunPayload {
        id: request.node_id.clone(),
        payload: request.payload.clone(),
        runtime_variables: request.runtime_variables.clone(),
        filter_secrets: Some(true),
    };

    // Create BufferedInterComHandler - this is REQUIRED for meaningful execution output
    // It batches InterCom events and forwards them to the API callback
    let event_tx_clone = event_tx.clone();
    let sequence_clone = sequence.clone();
    let run_id_clone = claims.run_id.clone();
    let intercom_handler = BufferedInterComHandler::new(
        Arc::new(move |events| {
            let tx = event_tx_clone.clone();
            let seq = sequence_clone.clone();
            let run_id = run_id_clone.clone();
            Box::pin(async move {
                for intercom_event in events {
                    let seq_num = seq.fetch_add(1, Ordering::SeqCst);
                    let exec_event = ExecutionEvent {
                        id: create_id(),
                        run_id: run_id.clone(),
                        sequence: seq_num,
                        event_type: string_to_event_type(&intercom_event.event_type),
                        payload: intercom_event.payload,
                        created_at: chrono::Utc::now(),
                    };
                    let _ = tx.send(exec_event);
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
        run_id = %claims.run_id,
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
        Some(claims.run_id.clone()),
    )
    .await
    .map_err(|e| ExecutorError::RunInit(e.to_string()))?;

    // Set user context if provided
    if let Some(user_context) = request.user_context.clone() {
        run.set_user_context(user_context);
    }

    // Execute with timeout
    let execution_result = tokio::time::timeout(config.execution_timeout(), async {
        run.execute(state.clone()).await
    })
    .await;

    // Flush any remaining buffered intercom events
    if let Err(e) = intercom_handler.flush().await {
        tracing::warn!(error = %e, "Failed to flush intercom handler");
    }

    let duration_ms = start.elapsed().as_millis() as u64;

    let (status, output, error) = match &execution_result {
        Ok(log_meta) => {
            // Flush logs to database if we have metadata
            tracing::debug!(
                has_log_meta = log_meta.is_some(),
                "Execution completed, checking for log metadata"
            );
            if let Some(meta) = log_meta {
                let (db_fn, write_options) = {
                    let guard = state.config.read().await;
                    (
                        guard.callbacks.build_logs_database.clone(),
                        guard.callbacks.lance_write_options.clone(),
                    )
                };
                tracing::debug!(
                    has_db_builder = db_fn.is_some(),
                    "Retrieved log database builder from state"
                );
                if let Some(db_fn) = db_fn.as_ref() {
                    let base_path = Path::from("runs")
                        .child(request.app_id.as_str())
                        .child(request.board_id.as_str());
                    tracing::info!(path = %base_path, "Opening log database to flush run metadata");
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
                } else {
                    tracing::warn!("No log database builder configured in state - run metadata will not be persisted");
                }
            } else {
                tracing::warn!(
                    "No log metadata returned from execution - logs may not have been flushed"
                );
            }

            send_event(
                &event_tx,
                &sequence,
                &claims.run_id,
                EventType::Log,
                serde_json::json!({ "message": "Execution completed" }),
            );
            (ExecutionStatus::Completed, None, None)
        }
        Err(_) => {
            send_event(
                &event_tx,
                &sequence,
                &claims.run_id,
                EventType::Error,
                serde_json::json!({ "message": "Execution timeout" }),
            );
            (
                ExecutionStatus::Failed,
                None,
                Some("Execution timeout".to_string()),
            )
        }
    };

    // Signal completion to callback batcher
    drop(event_tx);

    // Wait for callback batcher to finish
    let _ = callback_handle.await;

    // Send final progress update
    let progress_update = ProgressUpdateRequest {
        progress: Some(100),
        current_step: None,
        status: Some(format!("{:?}", status).to_lowercase()),
        output_len: None,
        error: error.clone(),
    };

    let progress_url = format!(
        "{}/progress",
        claims.callback_url.trim_end_matches("/events")
    );
    let _ = send_progress(&progress_url, &executor_jwt, &progress_update, &config).await;

    Ok(ExecutionResult {
        run_id: claims.run_id,
        status,
        output,
        error,
        duration_ms,
    })
}

fn string_to_event_type(s: &str) -> EventType {
    match s {
        "log" => EventType::Log,
        "progress" => EventType::Progress,
        "output" => EventType::Output,
        "error" => EventType::Error,
        "chunk" => EventType::Chunk,
        "node_start" => EventType::NodeStart,
        "node_end" => EventType::NodeEnd,
        other => EventType::Custom(other.to_string()),
    }
}

fn send_event(
    tx: &mpsc::UnboundedSender<ExecutionEvent>,
    sequence: &Arc<AtomicI32>,
    run_id: &str,
    event_type: EventType,
    payload: serde_json::Value,
) {
    let seq = sequence.fetch_add(1, Ordering::SeqCst);
    let event = ExecutionEvent {
        id: create_id(),
        run_id: run_id.to_string(),
        sequence: seq,
        event_type,
        payload,
        created_at: chrono::Utc::now(),
    };
    let _ = tx.send(event);
}

async fn run_callback_batcher(
    mut event_rx: mpsc::UnboundedReceiver<ExecutionEvent>,
    claims: ExecutorClaims,
    executor_jwt: String,
    config: ExecutorConfig,
) {
    let mut batch = Vec::new();
    let mut interval = tokio::time::interval(config.batch_interval());

    loop {
        tokio::select! {
            _ = interval.tick() => {
                if !batch.is_empty() {
                    let events = std::mem::take(&mut batch);
                    if let Err(e) = send_events_to_api(
                        &claims.callback_url,
                        &executor_jwt,
                        events,
                        &config,
                    ).await {
                        tracing::warn!(error = %e, "Failed to send events batch");
                    }
                }
            }
            event = event_rx.recv() => {
                match event {
                    Some(e) => {
                        batch.push(e);
                        if batch.len() >= config.max_batch_size {
                            let events = std::mem::take(&mut batch);
                            if let Err(e) = send_events_to_api(
                                &claims.callback_url,
                                &executor_jwt,
                                events,
                                &config,
                            ).await {
                                tracing::warn!(error = %e, "Failed to send events batch");
                            }
                        }
                    }
                    None => {
                        if !batch.is_empty() {
                            let events = std::mem::take(&mut batch);
                            let _ = send_events_to_api(
                                &claims.callback_url,
                                &executor_jwt,
                                events,
                                &config,
                            ).await;
                        }
                        break;
                    }
                }
            }
        }
    }
}

fn event_type_to_string(event_type: &EventType) -> String {
    match event_type {
        EventType::Log => "log".to_string(),
        EventType::Progress => "progress".to_string(),
        EventType::Output => "output".to_string(),
        EventType::Error => "error".to_string(),
        EventType::Chunk => "chunk".to_string(),
        EventType::NodeStart => "node_start".to_string(),
        EventType::NodeEnd => "node_end".to_string(),
        EventType::Custom(s) => s.clone(),
    }
}

async fn send_events_to_api(
    url: &str,
    jwt: &str,
    events: Vec<ExecutionEvent>,
    config: &ExecutorConfig,
) -> Result<(), ExecutorError> {
    let api_events: Vec<ApiEventInput> = events
        .into_iter()
        .map(|e| ApiEventInput {
            event_type: event_type_to_string(&e.event_type),
            payload: e.payload,
        })
        .collect();

    let request = PushEventsRequest { events: api_events };
    let client = reqwest::Client::new();

    for attempt in 0..=config.callback_retries {
        let result = client
            .post(url)
            .header("Authorization", format!("Bearer {}", jwt))
            .header("Content-Type", "application/json")
            .timeout(config.callback_timeout())
            .json(&request)
            .send()
            .await;

        match result {
            Ok(response) if response.status().is_success() => return Ok(()),
            Ok(response) => {
                let status = response.status();
                let body = response.text().await.unwrap_or_default();
                tracing::warn!(attempt, status = %status, body = %body, "Events callback failed");
            }
            Err(e) => {
                tracing::warn!(attempt, error = %e, "Events callback error");
            }
        }

        if attempt < config.callback_retries {
            tokio::time::sleep(std::time::Duration::from_millis(100 * (attempt as u64 + 1))).await;
        }
    }

    Err(ExecutorError::Callback(format!(
        "Failed after {} retries",
        config.callback_retries
    )))
}

async fn send_progress(
    url: &str,
    jwt: &str,
    progress: &ProgressUpdateRequest,
    config: &ExecutorConfig,
) -> Result<(), ExecutorError> {
    let client = reqwest::Client::new();

    for attempt in 0..=config.callback_retries {
        let result = client
            .post(url)
            .header("Authorization", format!("Bearer {}", jwt))
            .header("Content-Type", "application/json")
            .timeout(config.callback_timeout())
            .json(progress)
            .send()
            .await;

        match result {
            Ok(response) if response.status().is_success() => return Ok(()),
            Ok(response) => {
                let status = response.status();
                let body = response.text().await.unwrap_or_default();
                tracing::warn!(attempt, status = %status, body = %body, "Progress callback failed");
            }
            Err(e) => {
                tracing::warn!(attempt, error = %e, "Progress callback error");
            }
        }

        if attempt < config.callback_retries {
            tokio::time::sleep(std::time::Duration::from_millis(100 * (attempt as u64 + 1))).await;
        }
    }

    Err(ExecutorError::Callback(format!(
        "Failed after {} retries",
        config.callback_retries
    )))
}
