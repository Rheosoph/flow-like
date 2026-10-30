use super::super::{
    native_platform_tools::{
        NativePlatformSession, boolean, err, limit, required, segment, string,
    },
    native_tool_bridge::{
        execution_cancellation, interaction_unavailable, unavailable_result, with_execution_profile,
    },
};
use crate::{functions::flow::run, state::TauriFlowLikeState};
use flow_like::{
    app::App,
    flow::{
        event::{Event, EventExecutionMode},
        execution::{LogMeta, RunPayload},
        regression::{RunGradeEvidence, discover_board_tests, grade_run},
    },
};
use reqwest::Method;
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};
use tauri::ipc::{Channel, InvokeResponseBody};

pub(crate) type EventObserver = Arc<dyn Fn(&Value) + Send + Sync>;

#[derive(Default)]
struct Events {
    count: usize,
    tail: Vec<Value>,
    run_id: Option<String>,
    terminal: Option<String>,
    observer: Option<EventObserver>,
    refuse_interactions: bool,
    pending_interaction: Option<Value>,
    interaction_ready: Option<Arc<tokio::sync::Notify>>,
    unavailable: Option<UnavailableInteraction>,
}

struct UnavailableInteraction {
    cancellation_requested: bool,
    cancellation_error: Option<String>,
}
impl Events {
    fn push(&mut self, mut event: Value) {
        if let Some(events) = event.as_array() {
            for event in events {
                self.push(event.clone());
            }
            return;
        }
        if let Some(observer) = &self.observer {
            observer(&event);
        }
        if event["event_type"] == "interaction_request" {
            if self.refuse_interactions && self.pending_interaction.is_none() {
                self.pending_interaction = Some(event["payload"].clone());
                if let Some(ready) = &self.interaction_ready {
                    ready.notify_one();
                }
            }
            // Reply capabilities stay in the host; diagnostic logs can reach the model.
            if let Some(payload) = event.get_mut("payload").and_then(Value::as_object_mut) {
                payload.remove("channel");
            }
        }
        self.count += 1;
        if event.get("event_type").and_then(Value::as_str) == Some("run_initiated")
            && self.run_id.is_none()
        {
            self.run_id = event
                .pointer("/payload/run_id")
                .and_then(Value::as_str)
                .map(str::to_owned);
        }
        if matches!(
            event.get("event_type").and_then(Value::as_str),
            Some("completed" | "execution_completed")
        ) && event
            .pointer("/payload/run_id")
            .and_then(Value::as_str)
            .zip(self.run_id.as_deref())
            .is_none_or(|(finished, run)| finished == run)
        {
            self.terminal = event
                .pointer("/payload/status")
                .and_then(Value::as_str)
                .map(str::to_owned);
        }
        if self.tail.len() == 200 {
            self.tail.remove(0);
        }
        let encoded = event.to_string();
        self.tail.push(if encoded.len() > 8000 {
            json!({"event_type":event.get("event_type"),"payload_truncated":true})
        } else {
            event
        });
    }

    fn outcome(&self) -> Value {
        if let Some(unavailable) = &self.unavailable {
            let mut result = unavailable_result("workflow_interaction");
            result["run_status"] = json!("interaction_unavailable");
            result["cancellation_requested"] = json!(unavailable.cancellation_requested);
            if let Some(error) = &unavailable.cancellation_error {
                result["cancellation_error"] = json!(error);
            }
            return result;
        }
        let run_status = self.terminal.as_deref().unwrap_or("interrupted");
        let status = match run_status.to_ascii_lowercase().as_str() {
            "completed" | "success" => "ok",
            "cancelled" | "canceled" | "stopped" => "cancelled",
            _ => "error",
        };
        let mut outcome = json!({"status":status,"run_status":run_status});
        if status != "ok" {
            outcome["code"] = json!(if self.terminal.is_some() {
                "execution_unsuccessful"
            } else {
                "execution_incomplete"
            });
            outcome["error"] = json!(format!(
                "Workflow execution ended with status '{run_status}'. Inspect its run and logs before retrying actions."
            ));
        }
        outcome
    }
}

async fn await_local_execution<T, E: std::fmt::Display>(
    mut work: tokio::task::JoinHandle<Result<Option<T>, E>>,
    events: Arc<Mutex<Events>>,
    interaction_ready: Arc<tokio::sync::Notify>,
    refuse_interactions: bool,
    cancellation: flow_like_types::tokio_util::sync::CancellationToken,
) -> Result<(Option<T>, Events), String> {
    let _cancel_on_drop = cancellation.clone().drop_guard();
    let completed = tokio::select! {
        biased;
        _ = interaction_ready.notified(), if refuse_interactions => None,
        result = &mut work => Some(result),
    };
    let mut output = std::mem::take(&mut *events.lock().unwrap_or_else(|e| e.into_inner()));
    // An input request and completion may arrive in the same poll or event batch.
    let metadata = if output.pending_interaction.take().is_some() {
        cancellation.cancel();
        output.unavailable = Some(UnavailableInteraction {
            cancellation_requested: true,
            cancellation_error: None,
        });
        None
    } else {
        completed
            .ok_or("Workflow interaction notification had no request")?
            .map_err(|_| "The native workflow worker failed")?
            .map_err(err)?
    };
    Ok((metadata, output))
}

/// SSE frames are decoded only after their byte boundary, including split UTF-8 sequences.
#[derive(Default)]
struct SseDecoder {
    pending: Vec<u8>,
}
impl SseDecoder {
    fn feed(&mut self, chunk: &[u8]) -> Result<Vec<Value>, String> {
        self.pending.extend_from_slice(chunk);
        let mut values = Vec::new();
        loop {
            let lf = self
                .pending
                .windows(2)
                .position(|w| w == b"\n\n")
                .map(|i| (i, 2));
            let crlf = self
                .pending
                .windows(4)
                .position(|w| w == b"\r\n\r\n")
                .map(|i| (i, 4));
            let boundary = lf.into_iter().chain(crlf).min_by_key(|(at, _)| *at);
            let Some((at, width)) = boundary else { break };
            if at > 1024 * 1024 {
                return Err("Execution stream frame exceeds the limit.".into());
            }
            let frame = self.pending.drain(..at + width).collect::<Vec<_>>();
            let text =
                std::str::from_utf8(&frame).map_err(|_| "Invalid UTF-8 in execution stream")?;
            let data = text
                .lines()
                .filter_map(|line| {
                    line.strip_prefix("data:")
                        .map(|value| value.strip_prefix(' ').unwrap_or(value))
                })
                .collect::<Vec<_>>()
                .join("\n");
            if !data.is_empty() && data != "[DONE]" {
                values.push(
                    serde_json::from_str(&data).map_err(|_| "Invalid JSON in execution stream")?,
                );
            }
        }
        if self.pending.len() > 1024 * 1024 {
            return Err("Execution stream frame exceeds the limit.".into());
        }
        Ok(values)
    }
    fn finish(&self) -> Result<(), String> {
        if self.pending.iter().all(u8::is_ascii_whitespace) {
            Ok(())
        } else {
            Err("Execution stream ended in an incomplete frame.".into())
        }
    }
}

async fn stream(
    session: &NativePlatformSession,
    app_id: &str,
    path: &str,
    body: &Value,
    observer: Option<EventObserver>,
) -> Result<Events, String> {
    let mut response = session
        .request_response(
            Method::POST,
            path,
            Some(body),
            &[("Accept", "text/event-stream")],
        )
        .await?;
    let mut decoder = SseDecoder::default();
    let mut events = Events {
        observer,
        refuse_interactions: interaction_unavailable(session.context.as_ref()),
        ..Default::default()
    };
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| "Execution stream was interrupted.")?
    {
        session.assert_current_profile().await?;
        for event in decoder.feed(&chunk)? {
            events.push(event);
            if let Some(interaction) = events.pending_interaction.take() {
                let cancellation_error = if !interaction_matches_run(
                    events.run_id.as_deref(),
                    &interaction,
                ) {
                    Some("The input request could not be matched to the current workflow; no cancellation was sent.".into())
                } else {
                    match tokio::time::timeout(
                        std::time::Duration::from_secs(5),
                        super::super::native_platform_tools::cancel_unavailable_interaction(
                            session,
                            app_id,
                            &interaction,
                        ),
                    ).await {
                        Ok(Ok(())) => None,
                        Ok(Err(error)) => Some(error),
                        Err(_) => Some("The workflow cancellation request timed out. Its termination is unconfirmed.".into()),
                    }
                };
                events.unavailable = Some(UnavailableInteraction {
                    cancellation_requested: cancellation_error.is_none(),
                    cancellation_error,
                });
                return Ok(events);
            }
        }
    }
    decoder.finish()?;
    Ok(events)
}

pub(crate) fn interaction_matches_run(run_id: Option<&str>, interaction: &Value) -> bool {
    run_id
        .is_some_and(|run_id| !run_id.is_empty() && interaction["run_id"].as_str() == Some(run_id))
}

pub(super) async fn invoke_ontology(
    session: &NativePlatformSession,
    args: &Value,
    base: &str,
    action: &str,
) -> Result<Value, String> {
    let prerun = session
        .request(
            Method::GET,
            &format!("{base}/actions/{}/prerun", segment(action)),
            None,
        )
        .await?;
    let requirements: Vec<flow_like::flow::compiled::prerun::PrerunOAuthRequirement> =
        serde_json::from_value(
            prerun
                .get("oauth_requirements")
                .cloned()
                .unwrap_or(json!([])),
        )
        .map_err(err)?;
    let oauth_tokens = crate::native_auth::oauth::resolve(
        &session.app_handle,
        session.auth.as_ref(),
        &session.app_id(args)?,
        &requirements,
    )
    .await?;
    let body = json!({"object_refs":args.get("object_refs").cloned().unwrap_or(json!([])),"parameters":args.get("parameters"),"idempotency_key":args.get("idempotency_key"),"profile_id":session.profile.id,"oauth_tokens":oauth_tokens});
    let events = stream(
        session,
        &session.app_id(args)?,
        &format!("{base}/actions/{}/invoke", segment(action)),
        &body,
        None,
    )
    .await?;
    let mut result = events.outcome();
    result["run"] = json!({"run_id":events.run_id,"status":result["run_status"]});
    Ok(result)
}

pub(super) async fn execute(
    session: &NativePlatformSession,
    tool: &str,
    args: &Value,
) -> Result<Value, String> {
    let app_id = session.app_id(args)?;
    if tool == "query_execution_logs" {
        return query_logs(session, args, &app_id).await;
    }
    if tool == "run_board_tests" {
        return run_tests(session, args, &app_id).await;
    }
    execute_run(session, tool, args, &app_id, None).await
}

pub(crate) async fn execute_with_events(
    session: &NativePlatformSession,
    tool: &str,
    args: &Value,
    observer: EventObserver,
) -> Result<Value, String> {
    let app_id = session.app_id(args)?;
    execute_run(session, tool, args, &app_id, Some(observer)).await
}

async fn execute_run(
    session: &NativePlatformSession,
    tool: &str,
    args: &Value,
    app_id: &str,
    observer: Option<EventObserver>,
) -> Result<Value, String> {
    let event_id = if tool == "execute_event" {
        Some(required(args, "event_id", "eventId")?)
    } else {
        None
    };
    let board_id = if event_id.is_none() {
        required(args, "board_id", "boardId")?
    } else {
        String::new()
    };
    let target = event_id
        .clone()
        .unwrap_or(required(args, "node_id", "nodeId").unwrap_or_default());
    if target.is_empty() {
        return Err("execute_node requires node_id.".into());
    }
    let payload = build_payload(&target, args.get("payload"))?;
    let stream_state = boolean(args, "stream_state", "streamState", true);
    let (local, resolved_event) =
        local_execution_target(session, app_id, &board_id, event_id.as_deref()).await?;
    let (metadata, events) = if local {
        let refuse_interactions = interaction_unavailable(session.context.as_ref());
        let interaction_ready = Arc::new(tokio::sync::Notify::new());
        let events = Arc::new(Mutex::new(Events {
            observer: observer.clone(),
            refuse_interactions,
            interaction_ready: Some(interaction_ready.clone()),
            ..Default::default()
        }));
        let output = events.clone();
        let channel = Channel::new(move |body| {
            if let InvokeResponseBody::Json(text) = body {
                if let Ok(value) = serde_json::from_str::<Value>(&text) {
                    output.lock().unwrap_or_else(|e| e.into_inner()).push(value);
                }
            }
            Ok(())
        });
        session.assert_current_profile().await?;
        // Cancelling one workflow must leave its owning FlowPilot turn available for other work.
        let cancellation = execution_cancellation().child_token();
        let _cancel_on_drop = cancellation.clone().drop_guard();
        let work = run::execute_flowpilot(
            session.app_handle.clone(),
            app_id.into(),
            board_id.clone(),
            event_id.clone(),
            resolved_event,
            payload,
            channel,
            session.auth.clone(),
            session.user_profile.clone(),
            cancellation.clone(),
            stream_state,
        );
        // The owned worker finishes cancellation and releases the runtime registry after this
        // tool returns unavailable. Dropping its caller must not skip that cleanup.
        let work = tokio::spawn(with_execution_profile(session.profile.id.clone(), work));
        await_local_execution(
            work,
            events,
            interaction_ready,
            refuse_interactions,
            cancellation,
        )
        .await?
    } else {
        let path = match event_id.as_deref() {
            Some(event) => format!("apps/{}/events/{}/invoke", segment(app_id), segment(event)),
            None => format!(
                "apps/{}/board/{}/invoke",
                segment(app_id),
                segment(&board_id)
            ),
        };
        let prerun = session
            .request(Method::GET, &path.replace("/invoke", "/prerun"), None)
            .await?;
        let requirements: Vec<flow_like::flow::compiled::prerun::PrerunOAuthRequirement> =
            serde_json::from_value(
                prerun
                    .get("oauth_requirements")
                    .cloned()
                    .unwrap_or(json!([])),
            )
            .map_err(err)?;
        let oauth_tokens = crate::native_auth::oauth::resolve(
            &session.app_handle,
            session.auth.as_ref(),
            app_id,
            &requirements,
        )
        .await?;
        let body = json!({"id":payload.id,"node_id":payload.id,"payload":payload.payload,"runtime_variables":payload.runtime_variables,"stream_state":stream_state,"profile_id":session.profile.id,"oauth_tokens":oauth_tokens});
        let events = stream(session, app_id, &path, &body, observer).await?;
        (None, events)
    };
    let mut result = events.outcome();
    if events.unavailable.is_some() {
        result["tool"] = json!(tool);
    }
    let run_id = metadata
        .as_ref()
        .map(|m| m.run_id.clone())
        .or(events.run_id);
    let details = if tool == "execute_event" {
        json!({"app_id":app_id,"event_id":event_id,"run_id":run_id,"metadata":compact_metadata(metadata.as_ref()),"log_count":events.count,"logs":events.tail})
    } else {
        json!({"app_id":app_id,"board_id":board_id,"node_id":target,"run_id":run_id,"metadata":compact_metadata(metadata.as_ref()),"live_event_count":events.count,"live_events":events.tail})
    };
    result
        .as_object_mut()
        .expect("outcome object")
        .extend(details.as_object().expect("details object").clone());
    Ok(result)
}

async fn local_execution_target(
    session: &NativePlatformSession,
    app_id: &str,
    board_id: &str,
    event_id: Option<&str>,
) -> Result<(bool, Option<Event>), String> {
    if session.is_local(app_id).await? {
        return Ok((true, None));
    }
    let state = TauriFlowLikeState::construct(&session.app_handle)
        .await
        .map_err(err)?;
    let Ok(app) = App::load(app_id.into(), state).await else {
        return Ok((false, None));
    };
    let resolved_event = match event_id {
        Some(id) => {
            let local = app.get_event(id, None).await.ok();
            let remote = session
                .request(
                    Method::GET,
                    &format!("apps/{}/events/{}", segment(app_id), segment(id)),
                    None,
                )
                .await;
            let event = match remote {
                Ok(value) => {
                    let remote: Event = serde_json::from_value(value)
                        .map_err(|_| "The hub returned an invalid Event record.")?;
                    if remote.id != id {
                        return Err("The hub returned a different Event than requested.".into());
                    }
                    select_event_snapshot(local, remote)
                }
                Err(error) if temporary_event_read_failure(&error) => match local {
                    Some(event) => event,
                    None => return Err(error),
                },
                Err(error) => return Err(error),
            };
            if event.execution_mode == EventExecutionMode::Remote {
                return Ok((false, None));
            }
            Some(event)
        }
        None => None,
    };
    let (board_id, version) = resolved_event
        .as_ref()
        .map(|event| (event.board_id.as_str(), event.board_version))
        .unwrap_or((board_id, None));
    // Select the server before starting if this device has no executable board. Once
    // dispatch starts, an error must never retry a mutation on another runtime.
    let local = app.open_board(board_id.into(), None, version).await.is_ok();
    Ok((local, resolved_event))
}

fn select_event_snapshot(local: Option<Event>, remote: Event) -> Event {
    match local {
        Some(local) if local.updated_at > remote.updated_at => local,
        _ => remote,
    }
}

fn temporary_event_read_failure(error: &str) -> bool {
    error == "The hub request could not be completed."
        || error == "The hub response was interrupted."
        || error.starts_with("The hub returned HTTP 5")
}

fn build_payload(target: &str, value: Option<&Value>) -> Result<RunPayload, String> {
    // Match buildRunPayload: callers may supply the execution envelope itself.
    let payload = match value {
        Some(Value::Object(object))
            if object.contains_key("payload") || object.contains_key("runtime_variables") =>
        {
            let mut object = object.clone();
            object.insert("id".into(), json!(target));
            Value::Object(object)
        }
        Some(Value::Object(_)) => json!({"id":target,"payload":value}),
        _ => json!({"id":target,"payload":{}}),
    };
    serde_json::from_value(payload).map_err(err)
}

fn compact_metadata(metadata: Option<&LogMeta>) -> Value {
    let Some(metadata) = metadata else {
        return Value::Null;
    };
    let mut value = serde_json::to_value(metadata).unwrap_or(Value::Null);
    if let Some(object) = value.as_object_mut() {
        let bytes = object
            .get("payload")
            .and_then(Value::as_array)
            .map_or(0, Vec::len);
        object.insert("payload".into(), json!([]));
        object.insert("payload_bytes".into(), json!(bytes));
        let truncated = object
            .get("nodes")
            .and_then(Value::as_array)
            .is_some_and(|nodes| nodes.len() > 100);
        if let Some(nodes) = object.get_mut("nodes").and_then(Value::as_array_mut) {
            nodes.truncate(100);
        }
        object.insert("nodes_truncated".into(), json!(truncated));
    }
    value
}

async fn metadata(
    session: &NativePlatformSession,
    args: &Value,
    app_id: &str,
    board_id: &str,
    run_id: &str,
) -> Result<LogMeta, String> {
    if let Some(value) = args
        .get("run_metadata")
        .or_else(|| args.get("runMetadata"))
        .filter(|v| !v.is_null())
    {
        let meta: LogMeta = serde_json::from_value(value.clone()).map_err(err)?;
        if meta.app_id != app_id || meta.board_id != board_id || meta.run_id != run_id {
            return Err("run_metadata does not match app_id, board_id, and run_id.".into());
        }
        return Ok(meta);
    }
    let offline = session.is_local(app_id).await?;
    for remote in [false, true] {
        if remote && offline {
            break;
        }
        for offset in (0..1000).step_by(100) {
            let rows = if !remote {
                run::list_runs(
                    session.app_handle.clone(),
                    app_id.into(),
                    board_id.into(),
                    None,
                    None,
                    None,
                    None,
                    Some(100),
                    Some(offset),
                    None,
                    Some(false),
                )
                .await
                .map_err(err)?
            } else {
                let value = session
                    .request(
                        Method::GET,
                        &format!(
                            "apps/{}/board/{}/runs?offset={offset}&limit=100",
                            segment(app_id),
                            segment(board_id)
                        ),
                        None,
                    )
                    .await?;
                serde_json::from_value(value).map_err(err)?
            };
            let count = rows.len();
            if let Some(meta) = rows.into_iter().find(|m| m.run_id == run_id) {
                return Ok(meta);
            }
            if count < 100 {
                break;
            }
        }
    }
    Err(format!(
        "Run '{run_id}' was not found on board '{board_id}' (searched the latest 1000 runs)."
    ))
}

async fn logs(
    session: &NativePlatformSession,
    meta: &LogMeta,
    filter: &str,
    offset: usize,
    limit: usize,
) -> Result<Vec<Value>, String> {
    if !meta.is_remote {
        let logs = run::query_run(
            session.app_handle.clone(),
            meta.clone(),
            filter.into(),
            Some(limit),
            Some(offset),
        )
        .await
        .map_err(err)?;
        logs.into_iter()
            .map(|log| serde_json::to_value(log).map_err(err))
            .collect()
    } else {
        let value = session
            .request(
                Method::GET,
                &format!(
                    "apps/{}/board/{}/logs?run_id={}&query={}&offset={offset}&limit={limit}",
                    segment(&meta.app_id),
                    segment(&meta.board_id),
                    segment(&meta.run_id),
                    segment(filter)
                ),
                None,
            )
            .await?;
        serde_json::from_value(value).map_err(err)
    }
}
fn level(log: &Value) -> u64 {
    log.get("log_level")
        .and_then(Value::as_u64)
        .unwrap_or_else(|| {
            match log
                .get("log_level")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_lowercase()
                .as_str()
            {
                "warn" => 2,
                "error" => 3,
                "fatal" => 4,
                _ => 0,
            }
        })
}
fn compact_logs(logs: Vec<Value>) -> Vec<Value> {
    logs.into_iter()
        .map(|log| {
            let mut object = serde_json::Map::new();
            for key in [
                "node_id",
                "log_level",
                "message",
                "operation_id",
                "start",
                "end",
                "stats",
            ] {
                if let Some(value) = log.get(key) {
                    object.insert(key.into(), value.clone());
                }
            }
            Value::Object(object)
        })
        .collect()
}
async fn query_logs(
    session: &NativePlatformSession,
    args: &Value,
    app_id: &str,
) -> Result<Value, String> {
    let board = required(args, "board_id", "boardId")?;
    let run_id = required(args, "run_id", "runId")?;
    let meta = metadata(session, args, app_id, &board, &run_id).await?;
    let limit = limit(args, "limit", "limit", 100, 100);
    let offset = args.get("offset").and_then(Value::as_u64).unwrap_or(0) as usize;
    let filter = string(args, "filter", "filter").unwrap_or("");
    let logs = logs(session, &meta, filter, offset, limit).await?;
    let warning = logs.iter().filter(|log| level(log) == 2).count();
    let error = logs.iter().filter(|log| level(log) == 3).count();
    let fatal = logs.iter().filter(|log| level(log) == 4).count();
    let more = logs.len() == limit;
    Ok(
        json!({"status":"ok","app_id":app_id,"board_id":board,"run_id":run_id,"filter":filter,"offset":offset,"limit":limit,"log_count":logs.len(),"has_more":more,"metadata":compact_metadata(Some(&meta)),"logs":compact_logs(logs),"verification":{"scope":"returned_page","complete":!more,"warning_count":warning,"error_count":error,"fatal_count":fatal,"has_errors":if error+fatal>0 {Some(true)} else if more {None} else {Some(false)}}}),
    )
}

async fn run_tests(
    session: &NativePlatformSession,
    args: &Value,
    app_id: &str,
) -> Result<Value, String> {
    let board_id = required(args, "board_id", "boardId")?;
    let state = TauriFlowLikeState::construct(&session.app_handle)
        .await
        .map_err(err)?;
    let app = App::load(app_id.into(), state).await.map_err(err)?;
    let board = app
        .open_board(board_id.clone(), None, None)
        .await
        .map_err(err)?;
    let board = board.snapshot();
    let filter = string(args, "filter", "filter")
        .unwrap_or("")
        .trim()
        .to_lowercase();
    let tests = discover_board_tests(&board)
        .into_iter()
        .filter(|test| test.alias.to_lowercase().contains(&filter))
        .collect::<Vec<_>>();
    drop(board);
    if tests.is_empty() {
        return Ok(
            json!({"status":"no_tests","app_id":app_id,"board_id":board_id,"message":"No test events found. A board test is a simple event whose name starts with `test` and checks outcomes with test::assert."}),
        );
    }
    let max = limit(args, "max_tests", "maxTests", 20, 20);
    let mut results = Vec::new();
    let mut passed = 0;
    for test in tests.iter().take(max) {
        session.assert_current_profile().await?;
        let started = std::time::Instant::now();
        let result=execute_run(session,"execute_node",&json!({"app_id":app_id,"board_id":board_id,"node_id":test.node_id,"payload":{},"stream_state":false}),app_id,None).await;
        let mut evidence = RunGradeEvidence::default();
        let mut assert_logs = Vec::new();
        let mut error_logs = Vec::new();
        let mut run_id = None;
        match result {
            Ok(value) => {
                if value["status"] != "ok" {
                    evidence.execution_error = Some(
                        value["error"]
                            .as_str()
                            .unwrap_or("Workflow execution did not complete successfully.")
                            .to_owned(),
                    );
                }
                run_id = value
                    .get("run_id")
                    .and_then(Value::as_str)
                    .map(str::to_owned);
                if let Some(id) = &run_id {
                    if let Ok(meta) = metadata(
                        session,
                        &json!({"run_metadata":value.get("metadata")}),
                        app_id,
                        &board_id,
                        id,
                    )
                    .await
                    {
                        evidence.metadata_present = true;
                        assert_logs = logs(session, &meta, "message LIKE 'ASSERT_%'", 0, 100)
                            .await
                            .unwrap_or_default();
                        error_logs = logs(session, &meta, "log_level >= 3", 0, 10)
                            .await
                            .unwrap_or_default();
                        evidence.assert_logs = assert_logs
                            .iter()
                            .filter_map(|v| {
                                v.get("message").and_then(Value::as_str).map(str::to_owned)
                            })
                            .collect();
                        evidence.error_logs = error_logs
                            .iter()
                            .filter_map(|v| {
                                v.get("message").and_then(Value::as_str).map(str::to_owned)
                            })
                            .collect();
                    }
                }
            }
            Err(error) => evidence.execution_error = Some(error),
        }
        let grade = grade_run(evidence);
        if grade.verdict.as_str() == "pass" {
            passed += 1;
        }
        assert_logs.truncate(25);
        results.push(json!({"node_id":test.node_id,"event_name":test.alias,"run_id":run_id,"verdict":grade.verdict,"assert_ok":grade.assert_ok,"assert_fail":grade.assert_fail,"assertions":compact_logs(assert_logs),"error_logs":compact_logs(error_logs),"execution_error":grade.execution_error,"duration_ms":started.elapsed().as_millis()}));
    }
    Ok(
        json!({"status":"ok","app_id":app_id,"board_id":board_id,"test_count":tests.len(),"executed":results.len(),"skipped":tests.len()-results.len(),"passed":passed,"failed":results.len()-passed,"tests":results}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remote_input_cannot_cancel_a_different_observed_workflow() {
        assert!(!interaction_matches_run(None, &json!({"run_id":"run"})));
        assert!(!interaction_matches_run(Some(""), &json!({"run_id":""})));
        assert!(interaction_matches_run(
            Some("run"),
            &json!({"run_id":"run"})
        ));
        assert!(!interaction_matches_run(
            Some("run"),
            &json!({"run_id":"other"})
        ));
        assert!(!interaction_matches_run(
            Some("run"),
            &json!({"id":"question"})
        ));
    }

    #[tokio::test]
    async fn background_input_returns_without_waiting_and_cancels_only_its_workflow() {
        let parent = flow_like_types::tokio_util::sync::CancellationToken::new();
        let cancellation = parent.child_token();
        let ready = Arc::new(tokio::sync::Notify::new());
        let events = Arc::new(Mutex::new(Events {
            refuse_interactions: true,
            interaction_ready: Some(ready.clone()),
            ..Default::default()
        }));
        let output = events.clone();
        let worker_cancellation = cancellation.clone();
        let (cleaned, cleanup) = tokio::sync::oneshot::channel();
        let work = tokio::spawn(async move {
            output
                .lock()
                .unwrap()
                .push(json!({"event_type":"run_initiated","payload":{"run_id":"workflow"}}));
            output.lock().unwrap().push(json!({"event_type":"interaction_request","payload":{"id":"question","channel":{"token":"private"}}}));
            worker_cancellation.cancelled().await;
            let _ = cleaned.send(());
            Ok::<_, String>(Some("finished cleanup"))
        });
        let (metadata, events) = tokio::time::timeout(
            std::time::Duration::from_secs(1),
            await_local_execution(work, events, ready, true, cancellation),
        )
        .await
        .expect("the tool must not wait for a human")
        .unwrap();
        assert!(metadata.is_none());
        assert!(!parent.is_cancelled());
        assert_eq!(events.run_id.as_deref(), Some("workflow"));
        assert_eq!(events.outcome()["status"], "unavailable");
        assert_eq!(events.outcome()["approved"], false);
        assert_eq!(events.outcome()["cancellation_requested"], true);
        tokio::time::timeout(std::time::Duration::from_secs(1), cleanup)
            .await
            .unwrap()
            .unwrap();
    }

    #[tokio::test]
    async fn a_completed_batch_cannot_hide_its_unanswered_input_request() {
        let cancellation = flow_like_types::tokio_util::sync::CancellationToken::new();
        let ready = Arc::new(tokio::sync::Notify::new());
        let events = Arc::new(Mutex::new(Events {
            refuse_interactions: true,
            interaction_ready: Some(ready.clone()),
            ..Default::default()
        }));
        events.lock().unwrap().push(json!([
            {"event_type":"interaction_request","payload":{"id":"question"}},
            {"event_type":"completed","payload":{"status":"completed"}}
        ]));
        let work = tokio::spawn(async { Ok::<_, String>(Some("done")) });
        let (metadata, events) = await_local_execution(work, events, ready, true, cancellation)
            .await
            .unwrap();
        assert!(metadata.is_none());
        assert_eq!(events.outcome()["status"], "unavailable");
        assert_eq!(events.outcome()["run_status"], "interaction_unavailable");
    }

    #[tokio::test]
    async fn interactive_runtime_keeps_waiting_for_its_normal_result() {
        let cancellation = flow_like_types::tokio_util::sync::CancellationToken::new();
        let ready = Arc::new(tokio::sync::Notify::new());
        let events = Arc::new(Mutex::new(Events::default()));
        let output = events.clone();
        let work = tokio::spawn(async move {
            output
                .lock()
                .unwrap()
                .push(json!({"event_type":"interaction_request","payload":{"id":"question"}}));
            tokio::task::yield_now().await;
            output
                .lock()
                .unwrap()
                .push(json!({"event_type":"completed","payload":{"status":"completed"}}));
            Ok::<_, String>(Some("answered"))
        });
        let (metadata, events) = await_local_execution(work, events, ready, false, cancellation)
            .await
            .unwrap();
        assert_eq!(metadata, Some("answered"));
        assert_eq!(events.outcome()["status"], "ok");
    }

    #[test]
    fn interaction_capabilities_stay_private_and_capture_survives_log_compaction() {
        let observed = Arc::new(Mutex::new(Vec::new()));
        let recorded = observed.clone();
        let mut events = Events {
            refuse_interactions: true,
            observer: Some(Arc::new(move |event| {
                recorded.lock().unwrap().push(event.clone())
            })),
            ..Default::default()
        };
        events.push(json!({"event_type":"interaction_request","payload":{"id":"first","channel":{"token":"private"}}}));
        assert_eq!(
            observed.lock().unwrap()[0]["payload"]["channel"]["token"],
            "private"
        );
        assert!(events.tail[0]["payload"].get("channel").is_none());
        events.push(json!({"event_type":"interaction_request","payload":{"id":"second"}}));
        for _ in 0..250 {
            events.push(json!({"event_type":"log","payload":{}}));
        }
        assert_eq!(events.pending_interaction.as_ref().unwrap()["id"], "first");
    }

    #[test]
    fn real_terminal_events_report_success_failure_and_cancellation() {
        for (terminal, expected) in [
            ("completed", "ok"),
            ("failed", "error"),
            ("cancelled", "cancelled"),
        ] {
            let mut events = Events::default();
            events.push(json!({"event_type":"run_initiated","payload":{"run_id":"parent"}}));
            events.push(json!({"event_type":"run_initiated","payload":{"run_id":"child"}}));
            events.push(
                json!({"event_type":"completed","payload":{"run_id":"child","status":"failed"}}),
            );
            assert_eq!(events.outcome()["run_status"], "interrupted");
            events.push(
                json!({"event_type":"completed","payload":{"run_id":"parent","status":terminal}}),
            );
            for _ in 0..250 {
                events.push(json!({"event_type":"log","payload":{}}));
            }
            assert_eq!(events.outcome()["status"], expected);
            assert_eq!(events.outcome()["run_status"], terminal);
            assert_eq!(events.run_id.as_deref(), Some("parent"));
        }
        assert_eq!(Events::default().outcome()["code"], "execution_incomplete");
    }

    fn snapshot(updated: u64, mode: &str, board: &str) -> Event {
        serde_json::from_value(json!({
            "id":"event","name":"Event","description":"","board_id":board,
            "node_id":"entry","variables":{},"config":[],"active":true,"priority":0,
            "event_type":"quick_action","event_version":[0,0,0],
            "created_at":{"secs_since_epoch":1,"nanos_since_epoch":0},
            "updated_at":{"secs_since_epoch":updated,"nanos_since_epoch":0},
            "execution_mode":mode
        }))
        .unwrap()
    }

    #[test]
    fn hosted_event_refresh_uses_current_mode_and_target_but_keeps_newer_local_edits() {
        let selected = select_event_snapshot(
            Some(snapshot(1, "Local", "old")),
            snapshot(2, "Remote", "published"),
        );
        assert_eq!(selected.execution_mode, EventExecutionMode::Remote);
        assert_eq!(selected.board_id, "published");
        let selected = select_event_snapshot(
            Some(snapshot(3, "Local", "draft")),
            snapshot(2, "Remote", "published"),
        );
        assert_eq!(selected.execution_mode, EventExecutionMode::Local);
        assert_eq!(selected.board_id, "draft");
        let selected = select_event_snapshot(
            Some(snapshot(2, "Local", "old")),
            snapshot(2, "Remote", "published"),
        );
        assert_eq!(selected.execution_mode, EventExecutionMode::Remote);
        for status in [401, 403, 404, 410] {
            assert!(!temporary_event_read_failure(&format!(
                "The hub returned HTTP {status} for this tool request."
            )));
        }
        assert!(temporary_event_read_failure(
            "The hub request could not be completed."
        ));
        assert!(temporary_event_read_failure(
            "The hub returned HTTP 503 for this tool request."
        ));
    }
    #[test]
    fn sse_handles_chunk_and_unicode_boundaries() {
        let input = "data: {\"name\":\"café\"}\r\n\r\n: heartbeat\n\ndata: [DONE]\n\n";
        for size in 1..input.len() {
            let mut decoder = SseDecoder::default();
            let mut result = Vec::new();
            for chunk in input.as_bytes().chunks(size) {
                result.extend(decoder.feed(chunk).unwrap());
            }
            decoder.finish().unwrap();
            assert_eq!(result, vec![json!({"name":"café"})]);
        }
    }
    #[test]
    fn incomplete_stream_is_not_success() {
        let mut decoder = SseDecoder::default();
        decoder.feed(b"data: {\"x\":1}").unwrap();
        assert!(decoder.finish().is_err());
    }
    #[test]
    fn observer_receives_all_events_before_diagnostic_compaction() {
        let observed = Arc::new(Mutex::new(Vec::new()));
        let output = observed.clone();
        let mut events = Events {
            observer: Some(Arc::new(move |event| {
                output.lock().unwrap().push(event.clone())
            })),
            ..Default::default()
        };
        let full = json!({"event_type":"chat_event","payload":{"text":"x".repeat(9000)}});
        for _ in 0..300 {
            events.push(full.clone());
        }
        assert_eq!(events.count, 300);
        assert_eq!(events.tail.len(), 200);
        assert_eq!(events.tail[0]["payload_truncated"], true);
        assert_eq!(observed.lock().unwrap().len(), 300);
        assert_eq!(observed.lock().unwrap()[299], full);
    }
    #[test]
    fn sse_rejects_oversized_frames_with_or_without_a_delimiter() {
        let oversized = vec![b'x'; 1024 * 1024 + 1];
        assert!(SseDecoder::default().feed(&oversized).is_err());
        let mut complete = oversized;
        complete.extend_from_slice(b"\n\n");
        assert!(SseDecoder::default().feed(&complete).is_err());
    }
    #[test]
    fn payload_envelope_cannot_replace_the_entry() {
        let payload =
            build_payload("entry", Some(&json!({"id":"other","payload":{"a":1}}))).unwrap();
        assert_eq!(payload.id, "entry");
        assert_eq!(payload.payload, Some(json!({"a":1})));
    }
}
