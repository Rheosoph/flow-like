use std::collections::BTreeMap;

use reqwest::Method;
use serde_json::{Value, json};

use super::{NativePlatformSession, err, limit, required, segment, string};
use crate::functions::{app, flow::event};

pub(super) async fn execute(
    session: &NativePlatformSession,
    tool: &str,
    args: &Value,
) -> Result<Value, String> {
    match tool {
        "list_apps" => list_apps(session, args).await,
        "describe_app_interface" => describe_interface(session, args).await,
        "get_app_detail" => app_detail(session, args).await,
        "search_apps" => search_apps(session, args).await,
        "read_flowscript_source" => read_source(session, args).await,
        "create_app" => create_app(session, args).await,
        "delete_event" => delete_event(session, args).await,
        "inspect_app" => inspect_app(session, args).await,
        "get_template_preview" | "fork_preview" => preview(session, tool, args).await,
        _ => Err(format!("Unsupported native platform tool '{tool}'.")),
    }
}

async fn preview(
    session: &NativePlatformSession,
    tool: &str,
    args: &Value,
) -> Result<Value, String> {
    let app_id = required(args, "app_id", "appId")?;
    let route = if tool == "get_template_preview" {
        format!(
            "apps/{}/templates/{}/preview",
            segment(&app_id),
            segment(&required(args, "template_id", "templateId")?)
        )
    } else {
        format!(
            "apps/{}/fork/preview?target={}",
            segment(&app_id),
            if args["target"] == "offline" {
                "offline"
            } else {
                "online"
            }
        )
    };
    let preview = session.request(Method::GET, &route, None).await?;
    if !preview.is_object() {
        return Err("The hub returned an invalid preview.".into());
    }
    Ok(json!({"status":"ok","preview":preview}))
}

async fn inspect_app(session: &NativePlatformSession, args: &Value) -> Result<Value, String> {
    let app_id = required(args, "app_id", "appId")?;
    if session.require_visible(&app_id).is_err() {
        return Ok(json!({
        "status":"ok","inaccessible":true,"app_id":app_id,"reason":"The user is not a member of this app, so its boards, events and tables cannot be read. Recommend acquire_app or fork_app instead of reusing a fragment from it."}
        ));
    }
    let all = [
        "boards",
        "events",
        "tables",
        "overlays",
        "widgets",
        "variables",
    ];
    let requested: Vec<_> = args["sections"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .filter(|section| all.contains(section))
        .collect();
    let sections = if requested.is_empty() {
        all.to_vec()
    } else {
        requested
    };
    let local = session.is_local(&app_id).await?;
    let base = format!("apps/{}", segment(&app_id));
    let mut result = json!({"status":"ok","app_id":app_id});
    let mut unreadable = Vec::new();
    for section in sections {
        let read = async {
            let value = if local {
                match section {
                    "boards" => serde_json::to_value(app::get_app_board_summaries(
                        session.app_handle.clone(), app_id.clone(), Some(true), None,
                    ).await.map_err(err)?).map_err(err)?,
                    "events" => json!(events(session, &app_id).await?),
                    "tables" => serde_json::to_value(app::tables::db_table_names(
                        session.app_handle.clone(), app_id.clone(), None, None,
                    ).await.map_err(err)?).map_err(err)?,
                    "overlays" => serde_json::to_value(app::graph::graph_list_overlays(
                        session.app_handle.clone(), app_id.clone(), None,
                    ).await.map_err(err)?).map_err(err)?,
                    "widgets" => serde_json::to_value(crate::functions::a2ui::widget::get_widgets(
                        session.app_handle.clone(), app_id.clone(),
                    ).await.map_err(err)?).map_err(err)?,
                    "variables" => serde_json::to_value(app::get_app_board_variables(
                        session.app_handle.clone(), app_id.clone(),
                    ).await.map_err(err)?).map_err(err)?,
                    _ => unreachable!(),
                }
            } else {
                let suffix = match section {
                    "boards" => "board/summaries?include=node_types",
                    "events" => "events",
                    "tables" => "db",
                    "overlays" => "graph",
                    "widgets" => "widgets",
                    "variables" => "board/variables",
                    _ => unreachable!(),
                };
                session.request(Method::GET, &format!("{base}/{suffix}"), None).await?
            };
            let entries = value.as_array().ok_or("The app section returned an invalid list.")?;
            let summary = match section {
                "boards" => {
                    let filter = string(args, "board_id", "boardId");
                    let boards: Vec<_> = entries.iter()
                        .filter(|board| filter.is_none_or(|id| board["id"] == id))
                        .take(40).map(|board| {
                            let types = board["nodeTypes"].as_array().cloned().unwrap_or_default();
                            let entry_nodes = board["entryNodes"].as_array().into_iter().flatten()
                                .map(|node| json!({"node_id":node["nodeId"],"node_type":node["nodeType"]}))
                                .collect::<Vec<_>>();
                            json!({
                                "board_id":board["id"], "name":board["name"], "description":board["description"],
                                "node_count":board["nodeCount"], "layer_count":board["layerCount"],
                                "entry_nodes":entry_nodes, "node_types":types.iter().take(12).collect::<Vec<_>>(),
                                "node_types_truncated":types.len()>12
                            })
                        }).collect();
                    if entries.len() > 40 { result["boards_truncated"] = json!(true); }
                    json!(boards)
                }
                "events" => {
                    if entries.len() > 60 { result["events_truncated"] = json!(true); }
                    let events: Vec<_> = entries.iter().take(60).map(|event| json!({
                        "event_id":event["id"], "name":event["name"], "description":event["description"],
                        "event_type":event["event_type"], "board_id":event["board_id"], "node_id":event["node_id"],
                        "route":event["route"], "active":event["active"], "execution_mode":event["execution_mode"]
                    })).collect();
                    json!(events)
                }
                "tables" => {
                    let mut tables = Vec::new();
                    for name in entries.iter().take(60).filter_map(Value::as_str) {
                        let schema = if local {
                            app::tables::db_schema(
                                session.app_handle.clone(), app_id.clone(), name.into(), None,
                                Some(false), None, None, None,
                            ).await.map_err(err).and_then(|schema| serde_json::to_value(schema).map_err(err))
                        } else {
                            session.request(Method::GET, &format!("{base}/db/{}/schema", segment(name)), None).await
                        };
                        tables.push(match schema {
                            Ok(schema) => {
                                let fields = schema["fields"].as_array().cloned().unwrap_or_default();
                                let columns: Vec<_> = fields.iter().take(40)
                                    .map(|field| json!({"name":field["name"],"type":field["data_type"]})).collect();
                                json!({"name":name,"columns":columns,"columns_truncated":fields.len()>40})
                            }
                            Err(_) => json!({"name":name,"columns":[],"schema_unreadable":true}),
                        });
                    }
                    if entries.len() > 60 { result["tables_truncated"] = json!(true); }
                    json!(tables)
                }
                "overlays" => {
                    let overlays: Vec<_> = entries.iter().map(|overlay| json!({
                        "overlay_id":overlay["id"],"name":overlay["name"],"description":overlay["description"],
                        "node_types":overlay["nodes"].as_array().into_iter().flatten().map(|node| &node["label"]).collect::<Vec<_>>(),
                        "edge_types":overlay["edges"].as_array().into_iter().flatten().map(|node| &node["label"]).collect::<Vec<_>>()
                    })).collect();
                    json!(overlays)
                }
                "widgets" => {
                    let widgets: Vec<_> = entries.iter().map(|widget| {
                        if widget.is_array() {
                            json!({"widget_id":widget[1],"name":widget[2]["name"],"description":widget[2]["description"]})
                        } else {
                            json!({"widget_id":widget["id"],"name":widget["name"],"description":widget["description"]})
                        }
                    }).collect();
                    json!(widgets)
                }
                "variables" => {
                    let variables: Vec<_> = entries.iter().take(40).flat_map(|board| {
                        board["variables"].as_object().into_iter().flat_map(|vars| vars.values()).map(|var| json!({
                            "board_id":board["board_id"],"name":var["name"],"data_type":var["data_type"],
                            "value_type":var["value_type"],"secret":var.get("secret").cloned().unwrap_or(json!(false))
                        }))
                    }).collect();
                    json!(variables)
                }
                _ => unreachable!(),
            };
            Ok::<_, String>(summary)
        }.await;
        match read {
            Ok(value) => result[section] = value,
            Err(_) => unreadable.push(section),
        }
    }
    session.assert_current_profile().await?;
    if !unreadable.is_empty() {
        result["unreadable_sections"] = json!(unreadable);
    }
    Ok(result)
}

async fn read_source(session: &NativePlatformSession, args: &Value) -> Result<Value, String> {
    let app_id = required(args, "app_id", "appId")?.trim().to_owned();
    let board_id = required(args, "board_id", "boardId")?.trim().to_owned();
    if session.require_visible(&app_id).is_err() {
        return Ok(json!({
        "status":"forbidden","code":"FLOWSCRIPT_SOURCE_APP_NOT_ACCESSIBLE","app_id":app_id,"board_id":board_id,"message":"The requested source app is not available in the user's current profile."}
        ));
    }
    let source = match crate::functions::flow::board::get_flowscript(
        session.app_handle.clone(),
        app_id.clone(),
        board_id.clone(),
        None,
        Some(true),
    )
    .await
    {
        Ok(source) => source,
        Err(_) if !session.is_local(&app_id).await? => {
            let response = session
                .request(
                    Method::GET,
                    &format!(
                        "apps/{}/board/{}/flowscript?anchors=true",
                        segment(&app_id),
                        segment(&board_id)
                    ),
                    None,
                )
                .await?;
            response["flowscript"]
                .as_str()
                .ok_or("The hub returned no FlowScript source.")?
                .to_owned()
        }
        Err(_) => {
            return Ok(json!({
            "status":"error","code":"FLOWSCRIPT_SOURCE_NOT_READABLE","app_id":app_id,"board_id":board_id,"message":"The requested board could not be read. Verify the app and board IDs and the user's board access."}
            ));
        }
    };
    let locator = string(args, "locator", "locator").map(str::trim);
    let mut result = bound_source(&source, locator);
    result["status"] = json!("ok");
    result["app_id"] = json!(app_id);
    result["board_id"] = json!(board_id);
    Ok(result)
}

fn bound_source(source: &str, locator: Option<&str>) -> Value {
    let units: Vec<u16> = source.encode_utf16().collect();
    let matched = locator
        .and_then(|locator| {
            source.find(locator).or_else(|| {
                let mut candidates: Vec<_> = locator
                    .split(|c: char| !c.is_ascii_alphanumeric() && !"_:@./-".contains(c))
                    .filter(|s| s.len() >= 8)
                    .collect();
                candidates.sort_by_key(|candidate| std::cmp::Reverse(candidate.len()));
                candidates
                    .into_iter()
                    .find_map(|candidate| source.find(candidate))
            })
        })
        .map(|byte| source[..byte].encode_utf16().count());
    let mut start = if units.len() <= 60_000 {
        0
    } else {
        matched.unwrap_or(0).saturating_sub(20_000)
    };
    if start > 0
        && let Some(next) = units[start..].iter().position(|c| *c == 10)
        && next < 15_000
    {
        start += next + 1;
    }
    let mut end = units.len().min(start.saturating_add(60_000));
    if end < units.len()
        && let Some(previous) = units[start..=end].iter().rposition(|c| *c == 10)
        && previous > 0
    {
        end = start + previous + 1;
    }
    let mut result = json!({
    "source":String::from_utf16_lossy(&units[start..end]),"source_chars":units.len(),"returned_chars":end-start,"start_offset":start,"end_offset":end,"truncated":start>0||end<units.len()}
    );

    if let Some(locator) = locator {
        result["locator"] = json!(locator);
        result["locator_matched"] = json!(matched.is_some());
        if matched.is_none() {
            result["note"] = json!(
                "The locator was not found verbatim or by stable identifier; the bounded board source is returned so the specialist can resolve it without inventing prior art."
            );
        }
    }
    result
}

#[derive(Clone)]
struct CreatedApp {
    id: String,
    online: bool,
    board_id: String,
    board_initialized: bool,
}
static CREATIONS: std::sync::LazyLock<tokio::sync::Mutex<BTreeMap<String, CreatedApp>>> =
    std::sync::LazyLock::new(Default::default);

async fn create_app(session: &NativePlatformSession, args: &Value) -> Result<Value, String> {
    use flow_like::{
        app::AppVisibility,
        bit::Metadata,
        flow::{board::ExecutionStage, execution::LogLevel},
    };
    let name = required(args, "name", "name")?.trim().to_owned();
    let description = string(args, "description", "description")
        .unwrap_or("")
        .to_owned();
    let online = super::boolean(args, "online", "online", session.subject.is_some())
        && session.subject.is_some();
    let conversation = session
        .context
        .as_ref()
        .and_then(|c| c.conversation_id.as_deref());
    let key = serde_json::to_string(&json!([
        session.hub,
        session.profile.id,
        session.subject,
        conversation.unwrap_or(""),
        name,
        description,
        string(args, "idempotency_key", "idempotencyKey")
    ]))
    .map_err(err)?;
    // Serialize the read/create/record sequence so concurrent retries cannot create duplicates.
    let mut journal = CREATIONS.lock().await;
    let existing = conversation.and_then(|_| journal.get(&key)).cloned();
    let already_created = existing.is_some();
    let mut created = if let Some(existing) = existing {
        if existing.online {
            session
                .request(
                    Method::GET,
                    &format!("apps/{}", segment(&existing.id)),
                    None,
                )
                .await?;
        } else {
            app::get_app(session.app_handle.clone(), existing.id.clone())
                .await
                .map_err(err)?;
        }
        existing
    } else {
        session.assert_current_profile().await?;
        let meta = Metadata {
            name: name.clone(),
            description: description.clone(),
            use_case: Some(String::new()),
            ..Default::default()
        };
        let id = if online {
            let response = session
                .request(
                    Method::PUT,
                    "apps/new",
                    Some(&json!({"meta":meta,"bits":[]})),
                )
                .await?;
            response["id"].as_str().ok_or("The hub created an app without returning its ID; inspect the app inventory before retrying.")?.to_owned()
        } else {
            flow_like_types::create_id()
        };
        let created = CreatedApp {
            id,
            online,
            board_id: flow_like_types::create_id(),
            board_initialized: false,
        };
        if conversation.is_some() {
            journal.insert(key.clone(), created.clone());
        }
        // Record the remote ID before caching or profile association can fail.
        session.assert_current_profile().await?;
        let mut app = app::create_app(
            session.app_handle.clone(),
            Some(created.id.clone()),
            meta,
            vec![],
            Some(false),
        )
        .await
        .map_err(err)?;
        if online {
            app.visibility = AppVisibility::Private;
            app::update_app(session.app_handle.clone(), app)
                .await
                .map_err(err)?;
        }
        created
    };
    let profile_error = async {
        session.assert_current_profile().await?;
        let settings = crate::state::TauriSettingsState::construct(&session.app_handle)
            .await
            .map_err(err)?;
        let mut settings = settings.lock().await;
        let profile = settings
            .profiles
            .get_mut(&session.profile.id)
            .ok_or("The originating profile no longer exists.")?;
        let apps = profile.hub_profile.apps.get_or_insert_with(Vec::new);
        if !apps.iter().any(|app| app.app_id == created.id) {
            apps.push(flow_like::profile::ProfileApp::new(created.id.clone()));
            profile.advance_revision(None);
            settings.try_serialize().map_err(err)?;
        }
        Ok::<_, String>(())
    }
    .await
    .err();
    let board_result=async {
        if created.board_initialized { return Ok(()); }
        session.assert_current_profile().await?;
        if created.online {
            session.request(Method::PUT,&format!("apps/{}/board/{}",segment(&created.id),segment(&created.board_id)),Some(&json!({
"name":"Initial Board","description":"A blank canvas ready for your ideas","log_level":"Debug","stage":"Dev"}
))).await?;

        }
        app::upsert_board(session.app_handle.clone(),created.id.clone(),Some(created.board_id.clone()),"Initial Board".into(),"A blank canvas ready for your ideas".into(),Some(LogLevel::Debug),Some(ExecutionStage::Dev),None,None,None,None).await.map_err(err)
    }.await;
    if board_result.is_ok() {
        created.board_initialized = true;
        if conversation.is_some() {
            journal.insert(key, created.clone());
        }
    }
    let error = profile_error.clone().or_else(|| board_result.err());
    let mut result = json!({
    "status":if error.is_some(){
    "partial"}
    else{
    "ok"}
    ,"app_id":created.id,"name":name,"online":created.online,"provisioning":{
    "app_created":true,"profile_registered":profile_error.is_none()}
    }
    );

    if already_created {
        result["already_created"] = json!(true);
        result["note"] = json!(
            "An app for this exact request already exists in this conversation. Continue with this app_id; use a distinct idempotency_key only for a separate app."
        );
    }
    if let Some(error) = error {
        result["code"] = json!("app_profile_incomplete");
        result["message"] = json!(error);
    } else {
        super::remember_created_app(session.context.as_ref(), &created.id);
    }
    Ok(result)
}

async fn delete_event(session: &NativePlatformSession, args: &Value) -> Result<Value, String> {
    let app_id = session.app_id(args)?;
    let event_id = required(args, "event_id", "eventId")?;
    let local = session.is_local(&app_id).await?;
    session.assert_current_profile().await?;
    if !local {
        session
            .request(
                Method::DELETE,
                &format!("apps/{}/events/{}", segment(&app_id), segment(&event_id)),
                None,
            )
            .await?;
    }
    let native =
        event::delete_event(session.app_handle.clone(), app_id.clone(), event_id.clone()).await;
    if local {
        native.map_err(err)?;
    }
    let _ = crate::functions::a2ui::route::delete_app_route_by_event(
        session.app_handle.clone(),
        app_id.clone(),
        event_id.clone(),
    )
    .await;
    if !local {
        let _ = session
            .request(
                Method::DELETE,
                &format!("apps/{}/routes/{}", segment(&app_id), segment(&event_id)),
                None,
            )
            .await;
    }
    Ok(json!({"status":"ok","note":"Event deleted."}))
}

async fn inventory(session: &NativePlatformSession) -> Result<Vec<(Value, Value)>, String> {
    let local = app::get_apps(session.app_handle.clone(), None)
        .await
        .map_err(err)?;
    let mut entries = BTreeMap::new();
    for (app, meta) in local {
        entries.insert(
            app.id.clone(),
            (
                serde_json::to_value(app).map_err(err)?,
                serde_json::to_value(meta).map_err(err)?,
            ),
        );
    }
    if session.subject.is_some()
        && let Ok(remote) = session.request(Method::GET, "apps", None).await
    {
        for entry in remote
            .as_array()
            .ok_or("The hub returned an invalid app inventory.")?
        {
            let pair = entry
                .as_array()
                .filter(|entry| entry.len() == 2)
                .ok_or("The hub returned an invalid app record.")?;
            let id = pair[0]["id"]
                .as_str()
                .ok_or("The hub app record has no ID.")?;
            session.assert_current_profile().await?;
            app::update_app(
                session.app_handle.clone(),
                serde_json::from_value(pair[0].clone()).map_err(err)?,
            )
            .await
            .map_err(err)?;
            let metadata = entries
                .get(id)
                .map(|(_, meta)| meta)
                .filter(|meta| meta.is_object())
                .cloned()
                .unwrap_or_else(|| pair[1].clone());
            entries.insert(id.into(), (pair[0].clone(), metadata));
        }
    }
    session.assert_current_profile().await?;
    Ok(entries.into_values().collect())
}

pub(super) async fn events(
    session: &NativePlatformSession,
    app_id: &str,
) -> Result<Vec<Value>, String> {
    if session.is_local(app_id).await? {
        let events = event::get_events(session.app_handle.clone(), app_id.into())
            .await
            .map_err(err)?;
        events
            .into_iter()
            .map(|event| serde_json::to_value(event).map_err(err))
            .collect()
    } else {
        let events = session
            .request(
                Method::GET,
                &format!("apps/{}/events", segment(app_id)),
                None,
            )
            .await?;
        events
            .as_array()
            .cloned()
            .ok_or_else(|| "The hub returned an invalid event inventory.".into())
    }
}

fn kind(event: &Value) -> &'static str {
    if event["event_type"] == "simple_chat" {
        return "chat";
    }
    let page = event["default_page_id"].as_str().unwrap_or("");
    if !page.is_empty() && page.trim() == page {
        return "page";
    }
    if event["event_type"]
        .as_str()
        .is_some_and(|value| value.trim().eq_ignore_ascii_case("page"))
        || !page.trim().is_empty()
    {
        return "unavailable";
    }
    "headless"
}

fn event_summary(event: &Value) -> Value {
    let kind = kind(event);
    let mut summary = json!({"id": event["id"], "name": event["name"], "description": event["description"], "event_type": event["event_type"], "kind":kind});
    let consumer = match kind {
        "chat" => Some("call_app_chat"),
        "page" => Some("open_app_page"),
        "headless" => Some("call_app_event"),
        _ => None,
    };
    if let Some(consumer) = consumer {
        summary["consumer_tool"] = json!(consumer);
    }
    if kind == "unavailable" {
        summary["interface_error"] = json!("page_target_missing");
    }
    if kind == "page" {
        summary["page_id"] = event["default_page_id"].clone();
    }
    if let Some(route) = event["route"]
        .as_str()
        .map(str::trim)
        .filter(|route| !route.is_empty())
    {
        summary["route"] = json!(route);
    }
    summary
}

async fn list_apps(session: &NativePlatformSession, args: &Value) -> Result<Value, String> {
    let profile_ids = session.profile_app_ids();
    let all = inventory(session).await?;
    if !profile_ids.is_empty() && all.is_empty() {
        return Ok(json!({
        "status":"error", "code":"app_inventory_unavailable", "retryable":true, "message":"The profile references apps but the backend inventory was empty. App absence is unproven; retry the inventory."}
        ));
    }
    let found: std::collections::HashSet<_> = all
        .iter()
        .filter_map(|(app, _)| app["id"].as_str())
        .collect();
    let mut missing: Vec<_> = profile_ids
        .iter()
        .filter(|id| !found.contains(id.as_str()))
        .cloned()
        .collect();
    missing.sort();
    let query = string(args, "query", "query").unwrap_or("").to_lowercase();
    let mut visible: Vec<_> = all
        .iter()
        .filter(|(app, meta)| {
            app["id"]
                .as_str()
                .is_some_and(|id| profile_ids.contains(id))
                && (query.is_empty()
                    || [&app["id"], &meta["name"], &meta["description"]]
                        .iter()
                        .any(|value| {
                            value
                                .as_str()
                                .is_some_and(|value| value.to_lowercase().contains(&query))
                        }))
        })
        .collect();
    visible.sort_by(|(_, a), (_, b)| {
        a["name"]
            .as_str()
            .unwrap_or("")
            .cmp(b["name"].as_str().unwrap_or(""))
    });
    let mut detailed = Vec::new();
    let mut events_complete = true;
    for (app, meta) in visible.iter().take(250) {
        let id = app["id"].as_str().ok_or("App has no ID.")?;
        let (events, status) = match events(session, id).await {
            Ok(events) => (
                events
                    .into_iter()
                    .filter(|event| event["active"] == true)
                    .map(|event| event_summary(&event))
                    .collect::<Vec<_>>(),
                "ok",
            ),
            Err(_) => {
                events_complete = false;
                (Vec::new(), "error")
            }
        };
        detailed.push(json!({"app_id":id, "name":meta["name"].as_str().unwrap_or(id), "description":meta["description"].as_str().unwrap_or(""), "events":events, "events_status":status}));
    }
    session.assert_current_profile().await?;
    let truncated = visible.len() > 250;
    let complete = !truncated && missing.is_empty() && events_complete;
    let mut result = json!({"status":if complete {"ok"} else {"partial"}, "complete":complete, "total":visible.len(), "returned":detailed.len(), "apps":detailed});
    if !query.is_empty() {
        result["query"] = json!(query);
        result["matched_total"] = json!(visible.len());
        result["profile_total"] = json!(profile_ids.len());
    }
    if !missing.is_empty() {
        result["missing_profile_app_count"] = json!(missing.len());
        result["missing_profile_app_ids"] = json!(&missing[..missing.len().min(50)]);
        if missing.len() > 50 {
            result["missing_profile_app_ids_truncated"] = json!(true);
        }
    }
    if truncated {
        result["truncated"] = json!(true);
    }
    if !complete {
        result["note"] = json!(
            "The inventory is incomplete. App or Event absence is unproven until all profile records and Event inventories can be read."
        );
    }
    Ok(result)
}

async fn describe_interface(
    session: &NativePlatformSession,
    args: &Value,
) -> Result<Value, String> {
    let app_id = required(args, "app_id", "appId")?;
    session.require_visible(&app_id)?;
    let event_id = required(args, "event_id", "eventId")?;
    let event = events(session, &app_id)
        .await?
        .into_iter()
        .find(|event| event["id"] == event_id)
        .ok_or_else(|| format!("Event '{event_id}' not found in app '{app_id}'."))?;
    let mut config = if let Some(bytes) = event["config"].as_array() {
        let bytes: Option<Vec<u8>> = bytes
            .iter()
            .map(|v| v.as_u64().and_then(|v| u8::try_from(v).ok()))
            .collect();
        bytes
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_else(|| json!({}))
    } else {
        event
            .get("config")
            .filter(|v| v.is_object())
            .cloned()
            .unwrap_or_else(|| json!({}))
    };
    let serialized = config.to_string();
    if serialized.encode_utf16().count() > 12_000 {
        config = json!({"truncated":true, "truncated_note":"Context preview only; the stored configuration is unchanged.", "preview":serialized.chars().take(12_000).collect::<String>()});
    }
    let mut summary = event_summary(&event);
    summary.as_object_mut().unwrap().remove("page_id");
    summary["default_page_id"] = event["default_page_id"].clone();
    summary["route"] = event["route"].clone();
    summary["active"] = event["active"].clone();
    summary["inputs"] = event.get("inputs").cloned().unwrap_or_else(|| json!([]));
    let mut result = json!({"status":"ok", "event":summary, "config":config});
    if event["event_type"]
        .as_str()
        .is_some_and(|ty| ty.trim().eq_ignore_ascii_case("mcp"))
    {
        match super::app_interface::describe_mcp(session, &app_id, &event_id).await {
            Ok(mcp) => result["mcp"] = mcp,
            Err(error) => {
                result["status"] = json!("partial");
                result["interface_error"] = json!(error);
            }
        }
    }
    Ok(result)
}

fn summarize_app(app: &Value, meta: &Value) -> Value {
    let mut result = json!({"app_id": app["id"], "visibility":app["visibility"], "price":app.get("price").unwrap_or(&json!(0)), "allow_forking":app.get("allow_forking").unwrap_or(&json!(false))});
    for key in [
        "forked_from",
        "primary_category",
        "avg_rating",
        "rating_count",
    ] {
        if let Some(value) = app.get(key) {
            result[key] = value.clone();
        }
    }
    for key in ["name", "description", "tags", "use_case"] {
        if let Some(value) = meta.get(key) {
            result[key] = value.clone();
        }
    }
    result
}

async fn app_detail(session: &NativePlatformSession, args: &Value) -> Result<Value, String> {
    let id = required(args, "app_id", "appId")?;
    let (app, meta) = if session.is_local(&id).await? {
        (
            serde_json::to_value(
                app::get_app(session.app_handle.clone(), id.clone())
                    .await
                    .map_err(err)?,
            )
            .map_err(err)?,
            serde_json::to_value(
                app::get_app_meta(session.app_handle.clone(), id.clone(), None)
                    .await
                    .map_err(err)?,
            )
            .map_err(err)?,
        )
    } else {
        (
            session
                .request(Method::GET, &format!("apps/{}", segment(&id)), None)
                .await?,
            session
                .request(Method::GET, &format!("apps/{}/meta", segment(&id)), None)
                .await
                .unwrap_or(Value::Null),
        )
    };
    let mut summary = summarize_app(&app, &meta);
    summary["long_description"] = meta["long_description"].clone();
    for (target, source) in [
        ("bits", "bits"),
        ("board_count", "boards"),
        ("event_count", "events"),
        ("template_count", "templates"),
    ] {
        summary[target] = json!(app[source].as_array().map_or(0, Vec::len));
    }
    Ok(json!({"status":"ok", "app":summary}))
}

async fn search_apps(session: &NativePlatformSession, args: &Value) -> Result<Value, String> {
    let query = required(args, "query", "query")?;
    let mut path = format!(
        "apps/search?query={}&limit={}",
        segment(&query),
        limit(args, "limit", "limit", 25, 100)
    );
    for key in ["category", "tag", "author"] {
        if let Some(value) = string(args, key, key) {
            path.push_str(&format!("&{key}={}", segment(value)));
        }
    }
    let result = session.request(Method::GET, &path, None).await?;
    let results = result
        .as_array()
        .ok_or("The hub returned an invalid search result.")?;
    Ok(json!({
    "status":"ok", "note":"Public store results are metadata only. Use inspect_app for apps the user is a member of.", "apps":results.iter().map(|pair| summarize_app(&pair[0],&pair[1])).collect::<Vec<_>>()}
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn event_classification_matches_web_consumer_contract() {
        for (event_type, page, expected) in [
            ("simple_chat", "p", "chat"),
            ("rest", "p", "page"),
            ("page", "", "unavailable"),
            ("rest", " p ", "unavailable"),
            ("rest", "", "headless"),
        ] {
            assert_eq!(
                kind(&json!({"event_type":event_type,"default_page_id":page})),
                expected
            );
        }
        let result = event_summary(&json!({"event_type":"page","default_page_id":""}));
        assert!(result.get("consumer_tool").is_none());
        assert_eq!(result["interface_error"], "page_target_missing");
    }
}
