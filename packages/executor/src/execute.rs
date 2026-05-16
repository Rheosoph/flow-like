//! Core execution logic
//!
//! Environment-agnostic flow execution with batched callback reporting

use crate::config::ExecutorConfig;
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
use std::sync::{Arc, LazyLock};
use std::time::{Duration, Instant};
use tokio::sync::mpsc;

/// Cached prepared registry - initialized once on first access.
/// Contains the static catalog nodes only; WASM nodes are overlaid per-request.
pub(crate) static PREPARED_REGISTRY: LazyLock<FlowNodeRegistryInner> = LazyLock::new(|| {
    let catalog = get_catalog();
    let catalog_arc = Arc::new(catalog);
    FlowNodeRegistryInner::prepare(&catalog_arc)
});

/// One entry in [`BOARD_PROTO_CACHE`]. Carries the storage-level identity
/// (`e_tag` / `last_modified`) captured during the original load so a cheap
/// HEAD can verify the cached body still matches what's on disk.
struct CachedProto {
    proto: flow_like_types::proto::Board,
    e_tag: Option<String>,
    last_modified: chrono::DateTime<chrono::Utc>,
}

/// Cache of decompressed board protos, keyed by `(app_id, board_id, version)`.
///
/// The proto is the on-disk representation: identical for all callers of the
/// same `(app_id, board_id, version)` tuple, with no per-request state. Sharing
/// it across requests skips the dominant cost of `Board::load` (object-store
/// fetch + lz4 decompression + protobuf decode) — typically the largest single
/// cost on warm Lambda invocations.
///
/// The per-request `Board` is still rebuilt from a clone of the cached proto
/// against the caller's `FlowLikeState` (and registry, which carries
/// per-request WASM packages) — that step keeps execution state isolated.
///
/// Freshness: pinned versions are immutable in storage and skip revalidation.
/// `latest` does a HEAD before reuse so rapid edit-test cycles never execute
/// stale boards — HEAD is ~10–30 ms vs. ~hundreds of ms for the full reload.
/// TTL bounds memory growth on long-lived instances.
static BOARD_PROTO_CACHE: LazyLock<moka::sync::Cache<String, Arc<CachedProto>>> =
    LazyLock::new(|| {
        moka::sync::Cache::builder()
            .max_capacity(256)
            .time_to_live(Duration::from_secs(30 * 60))
            .time_to_idle(Duration::from_secs(5 * 60))
            .build()
    });

fn board_cache_key(app_id: &str, board_id: &str, version: Option<(u32, u32, u32)>) -> String {
    match version {
        Some((m, n, p)) => format!("{}:{}:{}_{}_{}", app_id, board_id, m, n, p),
        None => format!("{}:{}:latest", app_id, board_id),
    }
}

/// True when the storage-level identity we recorded at load time still
/// matches what HEAD returns now. We prefer e_tag (cheap, exact) and fall
/// back to last_modified when the backend doesn't expose one.
fn meta_unchanged(
    cached: &CachedProto,
    head: &flow_like_storage::object_store::ObjectMeta,
) -> bool {
    match (&cached.e_tag, &head.e_tag) {
        (Some(a), Some(b)) => a == b,
        _ => cached.last_modified == head.last_modified,
    }
}

/// Resolve the request's board with the proto cache + HEAD revalidation.
///
/// On a cache hit for the floating "latest", we kick off the HEAD probe and
/// the per-request `from_loaded_proto` build (`node_updates` etc.) in
/// parallel — HEAD is ~5–30 ms and the build is typically larger, so the
/// validation drops out of the critical path entirely. If HEAD reports the
/// cached proto stale (or fails), we discard the speculative board and
/// reload from storage. Pinned versions are immutable and skip HEAD.
async fn resolve_board(
    state: &Arc<FlowLikeState>,
    request: &ExecutionRequest,
    storage_root: &Path,
) -> Result<flow_like::flow::board::Board, ExecutorError> {
    let board_id = &request.board_id;
    let cache_key = board_cache_key(&request.app_id, board_id, request.board_version);
    let proto_path = Board::proto_path(storage_root, board_id, request.board_version);
    let is_pinned_version = request.board_version.is_some();

    let meta_store = state
        .config
        .read()
        .await
        .stores
        .app_meta_store
        .clone()
        .ok_or_else(|| {
            ExecutorError::BoardLoad(format!(
                "Project store not found while loading board {}",
                board_id
            ))
        })?
        .as_generic();

    if let Some(cached) = BOARD_PROTO_CACHE.get(&cache_key) {
        if is_pinned_version {
            tracing::debug!(cache_key = %cache_key, "Board proto cache hit (pinned)");
            return Ok(Board::from_loaded_proto(
                cached.proto.clone(),
                storage_root.clone(),
                state.clone(),
            )
            .await);
        }

        let speculative_proto = cached.proto.clone();
        let head_fut = meta_store.head(&proto_path);
        let build_fut =
            Board::from_loaded_proto(speculative_proto, storage_root.clone(), state.clone());
        let (head_result, speculative_board) = tokio::join!(head_fut, build_fut);

        match head_result {
            Ok(head_meta) if meta_unchanged(&cached, &head_meta) => {
                tracing::debug!(cache_key = %cache_key, "Board proto cache hit (HEAD validated)");
                return Ok(speculative_board);
            }
            Ok(_) => {
                tracing::debug!(cache_key = %cache_key, "Board proto cache stale, reloading");
            }
            Err(e) => {
                tracing::warn!(
                    cache_key = %cache_key,
                    error = %e,
                    "HEAD failed during cache revalidation, falling back to full reload"
                );
            }
        }
        // The speculative board built from a stale proto is unsafe to return;
        // drop it and load fresh below.
        drop(speculative_board);
    }

    let (loaded, meta) =
        Board::load_proto_with_meta(meta_store, storage_root, board_id, request.board_version)
            .await
            .map_err(|e| {
                ExecutorError::BoardLoad(format!("Failed to load board {}: {}", board_id, e))
            })?;
    BOARD_PROTO_CACHE.insert(
        cache_key.clone(),
        Arc::new(CachedProto {
            proto: loaded.clone(),
            e_tag: meta.e_tag.clone(),
            last_modified: meta.last_modified,
        }),
    );
    tracing::debug!(cache_key = %cache_key, "Board proto cache populated");
    Ok(Board::from_loaded_proto(loaded, storage_root.clone(), state.clone()).await)
}

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

    let mut registry = PREPARED_REGISTRY.clone();

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

    let board_id = &request.board_id;
    let storage_root = Path::from("apps").child(request.app_id.to_string());
    let board = Arc::new(resolve_board(&state, &request, &storage_root).await?);

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

    let context_token = request
        .token
        .clone()
        .or_else(|| Some(request.executor_jwt.clone()));

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
        context_token,
        oauth_tokens,
        Some(claims.run_id.clone()),
    )
    .await
    .map_err(|e| ExecutorError::RunInit(e.to_string()))?;

    run.set_execution_sub(claims.sub.clone()).await;

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

    // The batcher only exits when every clone of event_tx is dropped.
    // run owns the InterComCallback chain and intercom_handler owns the
    // BatchedCallback Arc — both transitively hold event_tx_clone, so
    // they must be released before awaiting the batcher.
    drop(run);
    drop(intercom_handler);
    drop(event_tx);

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
        "{}/api/v1/execution/progress",
        claims.callback_url.trim_end_matches('/')
    );
    let http_client = reqwest::Client::new();
    let _ = send_progress(
        &progress_url,
        &executor_jwt,
        &progress_update,
        &config,
        &http_client,
    )
    .await;

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
    let events_url = format!(
        "{}/api/v1/execution/events",
        claims.callback_url.trim_end_matches('/')
    );
    let client = reqwest::Client::new();
    let mut batch = Vec::new();
    let mut interval = tokio::time::interval(config.batch_interval());

    loop {
        tokio::select! {
            _ = interval.tick() => {
                if !batch.is_empty() {
                    let events = std::mem::take(&mut batch);
                    if let Err(e) = send_events_to_api(
                        &events_url,
                        &executor_jwt,
                        events,
                        &config,
                        &client,
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
                                &events_url,
                                &executor_jwt,
                                events,
                                &config,
                                &client,
                            ).await {
                                tracing::warn!(error = %e, "Failed to send events batch");
                            }
                        }
                    }
                    None => {
                        if !batch.is_empty() {
                            let events = std::mem::take(&mut batch);
                            let _ = send_events_to_api(
                                &events_url,
                                &executor_jwt,
                                events,
                                &config,
                                &client,
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
    client: &reqwest::Client,
) -> Result<(), ExecutorError> {
    let api_events: Vec<ApiEventInput> = events
        .into_iter()
        .map(|e| ApiEventInput {
            event_type: event_type_to_string(&e.event_type),
            payload: e.payload,
        })
        .collect();

    let request = PushEventsRequest { events: api_events };

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
    client: &reqwest::Client,
) -> Result<(), ExecutorError> {
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
