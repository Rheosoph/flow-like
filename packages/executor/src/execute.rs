//! Core execution logic
//!
//! Environment-agnostic flow execution with batched callback reporting

use crate::config::ExecutorConfig;
use crate::error::ExecutorError;
use crate::jwt::{ExecutorClaims, verify_jwt_async};
use crate::types::{EventType, ExecutionEvent, ExecutionRequest, ExecutionResult, ExecutionStatus};
use flow_like::credentials::StoreType;
use flow_like::flow::board::Board;
use flow_like::flow::event::Event;
use flow_like::flow::execution::{ExecutionEnvironment, InternalRun, RunPayload};
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
use std::collections::{BTreeSet, HashMap};
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::{Arc, LazyLock};
use std::time::{Duration, Instant};
use tokio::sync::{mpsc, watch};

/// Cached prepared registry - initialized once on first access.
/// Contains the static catalog nodes only; WASM nodes are overlaid per-request.
pub(crate) static PREPARED_REGISTRY: LazyLock<FlowNodeRegistryInner> = LazyLock::new(|| {
    let catalog = get_catalog();
    let catalog_arc = Arc::new(catalog);
    FlowNodeRegistryInner::prepare(&catalog_arc)
});

/// One entry in [`BOARD_CACHE`]. Carries a cleaned, node-updated `Board` with
/// request-specific state stripped, plus the storage-level identity captured
/// during the original load so a cheap HEAD can verify freshness.
struct CachedBoard {
    board: flow_like::flow::board::Board,
    e_tag: Option<String>,
    last_modified: chrono::DateTime<chrono::Utc>,
}

/// Cache of prepared boards, keyed by `(app_id, board_id, version, wasm bundle)`.
///
/// The cached board has already gone through object-store fetch, lz4
/// decompression, protobuf decode, `Board::from_proto`, `node_updates`, and
/// cleanup. Before insertion, `app_state` and `logic_nodes` are removed so a
/// warm Lambda hit cannot reuse caller credentials or request-scoped logic.
///
/// The per-request `Board` is a clone with the current `FlowLikeState` attached.
/// `InternalRun` still builds its own mutable execution graph per run, so pin
/// values and node state remain isolated.
///
/// Freshness: pinned versions are immutable in storage and skip revalidation.
/// `latest` does a HEAD before reuse so rapid edit-test cycles never execute
/// stale boards. TTL bounds memory growth on long-lived instances.
static BOARD_CACHE: LazyLock<moka::sync::Cache<String, Arc<CachedBoard>>> = LazyLock::new(|| {
    moka::sync::Cache::builder()
        .max_capacity(256)
        .time_to_live(Duration::from_secs(30 * 60))
        .time_to_idle(Duration::from_secs(5 * 60))
        .build()
});

fn wasm_registry_signature(request: &ExecutionRequest) -> String {
    let Some(packages) = request.wasm_packages.as_ref() else {
        return "static".to_string();
    };

    if packages.is_empty() {
        return "static".to_string();
    }

    let mut parts = packages
        .iter()
        .map(|(package_id, package_ref)| {
            format!(
                "{}@{}:{}:{}",
                package_id, package_ref.version, package_ref.wasm_hash, package_ref.cwasm_checksum
            )
        })
        .collect::<Vec<_>>();
    parts.sort();
    parts.join("|")
}

fn board_cache_key(request: &ExecutionRequest, board_id: &str) -> String {
    let version = request.board_version;
    let board_key = match version {
        Some((m, n, p)) => format!("{}:{}:{}_{}_{}", request.app_id, board_id, m, n, p),
        None => format!("{}:{}:latest", request.app_id, board_id),
    };
    format!("{}:{}", board_key, wasm_registry_signature(request))
}

fn attach_cached_board(
    cached: &CachedBoard,
    board_dir: Path,
    state: Arc<FlowLikeState>,
) -> flow_like::flow::board::Board {
    let mut board = cached.board.clone();
    board.board_dir = board_dir;
    board.app_state = Some(state);
    board.logic_nodes = HashMap::new();
    board
}

fn strip_request_state(mut board: flow_like::flow::board::Board) -> flow_like::flow::board::Board {
    board.app_state = None;
    board.logic_nodes = HashMap::new();
    board
}

fn board_version_key(app_id: &str, board_id: &str, version: Option<(u32, u32, u32)>) -> String {
    match version {
        Some((m, n, p)) => format!("{}:{}:{}_{}_{}", app_id, board_id, m, n, p),
        None => format!("{}:{}:latest", app_id, board_id),
    }
}

/// True when the storage-level identity we recorded at load time still
/// matches what HEAD returns now. We prefer e_tag (cheap, exact) and fall
/// back to last_modified when the backend doesn't expose one.
fn meta_unchanged(
    cached: &CachedBoard,
    head: &flow_like_storage::object_store::ObjectMeta,
) -> bool {
    match (&cached.e_tag, &head.e_tag) {
        (Some(a), Some(b)) => a == b,
        _ => cached.last_modified == head.last_modified,
    }
}

/// Resolve the request's board with the prepared-board cache + HEAD revalidation.
///
/// On a cache hit for the floating "latest", we kick off the HEAD probe and
/// the per-request board clone in parallel. If HEAD reports the cached board
/// stale (or fails), we discard the speculative clone and reload from storage.
/// Pinned versions are immutable and skip HEAD.
pub(crate) async fn resolve_board(
    state: &Arc<FlowLikeState>,
    request: &ExecutionRequest,
    storage_root: &Path,
) -> Result<flow_like::flow::board::Board, ExecutorError> {
    let board_id = &request.board_id;
    let cache_key = board_cache_key(request, board_id);
    let version_key = board_version_key(&request.app_id, board_id, request.board_version);
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

    if let Some(cached) = BOARD_CACHE.get(&cache_key) {
        if is_pinned_version {
            tracing::debug!(cache_key = %cache_key, "Board cache hit (pinned)");
            return Ok(attach_cached_board(
                cached.as_ref(),
                storage_root.clone(),
                state.clone(),
            ));
        }

        let head_fut = meta_store.head(&proto_path);
        let build_fut =
            async { attach_cached_board(cached.as_ref(), storage_root.clone(), state.clone()) };
        let (head_result, speculative_board) = tokio::join!(head_fut, build_fut);

        match head_result {
            Ok(head_meta) if meta_unchanged(cached.as_ref(), &head_meta) => {
                tracing::debug!(cache_key = %cache_key, "Board cache hit (HEAD validated)");
                return Ok(speculative_board);
            }
            Ok(_) => {
                tracing::debug!(cache_key = %cache_key, "Board cache stale, reloading");
                BOARD_CACHE.invalidate(&cache_key);
            }
            Err(e) => {
                tracing::warn!(
                    cache_key = %cache_key,
                    error = %e,
                    "HEAD failed during board cache revalidation, falling back to full reload"
                );
                BOARD_CACHE.invalidate(&cache_key);
            }
        }
        // The speculative board built from stale storage is unsafe to return;
        // drop it and load fresh below.
        drop(speculative_board);
    }

    let (loaded, meta) =
        Board::load_proto_with_meta(meta_store, storage_root, board_id, request.board_version)
            .await
            .map_err(|e| {
                ExecutorError::BoardLoad(format!("Failed to load board {}: {}", board_id, e))
            })?;
    let board = Board::from_loaded_proto(loaded, storage_root.clone(), state.clone()).await;
    BOARD_CACHE.insert(
        cache_key.clone(),
        Arc::new(CachedBoard {
            board: strip_request_state(board.clone()),
            e_tag: meta.e_tag.clone(),
            last_modified: meta.last_modified,
        }),
    );
    tracing::debug!(cache_key = %cache_key, version_key = %version_key, "Board cache populated");
    Ok(board)
}

/// API-compatible event input format
#[derive(Debug, Clone, Serialize, Deserialize)]
struct ApiEventInput {
    id: Option<String>,
    sequence: Option<i32>,
    event_type: String,
    payload: serde_json::Value,
}

/// API-compatible events push request
#[derive(Debug, Clone, Serialize)]
struct PushEventsRequest {
    events: Vec<ApiEventInput>,
    #[serde(skip_serializing_if = "Option::is_none")]
    job_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    lease_token: Option<String>,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    job_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    lease_token: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    lease_duration_ms: Option<i64>,
}

/// API acknowledgement returned only after the execution state store has
/// accepted the progress update (or confirmed that the run is already
/// terminal). Queue consumers use this as their settlement barrier.
#[derive(Clone, Debug, Deserialize)]
struct ProgressUpdateResponse {
    accepted: bool,
    status: String,
    #[serde(default)]
    lease_acquired: Option<bool>,
    #[serde(default)]
    lease_expires_at: Option<i64>,
}

#[derive(Clone, Debug)]
struct QueueLeaseContext {
    job_id: String,
    token: String,
}

#[derive(Clone, Debug, PartialEq)]
enum StartAcknowledgement {
    Execute,
    Busy { expires_at: i64 },
    AlreadyTerminal(ExecutionStatus),
}

/// Build a callback HTTP client with an explicit User-Agent. The header is
/// load-bearing: the AWS edge runs the WAF Common Rule Set in block mode and
/// its `NoUserAgent_HEADER` rule rejects UA-less requests before they reach
/// the API.
fn callback_client() -> reqwest::Client {
    reqwest::Client::builder()
        .user_agent(concat!("flow-like-executor/", env!("CARGO_PKG_VERSION")))
        .build()
        // Only TLS/resolver initialization can fail here, which is exactly
        // where `reqwest::Client::new()` panics too.
        .expect("failed to build callback HTTP client")
}

/// Execute a flow with batched callback reporting
pub async fn execute(
    request: ExecutionRequest,
    config: ExecutorConfig,
) -> Result<ExecutionResult, ExecutorError> {
    let start = Instant::now();

    // Verify JWT and extract claims
    let claims = verify_jwt_async(&request.executor_jwt).await?;
    if claims.app_id != request.app_id || claims.board_id != request.board_id {
        return Err(ExecutorError::InvalidRequest(
            "executor JWT claims do not match the queued request".to_string(),
        ));
    }

    // Strict queue mode first acquires one conditional Cosmos lease. A live
    // owner serializes deliveries; an expired owner can be taken over; and a
    // terminal run is an idempotent broker redelivery that must not execute.
    let queue_lease = if config.terminal_status_ack_required() {
        if request.job_id.is_empty() {
            return Err(ExecutorError::InvalidRequest(
                "strict queue execution requires a broker job ID".to_string(),
            ));
        }
        let lease = QueueLeaseContext {
            job_id: request.job_id.clone(),
            token: create_id(),
        };
        let progress_url = format!(
            "{}/api/v1/execution/progress",
            claims.callback_url.trim_end_matches('/')
        );
        let client = callback_client();
        loop {
            let start_update = lease_progress_update(&lease, config.strict_lease_duration_ms());
            let acknowledgement = send_progress(
                &progress_url,
                &request.executor_jwt,
                &start_update,
                &config,
                &client,
            )
            .await?;

            match interpret_start_acknowledgement(&acknowledgement)? {
                StartAcknowledgement::Execute => break,
                StartAcknowledgement::Busy { expires_at } => {
                    let now = chrono::Utc::now().timestamp_millis();
                    let wait_ms = expires_at.saturating_sub(now).clamp(250, 30_000) as u64;
                    tracing::info!(
                        run_id = %claims.run_id,
                        wait_ms,
                        "another delivery owns the execution lease; waiting to retry claim"
                    );
                    tokio::time::sleep(Duration::from_millis(wait_ms)).await;
                }
                StartAcknowledgement::AlreadyTerminal(status) => {
                    tracing::info!(
                        run_id = %claims.run_id,
                        acknowledged_status = %acknowledgement.status,
                        "execution delivery was already terminal; skipping duplicate workflow execution"
                    );
                    return Ok(ExecutionResult {
                        run_id: claims.run_id,
                        status,
                        output: None,
                        error: None,
                        duration_ms: start.elapsed().as_millis() as u64,
                    });
                }
            }
        }
        Some(lease)
    } else {
        None
    };

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
    let (callback_failure_tx, mut callback_failure_rx) = watch::channel::<Option<String>>(None);
    let callback_claims = claims.clone();
    let callback_jwt = executor_jwt.clone();
    let callback_config = config.clone();
    let callback_lease = queue_lease.clone();
    let callback_handle = tokio::spawn(async move {
        let result = run_callback_batcher(
            event_rx,
            callback_claims,
            callback_jwt,
            callback_config,
            callback_lease,
        )
        .await;
        if let Err(error) = &result {
            let _ = callback_failure_tx.send(Some(error.to_string()));
        }
        result
    });

    // Build FlowLike state
    let mut flow_config = FlowLikeConfig::with_default_store(content_store);
    flow_config.register_app_meta_store(meta_store.clone());
    flow_config.register_log_store(log_store);

    // Request-file offloads and `/tmp` uploads live under tmp/*, which the
    // app-scoped content credential does not necessarily cover.
    match request.credentials.to_store_type(StoreType::Tmp).await {
        Ok(tmp_store) => flow_config.register_temporary_store(tmp_store),
        Err(e) => {
            tracing::error!(error = %e, "Failed to create scratch store - tmp/* paths will be unavailable");
        }
    }

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
    let execution_environment = ExecutionEnvironment::server_default();
    let mut state =
        FlowLikeState::new_with_model_config(flow_config, http_client, model_provider_config);
    state.execution_environment = execution_environment;

    let mut registry = PREPARED_REGISTRY.clone();
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
                        "Loaded WASM nodes for execution"
                    );
                    failed_wasm_package_ids = report.failed_package_ids;
                    for logic in report.nodes {
                        let node = logic.get_node();
                        registry.insert(node, logic);
                    }
                }
                Err(e) => {
                    return Err(e);
                }
            }
        }
    }

    state.node_registry.write().await.node_registry = Arc::new(registry);

    let state = Arc::new(state);

    let board_id = &request.board_id;
    let storage_root = Path::from("apps").child(request.app_id.to_string());
    let board = Arc::new(resolve_board(&state, &request, &storage_root).await?);
    let unavailable_wasm_packages = crate::wasm_loader::unavailable_board_wasm_packages(
        &board,
        request.wasm_packages.as_ref(),
        &failed_wasm_package_ids,
    );
    if !unavailable_wasm_packages.is_empty() {
        return Err(ExecutorError::Execution(format!(
            "Missing WASM package artifacts for board {}: {}",
            board_id,
            unavailable_wasm_packages.join(", ")
        )));
    }

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
                        id: execution_event_id(&run_id, seq_num),
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

    run.set_execution_environment(execution_environment);
    if let Some(mode) = request.execution_mode {
        run.set_execution_mode(mode);
    }

    run.set_execution_sub(claims.sub.clone()).await;

    // Set user context if provided
    if let Some(user_context) = request.user_context.clone() {
        run.set_user_context(user_context);
    }

    // Execute with timeout while continuously renewing the independent Cosmos
    // ownership lease. Losing that lease cancels the run before another
    // delivery is allowed to take over.
    let mut execution_future = Box::pin(tokio::time::timeout(config.execution_timeout(), async {
        run.execute(state.clone()).await
    }));
    let mut lease_failure = None;
    let execution_result = if let Some(lease) = queue_lease.as_ref() {
        let mut renewal = Box::pin(maintain_queue_lease(
            &claims.callback_url,
            &executor_jwt,
            lease,
            &config,
        ));
        tokio::select! {
            result = &mut execution_future => Some(result),
            error = &mut renewal => {
                lease_failure = Some(error);
                None
            }
            changed = callback_failure_rx.changed() => {
                let detail = match changed {
                    Ok(()) => callback_failure_rx
                        .borrow()
                        .clone()
                        .unwrap_or_else(|| "strict callback task stopped unexpectedly".to_string()),
                    Err(_) => "strict callback task stopped unexpectedly".to_string(),
                };
                lease_failure = Some(ExecutorError::Callback(detail));
                None
            }
        }
    } else {
        Some(execution_future.as_mut().await)
    };

    if let Some(error) = lease_failure {
        drop(execution_future);
        callback_handle.abort();
        drop(run);
        drop(intercom_handler);
        drop(event_tx);
        return Err(error);
    }
    let execution_result = execution_result.expect("execution result exists without lease failure");
    drop(execution_future);

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
                    tracing::warn!(
                        "No log database builder configured in state - run metadata will not be persisted"
                    );
                }
            } else {
                tracing::warn!(
                    "No log metadata returned from execution - logs may not have been flushed"
                );
            }

            let status = ExecutionStatus::from_final_run_status(&run.get_status().await);
            let (event_type, message, error) = match &status {
                ExecutionStatus::Completed => (EventType::Log, "Execution completed", None),
                ExecutionStatus::Cancelled => (
                    EventType::Error,
                    "Execution cancelled",
                    Some("Execution cancelled".to_string()),
                ),
                ExecutionStatus::Failed | ExecutionStatus::Running => (
                    EventType::Error,
                    "Execution failed",
                    Some("Execution failed".to_string()),
                ),
            };
            send_event(
                &event_tx,
                &sequence,
                &claims.run_id,
                event_type,
                serde_json::json!({ "message": message }),
            );
            (status, None, error)
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

    match callback_handle.await {
        Ok(result) if queue_lease.is_some() => result?,
        Ok(_) => {}
        Err(error) if queue_lease.is_some() => {
            return Err(ExecutorError::Callback(format!(
                "callback task failed: {error}"
            )));
        }
        Err(error) => {
            tracing::warn!(error = %error, "Callback task stopped unexpectedly");
        }
    }

    // The last event upload may consume most of the previous lease window.
    // Renew once immediately before the terminal conditional write so a slow
    // callback cannot turn a successful workflow into an expired-owner race.
    if let Some(lease) = queue_lease.as_ref() {
        let renewal = send_progress(
            &format!(
                "{}/api/v1/execution/progress",
                claims.callback_url.trim_end_matches('/')
            ),
            &executor_jwt,
            &lease_progress_update(lease, config.strict_lease_duration_ms()),
            &config,
            &callback_client(),
        )
        .await?;
        if !matches!(
            interpret_start_acknowledgement(&renewal)?,
            StartAcknowledgement::Execute
        ) {
            return Err(ExecutorError::Callback(
                "execution lease was not renewed before terminal acknowledgement".to_string(),
            ));
        }
    }

    // Send final progress update
    let progress_update = ProgressUpdateRequest {
        progress: Some(100),
        current_step: None,
        status: Some(format!("{:?}", status).to_lowercase()),
        output_len: None,
        error: error.clone(),
        job_id: queue_lease.as_ref().map(|lease| lease.job_id.clone()),
        lease_token: queue_lease.as_ref().map(|lease| lease.token.clone()),
        lease_duration_ms: None,
    };

    let progress_url = format!(
        "{}/api/v1/execution/progress",
        claims.callback_url.trim_end_matches('/')
    );
    let http_client = callback_client();
    let progress_result = send_progress(
        &progress_url,
        &executor_jwt,
        &progress_update,
        &config,
        &http_client,
    )
    .await;

    if config.terminal_status_ack_required() {
        let acknowledgement = progress_result?;
        ensure_terminal_acknowledgement(&acknowledgement, &status)?;
        if !acknowledgement.accepted {
            tracing::info!(
                run_id = %claims.run_id,
                acknowledged_status = %acknowledgement.status,
                "terminal status was already persisted by an earlier delivery"
            );
        }
    } else if let Err(error) = progress_result {
        tracing::warn!(error = %error, "Failed to send final progress update");
    }

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
        id: execution_event_id(run_id, seq),
        run_id: run_id.to_string(),
        sequence: seq,
        event_type,
        payload,
        created_at: chrono::Utc::now(),
    };
    let _ = tx.send(event);
}

fn execution_event_id(run_id: &str, sequence: i32) -> String {
    let digest = blake3::hash(format!("{run_id}:{sequence}").as_bytes());
    format!("evt-{}", digest.to_hex())
}

async fn run_callback_batcher(
    mut event_rx: mpsc::UnboundedReceiver<ExecutionEvent>,
    claims: ExecutorClaims,
    executor_jwt: String,
    config: ExecutorConfig,
    queue_lease: Option<QueueLeaseContext>,
) -> Result<(), ExecutorError> {
    let events_url = format!(
        "{}/api/v1/execution/events",
        claims.callback_url.trim_end_matches('/')
    );
    let client = callback_client();
    let mut batch = Vec::new();
    // Multiple producers can reserve an AtomicI32 value and reach the channel
    // in the opposite order. Strict mode assigns the durable sequence at the
    // single consumer instead, guaranteeing contiguous, ordered callbacks.
    let mut strict_next_sequence = 0_i32;
    let send_threshold = if queue_lease.is_some() {
        config.max_batch_size.clamp(1, 1_000)
    } else {
        config.max_batch_size.max(1)
    };
    let mut interval = tokio::time::interval(config.batch_interval());

    loop {
        tokio::select! {
            _ = interval.tick() => {
                if !batch.is_empty() {
                    if let Err(error) = send_events_to_api(
                        &events_url,
                        &executor_jwt,
                        &batch,
                        &config,
                        &client,
                        queue_lease.as_ref(),
                    ).await {
                        if queue_lease.is_some() {
                            return Err(error);
                        }
                        tracing::warn!(error = %error, "Failed to send events batch");
                    }
                    batch.clear();
                }
            }
            event = event_rx.recv() => {
                match event {
                    Some(mut e) => {
                        if queue_lease.is_some() {
                            e.sequence = strict_next_sequence;
                            e.id = execution_event_id(&e.run_id, strict_next_sequence);
                            strict_next_sequence = strict_next_sequence.checked_add(1).ok_or_else(|| {
                                ExecutorError::Callback(
                                    "execution event sequence exceeded i32 capacity".to_string(),
                                )
                            })?;
                        }
                        batch.push(e);
                        if batch.len() >= send_threshold {
                            if let Err(error) = send_events_to_api(
                                &events_url,
                                &executor_jwt,
                                &batch,
                                &config,
                                &client,
                                queue_lease.as_ref(),
                            ).await {
                                if queue_lease.is_some() {
                                    return Err(error);
                                }
                                tracing::warn!(error = %error, "Failed to send events batch");
                            }
                            batch.clear();
                        }
                    }
                    None => {
                        if !batch.is_empty() {
                            if let Err(error) = send_events_to_api(
                                &events_url,
                                &executor_jwt,
                                &batch,
                                &config,
                                &client,
                                queue_lease.as_ref(),
                            ).await {
                                if queue_lease.is_some() {
                                    return Err(error);
                                }
                                tracing::warn!(error = %error, "Failed to send final events batch");
                            }
                        }
                        return Ok(());
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
    events: &[ExecutionEvent],
    config: &ExecutorConfig,
    client: &reqwest::Client,
    queue_lease: Option<&QueueLeaseContext>,
) -> Result<(), ExecutorError> {
    let api_events: Vec<ApiEventInput> = events
        .iter()
        .map(|e| ApiEventInput {
            id: queue_lease.map(|_| e.id.clone()),
            sequence: queue_lease.map(|_| e.sequence),
            event_type: event_type_to_string(&e.event_type),
            payload: e.payload.clone(),
        })
        .collect();

    let request = PushEventsRequest {
        events: api_events,
        job_id: queue_lease.map(|lease| lease.job_id.clone()),
        lease_token: queue_lease.map(|lease| lease.token.clone()),
    };

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
) -> Result<ProgressUpdateResponse, ExecutorError> {
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
            Ok(response) if response.status().is_success() => {
                match response.json::<ProgressUpdateResponse>().await {
                    Ok(acknowledgement) => return Ok(acknowledgement),
                    Err(error) => {
                        tracing::warn!(attempt, error = %error, "Progress callback returned an invalid acknowledgement");
                    }
                }
            }
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

fn parse_acknowledged_status(status: &str) -> Option<ExecutionStatus> {
    if status.eq_ignore_ascii_case("running") || status.eq_ignore_ascii_case("pending") {
        Some(ExecutionStatus::Running)
    } else if status.eq_ignore_ascii_case("completed") {
        Some(ExecutionStatus::Completed)
    } else if status.eq_ignore_ascii_case("failed") || status.eq_ignore_ascii_case("timeout") {
        Some(ExecutionStatus::Failed)
    } else if status.eq_ignore_ascii_case("cancelled") {
        Some(ExecutionStatus::Cancelled)
    } else {
        None
    }
}

fn lease_progress_update(
    lease: &QueueLeaseContext,
    lease_duration_ms: i64,
) -> ProgressUpdateRequest {
    ProgressUpdateRequest {
        progress: None,
        current_step: None,
        status: Some("running".to_string()),
        output_len: None,
        error: None,
        job_id: Some(lease.job_id.clone()),
        lease_token: Some(lease.token.clone()),
        lease_duration_ms: Some(lease_duration_ms),
    }
}

/// Best-effort terminal `Failed` for a queued run whose broker message is
/// being dead-lettered without a successful terminal callback.
///
/// The strict-lease contract only accepts a terminal update from the current
/// delivery owner, so this claims the lease with a fresh token first and then
/// persists `Failed` through the lease-protected path. A run that is already
/// terminal is treated as settled. When another delivery still holds a live
/// lease the run is left to its owner and an error is returned; callers must
/// treat any error as non-fatal.
pub async fn report_queue_failure(
    executor_jwt: &str,
    job_id: &str,
    error: &str,
    config: &ExecutorConfig,
) -> Result<(), ExecutorError> {
    let claims = verify_jwt_async(executor_jwt).await?;
    let progress_url = format!(
        "{}/api/v1/execution/progress",
        claims.callback_url.trim_end_matches('/')
    );
    let client = callback_client();
    let lease = QueueLeaseContext {
        job_id: job_id.to_string(),
        token: create_id(),
    };

    let acknowledgement = send_progress(
        &progress_url,
        executor_jwt,
        &lease_progress_update(&lease, config.strict_lease_duration_ms()),
        config,
        &client,
    )
    .await?;
    match interpret_start_acknowledgement(&acknowledgement)? {
        StartAcknowledgement::Execute => {}
        StartAcknowledgement::AlreadyTerminal(_) => return Ok(()),
        StartAcknowledgement::Busy { .. } => {
            return Err(ExecutorError::Callback(
                "another delivery holds the execution lease; leaving the run to its owner"
                    .to_string(),
            ));
        }
    }

    let update = ProgressUpdateRequest {
        progress: None,
        current_step: None,
        status: Some("failed".to_string()),
        output_len: None,
        error: Some(error.to_string()),
        job_id: Some(lease.job_id.clone()),
        lease_token: Some(lease.token.clone()),
        lease_duration_ms: None,
    };
    let acknowledgement =
        send_progress(&progress_url, executor_jwt, &update, config, &client).await?;
    ensure_terminal_acknowledgement(&acknowledgement, &ExecutionStatus::Failed)
}

async fn maintain_queue_lease(
    callback_url: &str,
    executor_jwt: &str,
    lease: &QueueLeaseContext,
    config: &ExecutorConfig,
) -> ExecutorError {
    let progress_url = format!(
        "{}/api/v1/execution/progress",
        callback_url.trim_end_matches('/')
    );
    let client = callback_client();
    loop {
        tokio::time::sleep(config.strict_lease_renewal()).await;
        let update = lease_progress_update(lease, config.strict_lease_duration_ms());
        let acknowledgement =
            match send_progress(&progress_url, executor_jwt, &update, config, &client).await {
                Ok(acknowledgement) => acknowledgement,
                Err(error) => return error,
            };
        match interpret_start_acknowledgement(&acknowledgement) {
            Ok(StartAcknowledgement::Execute) => {
                tracing::debug!("renewed strict queue execution lease");
            }
            Ok(StartAcknowledgement::Busy { .. }) => {
                return ExecutorError::Callback(
                    "execution lease ownership was lost during renewal".to_string(),
                );
            }
            Ok(StartAcknowledgement::AlreadyTerminal(_)) => {
                return ExecutorError::Callback(
                    "execution became terminal before the owning delivery completed".to_string(),
                );
            }
            Err(error) => return error,
        }
    }
}

fn interpret_start_acknowledgement(
    acknowledgement: &ProgressUpdateResponse,
) -> Result<StartAcknowledgement, ExecutorError> {
    match parse_acknowledged_status(&acknowledgement.status) {
        Some(ExecutionStatus::Running)
            if acknowledgement.accepted && acknowledgement.lease_acquired == Some(true) =>
        {
            Ok(StartAcknowledgement::Execute)
        }
        Some(ExecutionStatus::Running)
            if !acknowledgement.accepted
                && acknowledgement.lease_acquired == Some(false)
                && acknowledgement.lease_expires_at.is_some() =>
        {
            Ok(StartAcknowledgement::Busy {
                expires_at: acknowledgement.lease_expires_at.unwrap(),
            })
        }
        Some(
            status @ (ExecutionStatus::Completed
            | ExecutionStatus::Failed
            | ExecutionStatus::Cancelled),
        ) => Ok(StartAcknowledgement::AlreadyTerminal(status)),
        _ => Err(ExecutorError::Callback(
            "API did not acknowledge a runnable or terminal execution state".to_string(),
        )),
    }
}

fn ensure_terminal_acknowledgement(
    acknowledgement: &ProgressUpdateResponse,
    expected: &ExecutionStatus,
) -> Result<(), ExecutorError> {
    let acknowledged = parse_acknowledged_status(&acknowledgement.status).ok_or_else(|| {
        ExecutorError::Callback("API returned an unknown execution status".to_string())
    })?;

    if matches!(acknowledged, ExecutionStatus::Running) {
        return Err(ExecutorError::Callback(
            "API did not persist a terminal execution status".to_string(),
        ));
    }

    // If this request performed the update, the persisted status must be the
    // status it submitted. `accepted = false` means an earlier retry already
    // committed a terminal status; any terminal value is then sufficient to
    // make broker settlement safe without replaying side effects.
    if acknowledgement.accepted && &acknowledged != expected {
        return Err(ExecutorError::Callback(
            "API acknowledged a different terminal execution status".to_string(),
        ));
    }

    Ok(())
}

#[cfg(test)]
mod callback_acknowledgement_tests {
    use super::*;

    fn acknowledgement(accepted: bool, status: &str) -> ProgressUpdateResponse {
        ProgressUpdateResponse {
            accepted,
            status: status.to_string(),
            lease_acquired: Some(accepted),
            lease_expires_at: None,
        }
    }

    #[test]
    fn start_acknowledgement_executes_only_for_an_accepted_running_state() {
        assert_eq!(
            interpret_start_acknowledgement(&acknowledgement(true, "Running")).unwrap(),
            StartAcknowledgement::Execute
        );
        assert!(interpret_start_acknowledgement(&acknowledgement(false, "Running")).is_err());
    }

    #[test]
    fn start_acknowledgement_waits_for_a_live_competing_lease() {
        let mut ack = acknowledgement(false, "Running");
        ack.lease_expires_at = Some(42_000);
        assert_eq!(
            interpret_start_acknowledgement(&ack).unwrap(),
            StartAcknowledgement::Busy { expires_at: 42_000 }
        );
    }

    #[test]
    fn start_acknowledgement_skips_a_previously_terminal_delivery() {
        assert_eq!(
            interpret_start_acknowledgement(&acknowledgement(false, "Completed")).unwrap(),
            StartAcknowledgement::AlreadyTerminal(ExecutionStatus::Completed)
        );
        assert_eq!(
            interpret_start_acknowledgement(&acknowledgement(false, "Timeout")).unwrap(),
            StartAcknowledgement::AlreadyTerminal(ExecutionStatus::Failed)
        );
    }

    #[test]
    fn terminal_acknowledgement_requires_persisted_terminal_state() {
        assert!(
            ensure_terminal_acknowledgement(
                &acknowledgement(true, "Completed"),
                &ExecutionStatus::Completed,
            )
            .is_ok()
        );
        assert!(
            ensure_terminal_acknowledgement(
                &acknowledgement(true, "Running"),
                &ExecutionStatus::Completed,
            )
            .is_err()
        );
        assert!(
            ensure_terminal_acknowledgement(
                &acknowledgement(true, "Failed"),
                &ExecutionStatus::Completed,
            )
            .is_err()
        );
        assert!(
            ensure_terminal_acknowledgement(
                &acknowledgement(false, "Failed"),
                &ExecutionStatus::Completed,
            )
            .is_ok()
        );
    }
}
