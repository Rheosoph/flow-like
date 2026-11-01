use super::home::{catalog, condition_matches, contracts, fingerprint};
use super::*;
use flow_like_types::regex::Regex;
use std::collections::{BTreeMap, HashSet};

pub(super) fn issue(severity: &str, code: &str, path: &str, message: &str) -> Value {
    json!({"severity":severity,"code":code,"path":path,"message":message})
}
fn push(issues: &mut Vec<Value>, code: &str, path: &str, message: &str) {
    issues.push(issue("error", code, path, message));
}
fn strings(value: &Value) -> impl Iterator<Item = &str> {
    value
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
}
fn value_at<'a>(config: &'a Value, contract: &'a Value, key: &str) -> &'a Value {
    config
        .get(key)
        .filter(|v| !v.is_null())
        .unwrap_or(&contract["fields"][key]["default"])
}
fn has_value(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::String(s) => !s.trim().is_empty(),
        Value::Array(a) => !a.is_empty(),
        _ => true,
    }
}
fn numeric_string(value: &Value) -> bool {
    value
        .as_str()
        .filter(|s| !s.trim().is_empty())
        .is_some_and(|s| {
            let s = s.trim();
            if let Some(hex) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
                u64::from_str_radix(hex, 16).is_ok()
            } else {
                s.parse::<f64>().is_ok_and(f64::is_finite)
            }
        })
}
fn copy_strings(source: &Value, target: &mut Value, keys: &[&str]) {
    for key in keys {
        if let Some(s) = source[*key].as_str() {
            target[*key] = json!(s);
        }
    }
}
pub(super) fn normalize_layout(raw: &Value) -> Option<Value> {
    if raw["version"] != 1 {
        return None;
    }
    let widgets = raw["widgets"].as_array()?;
    if widgets.len() > 80 {
        return None;
    }
    let mut ids = HashSet::new();
    let mut normalized = Vec::new();
    for widget in widgets {
        let id = widget["id"].as_str().filter(|s| !s.is_empty())?;
        let kind = widget["type"].as_str().filter(|s| !s.is_empty())?;
        if !ids.insert(id) {
            return None;
        }
        let size = &widget["size"];
        let appearance = &widget["appearance"];
        let clamp = |key: &str, fallback: f64| {
            size[key]
                .as_f64()
                .filter(|v| v.is_finite())
                .unwrap_or(fallback)
                .round()
                .clamp(1.0, 12.0) as u64
        };
        let mut result = json!({"id":id,"type":kind,"size":{"columns":clamp("columns",6.0),"rows":clamp("rows",3.0)},"appearance":{"variant":appearance["variant"].as_str().unwrap_or("card"),"accent":appearance["accent"].as_str().unwrap_or("neutral")},"config":widget.get("config").filter(|v|v.is_object()).cloned().unwrap_or(json!({}))});
        copy_strings(widget, &mut result, &["title", "description"]);
        if let Some(mode) = size["heightMode"]
            .as_str()
            .filter(|m| ["auto", "content", "fixed"].contains(m))
        {
            result["size"]["heightMode"] = json!(mode);
        }
        if let Some(height) = size["height"].as_f64().filter(|n| n.is_finite()) {
            result["size"]["height"] = json!(height.round().clamp(96.0, 1240.0) as u64);
        }
        if let Some(classes) = appearance["className"]
            .as_str()
            .filter(|s| !s.trim().is_empty())
        {
            result["appearance"]["className"] =
                json!(classes.split_whitespace().collect::<Vec<_>>().join(" "));
        }
        if let Some(css) = appearance["css"].as_str().filter(|s| !s.trim().is_empty()) {
            result["appearance"]["css"] = json!(css);
        }
        normalized.push(result);
    }
    let mut result = json!({"version":1,"widgets":normalized});
    copy_strings(raw, &mut result, &["title", "description"]);
    Some(result)
}
fn requirements(config: &Value, contract: &Value, path: &str, issues: &mut Vec<Value>) {
    for requirement in contract["requirements"].as_array().into_iter().flatten() {
        if !condition_matches(&requirement["when"], config, &contract["fields"]) {
            continue;
        }
        let code = requirement["code"]
            .as_str()
            .unwrap_or("home_widget_config_requirement");
        let message = requirement["message"]
            .as_str()
            .unwrap_or("The widget configuration does not satisfy its contract.");
        let severity = requirement["severity"].as_str().unwrap_or("error");
        let mut add = |key: &str| {
            issues.push(issue(
                severity,
                code,
                &if key.is_empty() {
                    path.to_owned()
                } else {
                    format!("{path}.{key}")
                },
                message,
            ))
        };
        for kind in [
            "required",
            "non_empty",
            "positive",
            "empty",
            "numeric_strings",
        ] {
            for key in strings(&requirement[kind]) {
                let value = value_at(config, contract, key);
                let valid = match kind {
                    "required" => config.get(key).is_some(),
                    "non_empty" => has_value(value),
                    "positive" => value.as_f64().is_some_and(|n| n > 0.0),
                    "empty" => !has_value(value),
                    _ => numeric_string(value),
                };
                if !valid {
                    add(key);
                }
            }
        }
        if requirement["any_non_empty"]
            .as_array()
            .is_some_and(|a| !a.is_empty())
            && !strings(&requirement["any_non_empty"])
                .any(|key| has_value(value_at(config, contract, key)))
        {
            add("");
        }
        for (key, expected) in requirement["equals"].as_object().into_iter().flatten() {
            if value_at(config, contract, key) != expected {
                add(key);
            }
        }
        for (key, allowed) in requirement["allowed_values"]
            .as_object()
            .into_iter()
            .flatten()
        {
            if !allowed
                .as_array()
                .is_some_and(|a| a.contains(value_at(config, contract, key)))
            {
                add(key);
            }
        }
    }
}
fn safe_url(value: &str) -> Option<String> {
    let value = value.trim();
    if value.chars().any(|c| c == '\\' || c.is_control()) {
        return None;
    }
    if value.starts_with('/') && !value.starts_with("//") {
        return Some(value.into());
    }
    let url = reqwest::Url::parse(value).ok()?;
    ["http", "https", "mailto", "tel"]
        .contains(&url.scheme())
        .then(|| url.to_string())
}
fn field(value: &Value, contract: &Value, path: &str, issues: &mut Vec<Value>) {
    if value.is_null() && contract["nullable"] == true {
        return;
    }
    let kind = contract["type"].as_str().unwrap_or("");
    let matches = match kind {
        "string" => value.is_string(),
        "number" => value.is_number(),
        "boolean" => value.is_boolean(),
        "object" => value.is_object(),
        "string_list" => value
            .as_array()
            .is_some_and(|v| v.iter().all(Value::is_string)),
        "object_list" => value
            .as_array()
            .is_some_and(|v| v.iter().all(Value::is_object)),
        _ => false,
    };
    if !matches {
        push(
            issues,
            "home_widget_config_type_invalid",
            path,
            &format!("Expected {kind}."),
        );
        return;
    }
    if contract["enum"]
        .as_array()
        .is_some_and(|a| !a.contains(value))
    {
        if let Some(legacy) = contract["legacy_values"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|legacy| legacy["value"] == *value)
        {
            issues.push(issue(
                "warning",
                "home_widget_config_legacy_value",
                path,
                &format!("This legacy value renders as {}.", legacy["maps_to"]),
            ));
        } else {
            push(
                issues,
                "home_widget_config_option_invalid",
                path,
                "Choose a value advertised by this widget contract.",
            );
        }
    }
    if let Some(text) = value.as_str() {
        if contract["pattern"]
            .as_str()
            .is_some_and(|pattern| Regex::new(pattern).is_ok_and(|r| !r.is_match(text)))
        {
            push(
                issues,
                "home_widget_config_pattern_invalid",
                path,
                "The value does not match the required pattern.",
            );
        }
        match contract["string_format"].as_str().unwrap_or("") {
            "route" if !text.is_empty() && (!text.starts_with('/') || text.starts_with("//")) => {
                push(
                    issues,
                    "home_widget_config_route_invalid",
                    path,
                    "App routes must start with one '/'.",
                )
            }
            "image-url"
                if !text.trim().is_empty()
                    && !safe_url(text).is_some_and(|url| {
                        url.starts_with('/')
                            || url.starts_with("http://")
                            || url.starts_with("https://")
                    }) =>
            {
                push(
                    issues,
                    "home_widget_config_image_url_invalid",
                    path,
                    "Use a relative path or an http or https image URL.",
                )
            }
            "storage-image-path"
                if !text.trim().is_empty()
                    && (text != text.trim()
                        || text.starts_with('/')
                        || text.ends_with('/')
                        || text.contains('\\')
                        || text.chars().any(char::is_control)
                        || Regex::new("^[a-zA-Z][a-zA-Z0-9+.-]*:")
                            .unwrap()
                            .is_match(text)
                        || text
                            .split('/')
                            .any(|s| s.is_empty() || s == "." || s == "..")) =>
            {
                push(
                    issues,
                    "home_widget_config_storage_image_path_invalid",
                    path,
                    "Use an app-relative path without a URL scheme or parent-directory segments.",
                )
            }
            "date-time"
                if !text.is_empty()
                    && chrono::DateTime::parse_from_rfc3339(text).is_err()
                    && chrono::NaiveDate::parse_from_str(text, "%Y-%m-%d").is_err() =>
            {
                push(
                    issues,
                    "home_widget_config_date_invalid",
                    path,
                    "Use a valid date or date-time.",
                )
            }
            _ => {}
        }
        if contract["reference"]["kind"] == "safe_home_url"
            && !text.trim().is_empty()
            && safe_url(text).is_none()
        {
            push(
                issues,
                "unsafe_home_url",
                path,
                "Use a relative path or an http, https, mailto, or tel URL.",
            );
        }
    }
    if let Some(n) = value.as_f64() {
        if contract["integer"] == true && n.fract() != 0.0 {
            push(
                issues,
                "home_widget_config_integer_required",
                path,
                "Use an integer.",
            );
        }
        if contract["minimum"].as_f64().is_some_and(|min| n < min)
            || contract["exclusive_minimum"]
                .as_f64()
                .is_some_and(|min| n <= min)
        {
            push(
                issues,
                "home_widget_config_number_too_small",
                path,
                "The value is below the supported minimum.",
            );
        }
        if contract["maximum"].as_f64().is_some_and(|max| n > max) {
            push(
                issues,
                "home_widget_config_number_too_large",
                path,
                "The value exceeds the supported maximum.",
            );
        }
        if contract["ranges"].as_array().is_some_and(|ranges| {
            !ranges.iter().any(|range| {
                range["minimum"].as_f64().unwrap_or(f64::NEG_INFINITY) <= n
                    && range["maximum"].as_f64().unwrap_or(f64::INFINITY) >= n
            })
        }) {
            push(
                issues,
                "home_widget_config_number_out_of_range",
                path,
                "Use a supported numeric range.",
            );
        }
    }
    if let Some(items) = value.as_array() {
        if contract["min_items"]
            .as_u64()
            .is_some_and(|min| items.len() < min as usize)
        {
            push(
                issues,
                "home_widget_config_list_too_short",
                path,
                "The list has fewer items than its contract permits.",
            );
        }
        if contract["max_items"]
            .as_u64()
            .is_some_and(|max| items.len() > max as usize)
        {
            push(
                issues,
                "home_widget_config_list_too_long",
                path,
                "The list has more items than its contract permits.",
            );
        }
        if contract["unique_items"] == true
            && items
                .iter()
                .map(Value::to_string)
                .collect::<HashSet<_>>()
                .len()
                != items.len()
        {
            push(
                issues,
                "home_widget_config_list_duplicate",
                path,
                "List values must be unique.",
            );
        }
        let mut seen = HashSet::new();
        for (i, item) in items.iter().enumerate() {
            let item_path = format!("{path}[{i}]");
            if contract["item_enum"]
                .as_array()
                .is_some_and(|allowed| !allowed.contains(item))
            {
                push(
                    issues,
                    "home_widget_config_list_option_invalid",
                    &item_path,
                    "Choose a list value advertised by this widget contract.",
                );
            }
            if kind == "object_list" && contract["item"].is_object() {
                if let Some(key) = contract["item"]["unique_by"].as_str() {
                    if let Some(id) = item.get(key).filter(|v| **v != json!("")) {
                        if !seen.insert(id.to_string()) {
                            push(
                                issues,
                                "home_widget_config_item_id_duplicate",
                                &format!("{item_path}.{key}"),
                                "Item identity values must be unique.",
                            );
                        }
                    }
                }
                object(item, &contract["item"], &item_path, issues);
            }
        }
    }
}
fn object(config: &Value, contract: &Value, path: &str, issues: &mut Vec<Value>) {
    if contract["additional_properties"] != true {
        for key in config.as_object().into_iter().flatten().map(|(key, _)| key) {
            if contract["fields"].get(key).is_none() {
                issues.push(issue("warning","home_widget_config_key_unknown",&format!("{path}.{key}"),"This config key is not advertised by this client. Preserve it on an existing widget."));
            }
        }
    }
    for key in strings(&contract["required"]) {
        if config.get(key).is_none() {
            push(
                issues,
                "home_widget_config_field_missing",
                &format!("{path}.{key}"),
                "The required config field is missing.",
            );
        }
    }
    for (key, contract_field) in contract["fields"].as_object().into_iter().flatten() {
        if let Some(value) = config.get(key) {
            if condition_matches(&contract_field["when"], config, &contract["fields"]) {
                field(value, contract_field, &format!("{path}.{key}"), issues);
            }
        }
    }
    requirements(config, contract, path, issues);
}
fn unknown_config(
    config: &Value,
    contract: &Value,
    path: &str,
    output: &mut BTreeMap<String, Value>,
) {
    for (key, value) in config.as_object().into_iter().flatten() {
        let Some(field) = contract["fields"].get(key) else {
            output.insert(format!("{path}.{key}"), value.clone());
            continue;
        };
        if field["type"] == "object_list" && field["item"].is_object() {
            for (i, item) in value.as_array().into_iter().flatten().enumerate() {
                let identity = field["item"]["unique_by"]
                    .as_str()
                    .and_then(|key| {
                        item.get(key)
                            .filter(|value| **value != json!(""))
                            .map(|id| format!("{key}={id}"))
                    })
                    .unwrap_or_else(|| i.to_string());
                unknown_config(
                    item,
                    &field["item"],
                    &format!("{path}.{key}[{identity}]"),
                    output,
                );
            }
        }
    }
}
fn preservation(layout: &Value, current: &Value, issues: &mut Vec<Value>) {
    let previous = current["widgets"].as_array().cloned().unwrap_or_default();
    let next = layout["widgets"].as_array().cloned().unwrap_or_default();
    for old in &previous {
        let kind = old["type"].as_str().unwrap_or("");
        let new = next.iter().find(|w| w["id"] == old["id"]);
        if contracts().get(kind).is_none() {
            if !new.is_some_and(|w| w == old) {
                push(
                    issues,
                    if new.is_some() {
                        "unknown_widget_changed"
                    } else {
                        "unknown_widget_removed"
                    },
                    "$.widgets",
                    "Unknown widgets must remain unchanged because this client cannot validate them.",
                );
            }
            continue;
        }
        let mut old_unknown = BTreeMap::new();
        unknown_config(
            &old["config"],
            &contracts()[kind],
            "config",
            &mut old_unknown,
        );
        if old_unknown.is_empty() {
            continue;
        }
        let Some(new) = new.filter(|w| w["type"] == old["type"]) else {
            push(
                issues,
                "unknown_widget_config_removed",
                "$.widgets",
                "Widgets containing unadvertised config must be preserved.",
            );
            continue;
        };
        let mut new_unknown = BTreeMap::new();
        unknown_config(
            &new["config"],
            &contracts()[kind],
            "config",
            &mut new_unknown,
        );
        if new_unknown != old_unknown {
            push(
                issues,
                "unknown_widget_config_changed",
                "$.widgets",
                "Unadvertised widget config must remain unchanged.",
            );
        }
    }
    for (i, new) in next.iter().enumerate() {
        let kind = new["type"].as_str().unwrap_or("");
        let old = previous
            .iter()
            .find(|w| w["id"] == new["id"] && w["type"] == new["type"]);
        let path = format!("$.widgets[{i}]");
        if contracts().get(kind).is_none() {
            if old.is_none() {
                push(
                    issues,
                    "unknown_widget_changed",
                    &path,
                    "FlowPilot cannot introduce an unknown widget type.",
                );
            }
            continue;
        }
        let mut new_unknown = BTreeMap::new();
        unknown_config(
            &new["config"],
            &contracts()[kind],
            "config",
            &mut new_unknown,
        );
        let mut old_unknown = BTreeMap::new();
        if let Some(old) = old {
            unknown_config(
                &old["config"],
                &contracts()[kind],
                "config",
                &mut old_unknown,
            );
        }
        if old_unknown.is_empty() && !new_unknown.is_empty() {
            push(
                issues,
                "unknown_widget_config_introduced",
                &format!("{path}.config"),
                "FlowPilot cannot introduce unadvertised configuration.",
            );
        }
    }
}
fn minimum_rows(widget: &Value) -> u64 {
    match widget["type"].as_str().unwrap_or("") {
        "saved-forecast" => 4,
        "data" => {
            if ["stat", "metricstrip", "progress", "bullet"]
                .contains(&widget["config"]["visualization"].as_str().unwrap_or(""))
            {
                2
            } else {
                3
            }
        }
        "app-embed" => 3,
        "section-heading" | "greeting" | "flowpilot" | "quick-actions" => 1,
        "workspace-pulse" if widget["config"]["mode"] == "strip" => 1,
        _ => 2,
    }
}
fn special_config(kind: &str, config: &Value, path: &str, issues: &mut Vec<Value>) {
    if kind == "data" {
        let visual = config["visualization"].as_str().unwrap_or("bar");
        let records = [
            "kanban",
            "record",
            "graph",
            "scatter",
            "timeline",
            "recordcalendar",
            "comparison",
            "table",
            "list",
            "cards",
        ]
        .contains(&visual)
            && (!["table", "list", "cards"].contains(&visual) || config["mode"] != "aggregate");
        let mode = if records { "records" } else { "aggregate" };
        if config["mode"].as_str().is_some_and(|requested| {
            ["aggregate", "records"].contains(&requested) && requested != mode
        }) {
            push(
                issues,
                "data_mode_incompatible",
                &format!("{path}.mode"),
                &format!("Use mode '{mode}' with visualization '{visual}'."),
            );
        }
        for key in config["queryParams"]
            .as_object()
            .into_iter()
            .flatten()
            .map(|(key, _)| key)
        {
            if key.starts_with("__home_") {
                push(
                    issues,
                    "data_query_parameter_reserved",
                    &format!("{path}.queryParams.{key}"),
                    "Saved-query parameter names cannot start with __home_.",
                );
            }
        }
    }
    if kind == "app-embed" {
        for key in ["query", "route"] {
            if let Some(raw) = config[key].as_str() {
                let query = if key == "route" {
                    raw.split_once('?').map(|(_, q)| q).unwrap_or("")
                } else {
                    raw.strip_prefix('?').unwrap_or(raw)
                };
                let mut query_url =
                    reqwest::Url::parse("https://home.invalid/").expect("Static URL is valid");
                query_url.set_query(Some(query));
                for (name, _) in query_url.query_pairs() {
                    if [
                        "id",
                        "route",
                        "eventId",
                        "__proto__",
                        "constructor",
                        "prototype",
                    ]
                    .contains(&name.as_ref())
                    {
                        push(
                            issues,
                            "app_embed_query_key_reserved",
                            &format!("{path}.{key}"),
                            &format!(
                                "Query parameter '{name}' is reserved and would be discarded."
                            ),
                        );
                    }
                }
            }
        }
    }
}
fn at_path<'a>(widget: &'a Value, path: &str) -> Option<&'a Value> {
    let normalized = path.replace('[', ".").replace(']', "");
    let mut value = widget;
    for key in normalized.split('.') {
        value = if let Ok(index) = key.parse::<usize>() {
            value.get(index)?
        } else {
            value.get(key)?
        };
    }
    Some(value)
}

pub(super) fn validate(raw: &Value, current: &Value) -> Value {
    let Some(layout) = normalize_layout(raw) else {
        return json!({"status":"validation_error","valid":false,"issues":[issue("error","home_layout_invalid","$","Expected a version 1 Home layout with at most 80 widgets, each with a nonempty unique id and type.")]});
    };
    if let Err(error) = flow_like::profile::validate_home_layout(&layout) {
        return json!({"status":"validation_error","valid":false,"issues":[issue("error","home_layout_invalid","$",&error)]});
    }
    let mut issues = Vec::new();
    if layout != *raw {
        issues.push(issue(
            "warning",
            "home_layout_normalized",
            "$",
            "Optional widget fields and bounded sizes were normalized in canonical_layout.",
        ));
    }
    if layout["widgets"].as_array().unwrap().is_empty() {
        issues.push(issue(
            "warning",
            "home_layout_empty",
            "$.widgets",
            "The Home layout has no widgets.",
        ));
    }
    for (i, widget) in layout["widgets"].as_array().unwrap().iter().enumerate() {
        let start = issues.len();
        let path = format!("$.widgets[{i}]");
        let kind = widget["type"].as_str().unwrap_or("");
        let previous = current["widgets"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|old| old["id"] == widget["id"] && old["type"] == widget["type"]);
        for field in ["className", "css"] {
            if raw["widgets"][i]["appearance"]
                .get(field)
                .is_some_and(|v| !v.is_string())
            {
                push(
                    &mut issues,
                    if field == "css" {
                        "home_widget_css_type_invalid"
                    } else {
                        "home_widget_class_name_type_invalid"
                    },
                    &format!("{path}.appearance.{field}"),
                    "Use a string or omit this styling field.",
                );
            }
        }
        if contracts().get(kind).is_none() {
            issues.push(issue(
                "warning",
                "unknown_widget_type",
                &format!("{path}.type"),
                "This widget type is unavailable in this client.",
            ));
            continue;
        }
        if widget["size"]["rows"].as_u64().unwrap_or(0) < minimum_rows(widget) {
            push(
                &mut issues,
                "widget_too_short",
                &format!("{path}.size.rows"),
                &format!("This widget needs at least {} rows.", minimum_rows(widget)),
            );
        }
        let variant = widget["appearance"]["variant"].as_str().unwrap_or("");
        let legacy = match kind {
            "app-collection" => [
                "grid",
                "standard",
                "compact",
                "list",
                "editorial",
                "icons",
                "carousel",
                "spotlight",
            ]
            .contains(&variant),
            "packages" => ["grid", "standard", "compact", "featured", "list"].contains(&variant),
            "models" => ["grid", "standard", "list"].contains(&variant),
            "quick-links" => ["grid", "list"].contains(&variant),
            _ => false,
        };
        if !["card", "borderless", "tinted", "solid"].contains(&variant) && !legacy {
            push(
                &mut issues,
                "unknown_widget_variant",
                &format!("{path}.appearance.variant"),
                "Use card, borderless, tinted, or solid.",
            );
        } else if variant == "solid"
            && ![
                "app-spotlight",
                "app-ranking",
                "app-collection-feature",
                "model-spotlight",
            ]
            .contains(&kind)
        {
            push(
                &mut issues,
                "unsupported_solid_variant",
                &format!("{path}.appearance.variant"),
                "This widget does not support the solid surface.",
            );
        }
        if !catalog()["accents"]
            .as_array()
            .unwrap()
            .contains(&widget["appearance"]["accent"])
        {
            push(
                &mut issues,
                "unknown_widget_accent",
                &format!("{path}.appearance.accent"),
                "Choose an accent advertised by the Home catalog.",
            );
        }
        styling(widget, &path, &mut issues);
        object(
            &widget["config"],
            &contracts()[kind],
            &format!("{path}.config"),
            &mut issues,
        );
        special_config(
            kind,
            &widget["config"],
            &format!("{path}.config"),
            &mut issues,
        );
        if let Some(previous) = previous {
            for entry in &mut issues[start..] {
                if [
                    "unknown_widget_variant",
                    "unknown_widget_accent",
                    "home_widget_config_option_invalid",
                    "home_widget_config_list_option_invalid",
                ]
                .contains(&entry["code"].as_str().unwrap_or(""))
                {
                    if let Some(relative) = entry["path"]
                        .as_str()
                        .and_then(|p| p.strip_prefix(&format!("{path}.")))
                    {
                        if at_path(widget, relative).is_some()
                            && at_path(widget, relative) == at_path(previous, relative)
                        {
                            entry["severity"] = json!("warning");
                            entry["code"] = json!("home_widget_future_value_preserved");
                            entry["message"] = json!(
                                "This unsupported value is unchanged. Preserve it exactly or choose an advertised value."
                            );
                        }
                    }
                }
            }
            for field in ["className", "css"] {
                if previous["appearance"][field]
                    .as_str()
                    .is_some_and(|s| !s.trim().is_empty())
                    && !widget["appearance"][field]
                        .as_str()
                        .is_some_and(|s| !s.trim().is_empty())
                {
                    issues.push(issue("warning","home_widget_styling_removed",&format!("{path}.appearance.{field}"),"The candidate drops existing styling. Preserve it unless the user requested removal."));
                }
            }
        }
    }
    preservation(&layout, current, &mut issues);
    let valid = !issues.iter().any(|issue| issue["severity"] == "error");
    let bytes = serde_json::to_vec(&layout).unwrap().len();
    json!({"status":if valid{"ok"}else{"validation_error"},"valid":valid,"issues":issues,"fingerprint":fingerprint(&layout),"byte_count":bytes,"canonical_layout":layout})
}
fn class_parts(token: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut depth = 0;
    let mut start = 0;
    for (i, c) in token.char_indices() {
        match c {
            '[' | '(' => depth += 1,
            ']' | ')' if depth > 0 => depth -= 1,
            ':' if depth == 0 => {
                parts.push(&token[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    parts.push(&token[start..]);
    parts
}
fn root_selector(selector: &str) -> bool {
    selector.starts_with(":root")
        || ["html", "body"].iter().any(|root| {
            selector
                .strip_prefix(root)
                .is_some_and(|tail| tail.is_empty() || tail.starts_with(['.', '#', ':', '[']))
        })
}
fn selector_flags(selector: &str) -> (bool, Option<char>) {
    let mut depth = 0;
    let mut quote = None;
    let mut escaped = false;
    let mut pseudo = false;
    for (i, c) in selector.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        if c == '\\' {
            escaped = true;
            continue;
        }
        if let Some(q) = quote {
            if c == q {
                quote = None;
            }
            continue;
        }
        if c == '\'' || c == '"' {
            quote = Some(c);
            continue;
        }
        match c {
            '[' | '(' => depth += 1,
            ']' | ')' => depth -= 1,
            _ => {}
        }
        if depth != 0 {
            continue;
        }
        if c == ':'
            && (selector[i..].starts_with("::")
                || [":before", ":after", ":first-line", ":first-letter"]
                    .iter()
                    .any(|p| {
                        selector[i..].strip_prefix(p).is_some_and(|tail| {
                            tail.chars()
                                .next()
                                .is_none_or(|c| !c.is_alphanumeric() && c != '-' && c != '_')
                        })
                    }))
        {
            pseudo = true;
        }
        if c.is_whitespace() || ['>', '+', '~'].contains(&c) {
            let next = selector[i..].trim_start().chars().next().unwrap_or(' ');
            return (
                pseudo,
                Some(if ['>', '+', '~'].contains(&next) {
                    next
                } else {
                    ' '
                }),
            );
        }
    }
    (pseudo, None)
}
fn surface_selector(selector: &str) -> bool {
    let (pseudo, combinator) = selector_flags(selector);
    root_selector(selector) && !pseudo && combinator.is_none()
}
fn css_problems(css: &str) -> Result<(Vec<String>, Vec<String>, Vec<String>), String> {
    let mut out = Vec::new();
    let mut tailwind = Vec::new();
    let mut wrapped = Vec::new();
    let mut stack = Vec::<(String, bool)>::new();
    let mut text = String::new();
    let chars = css.chars().collect::<Vec<_>>();
    let mut at = 0;
    let mut quote = None;
    let mut escaped = false;
    let mut nesting = 0;
    let add = |values: &mut Vec<String>, value: String| {
        if !values.contains(&value) {
            values.push(value);
        }
    };
    let global = [
        "property",
        "font-face",
        "counter-style",
        "font-palette-values",
        "page",
        "import",
    ];
    let tw = [
        "apply",
        "tailwind",
        "theme",
        "variant",
        "custom-variant",
        "utility",
        "config",
        "plugin",
        "source",
        "reference",
        "screen",
    ];
    let property=Regex::new(r"(?i)^(?:position|inset(?:-.+)?|top|right|bottom|left|z-index|grid-(?:area|column|row)(?:-.+)?|(?:align|justify|place)-self|order|(?:min-|max-)?(?:width|height))$").unwrap();
    let functions = Regex::new(r"(?:^|[^\w-])(theme|--alpha|--spacing)\(").unwrap();
    let colors =
        Regex::new(r"(?i)\b(hsla?|rgba?|oklch|oklab|lab|lch)\(\s*var\(\s*(--[\w-]+)").unwrap();
    while at < chars.len() {
        let c = chars[at];
        if escaped {
            text.push(c);
            escaped = false;
            at += 1;
            continue;
        }
        if c == '\\' {
            text.push(c);
            escaped = true;
            at += 1;
            continue;
        }
        if let Some(q) = quote {
            text.push(c);
            if c == q {
                quote = None;
            }
            at += 1;
            continue;
        }
        if c == '\'' || c == '"' {
            quote = Some(c);
            text.push(c);
            at += 1;
            continue;
        }
        if c == '/' && chars.get(at + 1) == Some(&'*') {
            at += 2;
            while at + 1 < chars.len() && !(chars[at] == '*' && chars[at + 1] == '/') {
                at += 1;
            }
            if at + 1 >= chars.len() {
                return Err("Unclosed CSS comment.".into());
            }
            at += 2;
            text.push(' ');
            continue;
        }
        if c == '(' || c == '[' {
            nesting += 1;
        } else if c == ')' || c == ']' {
            nesting -= 1;
            if nesting < 0 {
                return Err("Unbalanced CSS group.".into());
            }
        }
        if nesting > 0 || !['{', '}', ';'].contains(&c) {
            text.push(c);
            at += 1;
            continue;
        }
        let segment = text.trim().to_owned();
        text.clear();
        if segment.starts_with('@') {
            let name = segment[1..]
                .split(|c: char| !c.is_ascii_alphabetic() && c != '-')
                .next()
                .unwrap_or("")
                .to_lowercase();
            if global.contains(&name.as_str()) {
                add(&mut out, format!("@{name}"));
            }
            if tw.contains(&name.as_str()) {
                add(&mut tailwind, format!("@{name}"));
            }
        }
        if c == '{' {
            let surface = segment
                .split(',')
                .any(|selector| surface_selector(selector.trim()));
            let nested = stack.last().is_some_and(|(_, surface)| *surface);
            if segment.split(',').any(|raw| {
                let s = raw.trim();
                (nested && s.starts_with(['+', '~']))
                    || ((root_selector(s) || (nested && s.starts_with('&')))
                        && selector_flags(s).1.is_some_and(|c| c == '+' || c == '~'))
            }) {
                add(&mut out, segment.clone());
            }
            stack.push((segment.clone(), surface));
        } else if !segment.is_empty() && !segment.starts_with('@') {
            let Some((key, value)) = segment.split_once(':') else {
                return Err("A CSS declaration is missing its separator.".into());
            };
            let key = key.trim();
            let value = value.trim();
            if key.eq_ignore_ascii_case("position") && value.eq_ignore_ascii_case("fixed") {
                add(&mut out, "position: fixed".into());
            }
            if stack.last().is_some_and(|(_, surface)| *surface) && property.is_match(key) {
                add(&mut out, format!("{key} on :root"));
            }
            for capture in colors.captures_iter(value) {
                add(
                    &mut wrapped,
                    format!("{}(var({}))", capture[1].to_lowercase(), &capture[2]),
                );
            }
        }
        for capture in functions.captures_iter(&segment) {
            add(&mut tailwind, format!("{}()", &capture[1]));
        }
        if c == '}' && stack.pop().is_none() {
            return Err("Unexpected closing CSS block.".into());
        }
        at += 1;
    }
    if !stack.is_empty() || quote.is_some() || nesting != 0 || !text.trim().is_empty() {
        return Err("Unclosed CSS rule.".into());
    }
    Ok((out, tailwind, wrapped))
}
fn styling(widget: &Value, path: &str, issues: &mut Vec<Value>) {
    if let Some(classes) = widget["appearance"]["className"].as_str() {
        let owned=Regex::new(r"^(?:static|fixed|absolute|relative|sticky|z-.+|(?:col|row)-(?:span|start|end)-.+|self-.+)$").unwrap();
        let mut bad = Vec::new();
        for token in classes.split_whitespace() {
            let mut parts = class_parts(token);
            let utility = parts
                .pop()
                .unwrap_or("")
                .trim_start_matches('!')
                .trim_start_matches('-')
                .trim_end_matches('!');
            let other = parts.iter().any(|variant| {
                [
                    "before",
                    "after",
                    "first-letter",
                    "first-line",
                    "marker",
                    "selection",
                    "placeholder",
                    "file",
                    "backdrop",
                    "*",
                    "**",
                ]
                .contains(variant)
                    || (variant.starts_with("[&")
                        && (variant[2..].contains(['_', '>']) || variant[2..].contains("::")))
            });
            if utility == "fixed"
                || (owned.is_match(utility) && !other)
                || parts
                    .iter()
                    .any(|variant| variant.starts_with('[') && variant.contains(['~', '+']))
            {
                bad.push(token);
            }
        }
        if !bad.is_empty() {
            issues.push(issue("warning","home_widget_class_name_out_of_scope",&format!("{path}.appearance.className"),&format!("These classes have no effect or reach outside the widget: {}. The layout owns the surface position and grid placement.",bad.join(", "))));
        }
    }
    if let Some(css) = widget["appearance"]["css"].as_str() {
        let css_path = format!("{path}.appearance.css");
        match css_problems(css){Err(error)=>issues.push(issue("warning","home_widget_css_invalid",&css_path,&format!("This CSS does not parse ({error}). Only complete rules outside the broken block will apply."))),Ok((scope,tw,colors))=>{for(code,values,message)in[("home_widget_css_out_of_scope",scope,"These parts of the CSS reach outside the widget or fight the layout"),("home_widget_css_tailwind_syntax",tw,"This CSS uses Tailwind-only syntax that does nothing here"),("home_widget_css_wrapped_color_variable",colors,"These declarations wrap a complete color variable in a color function")]{if !values.is_empty(){issues.push(issue("warning",code,&css_path,&format!("{message}: {}.",values.join(", "))));}}}}
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn widget() -> Value {
        json!({"id":"one","type":"quick-links","size":{"columns":6,"rows":3},"appearance":{"variant":"card","accent":"neutral"},"config":{}})
    }
    fn layout(widget: Value) -> Value {
        json!({"version":1,"widgets":[widget]})
    }
    #[test]
    fn rejects_authored_unknown_config() {
        let mut w = widget();
        w["config"]["future"] = json!("private");
        let result = validate(&layout(w), &layout(widget()));
        assert_eq!(result["valid"], false);
        assert!(
            result["issues"]
                .as_array()
                .unwrap()
                .iter()
                .any(|i| i["code"] == "unknown_widget_config_introduced")
        );
    }
    #[test]
    fn preserves_unknown_widgets_and_fields() {
        let mut w = widget();
        w["type"] = json!("future-widget");
        w["config"]["future"] = json!({"value":42});
        let original = layout(w);
        assert_eq!(validate(&original, &original)["valid"], true);
        assert_eq!(
            validate(&json!({"version":1,"widgets":[]}), &original)["valid"],
            false
        );
    }
    #[test]
    fn rejects_unsafe_urls_and_reserved_embed_parameters() {
        let mut w = widget();
        w["type"] = json!("app-embed");
        w["config"] = json!({"appId":"app","query":"eventId=override"});
        assert!(
            validate(&layout(w), &json!({"version":1,"widgets":[]}))["issues"]
                .as_array()
                .unwrap()
                .iter()
                .any(|i| i["code"] == "app_embed_query_key_reserved")
        );
        assert!(safe_url("javascript:alert(1)").is_none());
        assert!(safe_url("//evil.test").is_none());
    }
    #[test]
    fn css_distinguishes_surface_from_decoration() {
        assert!(
            css_problems(
                ":root::before { position: absolute; width: 100%; } body:after { height: 2px; }"
            )
            .unwrap()
            .0
            .is_empty()
        );
        assert!(
            css_problems("p { position: fixed; }")
                .unwrap()
                .0
                .contains(&"position: fixed".into())
        );
        assert!(css_problems(":root { color: red;").is_err());
        assert!(
            css_problems(":root { h2 { height: 2rem; } & > p ~ p { margin: 0; } }")
                .unwrap()
                .0
                .is_empty()
        );
    }
}
