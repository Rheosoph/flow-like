use flow_like::app::App;
use flow_like::credentials::SharedCredentials;
use flow_like::flow::execution::InternalRun;
use flow_like::flow::execution::log::LogMessage;
use flow_like::flow::execution::{LogLevel, LogMeta, RunPayload, flush_run_cancelled};
use flow_like::flow::oauth::OAuthToken;
use flow_like::flow_like_storage::lancedb::query::{ExecutableQuery, QueryBase};
use flow_like::flow_like_storage::{Path, serde_arrow};
use flow_like::state::RunData;
use flow_like_types::intercom::{BufferedInterComHandler, InterComEvent};
use flow_like_types::tokio_util::sync::CancellationToken;
use flow_like_types::{json, tokio};
use futures::TryStreamExt;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tauri::{AppHandle, Manager};

use crate::utils::UiEmitTarget;
use crate::{
    functions::TauriFunctionError,
    state::{TauriFlowLikeState, TauriSettingsState},
};

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
) -> Result<Option<LogMeta>, TauriFunctionError> {
    let mut event = None;
    let flow_like_state = TauriFlowLikeState::construct(&app_handle).await?;
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

    let Ok(board) = app.open_board(board_id.clone(), Some(false), version).await else {
        return Err(TauriFunctionError::new("Board not found"));
    };

    let board = Arc::new(board.lock().await.clone());

    let profile = TauriSettingsState::current_profile(&app_handle).await?;

    let buffered_sender = Arc::new(BufferedInterComHandler::new(
        Arc::new(move |event| {
            if events.is_none() {
                return Box::pin(async { Ok(()) });
            }
            let events = events.as_ref().unwrap();
            let events_cb = events.clone();
            let app_handle = app_handle.clone();
            Box::pin({
                async move {
                    // Update last_node_update for run events
                    touch_run_last_update(&app_handle, &event);

                    if let Err(err) = events_cb.send(event.clone()) {
                        println!("Error emitting event: {}", err);
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

    // Set offline user context for desktop app (always admin/owner)
    internal_run.set_offline_user_context();

    let run_id = internal_run.run.lock().await.id.clone();

    let _send_result = buffered_sender
        .send(InterComEvent::with_type(
            "run_initiated",
            json::json!({ "run_id": run_id.clone()}),
        ))
        .await;

    let cancellation_token = CancellationToken::new();
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

    flow_like_state.register_run(&run_id, run_data);

    let run_arc = internal_run.run.clone();

    // Spawn execution as a task so we can abort it immediately on cancellation
    let flow_like_state_for_task = flow_like_state.clone();
    let handle = tokio::spawn(async move { internal_run.execute(flow_like_state_for_task).await });

    let abort_handle = handle.abort_handle();

    let meta = tokio::select! {
        biased;
        _ = cancellation_token.cancelled() => {
            println!("Board execution cancelled for run: {}", run_id);
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
        result = handle => {
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

    if let Some(meta) = &meta {
        let (db_fn, write_options) = {
            let guard = flow_like_state.config.read().await;
            (
                guard.callbacks.build_logs_database.clone(),
                guard.callbacks.lance_write_options.clone(),
            )
        };
        let db_fn = db_fn
            .as_ref()
            .ok_or_else(|| flow_like_types::anyhow!("No log database configured"))?;
        let base_path = Path::from("runs").child(app_id).child(board_id);
        let db = db_fn(base_path.clone()).execute().await.map_err(|e| {
            flow_like_types::anyhow!("Failed to open database: {}, {:?}", base_path, e)
        })?;
        meta.flush(db, write_options.as_ref())
            .await
            .map_err(|e| flow_like_types::anyhow!("Failed to flush run: {}, {:?}", base_path, e))?;
    }

    let _res = flow_like_state.remove_and_cancel_run(&run_id);

    Ok(meta)
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
