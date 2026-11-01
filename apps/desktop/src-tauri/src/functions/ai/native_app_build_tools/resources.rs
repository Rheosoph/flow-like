use super::fingerprint;
use crate::functions::{
    a2ui::{page, route, widget},
    ai::{
        native_platform_tools::{self, NativePlatformSession, err, segment, workflow_entries},
        native_specialist_tools,
    },
    app,
    flow::board as board_service,
};
use flow_like::{
    a2ui::{
        A2UIComponentType, SurfaceComponent,
        widget::{Widget, WidgetAction},
    },
    app::{App, AppStatus},
    flow::{
        ast::{RenderOptions, board_to_flowscript},
        board::{Board, ExecutionStage},
        execution::LogLevel,
    },
};
use reqwest::Method;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeSet, HashSet};

static FOUNDATION_WRITES: std::sync::LazyLock<tokio::sync::Mutex<()>> =
    std::sync::LazyLock::new(Default::default);

#[derive(Clone, Default, Serialize, Deserialize)]
pub(super) struct Readback {
    pub exists: bool,
    pub fingerprint: Option<String>,
    pub artifact: Option<Value>,
    pub issues: Vec<String>,
}
fn snapshot(artifact: Value, issues: Vec<String>) -> Readback {
    Readback {
        exists: true,
        fingerprint: Some(fingerprint("persisted-resource", &artifact)),
        artifact: Some(artifact),
        issues,
    }
}
fn text<'a>(v: &'a Value, key: &str) -> &'a str {
    v.get(key).and_then(Value::as_str).unwrap_or("")
}
fn array<'a>(v: &'a Value, key: &str) -> &'a [Value] {
    v.get(key)
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[])
}
fn encode(value: impl Serialize) -> Result<Value, String> {
    serde_json::to_value(value).map_err(err)
}
pub(super) fn resource_id<'a>(plan: &'a Value, key: &str) -> Result<&'a str, String> {
    array(plan, "resources")
        .iter()
        .find(|r| text(r, "key") == key)
        .map(|r| text(r, "physical_id"))
        .filter(|id| !id.is_empty())
        .ok_or_else(|| format!("Unknown app resource '{key}'."))
}

pub(super) async fn load_app(session: &NativePlatformSession, id: &str) -> Result<App, String> {
    session.require_visible(id)?;
    session.assert_current_profile().await?;
    if session.is_local(id).await? {
        app::get_app(session.app_handle.clone(), id.into())
            .await
            .map_err(err)
    } else {
        let value = session
            .request(Method::GET, &format!("apps/{}", segment(id)), None)
            .await?;
        let app: App = serde_json::from_value(value).map_err(err)?;
        if app.id != id {
            return Err("The hub returned another app.".into());
        }
        app::update_app(session.app_handle.clone(), app.clone())
            .await
            .map_err(err)?;
        Ok(app)
    }
}
fn private(app: &App) -> Result<(), String> {
    if matches!(
        app.visibility,
        flow_like::app::AppVisibility::Public | flow_like::app::AppVisibility::PublicRequestAccess
    ) {
        Err(format!(
            "App '{}' has public visibility and cannot be used as a FlowPilot staging target.",
            app.id
        ))
    } else {
        Ok(())
    }
}
pub(super) async fn staged_app(session: &NativePlatformSession, id: &str) -> Result<App, String> {
    let app = load_app(session, id).await?;
    private(&app)?;
    if !matches!(app.status, AppStatus::Inactive) {
        return Err(format!(
            "App '{id}' must remain Inactive throughout the build."
        ));
    }
    session.assert_current_profile().await?;
    Ok(app)
}
pub(super) async fn save_app(session: &NativePlatformSession, app: App) -> Result<(), String> {
    session.assert_current_profile().await?;
    if !session.is_local(&app.id).await? {
        session
            .request(
                Method::PUT,
                &format!("apps/{}", segment(&app.id)),
                Some(&json!({"app":app})),
            )
            .await?;
    }
    app::update_app(session.app_handle.clone(), app)
        .await
        .map_err(err)
}

pub(super) async fn board(
    session: &NativePlatformSession,
    app_id: &str,
    id: &str,
) -> Result<Board, String> {
    let app = load_app(session, app_id).await?;
    if !app.boards.iter().any(|board| board == id) {
        return Err(format!("Board '{id}' is not present in app '{app_id}'."));
    }
    let board = if session.is_local(app_id).await? {
        (*board_service::get_board(session.app_handle.clone(), app_id.into(), id.into(), None)
            .await
            .map_err(err)?)
        .clone()
    } else {
        serde_json::from_value(
            session
                .request(
                    Method::GET,
                    &format!("apps/{}/board/{}", segment(app_id), segment(id)),
                    None,
                )
                .await?,
        )
        .map_err(err)?
    };
    if board.id != id {
        return Err("The board did not read back at its reserved ID.".into());
    }
    Ok(board)
}
async fn board_list(session: &NativePlatformSession, app: &str) -> Result<Vec<Value>, String> {
    if session.is_local(app).await? {
        let manifest = load_app(session, app).await?;
        let mut result = Vec::new();
        for id in manifest.boards {
            let b = board(session, app, &id).await?;
            result.push(json!({"id":b.id,"name":b.name}));
        }
        Ok(result)
    } else {
        let value = session
            .request(
                Method::GET,
                &format!("apps/{}/board/summaries", segment(app)),
                None,
            )
            .await?;
        value
            .as_array()
            .cloned()
            .ok_or("The board inventory was not a list.".into())
    }
}
async fn source(
    session: &NativePlatformSession,
    app: &str,
    board: &Board,
) -> Result<String, String> {
    if session.is_local(app).await? {
        Ok(board_to_flowscript(
            board,
            &RenderOptions {
                anchors: true,
                ..Default::default()
            },
        ))
    } else {
        let value = session
            .request(
                Method::GET,
                &format!(
                    "apps/{}/board/{}/flowscript?anchors=true",
                    segment(app),
                    segment(&board.id)
                ),
                None,
            )
            .await?;
        value
            .as_str()
            .or_else(|| value["flowscript"].as_str())
            .map(str::to_owned)
            .ok_or("The authoritative board source is unavailable.".into())
    }
}
async fn hydrate_board(
    session: &NativePlatformSession,
    app: &str,
    board: &Board,
) -> Result<(), String> {
    if !session.is_local(app).await? {
        session.assert_current_profile().await?;
        app::upsert_board(
            session.app_handle.clone(),
            app.into(),
            Some(board.id.clone()),
            board.name.clone(),
            board.description.clone(),
            None,
            None,
            None,
            Some(board.clone()),
            None,
            Some(board.updated_at),
        )
        .await
        .map_err(err)?;
    }
    Ok(())
}
pub(super) async fn events(
    session: &NativePlatformSession,
    app_id: &str,
) -> Result<Vec<Value>, String> {
    if !session.is_local(app_id).await? {
        return native_platform_tools::app_events(session, app_id).await;
    }
    let manifest = load_app(session, app_id).await?;
    let mut values = Vec::new();
    for id in manifest.events {
        values.push(encode(
            crate::functions::flow::event::get_event(
                session.app_handle.clone(),
                app_id.into(),
                id,
                None,
            )
            .await
            .map_err(err)?,
        )?);
    }
    Ok(values)
}

pub(super) async fn pages(
    session: &NativePlatformSession,
    app: &str,
) -> Result<Vec<Value>, String> {
    if session.is_local(app).await? {
        let manifest = load_app(session, app).await?;
        for id in &manifest.boards {
            let board = board(session, app, id).await?;
            board.load_all_pages(None).await.map_err(err)?;
        }
        let mut values = page::get_pages(session.app_handle.clone(), app.into(), None)
            .await
            .map_err(err)?
            .into_iter()
            .map(encode)
            .collect::<Result<Vec<_>, _>>()?;
        for id in &manifest.page_ids {
            if !values.iter().any(|page| page["pageId"] == *id) {
                values.push(json!({"pageId":id,"unavailable":true}));
            }
        }
        Ok(values)
    } else {
        session
            .request(Method::GET, &format!("apps/{}/pages", segment(app)), None)
            .await?
            .as_array()
            .cloned()
            .ok_or("The page inventory was not a list.".into())
    }
}
pub(super) async fn load_page(
    session: &NativePlatformSession,
    app: &str,
    id: &str,
    board: Option<&str>,
) -> Result<Value, String> {
    let result = if session.is_local(app).await? {
        encode(
            page::get_page(
                session.app_handle.clone(),
                app.into(),
                id.into(),
                board.map(str::to_owned),
                None,
            )
            .await
            .map_err(err)?,
        )?
    } else {
        session
            .request(
                Method::GET,
                &format!(
                    "apps/{}/pages/{}{}",
                    segment(app),
                    segment(id),
                    board
                        .map(|id| format!("?board_id={}", segment(id)))
                        .unwrap_or_default()
                ),
                None,
            )
            .await?
    };
    if result["id"] != id {
        return Err("The page did not read back at its reserved ID.".into());
    }
    Ok(result)
}
async fn widgets(session: &NativePlatformSession, app: &str) -> Result<Vec<Value>, String> {
    if session.is_local(app).await? {
        let manifest = load_app(session, app).await?;
        let mut values = Vec::new();
        for id in &manifest.widget_ids {
            values.push(encode(
                manifest.open_widget(id.clone(), None).await.map_err(err)?,
            )?);
        }
        Ok(values)
    } else {
        session
            .request(Method::GET, &format!("apps/{}/widgets", segment(app)), None)
            .await?
            .as_array()
            .cloned()
            .ok_or("The widget inventory was not a list.".into())
    }
}
pub(super) async fn load_widget(
    session: &NativePlatformSession,
    app: &str,
    id: &str,
) -> Result<Value, String> {
    let result = if session.is_local(app).await? {
        encode(
            widget::get_widget(session.app_handle.clone(), app.into(), id.into(), None)
                .await
                .map_err(err)?,
        )?
    } else {
        session
            .request(
                Method::GET,
                &format!("apps/{}/widgets/{}", segment(app), segment(id)),
                None,
            )
            .await?
    };
    if result["id"] != id {
        return Err("The widget did not read back at its reserved ID.".into());
    }
    Ok(result)
}
async fn tables(session: &NativePlatformSession, app: &str) -> Result<Vec<Value>, String> {
    table_metadata(session, app, None)
        .await?
        .as_array()
        .cloned()
        .ok_or("The table inventory was not a list.".into())
}
async fn table_metadata(
    session: &NativePlatformSession,
    app: &str,
    table: Option<&str>,
) -> Result<Value, String> {
    if session.is_local(app).await? {
        native_platform_tools::table_metadata(session, app, false, table).await
    } else {
        session
            .request(
                Method::GET,
                &match table {
                    Some(table) => format!(
                        "apps/{}/db/{}/schema?user_scoped=false",
                        segment(app),
                        segment(table)
                    ),
                    None => format!("apps/{}/db", segment(app)),
                },
                None,
            )
            .await
    }
}
async fn route_event(
    session: &NativePlatformSession,
    app: &str,
    path: &str,
) -> Result<Option<String>, String> {
    if session.is_local(app).await? {
        Ok(
            route::get_app_route_by_path(session.app_handle.clone(), app.into(), path.into())
                .await
                .map_err(err)?
                .map(|route| route.event_id),
        )
    } else {
        let routes = session
            .request(Method::GET, &format!("apps/{}/routes", segment(app)), None)
            .await?;
        let routes = routes
            .as_array()
            .ok_or("The route inventory was not a list.")?;
        Ok(routes
            .iter()
            .find(|r| r["path"] == path)
            .and_then(|r| r["eventId"].as_str().or_else(|| r["event_id"].as_str()))
            .map(str::to_owned))
    }
}

pub(super) async fn resolve_entry(
    session: &NativePlatformSession,
    app: &str,
    board_id: &str,
    selector: Option<&str>,
    event_type: Option<&str>,
) -> Result<String, String> {
    let board = encode(board(session, app, board_id).await?)?;
    let matches = workflow_entries(&board, board_id)
        .into_iter()
        .filter(|entry| {
            selector.is_none_or(|selector| {
                ["id", "name", "node_type"]
                    .iter()
                    .any(|key| entry[key].as_str() == Some(selector))
            }) && event_type.is_none_or(|kind| {
                array(entry, "supported_event_types")
                    .iter()
                    .any(|supported| supported.as_str() == Some(kind))
            })
        })
        .collect::<Vec<_>>();
    if matches.len() != 1 {
        return Err(format!(
            "Entry '{}' resolves to {} compatible runnable nodes. Name one exact entry in the app contract.",
            selector.unwrap_or("(unspecified)"),
            matches.len()
        ));
    }
    Ok(text(&matches[0], "id").into())
}

fn fields(raw: &Value) -> Vec<Value> {
    raw.as_array()
        .or_else(|| raw["fields"].as_array())
        .into_iter()
        .flatten()
        .filter(|field| field.is_object())
        .cloned()
        .collect()
}
fn scalar(raw: &Value) -> Option<String> {
    let raw = raw.as_str()?.to_lowercase();
    Some(match raw.as_str() {
        "utf8" | "largeutf8" => "string".into(),
        "bool" => "boolean".into(),
        _ => raw,
    })
}
fn type_matches(raw: &Value, expected: &Value) -> bool {
    let kind = text(expected, "type");
    if kind != "vector" && scalar(raw).as_deref() == Some(kind.to_lowercase().as_str()) {
        return true;
    }
    if kind == "timestamp:ms:UTC" {
        return raw["Timestamp"] == json!(["Millisecond", "UTC"]);
    }
    if kind == "vector" {
        let vector = &raw["FixedSizeList"];
        return vector.as_array().is_some_and(|v| v.len() == 2)
            && vector[1] == expected["vector_size"]
            && scalar(vector[0].get("data_type").unwrap_or(&vector[0]["type"])).as_deref()
                == Some("float32");
    }
    false
}
pub(super) fn table_schema_issues(fields: &[Value], columns: &[Value]) -> Vec<String> {
    let mut issues = Vec::new();
    for column in columns {
        let name = text(column, "name");
        let Some(field) = fields.iter().find(|field| field["name"] == name) else {
            issues.push(format!("Missing table column '{name}'."));
            continue;
        };
        if !type_matches(field.get("data_type").unwrap_or(&field["type"]), column) {
            issues.push(format!("Column '{name}' has a different type."));
        }
        if column.get("nullable").is_some() && field["nullable"] != column["nullable"] {
            issues.push(format!("Column '{name}' has different nullability."));
        }
    }
    issues
}
fn component_issues(components: &[Value], label: &str) -> Vec<String> {
    let mut issues = Vec::new();
    if components.len() > 120 {
        issues.push(format!("{label} exceeds the 120-component limit."));
    }
    let mut ids = HashSet::new();
    for component in components {
        let id = text(component, "id");
        if id.is_empty() || !ids.insert(id) {
            issues.push(format!("{label} contains invalid or duplicate components."));
            continue;
        }
        if let Err(error) = serde_json::from_value::<SurfaceComponent>(component.clone()) {
            issues.push(format!("{label} requires component repair: {error}"));
            continue;
        }
        if let Err(error) =
            serde_json::from_value::<A2UIComponentType>(component["component"].clone())
        {
            issues.push(format!("{label} requires component repair: {error}"));
        }
        let data = &component["component"];
        if let Some(spec) =
            flow_like::a2ui::authoring::component_manifest().component(text(data, "type"))
        {
            for key in data
                .as_object()
                .into_iter()
                .flat_map(|object| object.keys())
            {
                if ![
                    "type",
                    "id",
                    "style",
                    "children",
                    "actions",
                    "eventHandlers",
                    "hidden",
                ]
                .contains(&key.as_str())
                    && !(text(data, "type") == "widgetInstance" && key == "inlineWidgetDef")
                    && spec.prop(key).is_none()
                {
                    issues.push(format!(
                        "{label} requires component repair: unknown prop '{key}' on {}.",
                        text(data, "type")
                    ));
                }
            }
        }
    }
    issues
        .into_iter()
        .take(9)
        .map(|s| s.chars().take(1600).collect())
        .collect()
}
fn config(event: &Value) -> Value {
    event["config"]
        .as_array()
        .and_then(|bytes| {
            bytes
                .iter()
                .map(|v| v.as_u64().and_then(|v| u8::try_from(v).ok()))
                .collect::<Option<Vec<_>>>()
        })
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .or_else(|| event["config"].is_object().then(|| event["config"].clone()))
        .unwrap_or(Value::Null)
}
fn without_times(mut value: Value) -> Value {
    if let Some(object) = value.as_object_mut() {
        object.remove("createdAt");
        object.remove("updatedAt");
    }
    value
}

pub(super) async fn read(
    session: &NativePlatformSession,
    plan: &Value,
    resource: &Value,
) -> Result<Readback, String> {
    let app = text(plan, "app_id");
    session.require_visible(app)?;
    let id = text(resource, "physical_id");
    let cfg = &resource["config"];
    let result = match text(resource, "kind") {
        "board" => {
            if !board_list(session, app)
                .await?
                .iter()
                .any(|b| b["id"] == id)
            {
                return Ok(Readback::default());
            }
            let board = board(session, app, id).await?;
            let source = source(session, app, &board).await?;
            let mut issues = Vec::new();
            if source.trim().is_empty() || board.nodes.is_empty() {
                issues.push("Board has no executable content.".into());
            }
            hydrate_board(session, app, &board).await?;
            let check = board_service::check_flowscript_reconcile(
                session.app_handle.clone(),
                app.into(),
                id.into(),
                source.clone(),
                None,
                None,
            )
            .await
            .map_err(err)?;
            if !check.parse_valid || !check.reconcile_valid {
                issues.extend(check.diagnostics);
                issues.push("FlowScript does not reconcile.".into());
            }
            let entries = workflow_entries(&encode(&board)?, id);
            snapshot(
                json!({"id":id,"source":source,"entries":entries,"page_ids":board.page_ids,"execution_mode":board.execution_mode}),
                issues,
            )
        }
        "page" => {
            if !pages(session, app)
                .await?
                .iter()
                .any(|p| p["pageId"] == id || p["id"] == id)
            {
                return Ok(Readback::default());
            }
            let board_id = resource_id(plan, text(cfg, "board"))?;
            let page = load_page(session, app, id, Some(board_id)).await?;
            let mut issues = Vec::new();
            if page["boardId"] != board_id {
                issues.push("Page belongs to the wrong board.".into());
            }
            if page["route"] != cfg["route"] {
                issues.push("Page route differs from the contract.".into());
            }
            for (selector, field, label) in [
                ("on_load_entry", "onLoadEventId", "load"),
                ("on_unload_entry", "onUnloadEventId", "unload"),
                ("on_interval_entry", "onIntervalEventId", "interval"),
            ] {
                if let Some(selector) = cfg[selector].as_str() {
                    let expected =
                        resolve_entry(session, app, board_id, Some(selector), Some("quick_action"))
                            .await?;
                    if page[field] != expected {
                        issues.push(format!("Page {label} handler differs from the contract."));
                    }
                }
            }
            if cfg.get("interval_seconds").is_some()
                && page["onIntervalSeconds"] != cfg["interval_seconds"]
            {
                issues.push("Page interval differs from the contract.".into());
            }
            if array(&page, "components").is_empty() && array(&page, "content").is_empty() {
                issues.push("Page is empty.".into());
            }
            issues.extend(component_issues(array(&page, "components"), "Page"));
            snapshot(without_times(page), issues)
        }
        "widget" => {
            if !widgets(session, app)
                .await?
                .iter()
                .any(|w| w["id"] == id || w.get(1).is_some_and(|v| v == id))
            {
                return Ok(Readback::default());
            }
            let widget = load_widget(session, app, id).await?;
            let mut issues = Vec::new();
            let components = array(&widget, "components");
            if components.is_empty() {
                issues.push("Widget is empty.".into());
            }
            if !components
                .iter()
                .any(|component| component["id"] == widget["rootComponentId"])
            {
                issues.push("Widget root does not exist.".into());
            }
            issues.extend(component_issues(components, "Widget"));
            snapshot(without_times(widget), issues)
        }
        "table" => {
            let name = text(cfg, "name");
            if !tables(session, app).await?.iter().any(|t| t == name) {
                return Ok(Readback::default());
            }
            let fields = fields(&table_metadata(session, app, Some(name)).await?);
            let issues = table_schema_issues(&fields, array(cfg, "columns"));
            snapshot(json!({"table":name,"fields":fields}), issues)
        }
        "event" => {
            let events = events(session, app).await?;
            let Some(event) = events.iter().find(|e| e["id"] == id) else {
                return Ok(Readback::default());
            };
            let mut issues = Vec::new();
            if event["event_type"] != cfg["event_type"] {
                issues.push("Event type differs from the contract.".into());
            }
            if let Some(page) = cfg["page"].as_str() {
                if event["default_page_id"] != resource_id(plan, page)? {
                    issues.push("Event targets the wrong page.".into());
                }
            }
            if let Some(board) = cfg["board"].as_str() {
                let board = resource_id(plan, board)?;
                if event["board_id"] != board {
                    issues.push("Event targets the wrong board.".into());
                }
                if cfg["page"].is_null() {
                    let expected = resolve_entry(
                        session,
                        app,
                        board,
                        cfg["entry_node"].as_str(),
                        cfg["event_type"].as_str(),
                    )
                    .await?;
                    if event["node_id"] != expected {
                        issues.push("Event targets a different workflow entry.".into());
                    }
                }
            }
            let config = config(event);
            for (key, value) in cfg["config"].as_object().into_iter().flatten() {
                if fingerprint("event-config", config.get(key).unwrap_or(&Value::Null))
                    != fingerprint("event-config", value)
                {
                    issues.push(format!("Event config '{key}' differs from the contract."));
                }
            }
            if let Some(path) = cfg["route"].as_str() {
                if route_event(session, app, path).await?.as_deref() != Some(id) {
                    issues.push("Event route is missing or targets another Event.".into());
                }
            }
            snapshot(
                json!({
                "id":event["id"],"name":event["name"],"event_type":event["event_type"],"board_id":event["board_id"],"node_id":event["node_id"],"board_version":event["board_version"],"default_page_id":event["default_page_id"],"config":config,"route":cfg["route"],"execution_mode":event["execution_mode"],"variables":event["variables"],"inputs":event["inputs"]}
                ),
                issues,
            )
        }
        kind => return Err(format!("Unsupported app build resource '{kind}'.")),
    };
    session.assert_current_profile().await?;
    Ok(result)
}

async fn inventory_empty(session: &NativePlatformSession, app: &App) -> Result<(), String> {
    let id = &app.id;
    let events = events(session, id).await?;
    let pages = pages(session, id).await?;
    let widgets = widgets(session, id).await?;
    let tables = tables(session, id).await?;
    if !app.events.is_empty()
        || !app.page_ids.is_empty()
        || !app.widget_ids.is_empty()
        || !events.is_empty()
        || !pages.is_empty()
        || !widgets.is_empty()
        || !tables.is_empty()
    {
        return Err(format!(
            "FlowPilot can only stage a new empty app; '{id}' already contains resources."
        ));
    }
    let mut ids = app.boards.iter().cloned().collect::<BTreeSet<_>>();
    for summary in board_list(session, id).await? {
        ids.insert(text(&summary, "id").into());
    }
    for board_id in ids {
        let board = encode(board(session, id, &board_id).await?)?;
        if ["nodes", "variables", "layers", "comments", "refs"]
            .iter()
            .any(|key| board[key].as_object().is_some_and(|v| !v.is_empty()))
            || !array(&board, "page_ids").is_empty()
        {
            return Err(format!(
                "FlowPilot can only stage a new empty app; board '{board_id}' already contains content."
            ));
        }
    }
    session.assert_current_profile().await
}
pub(super) async fn begin_staging(session: &NativePlatformSession, id: &str) -> Result<(), String> {
    let app = load_app(session, id).await?;
    private(&app)?;
    if !matches!(app.status, AppStatus::Active | AppStatus::Inactive) {
        return Err("Archived apps cannot become FlowPilot staging targets.".into());
    }
    inventory_empty(session, &app).await?;
    let mut fresh = load_app(session, id).await?;
    private(&fresh)?;
    if std::mem::discriminant(&fresh.status) != std::mem::discriminant(&app.status) {
        return Err("The app changed status while its staging inventory was being checked.".into());
    }
    inventory_empty(session, &fresh).await?;
    if matches!(fresh.status, AppStatus::Active) {
        fresh.status = AppStatus::Inactive;
        save_app(session, fresh).await?;
    }
    let staged = staged_app(session, id).await?;
    inventory_empty(session, &staged).await
}
pub(super) async fn structural_issues(
    session: &NativePlatformSession,
    plan: &Value,
) -> Result<Vec<String>, String> {
    let app = text(plan, "app_id");
    let mut issues = Vec::new();
    if let Err(error) = staged_app(session, app).await {
        issues.push(error);
    }
    let expected = array(plan, "resources")
        .iter()
        .filter(|r| r["kind"] == "event")
        .map(|r| text(r, "physical_id"))
        .collect::<HashSet<_>>();
    for event in events(session, app).await? {
        let id = text(&event, "id");
        if !expected.contains(id) {
            issues.push(format!("App contains unplanned Event '{id}'."));
        }
        if event["active"].as_bool() != Some(false) {
            issues.push(format!(
                "Event '{id}' must remain inactive until promotion."
            ));
        }
    }
    Ok(issues)
}

pub(super) async fn provision_foundation(
    session: &NativePlatformSession,
    plan: &Value,
) -> Result<Value, String> {
    let _foundation_write = FOUNDATION_WRITES.lock().await;
    let app = text(plan, "app_id");
    staged_app(session, app).await?;
    let table_resources = array(plan, "resources")
        .iter()
        .filter(|r| r["kind"] == "table")
        .collect::<Vec<_>>();
    let existing = tables(session, app).await?;
    for resource in &table_resources {
        let cfg = &resource["config"];
        let name = text(cfg, "name");
        if existing.iter().any(|v| v == name) {
            let fields = fields(&table_metadata(session, app, Some(name)).await?);
            let issues = table_schema_issues(&fields, array(cfg, "columns"));
            if !issues.is_empty() {
                return Err(format!("{name}: {}", issues.join(" ")));
            }
        }
    }
    let mut table_evidence = Vec::new();
    for resource in table_resources {
        staged_app(session, app).await?;
        let cfg = &resource["config"];
        let name = text(cfg, "name");
        let created = if existing.iter().any(|v| v == name) {
            false
        } else {
            let result=native_platform_tools::execute(&session.app_handle,session.context.as_ref(),"database_tool",&json!({
"app_id":app,"operation":"create_table","table_name":name,"fields":cfg["columns"],"if_not_exists":true}
)).await?;

            if result["status"] != "ok" {
                return Err(format!("Table '{name}' schema was not persisted: {result}"));
            }
            result["created"].as_bool().unwrap_or(false)
        };
        if !tables(session, app).await?.iter().any(|v| v == name) {
            return Err(format!("Table '{name}' was not persisted."));
        }
        let fields = fields(&table_metadata(session, app, Some(name)).await?);
        let issues = table_schema_issues(&fields, array(cfg, "columns"));
        if !issues.is_empty() {
            return Err(format!("{name}: {}", issues.join(" ")));
        }
        table_evidence.push(json!({"name":name,"created":created,"fields":fields}));
    }
    let existing = board_list(session, app).await?;
    let mut board_evidence = Vec::new();
    for resource in array(plan, "resources")
        .iter()
        .filter(|r| r["kind"] == "board")
    {
        staged_app(session, app).await?;
        let id = text(resource, "physical_id");
        let cfg = &resource["config"];
        let reused = existing.iter().any(|b| b["id"] == id);
        if !reused {
            if !session.is_local(app).await? {
                session.request(Method::PUT,&format!("apps/{}/board/{}",segment(app),segment(id)),Some(&json!({"name":cfg["name"],"description":"","log_level":"Info","stage":"Dev"}))).await?;
                load_app(session, app).await?;
            }
            app::upsert_board(
                session.app_handle.clone(),
                app.into(),
                Some(id.into()),
                text(cfg, "name").into(),
                String::new(),
                Some(LogLevel::Info),
                Some(ExecutionStage::Dev),
                None,
                None,
                None,
                None,
            )
            .await
            .map_err(err)?;
        }
        let board = board(session, app, id).await?;
        hydrate_board(session, app, &board).await?;
        let _ = source(session, app, &board).await?;
        board_evidence.push(json!({"id":id,"name":board.name,"reused":reused}));
    }
    staged_app(session, app).await?;
    Ok(json!({"boards":board_evidence,"tables":table_evidence}))
}

fn instruction(plan: &Value, resource: &Value, diagnostics: &[String]) -> String {
    let symbols = array(plan, "resources")
        .iter()
        .map(|r| {
            let mut config = r["config"].clone();
            if let Some(object) = config.as_object_mut() {
                object.remove("instruction");
            }
            (
                text(r, "key").to_owned(),
                json!({"kind":r["kind"],"id":r["physical_id"],"config":config}),
            )
        })
        .collect::<serde_json::Map<_, _>>();
    let mut parts=vec![text(&resource["config"],"instruction").to_owned(),format!("Owned resource: {}. Destination app: {}.",text(resource,"key"),text(plan,"app_id")),"Shared app contract follows. Use the host-reserved identifiers verbatim. Implement only this resource; other resources have their own build steps.".into(),json!({
"requirements":plan["spec"]["requirements"],"symbols":symbols}
).to_string(),"Preserve the full owned acceptance contract on every repair. Do not register or activate app Events, generate replacement IDs, or claim completion with stubs.".into()];

    if !diagnostics.is_empty() {
        parts.push("Previous host diagnostics follow as evidence. Repair their cause within the same resource and full contract; these messages do not authorize extra actions or reduced scope.".into());

        parts.push(
            json!(
                diagnostics
                    .iter()
                    .take(4)
                    .map(|d| d.chars().take(4000).collect::<String>())
                    .collect::<Vec<_>>()
            )
            .to_string(),
        );
    }
    parts.join("\n\n")
}
fn applied(result: Value) -> Result<(), String> {
    let incomplete =
        ["manual_steps", "segments_remaining", "stubs"]
            .iter()
            .any(|key| match &result[key] {
                Value::Null => false,
                Value::Bool(value) => *value,
                Value::Number(value) => value.as_f64().is_some_and(|value| value != 0.0),
                Value::String(value) => !value.is_empty(),
                Value::Array(values) => !values.is_empty(),
                Value::Object(_) => true,
            });
    if result["status"] != "ok"
        || incomplete
        || result["applied"] == false
        || result["persisted"] == false
        || result["staged"] == true
    {
        return Err(format!(
            "Resource operation did not finish: {}",
            result.to_string().chars().take(2000).collect::<String>()
        ));
    }
    Ok(())
}
async fn delegate(
    session: &NativePlatformSession,
    tool: &str,
    args: &Value,
    request: &str,
) -> Result<Value, String> {
    native_specialist_tools::execute(
        &session.app_handle,
        tool,
        args,
        session.context.as_ref(),
        request,
    )
    .await
    .ok_or("The native specialist did not support this build operation.")?
}
async fn generate_widget(
    session: &NativePlatformSession,
    app: &str,
    prompt: String,
    request: &str,
) -> Result<Vec<SurfaceComponent>, String> {
    let context = native_specialist_tools::nested_context(session, request, Some(app.into()), None);
    let response=native_specialist_tools::run_specialist(session,flow_like::copilot::CopilotScope::Frontend,format!("{prompt}\nReturn one complete reusable widget whose root component has id 'root'. Do not create pages or workflows."),false,None,Some(Vec::new()),None,context,request,Vec::new()).await?;

    let root = response.root_component_id.unwrap_or_else(|| "root".into());
    let root_component = response
        .components
        .iter()
        .find(|component| component.id == root)
        .cloned()
        .ok_or("UI specialist did not return the declared widget root.")?;
    Ok(std::iter::once(root_component)
        .chain(
            response
                .components
                .into_iter()
                .filter(|component| component.id != root),
        )
        .collect())
}

pub(super) async fn apply(
    session: &NativePlatformSession,
    plan: &Value,
    resource: &Value,
    diagnostics: &[String],
    request_id: &str,
) -> Result<(), String> {
    let app = text(plan, "app_id");
    staged_app(session, app).await?;
    let id = text(resource, "physical_id");
    let cfg = &resource["config"];
    let key = format!("{}:{}", text(plan, "build_id"), text(resource, "key"));
    let prompt = instruction(plan, resource, diagnostics);
    match text(resource, "kind") {
        "board" => {
            let args = json!({
                "app_id": app, "board_id": id, "board_name": cfg["name"],
                "create_new_board": false, "mode": "edit", "idempotency_key": key,
                "instruction": prompt
            });
            applied(delegate(session, "flowpilot_board", &args, request_id).await?)?;
        }
        "page" => {
            let board_id = resource_id(plan, text(cfg, "board"))?;
            let pages = pages(session, app).await?;
            let existing = pages
                .iter()
                .find(|page| page["pageId"] == id || page["id"] == id);
            if existing.is_some_and(|page| {
                page["boardId"]
                    .as_str()
                    .is_some_and(|owner| owner != board_id)
            }) {
                return Err("Reserved page ID belongs to another board.".into());
            }
            let mode = if existing.is_some() { "edit" } else { "create" };
            let args = json!({
                "app_id": app, "board_id": board_id, "page_id": id, "page_name": cfg["name"],
                "route": cfg["route"], "mode": mode, "idempotency_key": key, "instruction": prompt
            });
            applied(delegate(session, "flowpilot_widget", &args, request_id).await?)?;
            let mut lifecycle = json!({"app_id": app, "board_id": board_id, "page_id": id});
            for (selector, field) in [
                ("on_load_entry", "on_load_event_id"),
                ("on_unload_entry", "on_unload_event_id"),
                ("on_interval_entry", "on_interval_event_id"),
            ] {
                if let Some(selector) = cfg[selector].as_str() {
                    lifecycle[field] = json!(
                        resolve_entry(session, app, board_id, Some(selector), Some("quick_action"))
                            .await?
                    );
                }
            }
            if let Some(seconds) = cfg["interval_seconds"].as_u64() {
                lifecycle["on_interval_seconds"] = json!(seconds);
            }
            if lifecycle.as_object().unwrap().len() > 3 {
                applied(
                    native_platform_tools::execute(
                        &session.app_handle,
                        session.context.as_ref(),
                        "set_page_load_event",
                        &lifecycle,
                    )
                    .await?,
                )?;
            }
        }
        "widget" => {
            let components = generate_widget(session, app, prompt, request_id).await?;
            let values = components
                .iter()
                .map(encode)
                .collect::<Result<Vec<_>, _>>()?;
            let issues = component_issues(&values, "Widget");
            if components.is_empty() || !issues.is_empty() {
                return Err(format!(
                    "Widget generation returned empty or invalid components: {}",
                    issues.join(" ")
                ));
            }
            staged_app(session, app).await?;
            let mut definition = Widget::new(id, text(cfg, "name"), &components[0].id);
            let mut actions = Vec::<String>::new();
            for component in &components {
                let data = &component.component;
                let named = data["eventHandlers"]
                    .as_object()
                    .into_iter()
                    .flat_map(|handlers| handlers.values())
                    .flat_map(|value| value.as_array().into_iter().flatten());
                for action in array(data, "actions").iter().chain(named) {
                    if action["name"] != "widget_event" {
                        continue;
                    }
                    if let Some(id) = action["context"]["actionId"]
                        .as_str()
                        .filter(|id| !id.is_empty())
                    {
                        if !actions.iter().any(|existing| existing == id) {
                            actions.push(id.to_owned());
                        }
                    }
                }
            }
            definition.actions = actions
                .into_iter()
                .map(|id| WidgetAction::new(&id, &id))
                .collect();
            definition.components = components;
            if !session.is_local(app).await? {
                session
                    .request(
                        Method::PUT,
                        &format!("apps/{}/widgets/{}", segment(app), segment(id)),
                        Some(&json!({"widget": definition})),
                    )
                    .await?;
            }
            widget::update_widget(session.app_handle.clone(), app.into(), definition)
                .await
                .map_err(err)?;
        }
        "table" => {}
        "event" => {
            let page_board = cfg["page"]
                .as_str()
                .and_then(|key| {
                    array(plan, "resources")
                        .iter()
                        .find(|resource| resource["key"] == key && resource["kind"] == "page")
                })
                .and_then(|page| page["config"]["board"].as_str());
            let board_key = cfg["board"]
                .as_str()
                .or(page_board)
                .ok_or("Event contract does not resolve an owning board.")?;
            let board_id = resource_id(plan, board_key)?;
            if events(session, app)
                .await?
                .iter()
                .any(|event| event["id"] == id && event["active"] == true)
            {
                return Err(format!("Reserved Event '{id}' is already active."));
            }
            let mut args = json!({"app_id": app, "name": cfg["name"], "event_type": cfg["event_type"], "board_id": board_id, "active": false});
            if let Some(page_key) = cfg["page"].as_str() {
                if cfg["event_type"] != "page" || cfg.get("entry_node").is_some() {
                    return Err(
                        "Page Events require a page target without a workflow entry.".into(),
                    );
                }
                let page_id = resource_id(plan, page_key)?;
                let page = load_page(session, app, page_id, Some(board_id)).await?;
                if page["boardId"] != board_id {
                    return Err("The reserved page belongs to another board.".into());
                }
                args["page_id"] = json!(page_id);
            } else {
                args["node_id"] = json!(
                    resolve_entry(
                        session,
                        app,
                        board_id,
                        cfg["entry_node"].as_str(),
                        cfg["event_type"].as_str()
                    )
                    .await?
                );
            }
            for field in ["route", "config"] {
                if let Some(value) = cfg.get(field) {
                    args[field] = value.clone();
                }
            }
            staged_app(session, app).await?;
            applied(native_platform_tools::provision_inactive_event(session, &args, id).await?)?;
            let inventory = events(session, app).await?;
            let event = inventory
                .iter()
                .find(|event| event["id"] == id)
                .ok_or("Reserved Event did not read back.")?;
            let wrong_target = if args.get("page_id").is_some() {
                event["default_page_id"] != args["page_id"] || !text(event, "node_id").is_empty()
            } else {
                event["node_id"] != args["node_id"]
            };
            if event["active"] != false
                || event["event_type"] != cfg["event_type"]
                || event["board_id"] != board_id
                || wrong_target
            {
                return Err(format!(
                    "Event '{id}' did not read back with its exact inactive target."
                ));
            }
            if let Some(path) = cfg["route"].as_str() {
                if route_event(session, app, path).await?.as_deref() != Some(id) {
                    return Err(format!("Event '{id}' route did not read back."));
                }
            }
        }
        kind => return Err(format!("Unsupported app build resource '{kind}'.")),
    }
    staged_app(session, app).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn arrow_schema_checks_vector_width_and_nullability() {
        let fields = vec![
            json!({"name":"embedding","data_type":{"FixedSizeList":[{"name":"item","data_type":"Float32"},4]},"nullable":false}),
        ];
        assert!(
            table_schema_issues(
                &fields,
                &[json!({"name":"embedding","type":"vector","vector_size":4,"nullable":false})]
            )
            .is_empty()
        );
        assert_eq!(
            table_schema_issues(
                &fields,
                &[json!({"name":"embedding","type":"vector","vector_size":8,"nullable":true})]
            )
            .len(),
            2
        );
    }
    #[test]
    fn delegated_previews_are_not_applied_evidence() {
        assert!(applied(json!({"status":"ok","staged":true})).is_err());
        assert!(applied(json!({"status":"ok","stubs":["TODO"]})).is_err());
        assert!(applied(json!({"status":"ok","stubs":"","manual_steps":[]})).is_ok());
        assert!(applied(json!({"status":"ok","applied":true})).is_ok());
    }
}
