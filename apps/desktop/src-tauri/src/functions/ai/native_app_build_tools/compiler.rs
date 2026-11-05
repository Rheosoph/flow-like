use super::*;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

fn issue(code: &str, path: Value, message: impl Into<String>) -> Value {
    json!({"code":code,"path":path,"message":message.into()})
}
fn array(value: &Value) -> &[Value] {
    value.as_array().map(Vec::as_slice).unwrap_or(&[])
}
fn text(value: &Value) -> &str {
    value.as_str().unwrap_or("")
}
fn strings(value: &Value) -> Vec<&str> {
    array(value).iter().filter_map(Value::as_str).collect()
}
fn sorted_unique(value: &Value) -> Vec<String> {
    let mut values = strings(value)
        .into_iter()
        .map(str::to_owned)
        .collect::<Vec<_>>();
    values.sort_by(|a, b| compare(a, b));
    values.dedup();
    values
}
fn trim_fields(value: &mut Value, fields: &[&str]) {
    for field in fields {
        if let Some(s) = value[*field].as_str() {
            value[*field] = json!(s.trim());
        }
    }
}
pub(super) fn normalize(mut input: Value) -> Value {
    trim_fields(&mut input, &["name", "description"]);
    for requirement in input
        .get_mut("requirements")
        .and_then(Value::as_array_mut)
        .into_iter()
        .flatten()
    {
        trim_fields(requirement, &["id", "description"]);
    }
    for resource in input
        .get_mut("resources")
        .and_then(Value::as_array_mut)
        .into_iter()
        .flatten()
    {
        trim_fields(resource, &["key"]);
        for key in ["depends_on", "requirement_ids"] {
            for value in resource
                .get_mut(key)
                .and_then(Value::as_array_mut)
                .into_iter()
                .flatten()
            {
                if let Some(s) = value.as_str() {
                    *value = json!(s.trim());
                }
            }
        }
        if let Some(config) = resource.get_mut("config") {
            trim_fields(config, &["name", "instruction", "route", "board", "page"]);
        }
        for column in resource
            .get_mut("config")
            .and_then(|config| config.get_mut("columns"))
            .and_then(Value::as_array_mut)
            .into_iter()
            .flatten()
        {
            trim_fields(column, &["name"]);
        }
    }
    for scenario in input
        .get_mut("scenarios")
        .and_then(Value::as_array_mut)
        .into_iter()
        .flatten()
    {
        trim_fields(scenario, &["description"]);
    }
    input
}
fn unique(values: Vec<&str>, code: &str, path: Value, issues: &mut Vec<Value>) {
    let mut seen = HashSet::new();
    let mut duplicated = HashSet::new();
    for value in values {
        if !seen.insert(value) && duplicated.insert(value) {
            issues.push(issue(
                code,
                path.clone(),
                format!("Identifier '{value}' is repeated."),
            ));
        }
    }
}
fn pointer(value: &str) -> bool {
    if !value.is_empty() && !value.starts_with('/') {
        return false;
    }
    let mut chars = value.chars();
    while let Some(c) = chars.next() {
        if c == '~' && !matches!(chars.next(), Some('0' | '1')) {
            return false;
        }
    }
    true
}
fn check_references(
    value: &Value,
    steps: &HashMap<&str, usize>,
    before: usize,
    path: Value,
    issues: &mut Vec<Value>,
) {
    if let Some(reference) = value.get("$scenario_ref") {
        let source = text(&reference["source"]);
        let step = text(&reference["step_id"]);
        if !reference.is_object()
            || !matches!(source, "app" | "target" | "step")
            || source == "step"
                && (step.is_empty() || steps.get(step).is_none_or(|&index| index >= before))
            || source != "step" && reference.get("step_id").is_some()
            || reference.get("path").is_some_and(|p| {
                p.as_str()
                    .is_none_or(|p| !pointer(p) || p.encode_utf16().count() > 512)
            })
            || reference.as_object().is_some_and(|o| {
                o.keys()
                    .any(|key| !["source", "step_id", "path"].contains(&key.as_str()))
            })
            || value.as_object().is_some_and(|o| o.len() != 1)
        {
            issues.push(issue("invalid_schema",path,"Scenario references must identify an earlier step, app, or target with a valid JSON pointer."));
        }
        return;
    }
    match value {
        Value::Array(items) => {
            for (index, item) in items.iter().enumerate() {
                let mut p = path.clone();
                p.as_array_mut().unwrap().push(json!(index));
                check_references(item, steps, before, p, issues);
            }
        }
        Value::Object(map) => {
            for (key, item) in map {
                let mut p = path.clone();
                p.as_array_mut().unwrap().push(json!(key));
                check_references(item, steps, before, p, issues);
            }
        }
        _ => {}
    }
}
fn semantic(spec: &Value) -> Vec<Value> {
    let mut issues = Vec::new();
    let resources = array(&spec["resources"]);
    let scenarios = array(&spec["scenarios"]);
    let requirements = array(&spec["requirements"]);
    unique(
        requirements.iter().map(|r| text(&r["id"])).collect(),
        "duplicate_identifier",
        json!(["requirements"]),
        &mut issues,
    );
    unique(
        resources.iter().map(|r| text(&r["key"])).collect(),
        "duplicate_identifier",
        json!(["resources"]),
        &mut issues,
    );
    unique(
        scenarios.iter().map(|r| text(&r["id"])).collect(),
        "duplicate_identifier",
        json!(["scenarios"]),
        &mut issues,
    );
    let keys = resources
        .iter()
        .map(|r| (text(&r["key"]), r))
        .collect::<HashMap<_, _>>();
    let requirements_set = requirements
        .iter()
        .map(|r| text(&r["id"]))
        .collect::<HashSet<_>>();
    for (category, items) in [("resources", resources), ("scenarios", scenarios)] {
        for (index, item) in items.iter().enumerate() {
            let ids = strings(&item["requirement_ids"]);
            unique(
                ids.clone(),
                "duplicate_requirement_reference",
                json!([category, index, "requirement_ids"]),
                &mut issues,
            );
            for id in ids {
                if !requirements_set.contains(id) {
                    issues.push(issue(
                        "unknown_requirement",
                        json!([category, index, "requirement_ids"]),
                        format!("Unknown requirement '{id}'."),
                    ));
                }
            }
        }
    }
    for (index, resource) in resources.iter().enumerate() {
        let key = text(&resource["key"]);
        let kind = text(&resource["kind"]);
        let config = &resource["config"];
        let dependencies = strings(&resource["depends_on"]);
        unique(
            dependencies.clone(),
            "duplicate_dependency",
            json!(["resources", index, "depends_on"]),
            &mut issues,
        );
        for dependency in &dependencies {
            if *dependency == key {
                issues.push(issue(
                    "self_dependency",
                    json!(["resources", index, "depends_on"]),
                    "A resource cannot depend on itself.",
                ));
            } else if !keys.contains_key(dependency) {
                issues.push(issue(
                    "unknown_dependency",
                    json!(["resources", index, "depends_on"]),
                    format!("Unknown resource '{dependency}'."),
                ));
            }
        }
        let mut typed = |field: &str, expected: &str| {
            if let Some(reference) = config[field].as_str() {
                match keys.get(reference) {
                    None => issues.push(issue(
                        "unknown_dependency",
                        json!(["resources", index, "config", field]),
                        format!("Unknown {expected} '{reference}'."),
                    )),
                    Some(target) => {
                        if target["kind"] != expected {
                            issues.push(issue(
                                "wrong_resource_kind",
                                json!(["resources", index, "config", field]),
                                format!("'{reference}' must be a {expected}."),
                            ));
                        }
                        if !dependencies.contains(&reference) {
                            issues.push(issue(
                                "missing_symbolic_dependency",
                                json!(["resources", index, "depends_on"]),
                                format!(
                                    "Symbolic reference '{reference}' must appear in depends_on."
                                ),
                            ));
                        }
                    }
                }
            }
        };
        if kind == "page" {
            typed("board", "board");
        }
        if kind == "event" {
            typed("board", "board");
            typed("page", "page");
        }
        if kind == "page" {
            if config.get("on_interval_entry").is_some() != config.get("interval_seconds").is_some()
            {
                issues.push(issue(
                    "invalid_schema",
                    json!(["resources", index, "config"]),
                    "on_interval_entry and interval_seconds must appear together.",
                ));
            }
            for field in ["on_load_entry", "on_unload_entry", "on_interval_entry"] {
                if config[field].as_str().is_some_and(|s| s.trim().is_empty()) {
                    issues.push(issue(
                        "invalid_schema",
                        json!(["resources", index, "config", field]),
                        "Entry selectors must not be blank.",
                    ));
                }
            }
            let count = resources
                .iter()
                .filter(|r| {
                    r["kind"] == "event"
                        && r["config"]["event_type"] == "page"
                        && r["config"]["page"] == key
                })
                .count();
            if count != 1 {
                issues.push(issue(
                    if count == 0 {
                        "missing_page_event"
                    } else {
                        "duplicate_page_event"
                    },
                    json!(["resources", index]),
                    format!("Page '{key}' requires exactly one page Event."),
                ));
            }
        }
        if kind == "event" {
            let page = config["event_type"] == "page";
            if page && (config.get("page").is_none() || config.get("entry_node").is_some())
                || !page
                    && (config.get("board").is_none()
                        || config.get("entry_node").is_none()
                        || config.get("page").is_some())
                || config["entry_node"]
                    .as_str()
                    .is_some_and(|s| s.trim().is_empty())
            {
                issues.push(issue(
                    "invalid_event_target",
                    json!(["resources", index, "config"]),
                    "Page Events require page; workflow Events require board and exact entry_node.",
                ));
            }
            if page {
                if let Some(target) = keys.get(text(&config["page"])) {
                    if target["kind"] == "page"
                        && (config["route"] != target["config"]["route"]
                            || config
                                .get("board")
                                .is_some_and(|board| *board != target["config"]["board"]))
                    {
                        issues.push(issue(
                            "invalid_event_target",
                            json!(["resources", index, "config"]),
                            "Page Event route and optional board must match its page.",
                        ));
                    }
                }
            }
        }
        if kind == "table" {
            unique(
                array(&config["columns"])
                    .iter()
                    .map(|c| text(&c["name"]))
                    .collect(),
                "duplicate_identifier",
                json!(["resources", index, "config", "columns"]),
                &mut issues,
            );
            for (ci, column) in array(&config["columns"]).iter().enumerate() {
                if (column["type"] == "vector") != column.get("vector_size").is_some() {
                    issues.push(issue(
                        "invalid_schema",
                        json!(["resources", index, "config", "columns", ci]),
                        "vector_size is required exactly for vector columns.",
                    ));
                }
            }
        }
    }
    let mut page_routes = HashMap::new();
    let mut event_routes = HashMap::new();
    let mut table_names = HashMap::new();
    for (index, resource) in resources.iter().enumerate() {
        if resource["kind"] == "page" {
            if page_routes
                .insert(text(&resource["config"]["route"]), text(&resource["key"]))
                .is_some()
            {
                issues.push(issue(
                    "duplicate_route",
                    json!(["resources", index, "config", "route"]),
                    "Two pages claim the same route.",
                ));
            }
        }
    }
    for (index, resource) in resources.iter().enumerate() {
        if resource["kind"] == "event" {
            if let Some(route) = resource["config"]["route"].as_str() {
                if event_routes.insert(route, text(&resource["key"])).is_some()
                    || page_routes
                        .get(route)
                        .is_some_and(|page| resource["config"]["page"] != *page)
                {
                    issues.push(issue(
                        "duplicate_route",
                        json!(["resources", index, "config", "route"]),
                        "Event route collides with another page or Event.",
                    ));
                }
            }
        }
        if resource["kind"] == "table"
            && table_names
                .insert(text(&resource["config"]["name"]), text(&resource["key"]))
                .is_some()
        {
            issues.push(issue(
                "duplicate_table_name",
                json!(["resources", index, "config", "name"]),
                "Two tables claim the same physical name.",
            ));
        }
    }
    for (index, scenario) in scenarios.iter().enumerate() {
        let target = &scenario["target"];
        let resource = keys.get(text(&target["resource_key"]));
        let valid = match text(&target["kind"]) {
            "app" => target["resource_key"] == "app",
            "chat" => resource.is_some_and(|r| {
                r["kind"] == "event" && r["config"]["event_type"] == "simple_chat"
            }),
            kind => resource.is_some_and(|r| r["kind"] == kind),
        };
        if !valid {
            issues.push(issue(
                "invalid_scenario_target",
                json!(["scenarios", index, "target"]),
                "Scenario target has no compatible resource.",
            ));
        }
        let steps = array(&scenario["steps"]);
        unique(
            steps.iter().map(|s| text(&s["id"])).collect(),
            "invalid_schema",
            json!(["scenarios", index, "steps"]),
            &mut issues,
        );
        let step_indexes = steps
            .iter()
            .enumerate()
            .map(|(i, s)| (text(&s["id"]), i))
            .collect::<HashMap<_, _>>();
        unique(
            array(&scenario["assertions"])
                .iter()
                .map(|a| text(&a["id"]))
                .collect(),
            "invalid_schema",
            json!(["scenarios", index, "assertions"]),
            &mut issues,
        );
        for (si, step) in steps.iter().enumerate() {
            check_references(
                &step["arguments"],
                &step_indexes,
                si,
                json!(["scenarios", index, "steps", si, "arguments"]),
                &mut issues,
            );
        }
        for (ai, assertion) in array(&scenario["assertions"]).iter().enumerate() {
            let path = json!(["scenarios", index, "assertions", ai]);
            let step = step_indexes
                .get(text(&assertion["step_id"]))
                .and_then(|&i| steps.get(i));
            if step.is_none() {
                issues.push(issue(
                    "invalid_schema",
                    path.clone(),
                    "Assertion references an unknown step.",
                ));
            }
            if ["state", "run_outcome"].contains(&text(&assertion["kind"]))
                && step.is_some_and(|s| {
                    !["call_app_event", "call_app_chat", "interact_app_page"]
                        .contains(&text(&s["tool"]))
                })
            {
                issues.push(issue(
                    "invalid_schema",
                    path.clone(),
                    "State and run-outcome assertions require a runtime step.",
                ));
            }
            for field in ["path", "run_id_path", "status_path"] {
                if assertion[field].as_str().is_some_and(|s| !pointer(s)) {
                    issues.push(issue(
                        "invalid_schema",
                        path.clone(),
                        "Assertion paths must be RFC 6901 JSON pointers.",
                    ));
                }
            }
            if assertion["kind"] == "run_outcome" {
                if assertion["run_id_path"] == ""
                    || assertion["status_path"] == "/status"
                    || assertion.get("allowed_statuses").is_some()
                        && assertion["status_path"].as_str().is_none_or(str::is_empty)
                {
                    issues.push(issue("invalid_schema",path,"Run outcome requires a run-id field and cannot certify a transport acknowledgement."));
                }
            } else {
                let has = assertion.get("expected").is_some();
                if (assertion["operator"] == "exists") == has
                    || assertion["operator"] == "length_at_least"
                        && assertion["expected"]
                            .as_u64()
                            .is_none_or(|n| n > 9_007_199_254_740_991)
                {
                    issues.push(issue(
                        "invalid_schema",
                        path,
                        "The assertion expected value does not match its operator.",
                    ));
                }
            }
        }
    }
    for requirement in requirements {
        let id = text(&requirement["id"]);
        if !resources
            .iter()
            .chain(scenarios)
            .any(|item| strings(&item["requirement_ids"]).contains(&id))
        {
            issues.push(issue(
                "uncovered_requirement",
                json!(["requirements"]),
                format!("Requirement '{id}' is not covered by a resource or scenario."),
            ));
        }
    }
    if waves(resources).iter().map(Vec::len).sum::<usize>() != resources.len() {
        issues.push(issue(
            "dependency_cycle",
            json!(["resources"]),
            "The resource dependency graph contains a cycle.",
        ));
    }
    issues
}
fn waves(resources: &[Value]) -> Vec<Vec<String>> {
    let mut pending = resources
        .iter()
        .map(|r| {
            (
                text(&r["key"]).to_owned(),
                strings(&r["depends_on"])
                    .into_iter()
                    .map(str::to_owned)
                    .collect::<BTreeSet<_>>(),
            )
        })
        .collect::<BTreeMap<_, _>>();
    let mut result = Vec::new();
    while !pending.is_empty() {
        let mut ready = pending
            .iter()
            .filter(|(_, deps)| deps.is_empty())
            .map(|(key, _)| key.clone())
            .collect::<Vec<_>>();
        ready.sort_by(|a, b| compare(a, b));
        if ready.is_empty() {
            break;
        }
        for key in &ready {
            pending.remove(key);
        }
        for deps in pending.values_mut() {
            for key in &ready {
                deps.remove(key);
            }
        }
        result.push(ready);
    }
    result
}
pub(crate) fn compile(input: &Value, app_id: &str, build_id: &str) -> Result<Value, String> {
    let mut spec = normalize(input.clone());
    let validator =
        jsonschema::validator_for(&contract()["spec_schema"]).map_err(|e| e.to_string())?;
    let errors = validator
        .iter_errors(&spec)
        .take(64)
        .map(|error| {
            issue(
                "invalid_schema",
                json!([error.instance_path.to_string()]),
                error.to_string(),
            )
        })
        .collect::<Vec<_>>();
    if !errors.is_empty() {
        return Err(json!({"status":"validation_error","issues":errors}).to_string());
    }
    if serde_json::to_vec(&spec).map_err(|e| e.to_string())?.len() > 512 * 1024 {
        return Err("AppSpec exceeds 512 KiB.".into());
    }
    let issues = semantic(&spec);
    if !issues.is_empty() {
        return Err(json!({"status":"validation_error","issues":issues}).to_string());
    }
    for resource in spec["resources"].as_array_mut().unwrap() {
        for key in ["depends_on", "requirement_ids"] {
            resource[key] = json!(sorted_unique(&resource[key]));
        }
    }
    for scenario in spec["scenarios"].as_array_mut().unwrap() {
        scenario["requirement_ids"] = json!(sorted_unique(&scenario["requirement_ids"]));
    }
    for (key, id) in [
        ("requirements", "id"),
        ("resources", "key"),
        ("scenarios", "id"),
    ] {
        spec[key]
            .as_array_mut()
            .unwrap()
            .sort_by(|a, b| compare(text(&a[id]), text(&b[id])));
    }
    let resources = array(&spec["resources"])
        .iter()
        .map(|resource| {
            let mut compiled = resource.clone();
            let key = text(&resource["key"]);
            let kind = text(&resource["kind"]);
            let id = if kind == "table" {
                text(&resource["config"]["name"]).to_owned()
            } else {
                let digest = fingerprint(
                    "resource-id",
                    &json!({"app_id":app_id,"build_id":build_id,"kind":kind,"logical_key":key}),
                );
                format!("fp_{kind}_{}", &digest[4..28])
            };
            compiled["physical_id"] = json!(id);
            compiled["desired_fingerprint"] = json!(fingerprint("resource-desired", resource));
            compiled["dependents"] = json!(
                array(&spec["resources"])
                    .iter()
                    .filter(|r| strings(&r["depends_on"]).contains(&key))
                    .map(|r| text(&r["key"]))
                    .collect::<Vec<_>>()
            );
            compiled
        })
        .collect::<Vec<_>>();
    let scenarios = array(&spec["scenarios"])
        .iter()
        .map(|scenario| {
            let mut compiled = scenario.clone();
            compiled["desired_fingerprint"] = json!(fingerprint("scenario-desired", scenario));
            compiled
        })
        .collect::<Vec<_>>();
    let waves = waves(array(&spec["resources"]));
    let order = waves.iter().flatten().collect::<Vec<_>>();
    let coverage=array(&spec["requirements"]).iter().map(|requirement|{let id=text(&requirement["id"]);json!({"requirement_id":id,"resource_keys":resources.iter().filter(|r|strings(&r["requirement_ids"]).contains(&id)).map(|r|text(&r["key"])).collect::<Vec<_>>(),"scenario_ids":scenarios.iter().filter(|s|strings(&s["requirement_ids"]).contains(&id)).map(|s|text(&s["id"])).collect::<Vec<_>>()})}).collect::<Vec<_>>();
    Ok(
        json!({"schema_version":1,"app_id":app_id,"build_id":build_id,"spec":spec,"spec_fingerprint":fingerprint("app-spec",&spec),"resources":resources,"scenarios":scenarios,"waves":waves,"topological_order":order,"requirement_coverage":coverage}),
    )
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn compiler_matches_web_vectors() {
        for vector in array(&contract()["compiler_vectors"]) {
            assert_eq!(
                compile(
                    &vector["input"],
                    text(&vector["app_id"]),
                    text(&vector["build_id"])
                )
                .unwrap(),
                vector["output"]
            );
        }
    }
    #[test]
    fn malformed_input_returns_validation_errors() {
        for value in [
            json!(42),
            json!(null),
            json!([]),
            json!({"resources":[null,42,{"config":false}],"requirements":[true],"scenarios":null}),
        ] {
            assert!(compile(&value, "app", "build").is_err());
        }
    }
    #[test]
    fn rejects_cross_resource_and_scenario_violations() {
        for spec in array(&contract()["invalid_vectors"]) {
            assert!(compile(spec, "app", "build").is_err());
        }

        let mut spec = contract()["compiler_vectors"][2]["input"].clone();
        spec["resources"][0]["depends_on"] = json!(["missing"]);
        assert!(compile(&spec, "app", "build").is_err());
        let mut spec = contract()["compiler_vectors"][2]["input"].clone();
        spec["scenarios"][0]["assertions"][0]["status_path"] = json!("/status");
        assert!(compile(&spec, "app", "build").is_err());
    }
}
