use std::{collections::HashSet, sync::LazyLock};

use flow_like::{a2ui::widget::Page, flow::event::Event};
use reqwest::Method;
use serde_json::{Value, json};

use super::{NativePlatformSession, boolean, err, required, segment, string};
use crate::functions::{
    a2ui::{page, route},
    app,
    flow::event,
};

static EVENT_WRITES: LazyLock<tokio::sync::Mutex<()>> = LazyLock::new(Default::default);

pub(super) async fn execute(
    session: &NativePlatformSession,
    tool: &str,
    args: &Value,
) -> Result<Value, String> {
    let _lease = EVENT_WRITES.lock().await;
    session.assert_current_profile().await?;
    match tool {
        "upsert_event" => upsert(session, args).await,
        "set_page_load_event" => page_lifecycle(session, args).await,
        _ => Err(format!("Unsupported Event tool '{tool}'.")),
    }
}

async fn board(
    session: &NativePlatformSession,
    app_id: &str,
    board_id: &str,
) -> Result<Value, String> {
    if session.is_local(app_id).await? {
        serde_json::to_value(
            app::get_app_board(
                session.app_handle.clone(),
                app_id.into(),
                board_id.into(),
                true,
            )
            .await
            .map_err(err)?,
        )
        .map_err(err)
    } else {
        let value = session
            .request(
                Method::GET,
                &format!("apps/{}/board/{}", segment(app_id), segment(board_id)),
                None,
            )
            .await?;
        let board: flow_like::flow::board::Board =
            serde_json::from_value(value.clone()).map_err(err)?;
        session.assert_current_profile().await?;
        app::upsert_board(
            session.app_handle.clone(),
            app_id.into(),
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
        Ok(value)
    }
}

pub(super) fn runnable(board: &Value, node_id: &str) -> bool {
    let node = &board["nodes"][node_id];
    if !matches!(
        node["name"].as_str(),
        Some("events_simple" | "events_generic" | "events_chat")
    ) {
        return false;
    }
    fn collect<'a>(layer: &'a Value, pins: &mut HashSet<&'a str>) {
        if let Some(nodes) = layer["nodes"].as_object() {
            for node in nodes.values() {
                if let Some(node_pins) = node["pins"].as_object() {
                    for (key, pin) in node_pins {
                        pins.insert(key);
                        if let Some(id) = pin["id"].as_str() {
                            pins.insert(id);
                        }
                    }
                }
            }
        }
        if let Some(layer_pins) = layer["pins"].as_object() {
            for (key, pin) in layer_pins {
                pins.insert(key);
                if let Some(id) = pin["id"].as_str() {
                    pins.insert(id);
                }
            }
        }
    }
    let mut pins = HashSet::new();
    collect(board, &mut pins);
    if let Some(layers) = board["layers"].as_object() {
        for layer in layers.values() {
            collect(layer, &mut pins);
        }
    }
    node["pins"].as_object().is_some_and(|values| {
        values.values().any(|pin| {
            pin["pin_type"] == "Output"
                && pin["data_type"] == "Execution"
                && pin["connected_to"].as_array().is_some_and(|targets| {
                    targets
                        .iter()
                        .filter_map(Value::as_str)
                        .any(|id| pins.contains(id))
                })
        })
    })
}

pub(super) fn supported_types(name: &str) -> (&'static str, &'static [&'static str]) {
    match name {
        "events_simple" => (
            "quick_action",
            &[
                "quick_action",
                "api",
                "cron",
                "daemon",
                "deeplink",
                "rest",
                "mcp",
            ],
        ),
        "events_generic" => ("generic_form", &["generic_form", "api", "deeplink"]),
        "events_chat" => (
            "simple_chat",
            &["simple_chat", "discord", "telegram", "teams"],
        ),
        _ => ("", &[]),
    }
}

static EVENT_DEFAULTS: LazyLock<Value> = LazyLock::new(|| {
    serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../packages/ui/lib/flowpilot/generated/event-defaults.json"
    )))
    .expect("Bundled Event defaults are valid JSON")
});

fn default_config(event_type: &str) -> Value {
    let mut config = EVENT_DEFAULTS
        .get(event_type)
        .cloned()
        .unwrap_or_else(|| json!({}));
    if event_type == "api" {
        config["path"] = json!(format!("/{}", flow_like_types::create_id()));
    }
    if event_type == "deeplink" {
        config["route"] = json!(flow_like_types::create_id());
    }
    config
}

async fn upsert(session: &NativePlatformSession, args: &Value) -> Result<Value, String> {
    upsert_with_id(session, args, None).await
}

pub(super) async fn upsert_reserved(
    session: &NativePlatformSession,
    args: &Value,
    reserved: &str,
) -> Result<Value, String> {
    if args["active"].as_bool() != Some(false) {
        return Err("Reserved build Events must remain inactive.".into());
    }
    let _lease = EVENT_WRITES.lock().await;
    upsert_with_id(session, args, Some(reserved)).await
}

async fn upsert_with_id(
    session: &NativePlatformSession,
    args: &Value,
    reserved: Option<&str>,
) -> Result<Value, String> {
    let app_id = session.app_id(args)?;
    let local = session.is_local(&app_id).await?;
    let name = required(args, "name", "name")?.trim().to_owned();
    let app_record = if local {
        serde_json::to_value(
            app::get_app(session.app_handle.clone(), app_id.clone())
                .await
                .map_err(err)?,
        )
        .map_err(err)?
    } else {
        session
            .request(Method::GET, &format!("apps/{}", segment(&app_id)), None)
            .await?
    };
    if !local {
        let manifest: flow_like::app::App =
            serde_json::from_value(app_record.clone()).map_err(err)?;
        session.assert_current_profile().await?;
        app::update_app(session.app_handle.clone(), manifest)
            .await
            .map_err(err)?;
    }
    if app_record["status"] == "Inactive" && boolean(args, "active", "active", true) {
        return Ok(
            json!({"status":"error","code":"staged_app_activation_blocked","message":"This app is inactive. Use app_build promotion after verification; direct Event activation is blocked."}),
        );
    }
    let requested_id = reserved.or_else(|| string(args, "event_id", "eventId"));
    let existing = if let Some(id) = requested_id {
        let found = super::apps::events(session, &app_id)
            .await?
            .into_iter()
            .find(|event| event["id"] == id);
        if found.is_none() && reserved.is_none() {
            return Err(format!("Cannot update missing Event '{id}'."));
        }
        found
    } else {
        None
    };
    let empty = json!({});
    let previous = existing.as_ref().unwrap_or(&empty);
    let page_id = string(args, "page_id", "pageId")
        .or_else(|| previous["default_page_id"].as_str())
        .filter(|id| !id.trim().is_empty());
    let requested_board = string(args, "board_id", "boardId");
    let requested_node = string(args, "node_id", "nodeId");
    if page_id.is_some() && requested_node.is_some() {
        return Err("page_id and node_id target different Event kinds. Create the page Event with page_id and register workflow entries separately with board_id + node_id.".into());
    }
    let board_id = requested_board
        .or_else(|| previous["board_id"].as_str())
        .unwrap_or("");
    let node_id = if page_id.is_some() {
        ""
    } else {
        requested_node
            .or_else(|| previous["node_id"].as_str())
            .unwrap_or("")
    };
    if page_id.is_none() && (board_id.is_empty() || node_id.is_empty()) {
        return Err("Provide page_id for a page Event, or board_id + node_id for a runnable workflow entry.".into());
    }
    let mut entry_type = None;
    let mut board_mode = None;
    let event_type = if page_id.is_some() {
        "page".to_owned()
    } else {
        let board = board(session, &app_id, board_id).await?;
        if !runnable(&board, node_id) {
            return Err(format!(
                "Node '{node_id}' is not a connected workflow Event entry. Build and connect the board first; no Event was registered."
            ));
        }
        let name = board["nodes"][node_id]["name"].as_str().unwrap_or("");
        let (default, supported) = supported_types(name);
        let selected = string(args, "event_type", "event_type")
            .or_else(|| {
                previous["event_type"]
                    .as_str()
                    .filter(|ty| supported.contains(ty))
            })
            .unwrap_or(default);
        if !supported.contains(&selected) {
            return Err(format!(
                "Event type '{selected}' is incompatible with {name}. Supported types: {}.",
                supported.join(", ")
            ));
        }
        entry_type = Some(name.to_owned());
        board_mode = board["execution_mode"].as_str().map(str::to_owned);
        selected.to_owned()
    };
    let execution_mode = match board_mode.as_deref() {
        Some("Local") => "Local",
        Some("Remote") => "Remote",
        _ => {
            if string(args, "execution_mode", "executionMode")
                .or_else(|| previous["execution_mode"].as_str())
                .is_some_and(|value| value.eq_ignore_ascii_case("remote"))
            {
                "Remote"
            } else {
                "Local"
            }
        }
    };
    let mut config = default_config(&event_type);
    if previous["event_type"] == event_type
        && let Some(bytes) = previous["config"].as_array()
    {
        let bytes: Vec<_> = bytes
            .iter()
            .filter_map(Value::as_u64)
            .filter_map(|n| u8::try_from(n).ok())
            .collect();
        if let Ok(value) = serde_json::from_slice::<Value>(&bytes)
            && value.is_object()
        {
            config = value;
        }
    }
    if let Some(extra) = args["config"].as_object() {
        config.as_object_mut().unwrap().extend(extra.clone());
    }
    if event_type == "cron" {
        let expression = string(args, "cron_expression", "cronExpression")
            .or_else(|| string(&config, "expression", "expression"))
            .map(str::to_owned);
        let scheduled = args
            .get("scheduled_for")
            .or_else(|| args.get("scheduledFor"))
            .or_else(|| config.get("scheduled_for"))
            .cloned();
        if expression.is_none() && scheduled.is_none() {
            return Err(
                "A cron Event requires cron_expression or scheduled_for {date,time}.".into(),
            );
        }
        config["sink_type"] = json!("cron");
        config["timezone"] = json!(
            string(args, "timezone", "timezone")
                .or_else(|| string(&config, "timezone", "timezone"))
                .unwrap_or("UTC")
        );
        config["last_fired"] = Value::Null;
        config["sink_execution"] = json!(if execution_mode == "Remote" {
            "REMOTE"
        } else {
            "LOCAL"
        });
        if let Some(expression) = expression {
            config["expression"] = json!(expression);
            config.as_object_mut().unwrap().remove("scheduled_for");
        } else if let Some(scheduled) = scheduled {
            if !scheduled["date"].is_string() || !scheduled["time"].is_string() {
                return Err("scheduled_for requires string fields date and time.".into());
            }
            config["scheduled_for"] = scheduled;
            config.as_object_mut().unwrap().remove("expression");
        }
    }
    let now = serde_json::to_value(std::time::SystemTime::now()).map_err(err)?;
    let mut event=existing.clone().unwrap_or_else(||json!({
"id":reserved.map(str::to_owned).unwrap_or_else(flow_like_types::create_id),"created_at":now,"event_version":[0,0,0],"priority":0,"variables":{
}
,"notes":null,"canary":null,"board_version":null,"default_page_id":null}
));

    event["name"] = json!(name);
    event["description"] = json!(
        string(args, "description", "description")
            .or_else(|| previous["description"].as_str())
            .unwrap_or("")
    );
    event["board_id"] = json!(board_id);
    event["node_id"] = json!(node_id);
    event["active"] = json!(
        args["active"]
            .as_bool()
            .or_else(|| previous["active"].as_bool())
            .unwrap_or(true)
    );
    event["event_type"] = json!(event_type);
    event["updated_at"] = now;
    event["execution_mode"] = json!(execution_mode);
    event["config"] = if page_id.is_some() {
        json!([])
    } else {
        json!(serde_json::to_vec(&config).map_err(err)?)
    };
    if let Some(page_id) = page_id {
        event["default_page_id"] = json!(page_id);
        event["inputs"] = json!([]);
        event["canary"] = Value::Null;
    }
    let typed: Event = serde_json::from_value(event.clone()).map_err(err)?;
    let authorization = super::event_trigger::authorize(session, &app_id, &typed).await?;
    session.assert_current_profile().await?;
    let mut mirror_error = None;
    let mut local_trigger = None;
    let saved = if local {
        serde_json::to_value(
            event::upsert_event(
                session.app_handle.clone(),
                app_id.clone(),
                typed,
                None,
                Some(true),
                authorization.pat,
                None,
                Some(authorization.mode),
            )
            .await
            .map_err(err)?,
        )
        .map_err(err)?
    } else {
        let saved = session
            .request(
                Method::PUT,
                &format!("apps/{}/events/{}", segment(&app_id), segment(&typed.id)),
                Some(&json!({"event":event,"profile_id":session.profile.id})),
            )
            .await?;
        let typed: Event = serde_json::from_value(saved.clone()).map_err(err)?;
        match event::upsert_event(
            session.app_handle.clone(),
            app_id.clone(),
            typed,
            None,
            Some(true),
            authorization.pat,
            None,
            Some(authorization.mode),
        )
        .await
        {
            Ok(outcome) => local_trigger = Some(serde_json::to_value(outcome).map_err(err)?),
            Err(error) => mirror_error = Some(err(error)),
        }
        saved
    };
    let saved_event = saved.get("event").unwrap_or(&saved);
    let id = saved_event["id"]
        .as_str()
        .ok_or("Event save did not return its ID.")?;
    let route_path = string(args, "route", "route").map(|route| {
        if route.starts_with('/') {
            route.into()
        } else {
            format!("/{route}")
        }
    });
    let route_error = if let Some(path) = &route_path {
        async {
            session.assert_current_profile().await?;
            route::set_app_route(
                session.app_handle.clone(),
                app_id.clone(),
                path.clone(),
                id.into(),
            )
            .await
            .map_err(err)?;
            if !local {
                session
                    .request(
                        Method::POST,
                        &format!("apps/{}/routes", segment(&app_id)),
                        Some(&json!({"path":path,"eventId":id,"isDefault":path=="/"})),
                    )
                    .await?;
            }
            Ok::<_, String>(())
        }
        .await
        .err()
    } else {
        None
    };
    let mut result = json!({
    "status":if route_error.is_some()||mirror_error.is_some(){
    "partial"}
    else{
    "ok"}
    ,"event_id":id,"event_type":saved_event["event_type"],"execution_mode":saved_event["execution_mode"],"provisioning":{
    "event_saved":true,"route_applied":route_path.as_ref().map(|_|route_error.is_none())}
    ,"note":if page_id.is_some(){
    "Page event upserted (bound to the page)."}
    else{
    "Compatible Event setup attached to the workflow entry."}
    }
    );

    if let Some(error) = mirror_error {
        result["local_cache_error"] = json!(error);
    }
    if let Some(name) = entry_type {
        result["entry_node_type"] = json!(name);
    }
    if let Some(page) = page_id {
        result["page_id"] = json!(page);
    }
    if let Some(path) = route_path {
        result["route"] = json!(path);
    }
    if let Some(error) = route_error {
        result["code"] = json!("event_route_incomplete");
        result["message"] = json!(error);
    }
    let trigger = local_trigger.as_ref().unwrap_or(&saved);
    if let Some(state) = trigger.get("trigger_state") {
        result["trigger_state"] = state.clone();
    }
    if let Some(error) = trigger
        .get("trigger_error")
        .filter(|value| !value.is_null())
    {
        result["status"] = json!("partial");
        result["trigger_error"] = error.clone();
    }
    Ok(result)
}

async fn read_page(
    session: &NativePlatformSession,
    app_id: &str,
    page_id: &str,
    local: bool,
) -> Result<Page, String> {
    if local {
        page::get_page(
            session.app_handle.clone(),
            app_id.into(),
            page_id.into(),
            None,
            None,
        )
        .await
        .map_err(err)
    } else {
        serde_json::from_value(
            session
                .request(
                    Method::GET,
                    &format!("apps/{}/pages/{}", segment(app_id), segment(page_id)),
                    None,
                )
                .await?,
        )
        .map_err(err)
    }
}

async fn page_lifecycle(session: &NativePlatformSession, args: &Value) -> Result<Value, String> {
    let app_id = session.app_id(args)?;
    let page_id = required(args, "page_id", "pageId")?;
    let local = session.is_local(&app_id).await?;
    let mut page = read_page(session, &app_id, &page_id, local).await?;
    let board_id = page.board_id.clone().filter(|id| !id.is_empty()).ok_or(
        "The page has no owning board; bind it to a board before configuring lifecycle events.",
    )?;
    if string(args, "board_id", "boardId").is_some_and(|id| id != board_id) {
        return Ok(
            json!({"status":"error","code":"FLOWPILOT_PAGE_BOARD_MISMATCH","message":"The supplied board_id does not own this page. No lifecycle event was changed."}),
        );
    }
    crate::functions::ai::copilot::ensure_board_mutation_not_reserved_by_flowpilot(
        &app_id, &board_id,
    )?;
    let board = board(session, &app_id, &board_id).await?;
    for (snake, camel) in [
        ("on_load_event_id", "onLoadEventId"),
        ("on_unload_event_id", "onUnloadEventId"),
        ("on_interval_event_id", "onIntervalEventId"),
    ] {
        if let Some(id) = string(args, snake, camel)
            && (board["nodes"][id]["name"] != "events_simple" || !runnable(&board, id))
        {
            return Err(format!(
                "{snake} '{id}' is not a connected events_simple entry on board '{board_id}'. The page was not changed."
            ));
        }
    }
    let current = read_page(session, &app_id, &page_id, local).await?;
    if current.board_id != page.board_id || current.updated_at != page.updated_at {
        return Ok(
            json!({"status":"error","code":"FLOWPILOT_PAGE_BOARD_CHANGED","message":"The page changed during lifecycle validation. Read the current page and retry."}),
        );
    }
    page.on_load_event_id = string(args, "on_load_event_id", "onLoadEventId").map(str::to_owned);
    if let Some(id) = string(args, "on_unload_event_id", "onUnloadEventId") {
        page.on_unload_event_id = Some(id.into());
    }
    if let Some(id) = string(args, "on_interval_event_id", "onIntervalEventId") {
        page.on_interval_event_id = Some(id.into());
        if let Some(seconds) = args
            .get("on_interval_seconds")
            .or_else(|| args.get("onIntervalSeconds"))
            .and_then(Value::as_u64)
            .filter(|n| *n > 0 && *n <= u32::MAX as u64)
        {
            page.on_interval_seconds = Some(seconds as u32);
        }
    }
    if let Some(value) = args
        .get("no_cache")
        .or_else(|| args.get("noCache"))
        .and_then(Value::as_bool)
    {
        page.no_cache = value;
    }
    session.assert_current_profile().await?;
    let stored = page::update_page(session.app_handle.clone(), app_id.clone(), page)
        .await
        .map_err(err)?;
    if !local {
        session
            .request(
                Method::PUT,
                &format!("apps/{}/pages/{}", segment(&app_id), segment(&page_id)),
                Some(&json!({"page":stored})),
            )
            .await?;
    }
    Ok(
        json!({"status":"ok","note":if stored.on_load_event_id.is_some(){"Page onLoad event wired; it runs when the page opens."}else{"Page onLoad event cleared."},"no_cache":stored.no_cache}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn event_requires_an_edge_to_a_persisted_pin() {
        let mut board = json!({
        "nodes":{
        "event":{
        "name":"events_simple","pins":{
        "out":{
        "pin_type":"Output","data_type":"Execution","connected_to":["missing"]}
        }
        }
        ,"target":{
        "pins":{
        "in":{
        "id":"in"}
        }
        }
        }
        }
        );

        assert!(!runnable(&board, "event"));
        board["nodes"]["event"]["pins"]["out"]["connected_to"] = json!(["in"]);
        assert!(runnable(&board, "event"));
        board["nodes"]["event"]["name"] = json!("unrelated");
        assert!(!runnable(&board, "event"));
    }
}
