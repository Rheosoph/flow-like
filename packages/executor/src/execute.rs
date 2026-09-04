//! Core execution logic
//!
//! Environment-agnostic flow execution with batched callback reporting

use crate::config::ExecutorConfig;
use crate::error::ExecutorError;
use crate::jwt::{verify_jwt_async, ExecutorClaims, ExecutorPageExecutionClaims};
use crate::resolve::{fetch_bounded, max_remote_payload_bytes};
use crate::types::{EventType, ExecutionEvent, ExecutionRequest, ExecutionResult, ExecutionStatus};
use crate::widgets::{HubAccess, HubWidgetSource};
use flow_like::credentials::StoreType;
use flow_like::flow::compiled::{template_from_bytes, CompiledRunTemplate, TemplateCache};
use flow_like::flow::event::Event;
use flow_like::flow::execution::rejection::{RejectedRun, RejectionStage};
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
pub(crate) static PREPARED_REGISTRY: LazyLock<Arc<FlowNodeRegistryInner>> = LazyLock::new(|| {
    let catalog = get_catalog();
    let catalog_arc = Arc::new(catalog);
    Arc::new(FlowNodeRegistryInner::prepare(&catalog_arc))
});

/// The registry for one request: the shared prepared catalog when the request
/// brings no WASM overlay, a copy-on-write extension otherwise. The full deep
/// clone (every catalog Node) is only paid on the WASM path.
pub(crate) fn request_registry(
    wasm_nodes: Vec<Arc<dyn flow_like::flow::node::NodeLogic>>,
) -> Arc<FlowNodeRegistryInner> {
    if wasm_nodes.is_empty() {
        return PREPARED_REGISTRY.clone();
    }
    let mut registry = PREPARED_REGISTRY.as_ref().clone();
    for logic in wasm_nodes {
        let node = logic.get_node();
        registry.insert(node, logic);
    }
    Arc::new(registry)
}

/// Cache of compiled run templates, keyed by
/// `(app_id, board_id, version, registry fingerprint, wasm bundle)`.
///
/// A cached template has already gone through artifact decode (or full
/// compile: fetch, decompress, decode, `node_updates`, `compile_board`) plus
/// registry resolution and default parsing. Runs built from it only allocate
/// per-run state — no per-request Board clone, no graph rebuild.
static TEMPLATE_CACHE: LazyLock<TemplateCache> = LazyLock::new(TemplateCache::default);

fn wasm_registry_signature(request: &ExecutionRequest) -> String {
    flow_like_types::dispatch::wasm_package_set_revision(request.wasm_packages.as_ref())
}

/// Resolve the request's board into a shared run template from the compiled
/// artifact the API presigned into the dispatch payload.
///
/// The API compiled and persisted the `.flcb` before dispatch and handed us a
/// GET for exactly that object, so this runtime never reads a board proto,
/// never compiles and never writes anything back — it has no storage
/// credential that could. The cache is keyed on the board's content identity
/// (pinned version or source ETag), the registry fingerprint and the WASM
/// bundle signature, so a hit needs no revalidation. Anything short of a
/// decodable artifact for our own registry fails the run at Resolution.
pub(crate) async fn resolve_run_template(
    state: &Arc<FlowLikeState>,
    request: &ExecutionRequest,
) -> Result<Arc<CompiledRunTemplate>, ExecutorError> {
    let registry = state.node_registry.read().await.node_registry.clone();
    let fingerprint = registry.fingerprint();
    let version_key = artifact_version_key(request)?;
    let cache_key = TemplateCache::cache_key(
        &request.app_id,
        &request.board_id,
        &version_key,
        &fingerprint,
        &wasm_registry_signature(request),
    );
    if let Some(template) = TEMPLATE_CACHE.get(&cache_key) {
        tracing::debug!(cache_key = %cache_key, "Template cache hit");
        return Ok(template);
    }

    // The URL is a bearer capability: fetch errors name the object path only.
    let bytes = fetch_bounded(&request.artifact.url, max_remote_payload_bytes())
        .await
        .map_err(|e| {
            ExecutorError::BoardLoad(format!(
                "failed to fetch compiled artifact {} for board {}: {e}",
                request.artifact.path, request.board_id
            ))
        })?;
    let template = template_from_fetched(&bytes, &fingerprint, registry.as_ref(), request)?;
    TEMPLATE_CACHE.insert(cache_key, template.clone());
    Ok(template)
}

/// The content identity half of the template cache key. Pinned versions are
/// immutable; a floating board is pinned to the source ETag the API compiled
/// from — the Page-claim ETag when the run carries one, otherwise the one the
/// API resolved for an ordinary run and sent with the artifact reference.
fn artifact_version_key(request: &ExecutionRequest) -> Result<String, ExecutorError> {
    match (
        request.board_version,
        request.board_etag.as_deref(),
        request.artifact.source_etag.as_deref(),
    ) {
        (Some((m, n, p)), _, _) => Ok(format!("{m}_{n}_{p}")),
        (None, Some(etag), _) | (None, None, Some(etag)) => Ok(format!("latest@{etag}")),
        (None, None, None) => Err(ExecutorError::BoardLoad(format!(
            "floating Latest run of board {} carries no source etag in its compiled artifact reference",
            request.board_id
        ))),
    }
}

/// Decode fetched artifact bytes into a template for this request. Split from
/// the fetch so it is testable with in-memory bytes.
pub(crate) fn template_from_fetched(
    bytes: &[u8],
    fingerprint: &[u8; 32],
    registry: &FlowNodeRegistryInner,
    request: &ExecutionRequest,
) -> Result<Arc<CompiledRunTemplate>, ExecutorError> {
    let storage_root = Path::from("apps").child(request.app_id.clone());
    let template =
        template_from_bytes(bytes, fingerprint, registry, &storage_root).map_err(|e| {
            let ours = blake3::Hash::from_bytes(*fingerprint).to_hex();
            ExecutorError::BoardLoad(format!(
            "compiled artifact {} rejected: {e} (API compiled against {}, this executor runs {})",
            request.artifact.path,
            request
                .artifact
                .registry_fingerprint
                .get(..16)
                .unwrap_or(&request.artifact.registry_fingerprint),
            &ours.as_str()[..16]
        ))
        })?;
    if template.board.id != request.board_id {
        return Err(ExecutorError::BoardLoad(format!(
            "compiled artifact {} is for board {}, expected {}",
            request.artifact.path, template.board.id, request.board_id
        )));
    }
    Ok(template)
}

fn validate_page_request_binding(
    page: &ExecutorPageExecutionClaims,
    board_version: Option<(u32, u32, u32)>,
    board_etag: Option<&str>,
    node_id: &str,
    wasm_packages: Option<
        &std::collections::HashMap<String, flow_like_types::dispatch::WasmPackageRef>,
    >,
) -> Result<(), ExecutorError> {
    let page_etag = page
        .board_etag
        .as_deref()
        .map(str::trim)
        .filter(|etag| !etag.is_empty());
    match (page.board_version, page_etag) {
        (Some(version), None) if board_version == Some(version) && board_etag.is_none() => {}
        (None, Some(etag)) if board_version.is_none() && board_etag == Some(etag) => {}
        _ => {
            return Err(ExecutorError::InvalidRequest(
                "executor JWT Page selector does not match the queued request".to_string(),
            ));
        }
    }

    if let Some(target_node_id) = page.target_node_id.as_deref() {
        if target_node_id != node_id {
            return Err(ExecutorError::InvalidRequest(
                "queued Page node does not match the executor JWT target".to_string(),
            ));
        }
    } else if page.allowed_entry_node_ids.is_empty() {
        // Pinned Page JWTs issued before the allow-list field existed remain
        // compatible. Latest Page execution was not supported by those
        // issuers, so an empty Latest list is always invalid.
        if page.board_version.is_none() {
            return Err(ExecutorError::InvalidRequest(
                "Latest Page executor JWT is missing its entry-node authority".to_string(),
            ));
        }
    } else if !page
        .allowed_entry_node_ids
        .iter()
        .any(|allowed| allowed == node_id)
    {
        return Err(ExecutorError::InvalidRequest(
            "queued Page node is outside the executor JWT entry-node authority".to_string(),
        ));
    }

    if let Some(expected) = page.wasm_authority_revision.as_deref() {
        if expected != flow_like_types::dispatch::wasm_package_set_revision(wasm_packages) {
            return Err(ExecutorError::InvalidRequest(
                "queued Page WASM bundle does not match the executor JWT authority".to_string(),
            ));
        }
    }
    Ok(())
}

pub(crate) fn validate_executor_request_claims(
    claims: &ExecutorClaims,
    request: &ExecutionRequest,
) -> Result<(), ExecutorError> {
    if claims.app_id != request.app_id || claims.board_id != request.board_id {
        return Err(ExecutorError::InvalidRequest(
            "executor JWT claims do not match the queued request".to_string(),
        ));
    }

    // The in-process shadow isolation must be driven by the signed claim, not
    // an unsigned payload byte. Old JWTs without the claim mean a normal run.
    if request.shadow != claims.shadow.unwrap_or(false) {
        return Err(ExecutorError::InvalidRequest(
            "queued shadow flag does not match the executor JWT claims".to_string(),
        ));
    }

    match &claims.page_execution {
        Some(page) => {
            if claims
                .event_id
                .as_deref()
                .is_none_or(|event_id| event_id.trim().is_empty())
            {
                return Err(ExecutorError::InvalidRequest(
                    "Page executor JWT does not identify its source Event".to_string(),
                ));
            }
            validate_page_request_binding(
                page,
                request.board_version,
                request.board_etag.as_deref(),
                &request.node_id,
                request.wasm_packages.as_ref(),
            )
        }
        None if request.board_etag.is_some() => Err(ExecutorError::InvalidRequest(
            "ETag-bound Latest execution requires signed Page context".to_string(),
        )),
        None => Ok(()),
    }
}

/// Build the `FlowLikeState` every execution path shares: stores from the
/// request credentials, the logs database builder, the server execution
/// environment and — when the run has a hub — the widget source that keeps
/// `Instantiate Widget` off the meta store. WASM registry overlays are applied
/// by the caller.
///
/// There is deliberately no meta store. Boards arrive as presigned compiled
/// artifacts and widgets come from the hub, so the run credential carries no
/// meta grant — and `with_default_store` would otherwise alias the meta slot
/// to the content bucket, letting any stray meta read land on the wrong data.
/// Leaving it `None` makes such a read fail loudly instead.
pub(crate) async fn build_flow_state(
    credentials: &flow_like::credentials::SharedCredentials,
    hub: Option<HubAccess>,
) -> Result<FlowLikeState, ExecutorError> {
    let content_store = credentials
        .to_store_type(StoreType::Content)
        .await
        .map_err(|e| ExecutorError::Storage(e.to_string()))?;

    let log_store = credentials
        .to_store_type(StoreType::Logs)
        .await
        .map_err(|e| ExecutorError::Storage(e.to_string()))?;

    let mut flow_config = FlowLikeConfig::with_default_store(content_store);
    flow_config.stores.app_meta_store = None;
    flow_config.register_log_store(log_store);

    // Request-file offloads and `/tmp` uploads live under tmp/*, which the
    // app-scoped content credential does not necessarily cover.
    match credentials.to_store_type(StoreType::Tmp).await {
        Ok(tmp_store) => flow_config.register_temporary_store(tmp_store),
        Err(e) => {
            tracing::error!(error = %e, "Failed to create scratch store - tmp/* paths will be unavailable");
        }
    }

    // Register logs database builder for LanceDB log storage
    match credentials.to_logs_db_builder() {
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
    let mut state =
        FlowLikeState::new_with_model_config(flow_config, http_client, model_provider_config);
    state.execution_environment = ExecutionEnvironment::server_default();
    if let Some(hub) = hub {
        state
            .register_app_widget_source(Arc::new(HubWidgetSource::new(&hub.callback_url, hub.jwt)))
            .await;
    }
    Ok(state)
}

/// A claims-validation failure happens before any state exists, so build one
/// from the request credentials just to persist the rejection. When the state
/// itself cannot be built the rejection stays unrecorded and the caller
/// returns the original validation error untouched.
pub(crate) async fn record_claims_rejection(
    request: &ExecutionRequest,
    run_id: &str,
    error: &ExecutorError,
) {
    match build_flow_state(&request.credentials, None).await {
        Ok(state) => {
            record_executor_rejection(
                &Arc::new(state),
                request,
                run_id,
                RejectionStage::Permission,
                error.to_string(),
            )
            .await;
        }
        Err(state_error) => {
            tracing::warn!(
                error = %state_error,
                run_id = %run_id,
                app_id = %request.app_id,
                "Failed to build state to record a rejected execution request"
            );
        }
    }
}

/// A run the API already created but the executor never started leaves the run
/// row carrying a reason and no logs at all, so opening it in the UI explains
/// nothing. Write the same per-run log table a real run would have produced.
pub(crate) async fn record_executor_rejection(
    state: &Arc<FlowLikeState>,
    request: &ExecutionRequest,
    run_id: &str,
    stage: RejectionStage,
    reason: String,
) {
    let mut rejection = RejectedRun::new(
        request.app_id.clone(),
        request.board_id.clone(),
        stage,
        reason,
    )
    .with_run_id(run_id)
    .with_node(request.node_id.clone())
    .with_board_version(request.board_version)
    .with_payload(request.payload.as_ref());

    if let Some(event) = request
        .event_json
        .as_ref()
        .and_then(|json| serde_json::from_str::<Event>(json).ok())
    {
        rejection = rejection.with_event_definition(&event);
    }

    if let Err(error) = state.record_rejected_run(&rejection).await {
        tracing::warn!(
            error = %error,
            run_id = %run_id,
            app_id = %request.app_id,
            "Failed to record a run that never started"
        );
    }
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
    if let Err(error) = validate_executor_request_claims(&claims, &request) {
        record_claims_rejection(&request, &claims.run_id, &error).await;
        return Err(error);
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

    // Build FlowLike state from the request credentials
    let state = build_flow_state(
        &request.credentials,
        Some(HubAccess {
            callback_url: claims.callback_url.clone(),
            jwt: request.executor_jwt.clone(),
        }),
    )
    .await?;
    let execution_environment = state.execution_environment;

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
                        "Loaded WASM nodes for execution"
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

    state.node_registry.write().await.node_registry = request_registry(wasm_nodes);

    let state = Arc::new(state);

    let board_id = &request.board_id;
    // Template build resolves every node against the registry, so a board
    // whose WASM packages failed to download errors here first — keep the
    // actionable package list in that error.
    let template = match resolve_run_template(&state, &request)
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
            record_executor_rejection(
                &state,
                &request,
                &claims.run_id,
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
        record_executor_rejection(
            &state,
            &request,
            &claims.run_id,
            RejectionStage::Setup,
            error.to_string(),
        )
        .await;
        return Err(error);
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

    let channel = match crate::channel::build_run_channel(
        request.channel.as_ref(),
        &claims.run_id,
        &claims.callback_url,
        context_token.as_deref(),
    )
    .await
    {
        Ok(channel) => channel,
        Err(error) => {
            let error = ExecutorError::RunInit(error.to_string());
            record_executor_rejection(
                &state,
                &request,
                &claims.run_id,
                RejectionStage::Setup,
                error.to_string(),
            )
            .await;
            return Err(error);
        }
    };

    let run = InternalRun::from_template(
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
        Some(claims.run_id.clone()),
        Some(channel.clone()),
    )
    .await
    .map_err(|e| ExecutorError::RunInit(e.to_string()));

    let mut run = match run {
        Ok(run) => run,
        Err(error) => {
            record_executor_rejection(
                &state,
                &request,
                &claims.run_id,
                RejectionStage::Setup,
                error.to_string(),
            )
            .await;
            return Err(error);
        }
    };

    run.set_execution_environment(execution_environment);
    if let Some(mode) = request.execution_mode {
        run.set_execution_mode(mode);
    }

    run.set_shadow(request.shadow).await;
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

    channel.close().await;

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

fn assign_callback_identity(event: &mut ExecutionEvent, sequence: i32) {
    event.sequence = sequence;
    event.id = execution_event_id(&event.run_id, sequence);
}

fn api_event_input(event: &ExecutionEvent) -> ApiEventInput {
    ApiEventInput {
        id: Some(event.id.clone()),
        sequence: Some(event.sequence),
        event_type: event_type_to_string(&event.event_type),
        payload: event.payload.clone(),
    }
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
    // in the opposite order. Assign the durable identity at this single
    // consumer for every runtime, guaranteeing contiguous ordered callbacks
    // and avoiding the API's legacy read/max/write allocation path.
    let mut callback_next_sequence = 0_i32;
    let send_threshold = config.max_batch_size.clamp(1, 1_000);
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
                        assign_callback_identity(&mut e, callback_next_sequence);
                        callback_next_sequence = callback_next_sequence.checked_add(1).ok_or_else(|| {
                            ExecutorError::Callback(
                                "execution event sequence exceeded i32 capacity".to_string(),
                            )
                        })?;
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
    let api_events: Vec<ApiEventInput> = events.iter().map(api_event_input).collect();

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
mod callback_event_identity_tests {
    use super::*;

    fn event(run_id: &str, provisional_sequence: i32) -> ExecutionEvent {
        ExecutionEvent {
            id: "provisional".into(),
            run_id: run_id.into(),
            sequence: provisional_sequence,
            event_type: EventType::Chunk,
            payload: serde_json::json!({"sequence": provisional_sequence}),
            created_at: chrono::Utc::now(),
        }
    }

    #[test]
    fn callback_consumer_assigns_canonical_identity_without_a_queue_lease() {
        let mut first = event("run-1", 9);
        let mut second = event("run-1", 3);
        assign_callback_identity(&mut first, 0);
        assign_callback_identity(&mut second, 1);

        assert_eq!(first.sequence, 0);
        assert_eq!(first.id, execution_event_id("run-1", 0));
        assert_eq!(second.sequence, 1);
        assert_eq!(second.id, execution_event_id("run-1", 1));
        let api = api_event_input(&second);
        assert_eq!(api.id.as_deref(), Some(second.id.as_str()));
        assert_eq!(api.sequence, Some(1));
    }
}

#[cfg(test)]
mod shadow_claim_binding_tests {
    use super::*;

    fn claims(shadow: Option<bool>) -> ExecutorClaims {
        serde_json::from_value(serde_json::json!({
            "sub": "user-1",
            "run_id": "run-1",
            "app_id": "app-1",
            "board_id": "board-1",
            "shadow": shadow,
            "callback_url": "https://api.example",
            "typ": "executor",
            "iss": "flow-like",
            "aud": "flow-like-executor",
            "iat": 0,
            "nbf": 0,
            "exp": 0,
            "jti": "jti-1"
        }))
        .expect("claims deserialize")
    }

    fn request(shadow: bool) -> ExecutionRequest {
        serde_json::from_value(serde_json::json!({
            "app_id": "app-1",
            "board_id": "board-1",
            "node_id": "node-1",
            "shadow": shadow,
            "credentials": {
                "Aws": {
                    "access_key_id": "AKIAIOSFODNN7EXAMPLE",
                    "secret_access_key": "secret",
                    "session_token": null,
                    "meta_bucket": "meta",
                    "content_bucket": "content",
                    "logs_bucket": "logs",
                    "region": "us-east-1",
                    "expiration": null
                }
            },
            "executor_jwt": "jwt",
            "artifact": {
                "url": "https://meta.example/apps/app-1/compiled/board-1/1_0_0.flcb?sig",
                "path": "apps/app-1/compiled/board-1/1_0_0.flcb",
                "registry_fingerprint": "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
            }
        }))
        .expect("request deserializes")
    }

    /// Bytes the API would have persisted: a board holding one real catalog
    /// node, compiled and encoded against the prepared registry.
    fn artifact_bytes_for(board_id: &str, fingerprint: &[u8; 32]) -> Vec<u8> {
        use flow_like::flow::board::Board;
        use flow_like::flow::compiled::{compile::compile_board_with_catalog, encode_artifact};

        let registry = PREPARED_REGISTRY.clone();
        let mut node = registry
            .get_nodes()
            .into_iter()
            .next()
            .expect("the catalog is not empty");
        node.id = "n1".into();
        let mut board = Board::new_detached(
            Some(board_id.to_string()),
            Path::from("apps").child("app-1"),
        );
        board.nodes.insert(node.id.clone(), node);
        let compiled = compile_board_with_catalog(&board, registry.as_ref()).expect("compile");
        encode_artifact(&compiled, fingerprint).expect("encode")
    }

    #[test]
    fn fetched_artifact_bytes_become_a_template_for_exactly_the_requested_board() {
        let registry = PREPARED_REGISTRY.clone();
        let fingerprint = registry.fingerprint();
        let request = request(false);

        let template = template_from_fetched(
            &artifact_bytes_for("board-1", &fingerprint),
            &fingerprint,
            registry.as_ref(),
            &request,
        )
        .expect("an artifact for this board and registry is accepted");
        assert_eq!(template.board.id, "board-1");

        let error = template_from_fetched(
            &artifact_bytes_for("board-9", &fingerprint),
            &fingerprint,
            registry.as_ref(),
            &request,
        )
        .err()
        .expect("an artifact for another board is refused");
        assert!(error.to_string().contains("board-9"), "{error}");

        let error = template_from_fetched(
            &artifact_bytes_for("board-1", &[9u8; 32]),
            &fingerprint,
            registry.as_ref(),
            &request,
        )
        .err()
        .expect("an artifact for another registry is refused");
        assert!(error.to_string().contains("this executor runs"), "{error}");
    }

    #[test]
    fn the_cache_key_pins_a_floating_run_to_the_artifacts_source_etag() {
        let mut request = request(false);
        request.artifact.source_etag = Some("etag-a".into());
        assert_eq!(artifact_version_key(&request).unwrap(), "latest@etag-a");

        request.board_etag = Some("etag-page".into());
        assert_eq!(
            artifact_version_key(&request).unwrap(),
            "latest@etag-page",
            "a Page run's authorized ETag wins over the resolved one"
        );

        request.board_etag = None;
        request.board_version = Some((1, 2, 3));
        assert_eq!(artifact_version_key(&request).unwrap(), "1_2_3");

        request.board_version = None;
        request.artifact.source_etag = None;
        assert!(
            artifact_version_key(&request).is_err(),
            "no identity, no key"
        );
    }

    /// The in-process isolation is driven by the signed claim, never by the
    /// unsigned payload byte: any disagreement fails closed.
    #[test]
    fn shadow_flag_is_load_bearing_from_the_signed_claims() {
        validate_executor_request_claims(&claims(Some(true)), &request(true))
            .expect("matching shadow flags are accepted");
        validate_executor_request_claims(&claims(None), &request(false))
            .expect("old JWTs without the claim mean a normal run");
        validate_executor_request_claims(&claims(Some(false)), &request(false))
            .expect("an explicit non-shadow claim matches a normal payload");

        assert!(validate_executor_request_claims(&claims(None), &request(true)).is_err());
        assert!(validate_executor_request_claims(&claims(Some(true)), &request(false)).is_err());
        assert!(validate_executor_request_claims(&claims(Some(false)), &request(true)).is_err());
    }
}

#[cfg(test)]
mod page_request_binding_tests {
    use super::*;

    fn latest_page() -> ExecutorPageExecutionClaims {
        ExecutorPageExecutionClaims {
            page_id: "page-1".into(),
            manifest_revision: "revision-1".into(),
            board_version: None,
            board_etag: Some("etag-a".into()),
            target_node_id: Some("entry-1".into()),
            entry_authority_revision: Some("authority-1".into()),
            wasm_authority_revision: Some(flow_like_types::dispatch::wasm_package_set_revision(
                None,
            )),
            allowed_entry_node_ids: vec!["entry-1".into(), "entry-2".into()],
        }
    }

    #[test]
    fn latest_page_selector_and_node_are_bound_by_the_executor_jwt() {
        let page = latest_page();
        validate_page_request_binding(&page, None, Some("etag-a"), "entry-1", None)
            .expect("the signed selector and entry are accepted");
        assert!(
            validate_page_request_binding(&page, None, Some("etag-b"), "entry-1", None).is_err()
        );
        assert!(
            validate_page_request_binding(&page, None, Some("etag-a"), "foreign", None).is_err()
        );
        assert!(
            validate_page_request_binding(&page, Some((1, 2, 3)), None, "entry-1", None).is_err()
        );
    }

    #[test]
    fn latest_page_accepts_the_decoded_selector_and_rejects_the_raw_wire_sentinel() {
        use flow_like_types::dispatch::ETAG_BOUND_LATEST_VERSION_SENTINEL;

        let page = latest_page();
        validate_page_request_binding(&page, None, Some("etag-a"), "entry-1", None)
            .expect("the decoded ETag-bound Latest selector is accepted");
        assert!(validate_page_request_binding(
            &page,
            Some(ETAG_BOUND_LATEST_VERSION_SENTINEL),
            Some("etag-a"),
            "entry-1",
            None,
        )
        .is_err());
    }

    #[test]
    fn latest_page_never_accepts_a_missing_signed_allow_list() {
        let mut page = latest_page();
        page.allowed_entry_node_ids.clear();
        page.target_node_id = None;
        page.entry_authority_revision = None;
        assert!(
            validate_page_request_binding(&page, None, Some("etag-a"), "entry-1", None).is_err()
        );
    }

    #[test]
    fn signed_target_cannot_be_swapped_for_another_allowed_entry() {
        let page = latest_page();
        assert!(
            validate_page_request_binding(&page, None, Some("etag-a"), "entry-2", None).is_err()
        );
    }

    #[test]
    fn signed_wasm_authority_rejects_a_substituted_package_set() {
        let page = latest_page();
        let packages = std::collections::HashMap::from([(
            "package-1".to_string(),
            flow_like_types::dispatch::WasmPackageRef {
                version: "1.0.0".into(),
                wasm_hash: "wasm-hash".into(),
                wasm_url: "https://example.invalid/package.wasm".into(),
                cwasm_url: "https://example.invalid/package.cwasm".into(),
                cwasm_checksum: "cwasm-checksum".into(),
            },
        )]);
        assert!(validate_page_request_binding(
            &page,
            None,
            Some("etag-a"),
            "entry-1",
            Some(&packages),
        )
        .is_err());
    }
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
        assert!(ensure_terminal_acknowledgement(
            &acknowledgement(true, "Completed"),
            &ExecutionStatus::Completed,
        )
        .is_ok());
        assert!(ensure_terminal_acknowledgement(
            &acknowledgement(true, "Running"),
            &ExecutionStatus::Completed,
        )
        .is_err());
        assert!(ensure_terminal_acknowledgement(
            &acknowledgement(true, "Failed"),
            &ExecutionStatus::Completed,
        )
        .is_err());
        assert!(ensure_terminal_acknowledgement(
            &acknowledgement(false, "Failed"),
            &ExecutionStatus::Completed,
        )
        .is_ok());
    }
}
