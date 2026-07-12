use flow_like::app::App;
use flow_like::credentials::SharedCredentials;
use flow_like::flow::execution::log::LogMessage;
use flow_like::flow::execution::{
    DEFAULT_CONTEXT_LOG_SPILL_THRESHOLD, DEFAULT_RUN_LOG_FLUSH_INTERVAL, InternalRun,
};
use flow_like::flow::execution::{LogLevel, LogMeta, RunPayload, flush_run_cancelled};
use flow_like::flow::oauth::OAuthToken;
use flow_like::flow_like_storage::lancedb::query::{ExecutableQuery, QueryBase};
use flow_like::flow_like_storage::{Path, serde_arrow};
use flow_like::hub::Hub;
use flow_like::state::RunData;
use flow_like_types::intercom::{BufferedInterComHandler, InterComEvent};
use flow_like_types::tokio_util::sync::CancellationToken;
use flow_like_types::{json, tokio};
use futures::TryStreamExt;
use serde::Serialize;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tauri::{AppHandle, Manager};

use crate::utils::{UiEmitTarget, local_execution_environment};
use crate::{
    functions::TauriFunctionError,
    state::{TauriFlowLikeState, TauriSettingsState},
};

#[derive(Serialize)]
struct ReportRunRequest {
    run_id: String,
    node_id: String,
    event_id: Option<String>,
    version: Option<String>,
    log_level: u8,
    start: u64,
    end: u64,
    error_message: Option<String>,
}

#[derive(Default)]
struct ExecutionOverrides {
    cancellation_token: Option<CancellationToken>,
    cancellation_log_level: Option<LogLevel>,
    cancellation_log_message: Option<String>,
    log_flush_interval: Option<Duration>,
    log_batch_size: Option<usize>,
    run_sub_override: Option<String>,
}

async fn report_run_to_backend(app_handle: &AppHandle, token: &str, meta: &LogMeta) {
    let hub_url = match TauriSettingsState::current_profile(app_handle).await {
        Ok(profile) => profile.hub_profile.hub.clone(),
        Err(_) => return,
    };

    if hub_url.is_empty() {
        return;
    }

    let url = format!(
        "{}/api/v1/apps/{}/board/{}/runs/report",
        hub_url.trim_end_matches('/'),
        meta.app_id,
        meta.board_id,
    );

    let error_message = if meta.log_level >= 3 {
        Some(format!(
            "Local run failed with log_level {}",
            meta.log_level
        ))
    } else {
        None
    };

    let body = ReportRunRequest {
        run_id: meta.run_id.clone(),
        node_id: meta.node_id.clone(),
        event_id: if meta.event_id.is_empty() {
            None
        } else {
            Some(meta.event_id.clone())
        },
        version: if meta.version.is_empty() {
            None
        } else {
            Some(meta.version.clone())
        },
        log_level: meta.log_level,
        start: meta.start,
        end: meta.end,
        error_message,
    };

    let auth_val = if token.starts_with("Bearer ") {
        token.to_string()
    } else {
        format!("Bearer {}", token)
    };

    let client = flow_like_types::reqwest::Client::new();
    match client
        .post(&url)
        .header("Authorization", &auth_val)
        .json(&body)
        .timeout(Duration::from_secs(10))
        .send()
        .await
    {
        Ok(resp) if resp.status().is_success() => {
            tracing::info!(run_id = %meta.run_id, "Reported local run to backend");
        }
        Ok(resp) => {
            tracing::warn!(
                run_id = %meta.run_id,
                status = %resp.status(),
                "Failed to report local run to backend"
            );
        }
        Err(e) => {
            tracing::warn!(
                run_id = %meta.run_id,
                error = %e,
                "Failed to report local run to backend"
            );
        }
    }
}

/// Update the last_node_update timestamp for a run when we see run events
fn touch_run_last_update(app_handle: &AppHandle, events: &[InterComEvent]) {
    for event in events {
        // Run events have type "run:{run_id}"
        if event.event_type.starts_with("run:") {
            let run_id = &event.event_type[4..]; // Skip "run:" prefix
            if let Some(state) = app_handle.try_state::<TauriFlowLikeState>()
                && let Some(run_data) = state.0.board_run_registry.get(run_id)
            {
                run_data.touch_last_node_update();
            }
        }
    }
}

fn credential_content_prefix(credentials: &SharedCredentials) -> Option<&str> {
    match credentials {
        SharedCredentials::Aws(aws) => aws.content_path_prefix.as_deref(),
        SharedCredentials::Azure(azure) => azure.content_path_prefix.as_deref(),
        SharedCredentials::Gcp(gcp) => gcp.content_path_prefix.as_deref().or_else(|| {
            gcp.allowed_prefixes
                .iter()
                .find(|prefix| prefix.starts_with("apps/"))
                .map(String::as_str)
        }),
        SharedCredentials::Mixed(mixed) => credential_content_prefix(&mixed.content),
    }
}

fn credential_user_content_prefix(credentials: &SharedCredentials) -> Option<&str> {
    match credentials {
        SharedCredentials::Aws(aws) => aws.user_content_path_prefix.as_deref(),
        SharedCredentials::Azure(azure) => azure.user_content_path_prefix.as_deref(),
        SharedCredentials::Gcp(gcp) => gcp.user_content_path_prefix.as_deref().or_else(|| {
            gcp.allowed_prefixes
                .iter()
                .find(|prefix| prefix.starts_with("users/"))
                .map(String::as_str)
        }),
        SharedCredentials::Mixed(mixed) => credential_user_content_prefix(&mixed.content),
    }
}

fn daemon_sub_from_credentials(credentials: &SharedCredentials, app_id: &str) -> Option<String> {
    let prefix = credential_user_content_prefix(credentials)?;
    let rest = prefix.strip_prefix("users/")?;
    let (sub, prefix_app_id) = rest.split_once("/apps/")?;

    if prefix_app_id == app_id {
        Some(sub.to_string())
    } else {
        None
    }
}

async fn execute_internal(
    app_handle: AppHandle,
    app_id: String,
    mut board_id: String,
    mut payload: RunPayload,
    events: Option<tauri::ipc::Channel<Vec<InterComEvent>>>,
    event_id: Option<String>,
    stream_state: bool,
    credentials: Option<SharedCredentials>,
    token: Option<String>,
    oauth_tokens: Option<HashMap<String, OAuthToken>>,
    overrides: ExecutionOverrides,
) -> Result<Option<LogMeta>, TauriFunctionError> {
    let mut event = None;
    let shared_flow_like_state = TauriFlowLikeState::construct(&app_handle).await?;
    let flow_like_state = Arc::new(shared_flow_like_state.for_execution_run());
    let mut version = None;
    let Ok(app) = App::load(app_id.clone(), flow_like_state.clone()).await else {
        return Err(TauriFunctionError::new("App not found"));
    };

    // Desktop execution is trusted — allow secret overrides from local runtime vars
    payload.filter_secrets = Some(false);

    if let Some(event_id) = &event_id {
        let intermediate_event = app.get_event(event_id, None).await?;
        payload.id = intermediate_event.node_id.clone();
        version = intermediate_event.board_version;
        board_id = intermediate_event.board_id.clone();
        event = Some(intermediate_event);
    }

    let Ok(board) = app.open_board(board_id.clone(), None, version).await else {
        return Err(TauriFunctionError::new("Board not found"));
    };

    let board = Arc::new(board.lock().await.clone());

    let profile = TauriSettingsState::current_profile(&app_handle).await?;

    let app_handle_for_report = app_handle.clone();
    let token_for_report = token.clone();

    let buffered_sender = Arc::new(BufferedInterComHandler::new(
        Arc::new(move |event| {
            let events_cb = events.as_ref().cloned();
            let app_handle = app_handle.clone();
            Box::pin({
                async move {
                    // Update last_node_update for run events
                    touch_run_last_update(&app_handle, &event);

                    if let Some(events_cb) = events_cb
                        && let Err(err) = events_cb.send(event.clone())
                    {
                        println!("Error sending event to execution channel: {}", err);
                    }

                    let first_event = event.first();

                    if let Some(first_event) = first_event {
                        crate::utils::emit_throttled(
                            &app_handle,
                            UiEmitTarget::All,
                            &first_event.event_type,
                            event.clone(),
                            std::time::Duration::from_millis(150),
                        );
                    }

                    Ok(())
                }
            })
        }),
        Some(100),
        Some(400),
        Some(true),
    ));

    let (event_name, event_type) = event
        .as_ref()
        .map(|e| (Some(e.name.clone()), Some(e.event_type.clone())))
        .unwrap_or((None, None));

    let mut internal_run = InternalRun::new(
        &app_id,
        board,
        event,
        &flow_like_state,
        &profile.hub_profile,
        &payload,
        stream_state,
        buffered_sender.into_callback(),
        credentials,
        token,
        oauth_tokens.unwrap_or_default().into_iter().collect(),
    )
    .await?;

    if let Some(run_sub_override) = overrides.run_sub_override {
        internal_run.set_execution_sub(run_sub_override).await;
    }
    internal_run.set_execution_environment(local_execution_environment());

    // Set offline user context for desktop app (always admin/owner)
    internal_run.set_offline_user_context();

    if overrides.log_flush_interval.is_some() || overrides.log_batch_size.is_some() {
        internal_run
            .set_log_flush_policy(
                overrides
                    .log_flush_interval
                    .unwrap_or(DEFAULT_RUN_LOG_FLUSH_INTERVAL),
                overrides
                    .log_batch_size
                    .unwrap_or(DEFAULT_CONTEXT_LOG_SPILL_THRESHOLD),
            )
            .await?;
    }

    let run_id = internal_run.run.lock().await.id.clone();

    let _send_result = buffered_sender
        .send(InterComEvent::with_type(
            "run_initiated",
            json::json!({ "run_id": run_id.clone()}),
        ))
        .await;

    let cancellation_token = overrides
        .cancellation_token
        .unwrap_or_else(CancellationToken::new);
    internal_run.set_cancellation_token(cancellation_token.clone());
    if overrides.cancellation_log_level.is_some() || overrides.cancellation_log_message.is_some() {
        internal_run.set_cancellation_log(
            overrides
                .cancellation_log_message
                .unwrap_or_else(|| "Run cancelled".to_string()),
            overrides.cancellation_log_level.unwrap_or(LogLevel::Fatal),
        );
    }

    let board_name = internal_run.board.name.clone();
    let run_data = RunData::with_metadata(
        &board_id,
        &payload.id,
        None,
        cancellation_token.clone(),
        Some(board_name),
        event_name,
        event_type,
    );

    shared_flow_like_state.register_run(&run_id, run_data);

    let run_arc = internal_run.run.clone();

    // Spawn execution as a task so cancellation can be observed while the UI remains responsive.
    let flow_like_state_for_task = flow_like_state.clone();
    let mut handle =
        tokio::spawn(async move { internal_run.execute(flow_like_state_for_task).await });

    let abort_handle = handle.abort_handle();

    let meta = tokio::select! {
        biased;
        _ = cancellation_token.cancelled() => {
            println!("Board execution cancelled for run: {}", run_id);
            match tokio::time::timeout(Duration::from_secs(30), &mut handle).await {
                Ok(Ok(meta)) => meta,
                Ok(Err(e)) if e.is_cancelled() => {
                    println!("Task was cancelled for run: {}", run_id);
                    None
                }
                Ok(Err(e)) => {
                    println!("Task panicked for run: {}, {:?}", run_id, e);
                    None
                }
                Err(_) => {
                    println!("Timeout while waiting for cancelled run to stop: {}", run_id);
                    abort_handle.abort();
                    match tokio::time::timeout(Duration::from_secs(30), flush_run_cancelled(&run_arc)).await {
                        Ok(Ok(meta)) => meta,
                        Ok(Err(e)) => {
                            println!("Error flushing logs for cancelled run: {}, {:?}", run_id, e);
                            None
                        }
                        Err(_) => {
                            println!("Timeout while flushing logs for cancelled run: {}", run_id);
                            None
                        }
                    }
                }
            }
        }
        result = &mut handle => {
            match result {
                Ok(meta) => meta,
                Err(e) if e.is_cancelled() => {
                    println!("Task was cancelled for run: {}", run_id);
                    None
                }
                Err(e) => {
                    println!("Task panicked for run: {}, {:?}", run_id, e);
                    None
                }
            }
        }
    };

    if let Err(err) = buffered_sender.flush().await {
        println!("Error flushing buffered sender: {}", err);
    }

    let flush_result: flow_like_types::Result<()> = if let Some(meta) = &meta {
        let (db_fn, write_options) = {
            let guard = flow_like_state.config.read().await;
            (
                guard.callbacks.build_logs_database.clone(),
                guard.callbacks.lance_write_options.clone(),
            )
        };
        async {
            let db_fn = db_fn
                .as_ref()
                .ok_or_else(|| flow_like_types::anyhow!("No log database configured"))?;
            let base_path = Path::from("runs").child(app_id).child(board_id);
            let db = flow_like_state
                .with_lance_session(db_fn(base_path.clone()))
                .execute()
                .await
                .map_err(|e| {
                    flow_like_types::anyhow!("Failed to open database: {}, {:?}", base_path, e)
                })?;
            meta.flush(db, write_options.as_ref()).await.map_err(|e| {
                flow_like_types::anyhow!("Failed to flush run: {}, {:?}", base_path, e)
            })?;
            Ok(())
        }
        .await
    } else {
        Ok(())
    };

    // Report online local runs so backend analytics can count executions.
    if let (Some(meta), Some(token)) = (&meta, &token_for_report) {
        let app_handle = app_handle_for_report.clone();
        let token = token.clone();
        let meta = meta.clone();
        tokio::spawn(async move {
            report_run_to_backend(&app_handle, &token, &meta).await;
        });
    }

    // Always release the finished run from the registry, even if flushing its
    // logs failed. Otherwise the run stays flagged "in use" and its logs can
    // never be deleted from storage management until the app restarts.
    let _res = shared_flow_like_state.remove_and_cancel_run(&run_id);
    flush_result?;

    Ok(meta)
}

pub(crate) async fn execute_daemon_event(
    app_handle: AppHandle,
    app_id: String,
    event_id: String,
    payload: Option<flow_like_types::Value>,
    cancellation_token: CancellationToken,
    offline: bool,
    token: Option<String>,
    oauth_tokens: Option<HashMap<String, OAuthToken>>,
    log_flush_interval: Duration,
    log_batch_size: usize,
) -> Result<Option<LogMeta>, TauriFunctionError> {
    if !offline && token.is_none() {
        return Err(TauriFunctionError::new(
            "No token registered, cannot run online daemon event",
        ));
    }

    let (credentials, run_sub_override) = if offline {
        (None, None)
    } else {
        let token = token.as_deref().ok_or_else(|| {
            TauriFunctionError::new("No token registered, cannot run online daemon event")
        })?;
        let profile = TauriSettingsState::current_profile(&app_handle).await?;
        let hub_url = profile.hub_profile.hub;

        if hub_url.is_empty() {
            return Err(TauriFunctionError::new(
                "No hub URL configured, cannot get daemon credentials",
            ));
        }

        let http_client = TauriFlowLikeState::http_client(&app_handle).await?;
        let hub = Hub::new(&hub_url, http_client).await?;
        tracing::info!(
            app_id = %app_id,
            event_id = %event_id,
            token_kind = if token.starts_with("pat_") { "pat" } else { "jwt" },
            "Fetching credentials for daemon event"
        );
        let shared_credentials = hub
            .shared_credentials(token, &app_id)
            .await
            .map_err(|err| {
                tracing::error!(
                    app_id = %app_id,
                    event_id = %event_id,
                    error = %err,
                    "Failed to fetch credentials for daemon event"
                );
                err
            })?;
        let content_prefix = credential_content_prefix(&shared_credentials).map(str::to_string);
        let user_content_prefix =
            credential_user_content_prefix(&shared_credentials).map(str::to_string);
        let run_sub_override = if token.starts_with("pat_") {
            daemon_sub_from_credentials(&shared_credentials, &app_id)
        } else {
            None
        };
        tracing::info!(
            app_id = %app_id,
            event_id = %event_id,
            content_prefix = ?content_prefix,
            user_content_prefix = ?user_content_prefix,
            has_run_sub_override = run_sub_override.is_some(),
            "Fetched credentials for daemon event"
        );
        (Some(shared_credentials), run_sub_override)
    };

    execute_internal(
        app_handle,
        app_id,
        String::new(),
        RunPayload {
            id: String::new(),
            payload,
            runtime_variables: None,
            filter_secrets: Some(false),
        },
        None,
        Some(event_id),
        false,
        credentials,
        token,
        oauth_tokens,
        ExecutionOverrides {
            cancellation_token: Some(cancellation_token),
            cancellation_log_level: Some(LogLevel::Info),
            cancellation_log_message: Some("Daemon run stopped".to_string()),
            log_flush_interval: Some(log_flush_interval),
            log_batch_size: Some(log_batch_size),
            run_sub_override,
        },
    )
    .await
}

#[tauri::command(async)]
pub async fn execute_board(
    app_handle: AppHandle,
    app_id: String,
    board_id: String,
    payload: RunPayload,
    stream_state: Option<bool>,
    events: tauri::ipc::Channel<Vec<InterComEvent>>,
    credentials: Option<SharedCredentials>,
    token: Option<String>,
    oauth_tokens: Option<HashMap<String, OAuthToken>>,
) -> Result<Option<LogMeta>, TauriFunctionError> {
    let stream_state = stream_state.unwrap_or(true);
    execute_internal(
        app_handle,
        app_id,
        board_id,
        payload,
        Some(events),
        None,
        stream_state,
        credentials,
        token,
        oauth_tokens,
        ExecutionOverrides::default(),
    )
    .await
}

#[tauri::command(async)]
pub async fn execute_event(
    app_handle: AppHandle,
    app_id: String,
    event_id: String,
    payload: RunPayload,
    stream_state: Option<bool>,
    events: tauri::ipc::Channel<Vec<InterComEvent>>,
    credentials: Option<SharedCredentials>,
    token: Option<String>,
    oauth_tokens: Option<HashMap<String, OAuthToken>>,
) -> Result<Option<LogMeta>, TauriFunctionError> {
    let stream_state = stream_state.unwrap_or(false);
    execute_internal(
        app_handle,
        app_id,
        String::new(), // Will be read from the event anyways
        payload,
        Some(events),
        Some(event_id),
        stream_state,
        credentials,
        token,
        oauth_tokens,
        ExecutionOverrides::default(),
    )
    .await
}

#[tauri::command(async)]
pub async fn cancel_execution(
    app_handle: AppHandle,
    run_id: String,
) -> Result<(), TauriFunctionError> {
    let flow_like_state = TauriFlowLikeState::construct(&app_handle).await?;
    let _cancel_result = flow_like_state.remove_and_cancel_run(&run_id);
    Ok(())
}

#[tauri::command(async)]
pub async fn list_runs(
    app_handle: AppHandle,
    app_id: String,
    board_id: String,
    node_id: Option<String>,
    from: Option<u64>,
    to: Option<u64>,
    status: Option<LogLevel>,
    limit: Option<usize>,
    offset: Option<usize>,
    _last_meta: Option<LogMeta>,
) -> Result<Vec<LogMeta>, TauriFunctionError> {
    let limit = limit.unwrap_or(100);
    let offset = offset.unwrap_or(0);
    let state = TauriFlowLikeState::construct(&app_handle).await?;
    let db = {
        let guard = state.config.read().await;

        guard.callbacks.build_logs_database.clone()
    };
    let db_fn = db
        .as_ref()
        .ok_or_else(|| flow_like_types::anyhow!("No log database configured"))?;
    let base_path = Path::from("runs").child(app_id).child(board_id);
    let db = db_fn(base_path.clone())
        .execute()
        .await
        .map_err(|_| flow_like_types::anyhow!("Failed to open database: {}", base_path))?;

    let db = db
        .open_table("runs")
        .execute()
        .await
        .map_err(|_| flow_like_types::anyhow!("Failed to open table: runs"))?;

    let mut query_string = String::from("");

    if let Some(node_id) = node_id {
        query_string.push_str(&format!("node_id = '{}'", node_id));
    }

    if let Some(from) = from {
        if !query_string.is_empty() {
            query_string.push_str(" AND ");
        }
        query_string.push_str(&format!("start >= {}", from));
    }

    if let Some(to) = to {
        if !query_string.is_empty() {
            query_string.push_str(" AND ");
        }
        query_string.push_str(&format!("start <= {}", to));
    }

    if let Some(status) = status {
        if !query_string.is_empty() {
            query_string.push_str(" AND ");
        }

        let status = status.to_u8();
        if status == 0 {
            query_string.push_str("log_level <= 1");
        } else {
            query_string.push_str(&format!("log_level = {}", status));
        }
    }

    let mut query = db.query();

    if !query_string.is_empty() {
        query = query.only_if(&query_string);
    }

    let runs = query
        .limit(limit)
        .offset(offset)
        .execute()
        .await
        .map_err(|_| flow_like_types::anyhow!("Failed to execute query"))?;
    let results = runs
        .try_collect::<Vec<_>>()
        .await
        .map_err(|_| flow_like_types::anyhow!("Failed to collect results"))?;
    let mut log_meta = Vec::with_capacity(results.len() * 10);
    for result in results {
        let stored: Vec<flow_like::flow::execution::StoredLogMeta> =
            serde_arrow::from_record_batch(&result).unwrap_or_default();
        log_meta.extend(stored.into_iter().map(LogMeta::from));
    }
    Ok(log_meta)

    // let mut stream = db
    //     .query()
    //     .execute()
    //     .await
    //     .map_err(|_| flow_like_types::anyhow!("Failed to execute query on table: runs"))?;

    // let client = ClientBuilder::new().open().await?;
    // let out = client.conn(move |conn| {
    //     conn.execute_batch("
    //         CREATE TABLE runs (
    //             start     UBIGINT,
    //             run_id    VARCHAR,
    //             log_level UTINYINT,
    //             node_id   VARCHAR
    //         )
    //     ")?;
    //     let mut appender = conn.appender("runs").unwrap();
    //     let (tx, rx) = std::sync::mpsc::channel();
    //     tokio::spawn(async move {
    //         while let Some(item_res) = stream.next().await {
    //             if let Ok(item) = item_res {
    //                 let _ = tx.send(item);
    //             }
    //         }
    //     });

    //     for meta in rx {
    //         appender.append_record_batch(meta)?;
    //     }
    //     appender.flush()?;
    //     let mut stmt = conn.prepare("SELECT start, run_id, log_level, node_id FROM runs ORDER BY start DESC LIMIT ?, ?")?;
    //     let mut rows = stmt.query(params![offset as i64, limit as i64])?;
    //     let mut out = Vec::new();
    //     while let Some(r) = rows.next()? {
    //         let start: u64 = r.get(0)?;
    //         println!("Row: {:?}", start);
    //     }
    //     Ok(out)
    // }).await?;

    // return Ok(out);
}

#[tauri::command(async)]
pub async fn query_run(
    app_handle: AppHandle,
    log_meta: LogMeta,
    query: String,
    limit: Option<usize>,
    offset: Option<usize>,
) -> Result<Vec<LogMessage>, TauriFunctionError> {
    let state = TauriFlowLikeState::construct(&app_handle).await?;
    let logs = state.query_run(&log_meta, &query, limit, offset).await?;
    Ok(logs)
}
