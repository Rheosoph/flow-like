use super::super::native_platform_tools::{
    NativePlatformSession, app_events, err, segment, string, table_metadata,
};
use crate::functions::app::{graph, saved_queries};
use reqwest::Method;
use serde_json::{Value, json};
use std::collections::{BTreeSet, HashSet};

fn text<'a>(value: &'a Value, key: &str) -> &'a str {
    value.get(key).and_then(Value::as_str).unwrap_or("")
}
fn list(value: &Value) -> &[Value] {
    value.as_array().map(Vec::as_slice).unwrap_or(&[])
}
fn issue(severity: &str, code: &str, path: &str, message: impl Into<String>) -> Value {
    json!({"severity":severity,"code":code,"path":path,"message":message.into()})
}
fn matches(query: &str, values: &[&str]) -> bool {
    query.is_empty()
        || values
            .iter()
            .any(|value| value.to_lowercase().contains(query))
}

async fn objects(
    session: &NativePlatformSession,
    app: &str,
    personal: bool,
    kind: &str,
) -> Result<Vec<Value>, String> {
    session.assert_current_profile().await?;
    let value = if session.is_local(app).await? {
        match kind {
            "ontology" => serde_json::to_value(
                graph::graph_list_overlays(session.app_handle.clone(), app.into(), Some(personal))
                    .await
                    .map_err(err)?,
            )
            .map_err(err)?,
            _ => saved_queries::query_saved_list(
                session.app_handle.clone(),
                app.into(),
                Some(personal),
            )
            .await
            .map_err(err)?,
        }
    } else {
        session
            .request(
                Method::GET,
                &format!(
                    "apps/{}/{}{}",
                    segment(app),
                    if kind == "ontology" {
                        "graph"
                    } else {
                        "db/queries"
                    },
                    if personal { "?scope=user" } else { "" }
                ),
                None,
            )
            .await?
    };
    value
        .as_array()
        .cloned()
        .ok_or_else(|| "The hub returned an invalid data source inventory.".into())
}

fn columns(schema: &Value) -> Vec<Value> {
    list(&schema["fields"])
        .iter()
        .enumerate()
        .filter_map(|(position, field)| {
            let name = field.get("name")?.as_str()?;
            let dtype = field
                .get("type")
                .or_else(|| field.get("data_type"))
                .unwrap_or(&Value::Null);
            let type_name = dtype.as_str().map(str::to_owned).unwrap_or_else(|| {
                if dtype.is_null() {
                    String::new()
                } else {
                    dtype.to_string()
                }
            });
            Some(json!({"name":name,"type_name":type_name,"position":position}))
        })
        .collect()
}
fn ontology_columns(node: &Value) -> Vec<Value> {
    let mut fields = Vec::<(String, String)>::new();
    for property in list(&node["property_columns"]) {
        let name = text(property, "name");
        let dtype = text(property, "data_type");
        if let Some(existing) = fields.iter_mut().find(|(existing, _)| existing == name) {
            existing.1 = dtype.into();
        } else {
            fields.push((name.into(), dtype.into()));
        }
    }
    for (name, dtype) in [
        (text(node, "id_column"), ""),
        (text(node, "display_column"), "Utf8"),
    ] {
        if !name.is_empty() && !fields.iter().any(|(existing, _)| existing == name) {
            fields.push((name.into(), dtype.into()));
        }
    }
    fields.into_iter().enumerate().map(|(position,(name,type_name))|json!({"name":name,"type_name":type_name,"position":position})).collect()
}

/// SQL literals and comments cannot add parameters to a saved query's Home contract.
fn parameters(sql: &str) -> Vec<String> {
    let bytes = sql.as_bytes();
    let mut i = 0;
    let mut result = Vec::new();
    while i < bytes.len() {
        if bytes[i..].starts_with(b"--") {
            while i < bytes.len() && bytes[i] != b'\n' {
                i += 1;
            }
        } else if bytes[i..].starts_with(b"/*") {
            i += 2;
            let mut depth = 1;
            while i < bytes.len() && depth > 0 {
                if bytes[i..].starts_with(b"/*") {
                    depth += 1;
                    i += 2;
                } else if bytes[i..].starts_with(b"*/") {
                    depth -= 1;
                    i += 2;
                } else {
                    i += 1;
                }
            }
        } else if matches!(bytes[i], b'\'' | b'"') {
            let start = i;
            let quote = bytes[i];
            let escaped = quote == b'\''
                && start > 0
                && matches!(bytes[start - 1], b'e' | b'E')
                && (start < 2
                    || !(bytes[start - 2].is_ascii_alphanumeric() || bytes[start - 2] == b'_'));
            i += 1;
            while i < bytes.len() {
                if escaped && bytes[i] == b'\\' {
                    i = (i + 2).min(bytes.len());
                    continue;
                }
                let c = bytes[i];
                i += 1;
                if c == quote {
                    if bytes.get(i) == Some(&quote) {
                        i += 1;
                    } else {
                        break;
                    }
                }
            }
        } else if bytes[i] == b'$' {
            let start = i;
            i += 1;
            let end = if bytes
                .get(i)
                .is_some_and(|b| b.is_ascii_alphabetic() || *b == b'_')
            {
                while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                    i += 1;
                }
                i
            } else {
                i
            };
            if bytes.get(end) == Some(&b'$') {
                let delimiter = &bytes[start..=end];
                i = end + 1;
                if let Some(offset) = bytes[i..]
                    .windows(delimiter.len())
                    .position(|w| w == delimiter)
                {
                    i += offset + delimiter.len();
                } else {
                    i = bytes.len();
                }
            } else {
                if end == start + 1 {
                    while i < bytes.len() && bytes[i].is_ascii_digit() {
                        i += 1;
                    }
                }
                if i > start + 1 {
                    let name = String::from_utf8_lossy(&bytes[start + 1..i]).into_owned();
                    if !result.contains(&name) {
                        result.push(name);
                    }
                }
            }
        } else {
            i += 1;
        }
    }
    result
}

pub(super) async fn data_sources(
    session: &NativePlatformSession,
    args: &Value,
) -> Result<Value, String> {
    let app = string(args, "app_id", "appId").unwrap_or("");
    if app.is_empty() {
        return Ok(
            json!({"status":"error","code":"home_data_app_required","message":"list_home_data_sources requires app_id."}),
        );
    }
    let scope = string(args, "scope", "scope").unwrap_or("project");
    if !matches!(scope, "project" | "personal") {
        return Ok(
            json!({"status":"error","code":"home_data_scope_invalid","message":"scope must be project or personal."}),
        );
    }
    if args.get("kinds").is_some_and(|v| {
        !v.is_array()
            || list(v)
                .iter()
                .any(|v| !matches!(v.as_str(), Some("table" | "ontology" | "query")))
    }) {
        return Ok(
            json!({"status":"error","code":"home_data_kinds_invalid","message":"kinds may contain table, ontology, and query."}),
        );
    }
    if session.require_visible(app).is_err() {
        return Ok(
            json!({"status":"error","code":"home_data_app_not_in_profile","message":format!("App '{app}' is not available in the current profile.")}),
        );
    }
    let requested = list(&args["kinds"]);
    let kinds = if requested.is_empty() {
        vec!["table", "ontology", "query"]
    } else {
        requested
            .iter()
            .filter_map(Value::as_str)
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect()
    };
    let query = text(args, "query").trim().to_lowercase();
    let source = string(args, "source_id", "sourceId").unwrap_or("");
    let object_query = string(args, "object_type_query", "objectTypeQuery")
        .unwrap_or("")
        .to_lowercase();
    let column_query = string(args, "column_query", "columnQuery")
        .unwrap_or("")
        .to_lowercase();
    let mut sources = json!({"tables":[],"ontologies":[],"queries":[]});
    let mut totals = json!({"tables":0,"ontologies":0,"queries":0});
    let mut unreadable = BTreeSet::new();
    let mut truncated = false;
    let mut omitted = false;
    let mut emitted = 0usize;
    for kind in kinds {
        let entries = if kind == "table" {
            table_metadata(session, app, scope == "personal", None)
                .await
                .and_then(|v| {
                    v.as_array()
                        .cloned()
                        .ok_or("Invalid table inventory".into())
                })
        } else {
            objects(session, app, scope == "personal", kind).await
        };
        let Ok(mut entries) = entries else {
            unreadable.insert(kind);
            continue;
        };
        entries.retain(|entry| {
            let id = if kind == "table" {
                entry.as_str().unwrap_or("")
            } else {
                text(entry, "id")
            };
            if !source.is_empty() {
                id == source
            } else {
                matches(&query, &[id, text(entry, "name")])
            }
        });
        entries.sort_by(|a, b| {
            let name = |v: &Value| {
                if kind == "table" {
                    v.as_str().unwrap_or("").to_owned()
                } else {
                    text(v, "name").to_owned()
                }
            };
            name(a).cmp(&name(b))
        });
        let field = match kind {
            "table" => "tables",
            "ontology" => "ontologies",
            _ => "queries",
        };
        totals[field] = json!(entries.len());
        let cap = if kind == "query" { 100 } else { 60 };
        truncated |= entries.len() > cap;
        for entry in entries.into_iter().take(cap) {
            let mut select = |columns: Vec<Value>| {
                let matching = columns
                    .into_iter()
                    .filter(|c| matches(&column_query, &[text(c, "name"), text(c, "type_name")]))
                    .collect::<Vec<_>>();
                let count = matching.len();
                let selected = matching
                    .into_iter()
                    .take(80.min(240usize.saturating_sub(emitted)))
                    .collect::<Vec<_>>();
                emitted += selected.len();
                let limited = selected.len() < count;
                truncated |= limited;
                omitted |= limited;
                (selected, count, limited)
            };
            let result = match kind {
                "table" => {
                    let name = entry.as_str().unwrap_or("");
                    if source.is_empty() {
                        omitted = true;
                        json!({"name":name,"columns_omitted":true})
                    } else {
                        match table_metadata(session, app, scope == "personal", Some(name)).await {
                            Ok(schema) => {
                                let (columns, count, limited) = select(columns(&schema));
                                json!({"name":name,"columns":columns,"columns_matched_total":count,"columns_truncated":limited})
                            }
                            Err(_) => {
                                unreadable.insert(kind);
                                json!({"name":name,"columns":[],"schema_unreadable":true})
                            }
                        }
                    }
                }
                "ontology" => {
                    let mut result = json!({"ontology_id":entry["id"],"name":entry["name"],"description":entry["description"]});
                    if source.is_empty() {
                        omitted = true;
                        result["object_type_count"] = json!(list(&entry["nodes"]).len());
                        result["object_types_omitted"] = json!(true);
                    } else {
                        let nodes = list(&entry["nodes"])
                            .iter()
                            .filter(|n| {
                                matches(&object_query, &[text(n, "label"), text(n, "table")])
                            })
                            .collect::<Vec<_>>();
                        let include_columns = !object_query.is_empty() && nodes.len() <= 12;
                        let mut types = Vec::new();
                        for node in nodes.iter().take(80) {
                            let mut ty = json!({"name":node["label"],"table":node["table"],"id_column":node["id_column"],"display_column":node["display_column"]});
                            let cols = ontology_columns(node);
                            if include_columns {
                                let (selected, count, limited) = select(cols);
                                ty["columns"] = json!(selected);
                                ty["columns_matched_total"] = json!(count);
                                ty["columns_truncated"] = json!(limited);
                            } else {
                                ty["column_count"] = json!(cols.len());
                                ty["columns_omitted"] = json!(true);
                            }
                            types.push(ty);
                        }
                        omitted |= !include_columns && !nodes.is_empty();
                        truncated |= nodes.len() > 80;
                        result["object_types"] = json!(types);
                        result["object_types_matched_total"] = json!(nodes.len());
                        result["object_types_truncated"] = json!(nodes.len() > 80);
                    }
                    result
                }
                _ => {
                    let params = parameters(text(&entry, "sql"));
                    let mut result = json!({"query_id":entry["id"],"name":entry["name"],"description":entry["description"],"kind":entry["kind"],"surface":entry["surface"],"overlay_id":entry["overlay_id"],"result_schema_available":false});
                    if source.is_empty() {
                        omitted = true;
                        result["parameter_count"] = json!(params.len());
                        result["details_omitted"] = json!(true);
                    } else {
                        result["parameters"] = json!(params);
                        for key in ["param_schema", "viz_config", "default_limit"] {
                            result[key] = entry[key].clone();
                        }
                    }
                    result
                }
            };
            sources[field].as_array_mut().unwrap().push(result);
        }
    }
    session.assert_current_profile().await?;
    let mut response = json!({"status":if unreadable.is_empty()&&!truncated{"ok"}else{"partial"},"app_id":app,"scope":scope,"matched_totals":totals,"column_records_returned":emitted,"column_record_limit":240,"detail_level":if source.is_empty(){"summaries"}else{"selected_source"},"details_omitted":omitted,"complete":unreadable.is_empty()&&!truncated,"truncated":truncated,"unreadable":unreadable,"sources":sources,"response_byte_limit":131072});
    for (key, value) in [
        ("query", query.as_str()),
        ("source_id", source),
        ("object_type_query", object_query.as_str()),
        ("column_query", column_query.as_str()),
    ] {
        if !value.is_empty() {
            response[key] = json!(value);
        }
    }
    if omitted {
        response["detail_hint"] = json!(
            "Call again with source_id for source details. Narrow object_type_query and column_query when object types or columns are omitted or truncated."
        );
    }
    if response.to_string().len() > 128 * 1024 {
        response["status"] = json!("partial");
        response["complete"] = json!(false);
        response["truncated"] = json!(true);
        response["payload_limited"] = json!(true);
        response["details_omitted"] = json!(true);
        response["detail_hint"] = json!(
            "The bounded response could not include these details. Retry with source_id, object_type_query, and column_query narrowed to the exact target."
        );
        response
            .as_object_mut()
            .unwrap()
            .remove("column_records_returned");
        response
            .as_object_mut()
            .unwrap()
            .remove("column_record_limit");
        for (field, id, flag) in [
            ("tables", "name", "columns_omitted"),
            ("ontologies", "ontology_id", "object_types_omitted"),
            ("queries", "query_id", "details_omitted"),
        ] {
            response["sources"][field] = json!(
                list(&sources[field])
                    .iter()
                    .take(20)
                    .map(|entry| {
                        let mut item = json!({});
                        item[id] = json!(text(entry, id).chars().take(512).collect::<String>());
                        if id != "name" {
                            item["name"] =
                                json!(text(entry, "name").chars().take(512).collect::<String>());
                        }
                        item[flag] = json!(true);
                        item
                    })
                    .collect::<Vec<_>>()
            );
        }
        for key in [
            "app_id",
            "query",
            "source_id",
            "object_type_query",
            "column_query",
        ] {
            if let Some(value) = response.get(key).and_then(Value::as_str) {
                let bounded = value.chars().take(512).collect::<String>();
                response[key] = json!(bounded);
            }
        }
    }
    Ok(response)
}

fn field_references(config: &Value) -> Vec<(String, String)> {
    let mut result = Vec::new();
    let mut add = |field: &str, path: String| {
        if !field.is_empty() {
            result.push((field.into(), path));
        }
    };
    if matches!(text(config, "dateRange"), "7d" | "30d" | "90d" | "year") {
        add(text(config, "dateField"), "dateField".into());
    }
    let viz = if text(config, "visualization").is_empty() {
        "stat"
    } else {
        text(config, "visualization")
    };
    let fields = list(&config["fields"])
        .iter()
        .filter_map(Value::as_str)
        .take(20)
        .collect::<Vec<_>>();
    let records = matches!(
        viz,
        "table"
            | "list"
            | "cards"
            | "kanban"
            | "record"
            | "graph"
            | "scatter"
            | "timeline"
            | "recordcalendar"
            | "comparison"
    ) && (!matches!(viz, "table" | "list" | "cards")
        || text(config, "mode") != "aggregate");
    if records && viz != "histogram" {
        if !fields.is_empty() || viz == "kanban" {
            add(text(config, "groupBy"), "groupBy".into());
        }
        let xy = matches!(viz, "graph" | "scatter");
        if !fields.is_empty() || xy || matches!(viz, "timeline" | "recordcalendar") {
            add(text(config, "xField"), "xField".into());
        }
        if !fields.is_empty() || xy {
            add(text(config, "yField"), "yField".into());
        }
        if !matches!(text(config, "sortBy"), "value" | "group") {
            add(text(config, "sortBy"), "sortBy".into());
        }
        for (index, field) in fields.iter().enumerate() {
            add(field, format!("fields[{index}]"));
        }
    } else {
        add(text(config, "groupBy"), "groupBy".into());
        add(text(config, "seriesBy"), "seriesBy".into());
        if viz == "boxplot" {
            add(text(config, "yField"), "yField".into());
        } else {
            for (index, measure) in list(&config["measures"]).iter().take(6).enumerate() {
                if matches!(
                    text(measure, "aggregation"),
                    "distinct" | "sum" | "avg" | "min" | "max" | "median"
                ) {
                    add(text(measure, "field"), format!("measures[{index}].field"));
                }
            }
        }
    }
    for (index, filter) in list(&config["filters"]).iter().take(12).enumerate() {
        add(text(filter, "field"), format!("filters[{index}].field"));
    }
    result
}
fn route(path: &str) -> String {
    let path = path.trim().split(['#', '?']).next().unwrap_or("");
    let path = if path.starts_with('/') {
        path.to_owned()
    } else {
        format!("/{path}")
    };
    let path = path.trim_end_matches('/');
    if path.is_empty() {
        "/".into()
    } else {
        path.into()
    }
}
fn embeddable(event: &Value) -> bool {
    event["active"].as_bool() == Some(true)
        && (!text(event, "default_page_id").trim().is_empty()
            || matches!(
                text(event, "event_type"),
                "simple_chat" | "generic_form" | "quick_action"
            ))
}
fn embed_issues(config: &Value, events: &[Value], path: &str) -> Vec<Value> {
    let app = text(config, "appId");
    let target = text(config, "target");
    let event_id = if target == "event" {
        text(config, "eventId")
    } else {
        ""
    };
    if !event_id.is_empty() {
        let Some(event) = events.iter().find(|e| text(e, "id") == event_id) else {
            return vec![issue(
                "error",
                "home_app_event_missing",
                &format!("{path}.eventId"),
                format!("Event '{event_id}' is not available in app '{app}'."),
            )];
        };
        let (code, message) = if event["active"].as_bool() != Some(true) {
            (
                "home_app_event_inactive",
                format!("Event '{event_id}' is inactive."),
            )
        } else if !embeddable(event) {
            (
                "home_app_event_not_embeddable",
                format!(
                    "Event '{event_id}' does not expose a Home-compatible page, chat, form, or quick action."
                ),
            )
        } else {
            return vec![];
        };
        return vec![issue("error", code, &format!("{path}.eventId"), message)];
    }
    let target_route = if target == "route" {
        route(text(config, "route"))
    } else {
        "/".into()
    };
    let mut routes = Vec::new();
    for event in events {
        if !text(event, "route").trim().is_empty() {
            let route = route(text(event, "route"));
            if !routes.iter().any(|(p, _)| p == &route) {
                routes.push((route, event));
            }
        }
    }
    for event in events {
        if event["is_default"].as_bool() == Some(true)
            && text(event, "route").trim().is_empty()
            && !routes.iter().any(|(p, _)| p == "/")
        {
            routes.push(("/".into(), event));
        }
    }
    let matching = routes.iter().find(|(p, _)| p == &target_route);
    if matching.is_some_and(|(_, e)| embeddable(e)) {
        return vec![];
    }
    let path = format!(
        "{path}.{}",
        if target == "route" { "route" } else { "target" }
    );
    if target_route == "/" && matching.is_none() && events.iter().any(embeddable) {
        vec![issue(
            "warning",
            "home_app_landing_fallback",
            &path,
            "No default route is configured. Home will use an available interface, which can depend on the last-used Event.",
        )]
    } else {
        vec![issue(
            "error",
            "home_app_route_missing",
            &path,
            format!(
                "Route '{target_route}' does not resolve to an active Home-compatible interface in app '{app}'."
            ),
        )]
    }
}

pub(super) async fn validate_references(
    session: &NativePlatformSession,
    layout: &Value,
) -> Result<Vec<Value>, String> {
    let mut issues = Vec::new();
    let visible = session.profile_app_ids();
    for (index, widget) in list(&layout["widgets"]).iter().enumerate() {
        let path = format!("$.widgets[{index}].config");
        let config = &widget["config"];
        let kind = text(widget, "type");
        let fields = &super::home::contracts()[kind]["fields"];
        if let Some(fields_map) = fields.as_object() {
            for (name, field) in fields_map {
                let reference = &field["reference"];
                if reference.is_null()
                    || !super::home::condition_matches(&reference["when"], config, fields)
                {
                    continue;
                }
                let refs = match text(reference, "kind") {
                    "profile_app_id" => vec![(
                        text(config, name).trim().to_owned(),
                        format!("{path}.{name}"),
                    )],
                    "profile_app_ids" => list(&config[name])
                        .iter()
                        .enumerate()
                        .filter_map(|(i, v)| {
                            v.as_str()
                                .map(|id| (id.trim().to_owned(), format!("{path}.{name}[{i}]")))
                        })
                        .collect(),
                    _ => vec![],
                };
                for (app, reference_path) in refs {
                    if !app.is_empty() && !visible.contains(&app) {
                        issues.push(issue(
                            "error",
                            "home_app_not_in_profile",
                            &reference_path,
                            format!("App '{app}' is not available in the current profile."),
                        ));
                    }
                }
            }
        }
        let app = text(config, "appId").trim();
        if app.is_empty() || !visible.contains(app) {
            continue;
        }
        if kind == "app-embed" {
            match app_events(session, app).await {
                Ok(events) => issues.extend(embed_issues(config, &events, &path)),
                Err(_) => issues.push(issue(
                    "warning",
                    "home_app_interfaces_unreadable",
                    &path,
                    format!("App '{app}' interface metadata could not be checked."),
                )),
            };
        }
        if kind != "data" {
            continue;
        }
        match validate_data(session, config, &path).await {
            Ok(found) => issues.extend(found),
            Err(_) => issues.push(issue(
                "warning",
                "home_data_source_unreadable",
                &path,
                "This data source could not be checked. Its identifiers were preserved.",
            )),
        }
    }
    session.assert_current_profile().await?;
    Ok(issues)
}
async fn validate_data(
    session: &NativePlatformSession,
    config: &Value,
    path: &str,
) -> Result<Vec<Value>, String> {
    let mut issues = Vec::new();
    let app = text(config, "appId");
    let personal = text(config, "scope") == "personal";
    let scope = if personal { "personal" } else { "project" };
    let mut known = HashSet::new();
    let fields = field_references(config);
    match text(config, "sourceKind") {
        "ontology" => {
            let overlays = objects(session, app, personal, "ontology").await?;
            let id = text(config, "ontologyId");
            let Some(overlay) = overlays.iter().find(|o| text(o, "id") == id) else {
                return Ok(vec![issue(
                    "error",
                    "home_ontology_missing",
                    &format!("{path}.ontologyId"),
                    format!("Ontology '{id}' is not available in {scope} data."),
                )]);
            };
            let object = text(config, "objectType");
            let Some(node) = list(&overlay["nodes"])
                .iter()
                .find(|n| text(n, "label") == object)
            else {
                return Ok(vec![issue(
                    "error",
                    "home_ontology_object_missing",
                    &format!("{path}.objectType"),
                    format!("Object type '{object}' is not part of ontology '{id}'."),
                )]);
            };
            known.extend(
                ontology_columns(node)
                    .iter()
                    .map(|c| text(c, "name").to_owned()),
            );
        }
        "query" => {
            let queries = objects(session, app, personal, "query").await?;
            let id = text(config, "queryId");
            let Some(query) = queries.iter().find(|q| text(q, "id") == id) else {
                return Ok(vec![issue(
                    "error",
                    "home_query_missing",
                    &format!("{path}.queryId"),
                    format!("Saved query '{id}' is not available in {scope} data."),
                )]);
            };
            for parameter in parameters(text(query, "sql")) {
                let name = text(query, "name");
                let parameter_path = format!("{path}.queryParams.{parameter}");
                if parameter.starts_with("__home_") {
                    issues.push(issue(
                        "error",
                        "home_query_parameter_reserved",
                        &format!("{path}.queryId"),
                        format!("Saved query '{name}' uses reserved parameter '{parameter}'."),
                    ));
                    continue;
                }
                let Some(value) = config["queryParams"].get(&parameter) else {
                    issues.push(issue(
                        "error",
                        "home_query_parameter_missing",
                        &parameter_path,
                        format!("Saved query '{name}' requires parameter '{parameter}'."),
                    ));
                    continue;
                };
                if value.as_str() == Some("$viewer.id") {
                    continue;
                }
                let expected = text(&query["param_schema"]["properties"][&parameter], "type");
                let valid = match expected {
                    "string" => Some(value.is_string()),
                    "boolean" => Some(value.is_boolean()),
                    "number" => Some(value.is_number()),
                    "integer" => Some(value.as_f64().is_some_and(|v| v.fract() == 0.0)),
                    _ => None,
                };
                if valid == Some(false) {
                    issues.push(issue(
                        "error",
                        "home_query_parameter_type_invalid",
                        &parameter_path,
                        format!("Parameter '{parameter}' must be a {expected}."),
                    ));
                } else if valid.is_none() {
                    issues.push(issue("warning","home_query_parameter_schema_unavailable",&parameter_path,format!("Parameter '{parameter}' has no supported type metadata, so its fixed value could not be type-checked.")));
                }
            }
            if !fields.is_empty() {
                issues.push(issue("warning","home_query_result_fields_unverified",path,"Saved-query result fields are not exposed by its metadata and could not be verified."));
            }
        }
        _ => {
            let table = text(config, "table");
            let tables = table_metadata(session, app, personal, None).await?;
            if !list(&tables).iter().any(|t| t.as_str() == Some(table)) {
                return Ok(vec![issue(
                    "error",
                    "home_table_missing",
                    &format!("{path}.table"),
                    format!("Table '{table}' is not available in {scope} data."),
                )]);
            }
            let schema = table_metadata(session, app, personal, Some(table)).await?;
            known.extend(columns(&schema).iter().map(|c| text(c, "name").to_owned()));
        }
    }
    if !known.is_empty() {
        for (field, suffix) in fields {
            if !known.contains(&field) {
                issues.push(issue(
                    "error",
                    "home_data_field_missing",
                    &format!("{path}.{suffix}"),
                    format!("Field '{field}' is not available in the selected source."),
                ));
            }
        }
    }
    Ok(issues)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn query_parameters_ignore_literals_and_nested_comments() {
        assert_eq!(
            parameters(
                "select $viewer, '$fake', E'\\\'$also_fake', \"$column\", $$ $body $$, $tag$ $body $tag$, $2 /* $outer /* $inner */ */ -- $line\n, $viewer, $__home_bad"
            ),
            vec!["viewer", "2", "__home_bad"]
        );
    }
    #[test]
    fn embed_missing_route_never_uses_default() {
        let events =
            json!([{"id":"root","active":true,"event_type":"simple_chat","is_default":true}]);
        let issues = embed_issues(
            &json!({"appId":"app","target":"route","route":"/missing"}),
            list(&events),
            "$.config",
        );
        assert_eq!(issues[0]["code"], "home_app_route_missing");
        assert!(
            embed_issues(
                &json!({"appId":"app","target":"route","route":"/"}),
                list(&events),
                "$.config"
            )
            .is_empty()
        );
    }
    #[test]
    fn record_fields_ignore_aggregate_measure() {
        let config = json!({"visualization":"table","measures":[{"field":"irrelevant","aggregation":"sum"}],"fields":["name"],"sortBy":"name"});
        assert_eq!(
            field_references(&config),
            vec![
                ("name".into(), "sortBy".into()),
                ("name".into(), "fields[0]".into())
            ]
        );
    }
}
