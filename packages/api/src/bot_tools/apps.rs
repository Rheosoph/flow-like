//! App tools of a headless server turn (room and inbox turns): the bot lists the apps it may
//! use, reads their interfaces, runs their Events, chats and MCP tools, reads their tables and
//! ontologies and changes its workspace's tables. Every call goes through the route handlers as
//! the bot, so the bot's own rights, quotas and run records apply.

mod chat;
mod data;
mod mcp;

use std::time::Duration;

use axum::{
    Extension, Json,
    extract::{Path, Query, State},
    http::HeaderMap,
    response::Response,
};
use flow_like::flow::{copilot::BotRunContext, event::Event};
use flow_like_storage::contracts::database::DatabaseSelector;
use flow_like_types::tokio;
use futures::StreamExt;
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
use serde_json::{Map, Value, json};

use crate::{
    entity::{app, meta},
    error::ApiError,
    middleware::jwt::{AppUser, BotUser},
    permission::role_permission::RolePermissions,
    routes::app::{
        db::{
            ScopeParams, ScopedPaginationParams, TableListParams, db_count::db_count,
            db_list::list_items, db_query::query_table, get_db_schema::get_db_schema,
            get_indices::get_db_indices, list_tables::list_tables,
        },
        events::{
            current_inputs::event_with_current_inputs,
            db::{
                filter_event_secrets, get_event_from_db_opt, get_events_for_app,
                is_listed_event_type, is_user_facing_event,
            },
            invoke_event::{InvokeEventQuery, InvokeEventRequest, invoke_event},
        },
    },
    state::AppState,
};

const LIST_APPS: &str = "list_apps";
const DESCRIBE_APP_INTERFACE: &str = "describe_app_interface";
const GET_APP_DETAIL: &str = "get_app_detail";
const CALL_APP_EVENT: &str = "call_app_event";
const CALL_APP_CHAT: &str = "call_app_chat";
const DATABASE_TOOL: &str = "database_tool";
const GRAPH_QUERY_TOOL: &str = "graph_query_tool";

const APP_TOOLS: [&str; 7] = [
    LIST_APPS,
    DESCRIBE_APP_INTERFACE,
    GET_APP_DETAIL,
    CALL_APP_EVENT,
    CALL_APP_CHAT,
    DATABASE_TOOL,
    GRAPH_QUERY_TOOL,
];

const MAX_LISTED_APPS: usize = 250;
const EVENT_READ_CONCURRENCY: usize = 8;
const MAX_CONFIG_CHARS: usize = 12_000;
const MAX_LOG_EVENTS: usize = 50;
const MAX_LOG_EVENT_BYTES: usize = 8_000;
const MAX_FRAME_BYTES: usize = 1024 * 1024;
const EVENT_RUN_TIMEOUT: Duration = Duration::from_secs(570);
const DEFAULT_ROWS: u64 = 50;
const DEFAULT_SAMPLE_ROWS: u64 = 10;
const MAX_ROWS: u64 = 200;
const MAX_OFFSET: u64 = 100_000;
const META_LANGUAGE: &str = "en";

pub(crate) fn is_app_tool(tool: &str) -> bool {
    APP_TOOLS.contains(&tool)
}

type ToolResult = Result<Value, String>;

/// One turn's app tools, acting as the bot.
pub(crate) struct AppTools {
    state: AppState,
    principal: BotUser,
    bot: BotRunContext,
}

fn text<'a>(args: &'a Value, key: &str) -> Option<&'a str> {
    args.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

fn required<'a>(args: &'a Value, key: &str) -> Result<&'a str, String> {
    text(args, key).ok_or_else(|| format!("This operation requires {key}."))
}

fn bounded(args: &Value, key: &str, default: u64, max: u64) -> u64 {
    args.get(key)
        .and_then(Value::as_u64)
        .unwrap_or(default)
        .min(max)
}

fn failed(error: ApiError) -> String {
    error.to_string()
}

fn event_kind(event: &Event) -> &'static str {
    let page = event.default_page_id.as_deref().unwrap_or_default();
    if event.event_type == "simple_chat" {
        "chat"
    } else if !page.is_empty() && page.trim() == page {
        "page"
    } else if event.event_type.trim().eq_ignore_ascii_case("page") || !page.trim().is_empty() {
        "unavailable"
    } else {
        "headless"
    }
}

fn is_mcp(event: &Event) -> bool {
    event.event_type.trim().eq_ignore_ascii_case("mcp")
}

fn event_summary(event: &Event) -> Value {
    let kind = event_kind(event);
    let mut summary = json!({
        "id": event.id,
        "name": event.name,
        "description": event.description,
        "event_type": event.event_type,
        "kind": kind,
    });
    match kind {
        "chat" => summary["consumer_tool"] = json!("call_app_chat"),
        "page" => {
            summary["consumer_tool"] = json!("open_app_page");
            summary["page_id"] = json!(event.default_page_id);
        }
        "headless" => summary["consumer_tool"] = json!(CALL_APP_EVENT),
        _ => summary["interface_error"] = json!("page_target_missing"),
    }
    if let Some(route) = event
        .route
        .as_deref()
        .filter(|route| !route.trim().is_empty())
    {
        summary["route"] = json!(route);
    }
    summary
}

fn event_config(event: &Event) -> Value {
    let config = serde_json::from_slice::<Value>(&event.config).unwrap_or(Value::Null);
    let serialized = config.to_string();
    if serialized.chars().count() <= MAX_CONFIG_CHARS {
        return config;
    }
    json!({
        "truncated": true,
        "truncated_note": "Context preview only; the full configuration is larger.",
        "preview": serialized.chars().take(MAX_CONFIG_CHARS).collect::<String>(),
    })
}

/// Splits a server-sent event stream into the JSON envelopes of its `data:` lines.
#[derive(Default)]
struct SseDecoder {
    buffer: Vec<u8>,
}

impl SseDecoder {
    fn push(&mut self, chunk: &[u8]) -> Result<Vec<Value>, String> {
        self.buffer.extend_from_slice(chunk);
        let mut envelopes = Vec::new();
        while let Some((end, separator)) = frame_end(&self.buffer) {
            let frame: Vec<u8> = self.buffer.drain(..end + separator).collect();
            envelopes.extend(frame_envelopes(&frame[..end]));
        }
        if self.buffer.len() > MAX_FRAME_BYTES {
            return Err("The run sent an event larger than this tool reads.".to_string());
        }
        Ok(envelopes)
    }
}

fn frame_end(buffer: &[u8]) -> Option<(usize, usize)> {
    let unix = buffer.windows(2).position(|pair| pair == b"\n\n");
    let dos = buffer.windows(4).position(|quad| quad == b"\r\n\r\n");
    match (unix, dos) {
        (Some(unix), Some(dos)) if dos < unix => Some((dos, 4)),
        (Some(unix), _) => Some((unix, 2)),
        (None, Some(dos)) => Some((dos, 4)),
        (None, None) => None,
    }
}

fn frame_envelopes(frame: &[u8]) -> Vec<Value> {
    let frame = String::from_utf8_lossy(frame);
    let data: Vec<&str> = frame
        .lines()
        .filter_map(|line| line.strip_prefix("data:"))
        .map(|line| line.strip_prefix(' ').unwrap_or(line))
        .collect();
    if data.is_empty() {
        return Vec::new();
    }
    match serde_json::from_str::<Value>(&data.join("\n")) {
        Ok(Value::Array(envelopes)) => envelopes,
        Ok(envelope @ Value::Object(_)) => vec![envelope],
        _ => Vec::new(),
    }
}

/// What a run's event stream said, folded as it arrives.
#[derive(Default)]
struct RunFold {
    run_id: Option<String>,
    terminal: Option<String>,
    result: Option<Value>,
    error: Option<Value>,
    count: usize,
    tail: std::collections::VecDeque<Value>,
}

impl RunFold {
    fn push(&mut self, envelope: Value) {
        let event_type = envelope["event_type"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        let payload = &envelope["payload"];
        match event_type.as_str() {
            "run_initiated" if self.run_id.is_none() => {
                self.run_id = payload["run_id"].as_str().map(str::to_string);
            }
            "generic_result" if self.result.is_none() => self.result = Some(payload.clone()),
            "completed" | "execution_completed" => {
                self.terminal = Some(
                    payload["status"]
                        .as_str()
                        .unwrap_or("completed")
                        .to_string(),
                );
            }
            "error" => self.error = Some(payload.clone()),
            _ => {}
        }
        self.count += 1;
        let kept = if envelope.to_string().len() > MAX_LOG_EVENT_BYTES {
            json!({ "event_type": event_type, "payload_truncated": true })
        } else {
            envelope
        };
        if self.tail.len() == MAX_LOG_EVENTS {
            self.tail.pop_front();
        }
        self.tail.push_back(kept);
    }

    fn finished(&self) -> bool {
        self.terminal.is_some()
    }

    fn outcome(self, timed_out: bool) -> Value {
        let run_status = match (&self.terminal, timed_out) {
            (Some(status), _) => status.clone(),
            (None, true) => "timeout".to_string(),
            (None, false) if self.error.is_some() => "failed".to_string(),
            (None, false) => "interrupted".to_string(),
        };
        let status = match run_status.as_str() {
            "completed" | "success" => "ok",
            "cancelled" | "canceled" | "stopped" => "cancelled",
            "timeout" => "timeout",
            _ => "error",
        };
        let mut outcome = json!({
            "status": status,
            "run_status": run_status,
            "run_id": self.run_id,
            "log_count": self.count,
            "logs": self.tail,
        });
        if let Some(result) = self.result {
            outcome["result"] = result;
        }
        if let Some(error) = self.error {
            outcome["run_error"] = error;
        }
        match status {
            "ok" => {}
            "timeout" => {
                outcome["code"] = json!("execution_timeout");
                outcome["error"] = json!(
                    "The run did not finish in the time this tool waits. It may still be running; do not start it again without checking."
                );
            }
            _ => {
                outcome["code"] = json!(if self.terminal.is_some() {
                    "execution_unsuccessful"
                } else {
                    "execution_incomplete"
                });
                outcome["error"] = json!(format!(
                    "Workflow execution ended with status '{run_status}'. Inspect its run and logs before retrying actions."
                ));
            }
        }
        outcome
    }
}

async fn read_run(response: Response) -> ToolResult {
    let streamed = response
        .headers()
        .get(axum::http::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.starts_with("text/event-stream"));
    let mut body = response.into_body().into_data_stream();
    if !streamed {
        let mut bytes = Vec::new();
        while let Some(chunk) = body.next().await {
            bytes.extend_from_slice(&chunk.map_err(|error| error.to_string())?);
            if bytes.len() > MAX_FRAME_BYTES {
                return Err("The run's answer is larger than this tool reads.".to_string());
            }
        }
        let answer = serde_json::from_slice::<Value>(&bytes).unwrap_or(Value::Null);
        return Ok(json!({ "status": "ok", "run_status": "started", "run": answer }));
    }
    let mut decoder = SseDecoder::default();
    let mut fold = RunFold::default();
    let reading = async {
        while let Some(chunk) = body.next().await {
            let chunk = chunk.map_err(|error| error.to_string())?;
            for envelope in decoder.push(&chunk)? {
                fold.push(envelope);
            }
            if fold.finished() {
                break;
            }
        }
        Ok::<(), String>(())
    };
    let timed_out = match tokio::time::timeout(EVENT_RUN_TIMEOUT, reading).await {
        Ok(read) => {
            read?;
            false
        }
        Err(_) => true,
    };
    Ok(fold.outcome(timed_out))
}

impl AppTools {
    pub(crate) fn new(state: &AppState, principal: BotUser, bot: &BotRunContext) -> Self {
        Self {
            state: state.clone(),
            principal,
            bot: bot.clone(),
        }
    }

    fn user(&self) -> AppUser {
        AppUser::Bot(self.principal.clone())
    }

    /// Runs `tool` and returns the JSON text the model reads.
    pub(crate) async fn run(&self, tool: &str, args: Value) -> String {
        let result = match tool {
            LIST_APPS => self.list_apps(&args).await,
            DESCRIBE_APP_INTERFACE => self.describe_app_interface(&args).await,
            GET_APP_DETAIL => self.get_app_detail(&args).await,
            CALL_APP_EVENT => self.call_app_event(&args).await,
            CALL_APP_CHAT => self.call_app_chat(&args).await,
            DATABASE_TOOL => self.database(&args).await,
            GRAPH_QUERY_TOOL => self.graph_query(&args).await,
            _ => Err("This host does not run this tool.".to_string()),
        };
        match result {
            Ok(value) => value,
            Err(error) => json!({ "status": "error", "tool": tool, "error": error }),
        }
        .to_string()
    }

    fn app_id<'a>(&self, args: &'a Value) -> Result<&'a str, String> {
        let app_id = required(args, "app_id")?;
        if self.bot.allows_app(app_id) {
            Ok(app_id)
        } else {
            Err(format!("App '{app_id}' is not one this bot may use."))
        }
    }

    /// The active events the bot may see in `app_id`, as the events route lists them.
    async fn events(&self, app_id: &str) -> Result<Vec<Event>, String> {
        let permission = self
            .user()
            .app_permission(app_id, &self.state)
            .await
            .map_err(failed)?;
        if !permission.has_permission(RolePermissions::ListEvents) {
            return Err(format!(
                "This bot may not list the events of app '{app_id}'."
            ));
        }
        let can_read = permission.has_permission(RolePermissions::ReadEvents);
        Ok(get_events_for_app(&self.state.db, app_id)
            .await
            .map_err(|error| error.to_string())?
            .into_iter()
            .filter(|event| {
                event.active
                    && is_listed_event_type(&event.event_type)
                    && (can_read || is_user_facing_event(event))
            })
            .collect())
    }

    async fn event(&self, app_id: &str, event_id: &str) -> Result<Event, String> {
        let permission = self
            .user()
            .app_permission(app_id, &self.state)
            .await
            .map_err(failed)?;
        if !permission.has_permission(RolePermissions::ReadEvents)
            && !permission.has_permission(RolePermissions::ExecuteEvents)
        {
            return Err(format!(
                "This bot may not use the events of app '{app_id}'."
            ));
        }
        get_event_from_db_opt(&self.state.db, event_id, app_id)
            .await
            .map_err(|error| error.to_string())?
            .filter(|event| is_listed_event_type(&event.event_type))
            .ok_or_else(|| format!("Event '{event_id}' not found in app '{app_id}'."))
    }

    async fn metas(&self, app_ids: &[String]) -> Result<Vec<meta::Model>, String> {
        if app_ids.is_empty() {
            return Ok(Vec::new());
        }
        meta::Entity::find()
            .filter(meta::Column::AppId.is_in(app_ids.iter().cloned()))
            .filter(meta::Column::Lang.eq(META_LANGUAGE))
            .all(&self.state.db)
            .await
            .map_err(|error| error.to_string())
    }

    async fn list_apps(&self, args: &Value) -> ToolResult {
        let query = text(args, "query").map(str::to_lowercase);
        let candidates: Vec<String> = super::scope::candidate_apps(&self.bot)
            .into_iter()
            .collect();
        let metas = self.metas(&candidates).await?;
        let meta_of = |app_id: &str| {
            metas
                .iter()
                .find(|meta| meta.app_id.as_deref() == Some(app_id))
        };
        let mut visible: Vec<(String, String, String)> = candidates
            .iter()
            .map(|app_id| {
                let meta = meta_of(app_id);
                (
                    app_id.clone(),
                    meta.map_or_else(|| app_id.clone(), |meta| meta.name.clone()),
                    meta.and_then(|meta| meta.description.clone())
                        .unwrap_or_default(),
                )
            })
            .filter(|(app_id, name, description)| {
                query.as_deref().is_none_or(|query| {
                    [app_id, name, description]
                        .iter()
                        .any(|field| field.to_lowercase().contains(query))
                })
            })
            .collect();
        visible.sort_by(|left, right| left.1.cmp(&right.1));
        let matched = visible.len();
        let truncated = matched > MAX_LISTED_APPS;
        visible.truncate(MAX_LISTED_APPS);
        let apps: Vec<(Value, bool)> = futures::stream::iter(visible)
            .map(|(app_id, name, description)| async move {
                let (events, read) = match self.events(&app_id).await {
                    Ok(events) => (events.iter().map(event_summary).collect::<Vec<_>>(), true),
                    Err(error) => {
                        tracing::debug!(%error, app_id, "[bot_tools] an app's events could not be listed");
                        (Vec::new(), false)
                    }
                };
                (
                    json!({
                        "app_id": app_id,
                        "name": name,
                        "description": description,
                        "events": events,
                        "events_status": if read { "ok" } else { "error" },
                    }),
                    read,
                )
            })
            .buffered(EVENT_READ_CONCURRENCY)
            .collect()
            .await;
        let complete = !truncated && apps.iter().all(|(_, read)| *read);
        let apps: Vec<Value> = apps.into_iter().map(|(app, _)| app).collect();
        let mut listing = json!({
            "status": if complete { "ok" } else { "partial" },
            "complete": complete,
            "total": apps.len(),
            "returned": apps.len(),
            "apps": apps,
        });
        if let Some(query) = query {
            listing["query"] = json!(query);
            listing["matched_total"] = json!(matched);
            listing["profile_total"] = json!(candidates.len());
        }
        if truncated {
            listing["truncated"] = json!(true);
        }
        if !complete {
            listing["note"] = json!(
                "The inventory is incomplete. Narrow it with query, or name the app you need."
            );
        }
        Ok(listing)
    }

    async fn describe_app_interface(&self, args: &Value) -> ToolResult {
        let app_id = self.app_id(args)?;
        let event_id = required(args, "event_id")?;
        let event = self.event(app_id, event_id).await?;
        let event =
            filter_event_secrets(event_with_current_inputs(&self.state, app_id, event).await);
        let mut summary = event_summary(&event);
        if let Some(fields) = summary.as_object_mut() {
            fields.remove("page_id");
        }
        summary["default_page_id"] = json!(event.default_page_id);
        summary["active"] = json!(event.active);
        summary["inputs"] = serde_json::to_value(&event.inputs).unwrap_or_else(|_| json!([]));
        let mut described = json!({
            "status": "ok",
            "event": summary,
            "config": event_config(&event),
        });
        if is_mcp(&event) {
            match self.mcp_tools(app_id, &event).await {
                Ok(mcp) => described["mcp"] = mcp,
                Err(error) => {
                    described["status"] = json!("partial");
                    described["interface_error"] = json!(error);
                }
            }
        }
        Ok(described)
    }

    async fn get_app_detail(&self, args: &Value) -> ToolResult {
        let app_id = self.app_id(args)?;
        self.user()
            .app_permission(app_id, &self.state)
            .await
            .map_err(failed)?;
        let row = app::Entity::find_by_id(app_id)
            .one(&self.state.db)
            .await
            .map_err(|error| error.to_string())?
            .ok_or_else(|| format!("App '{app_id}' not found."))?;
        let app = serde_json::to_value(flow_like::app::App::from(row)).unwrap_or(Value::Null);
        let mut summary = Map::new();
        summary.insert("app_id".into(), json!(app_id));
        for key in [
            "visibility",
            "price",
            "primary_category",
            "avg_rating",
            "rating_count",
        ] {
            if let Some(value) = app.get(key).filter(|value| !value.is_null()) {
                summary.insert(key.into(), value.clone());
            }
        }
        if let Some(meta) = self.metas(&[app_id.to_string()]).await?.into_iter().next() {
            summary.insert("name".into(), json!(meta.name));
            summary.insert("description".into(), json!(meta.description));
            summary.insert("long_description".into(), json!(meta.long_description));
            summary.insert("use_case".into(), json!(meta.use_case));
        }
        if let Ok(events) = self.events(app_id).await {
            summary.insert("event_count".into(), json!(events.len()));
        }
        Ok(json!({ "status": "ok", "app": summary }))
    }

    async fn call_app_event(&self, args: &Value) -> ToolResult {
        let app_id = self.app_id(args)?;
        let event_id = required(args, "event_id")?;
        let event = self.event(app_id, event_id).await?;
        if !event.active {
            return Err(format!(
                "Event '{event_id}' in app '{app_id}' is not active."
            ));
        }
        if event.event_type == "simple_chat" {
            return Err(format!(
                "Event '{event_id}' is a chat interface. Use call_app_chat instead."
            ));
        }
        if is_mcp(&event) {
            return self.call_mcp_tool(app_id, &event, args).await;
        }
        if text(args, "mcp_tool").is_some() {
            return Err(format!(
                "Event '{event_id}' is not an MCP interface; call it without mcp_tool."
            ));
        }
        let payload = match args.get("payload") {
            None | Some(Value::Null) => json!({}),
            Some(payload @ Value::Object(_)) => payload.clone(),
            Some(_) => return Err("payload must be an object.".to_string()),
        };
        let response = invoke_event(
            State(self.state.clone()),
            Extension(self.user()),
            Path((app_id.to_string(), event_id.to_string())),
            Query(InvokeEventQuery::default()),
            HeaderMap::new(),
            Json(InvokeEventRequest {
                version: None,
                payload: Some(payload),
                oauth_tokens: None,
                runtime_variables: None,
                profile_id: None,
                correlation: None,
                page_trigger: None,
            }),
        )
        .await
        .map_err(failed)?;
        let mut outcome = read_run(response).await?;
        outcome["app_id"] = json!(app_id);
        outcome["event_id"] = json!(event_id);
        outcome["event_type"] = json!(event.event_type);
        Ok(outcome)
    }

    async fn database(&self, args: &Value) -> ToolResult {
        let app_id = self.app_id(args)?;
        if args.get("user_scoped").and_then(Value::as_bool) == Some(true) {
            return Err(
                "A bot reads an app's shared tables only; call again without user_scoped."
                    .to_string(),
            );
        }
        let (state, user) = (State(self.state.clone()), Extension(self.user()));
        let operation = text(args, "operation").unwrap_or("list_tables");
        if operation == "list_tables" {
            let tables = list_tables(
                state,
                user,
                Path(app_id.to_string()),
                Query(TableListParams::default()),
            )
            .await
            .map_err(failed)?;
            return Ok(json!({
                "status": "ok",
                "app_id": app_id,
                "project_tables": tables.0,
                "user_tables": [],
            }));
        }
        let table = required(args, "table_name")?;
        let path = || Path((app_id.to_string(), table.to_string()));
        let selector = || Query(DatabaseSelector::default());
        let page = |limit: u64, offset: u64| {
            Query(ScopedPaginationParams {
                limit: Some(limit),
                offset: Some(offset),
                scope: None,
            })
        };
        match operation {
            "describe_table" => {
                let schema = get_db_schema(
                    state.clone(),
                    user.clone(),
                    path(),
                    Query(ScopeParams::default()),
                    selector(),
                )
                .await
                .map_err(failed)?;
                let indices = get_db_indices(
                    state.clone(),
                    user.clone(),
                    path(),
                    Query(ScopeParams::default()),
                    selector(),
                )
                .await
                .map_err(failed)?;
                let count = db_count(
                    state.clone(),
                    user.clone(),
                    path(),
                    Query(ScopeParams::default()),
                    selector(),
                )
                .await
                .map_err(failed)?;
                let mut described = json!({
                    "status": "ok",
                    "table_name": table,
                    "user_scoped": false,
                    "schema": serde_json::to_value(&schema.0).map_err(|error| error.to_string())?,
                    "indices": serde_json::to_value(&indices.0).map_err(|error| error.to_string())?,
                    "row_count": count.0,
                });
                if args.get("include_sample").and_then(Value::as_bool) != Some(false) {
                    let limit = bounded(args, "limit", DEFAULT_SAMPLE_ROWS, MAX_ROWS);
                    let sample = list_items(state, user, path(), page(limit, 0), selector())
                        .await
                        .map_err(failed)?;
                    described["sample"] = json!(sample.0);
                }
                Ok(described)
            }
            "query" => {
                let query = match args.get("query") {
                    None | Some(Value::Null) => json!({}),
                    Some(query @ Value::Object(_)) => query.clone(),
                    Some(_) => return Err("query must be an object.".to_string()),
                };
                let by_sql = text(&query, "sql").is_some();
                let limit = bounded(args, "limit", DEFAULT_ROWS, MAX_ROWS);
                let offset = bounded(args, "offset", 0, MAX_OFFSET);
                let payload = serde_json::from_value(query)
                    .map_err(|error| format!("The query is not valid: {error}"))?;
                let rows = query_table(
                    state,
                    user,
                    path(),
                    page(limit, offset),
                    selector(),
                    Json(payload),
                )
                .await
                .map_err(failed)?
                .0;
                let mut answer = json!({
                    "status": "ok",
                    "table_name": table,
                    "user_scoped": false,
                });
                if by_sql {
                    let total = rows.len();
                    let page: Vec<Value> = rows
                        .into_iter()
                        .skip(offset as usize)
                        .take(limit as usize)
                        .collect();
                    answer["truncated"] = json!(offset as usize + page.len() < total);
                    answer["total_rows"] = json!(total);
                    answer["row_count"] = json!(page.len());
                    answer["rows"] = json!(page);
                } else {
                    answer["row_count"] = json!(rows.len());
                    answer["rows"] = json!(rows);
                }
                Ok(answer)
            }
            other if self.bot.workspace_app_id.as_deref() == Some(app_id) => {
                self.change_workspace_table(app_id, other, args).await
            }
            other => Err(format!(
                "Operation '{other}' changes data, which a bot does only in its workspace's tables; change an app's data through its Events."
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn envelope(event_type: &str, payload: Value) -> Value {
        json!({ "event_type": event_type, "payload": payload })
    }

    #[test]
    fn frames_split_across_chunks_decode_once_complete() {
        let mut decoder = SseDecoder::default();
        assert!(
            decoder
                .push(b"event: log\ndata: {\"event_type\":\"log\",")
                .unwrap()
                .is_empty()
        );
        let decoded = decoder
            .push(b"\"payload\":{\"n\":1}}\n\n: keep-alive\n\ndata: [DONE]\r\n\r\n")
            .unwrap();
        assert_eq!(decoded, vec![envelope("log", json!({ "n": 1 }))]);
    }

    #[test]
    fn an_oversized_frame_is_refused() {
        let mut decoder = SseDecoder::default();
        assert!(decoder.push(&vec![b'a'; MAX_FRAME_BYTES + 1]).is_err());
    }

    #[test]
    fn a_completed_run_reports_its_result_and_a_bounded_log() {
        let mut fold = RunFold::default();
        fold.push(envelope("run_initiated", json!({ "run_id": "run-1" })));
        for index in 0..MAX_LOG_EVENTS + 5 {
            fold.push(envelope("log", json!({ "n": index })));
        }
        fold.push(envelope(
            "log",
            json!({ "blob": "x".repeat(MAX_LOG_EVENT_BYTES) }),
        ));
        fold.push(envelope("generic_result", json!({ "cards": 3 })));
        fold.push(envelope("completed", json!({ "status": "completed" })));
        assert!(fold.finished());
        let outcome = fold.outcome(false);
        assert_eq!(outcome["status"], "ok");
        assert_eq!(outcome["run_id"], "run-1");
        assert_eq!(outcome["result"], json!({ "cards": 3 }));
        assert_eq!(outcome["log_count"], MAX_LOG_EVENTS + 9);
        let logs = outcome["logs"].as_array().unwrap();
        assert_eq!(logs.len(), MAX_LOG_EVENTS);
        assert!(logs.iter().any(|log| log["payload_truncated"] == true));
    }

    #[test]
    fn unfinished_and_failed_runs_are_never_reported_as_ok() {
        let mut failed = RunFold::default();
        failed.push(envelope("completed", json!({ "status": "failed" })));
        let failed = failed.outcome(false);
        assert_eq!(failed["status"], "error");
        assert_eq!(failed["code"], "execution_unsuccessful");

        let interrupted = RunFold::default().outcome(false);
        assert_eq!(interrupted["status"], "error");
        assert_eq!(interrupted["code"], "execution_incomplete");

        let timed_out = RunFold::default().outcome(true);
        assert_eq!(timed_out["status"], "timeout");
        assert_eq!(timed_out["code"], "execution_timeout");
    }

    fn event(event_type: &str, page: Option<&str>) -> Event {
        let mut event: Event = serde_json::from_value(json!({
            "id": "event", "name": "Event", "description": "", "board_id": "board",
            "board_version": null, "node_id": "node", "variables": {}, "config": [],
            "active": true, "canary": null, "priority": 0, "event_type": event_type,
            "notes": null, "event_version": [0, 0, 1],
            "created_at": std::time::SystemTime::UNIX_EPOCH,
            "updated_at": std::time::SystemTime::UNIX_EPOCH,
            "default_page_id": page,
        }))
        .unwrap();
        event.route = Some("/".into());
        event
    }

    #[test]
    fn events_name_the_tool_that_uses_them() {
        let chat = event_summary(&event("simple_chat", None));
        assert_eq!(chat["consumer_tool"], "call_app_chat");
        let page = event_summary(&event("page", Some("home")));
        assert_eq!(
            (&page["kind"], &page["page_id"]),
            (&json!("page"), &json!("home"))
        );
        let broken = event_summary(&event("page", None));
        assert_eq!(broken["interface_error"], "page_target_missing");
        assert!(broken.get("consumer_tool").is_none());
        let headless = event_summary(&event("api", None));
        assert_eq!(headless["consumer_tool"], CALL_APP_EVENT);
        assert_eq!(headless["route"], "/");
    }
}
