use super::engine::{array, commit, now, text};
use crate::functions::ai::{native_platform_tools::NativePlatformSession, native_tool_bridge};
use serde_json::{Value, json};
use std::{collections::HashMap, time::Duration};

fn resolve(
    value: &Value,
    app: &Value,
    target: &Value,
    observations: &HashMap<String, Value>,
    depth: usize,
) -> Result<Value, String> {
    if depth > 32 {
        return Err("Scenario references exceed the nesting limit.".into());
    }
    if let Some(reference) = value.get("$scenario_ref") {
        let source = match text(reference, "source") {
            "app" => app,
            "target" => target,
            "step" => observations
                .get(text(reference, "step_id"))
                .ok_or("Scenario step has no fresh observation")?,
            _ => return Err("Invalid scenario reference source".into()),
        };
        return source
            .pointer(text(reference, "path"))
            .cloned()
            .ok_or_else(|| "Scenario reference did not resolve to JSON evidence.".into());
    }
    match value {
        Value::Array(values) => values
            .iter()
            .map(|v| resolve(v, app, target, observations, depth + 1))
            .collect::<Result<Vec<_>, _>>()
            .map(Value::Array),
        Value::Object(values) => values
            .iter()
            .map(|(k, v)| Ok((k.clone(), resolve(v, app, target, observations, depth + 1)?)))
            .collect::<Result<serde_json::Map<_, _>, String>>()
            .map(Value::Object),
        _ => Ok(value.clone()),
    }
}
fn equal(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Number(a), Value::Number(b)) => a.as_f64() == b.as_f64(),
        (Value::Array(a), Value::Array(b)) => {
            a.len() == b.len() && a.iter().zip(b).all(|(a, b)| equal(a, b))
        }
        (Value::Object(a), Value::Object(b)) => {
            a.len() == b.len() && a.iter().all(|(k, a)| b.get(k).is_some_and(|b| equal(a, b)))
        }
        _ => a == b,
    }
}
fn assertion(actual: Option<&Value>, operator: &str, expected: &Value) -> bool {
    match operator {
        "exists" => actual.is_some(),
        "equals" => actual.is_some_and(|v| equal(v, expected)),
        "not_equals" => actual.is_some_and(|v| !equal(v, expected)),
        "contains" => actual.is_some_and(|v| match v {
            Value::String(v) => expected.as_str().is_some_and(|s| v.contains(s)),
            Value::Array(v) => v.iter().any(|v| equal(v, expected)),
            _ => false,
        }),
        "length_at_least" => {
            let len = actual.and_then(|v| match v {
                Value::String(v) => Some(v.encode_utf16().count()),
                Value::Array(v) => Some(v.len()),
                Value::Object(v) => Some(v.len()),
                _ => None,
            });
            len.zip(expected.as_u64())
                .is_some_and(|(len, min)| len as u64 >= min)
        }
        _ => false,
    }
}
fn target(plan: &Value, scenario: &Value) -> Result<Value, String> {
    let app = text(plan, "app_id");
    let key = text(&scenario["target"], "resource_key");
    if key == "app" {
        return Ok(json!({"id":app,"appId":app,"kind":"app"}));
    }
    let resource = array(plan, "resources")
        .iter()
        .find(|r| text(r, "key") == key)
        .ok_or("Scenario target is not a reserved resource")?;
    let kind = text(resource, "kind");
    if !matches!(kind, "event" | "page") {
        return Err("Scenario target must be an app, page, or Event".into());
    }
    let mut value = json!({"id":resource["physical_id"],"appId":app,"kind":if kind=="event"&&matches!(text(&resource["config"],"event_type"),"chat"|"simple_chat"){"chat"}else{kind}});
    if kind == "page" {
        value["pageId"] = resource["physical_id"].clone();
        if let Some(event) = array(plan, "resources")
            .iter()
            .find(|r| text(r, "kind") == "event" && text(&r["config"], "page") == key)
        {
            value["eventId"] = event["physical_id"].clone();
        }
    } else {
        value["eventId"] = resource["physical_id"].clone();
    }
    if value["kind"] != scenario["target"]["kind"] {
        return Err("Scenario target kind does not match the reserved resource".into());
    }
    Ok(value)
}
fn has_errors(value: &Value) -> bool {
    matches!(
        text(value, "status").to_ascii_lowercase().as_str(),
        "error" | "failed" | "fatal"
    ) || array(value, "logs").iter().any(|log| {
        let level = log
            .get("level")
            .or_else(|| log.get("log_level"))
            .or_else(|| log.get("severity"));
        level.is_some_and(|v| {
            v.as_u64().is_some_and(|v| v >= 4)
                || v.as_str()
                    .is_some_and(|v| matches!(v.to_ascii_lowercase().as_str(), "error" | "fatal"))
        })
    })
}

async fn run(
    session: &NativePlatformSession,
    plan: &Value,
    scenario: &Value,
    request_id: &str,
) -> Result<Value, String> {
    let started = now();
    let timeout = scenario["timeout_ms"]
        .as_u64()
        .unwrap_or(60_000)
        .min(600_000);
    let deadline = tokio::time::Instant::now() + Duration::from_millis(timeout);
    let steps = array(scenario, "steps");
    let assertions = array(scenario, "assertions");
    let runtime = steps.iter().any(|step| {
        !matches!(
            text(step, "tool"),
            "describe_app_interface" | "query_execution_logs"
        )
    }) || assertions.iter().any(|a| text(a, "kind") != "json");
    let app = json!({"id":plan["app_id"],"appId":plan["app_id"]});
    let target = target(plan, scenario)?;
    let mut issues = vec![];
    let mut observations = HashMap::new();
    let mut results = vec![];
    let mut checks = vec![];
    let mut status = "pass";
    if runtime {
        status = "blocked";
        issues.push(json!({"code":"runtime_isolation_unavailable","message":"This host has no attested isolated runtime adapter."}));
    } else {
        for step in steps {
            session.assert_current_profile().await?;
            let mut arguments = resolve(&step["arguments"], &app, &target, &observations, 0)?;
            if arguments.to_string().len() > 64 * 1024 {
                return Err("Scenario arguments exceed the byte limit".into());
            }
            if arguments
                .get("app_id")
                .or_else(|| arguments.get("appId"))
                .and_then(Value::as_str)
                .is_some_and(|id| id != text(plan, "app_id"))
            {
                return Err("Scenario targets another app".into());
            }
            arguments["app_id"] = plan["app_id"].clone();
            let invocation = flow_like_types::create_id();
            let step_started = now();
            let observed = tokio::time::timeout_at(
                deadline,
                Box::pin(native_tool_bridge::execute(
                    &session.app_handle,
                    text(step, "tool"),
                    &arguments,
                    session.context.as_ref(),
                    &format!("{request_id}:{invocation}"),
                )),
            )
            .await;
            match observed {
                Ok(Ok(value)) => {
                    if value.to_string().len() > 512 * 1024 {
                        return Err("Scenario observation exceeds the byte limit".into());
                    }
                    if has_errors(&value) {
                        status = "fail";
                        issues.push(json!({"code":"tool_error_evidence","message":format!("Step '{}' returned error evidence.",text(step,"id"))}));
                    }
                    results.push(json!({"step_id":step["id"],"tool":step["tool"],"invocation_id":invocation,"status":"observed","started_at_ms":step_started,"completed_at_ms":now(),"observed_at_ms":now(),"arguments":arguments,"value":value}));
                    observations.insert(text(step, "id").into(), value);
                }
                error => {
                    status = if error.is_err() {
                        "timed_out"
                    } else {
                        "unknown"
                    };
                    let message = match error {
                        Ok(Err(message)) => message,
                        _ => "Scenario exceeded its execution deadline".into(),
                    };
                    issues.push(json!({"code":"observation_unavailable","message":message.chars().take(4000).collect::<String>()}));
                    results.push(json!({"step_id":step["id"],"tool":step["tool"],"invocation_id":invocation,"status":status,"started_at_ms":step_started,"completed_at_ms":now()}));
                    break;
                }
            }
        }
        for check in assertions {
            let actual = observations
                .get(text(check, "step_id"))
                .and_then(|v| v.pointer(text(check, "path")));
            let expected = resolve(
                check.get("expected").unwrap_or(&Value::Null),
                &app,
                &target,
                &observations,
                0,
            );
            let passed = expected
                .as_ref()
                .is_ok_and(|expected| assertion(actual, text(check, "operator"), expected));
            if !passed && status == "pass" {
                status = "fail";
            }
            checks.push(json!({"assertion_id":check["id"],"step_id":check["step_id"],"status":if passed{"pass"}else{"fail"},"message":format!("{} assertion {} at {}.",text(check,"operator"),if passed{"passed"}else{"failed"},text(check,"path")),"path":check["path"]}));
        }
    }
    let certification = if status == "pass" {
        "contract_only"
    } else {
        "none"
    };
    let mut value = json!({"schema":"flowpilot.app-behavior-scenario-result/v1","scenario_id":scenario["id"],"app_id":plan["app_id"],"status":status,"outcome_known":matches!(status,"pass"|"fail"|"blocked"),"outstanding":false,"certification":certification,"required_capability":if runtime{"isolated_runtime"}else{"read_only"},"started_at_ms":started,"completed_at_ms":now(),"deadline_at_ms":started+timeout,"steps":results,"assertions":checks,"started_runs":[],"issues":issues,"metrics":{"steps_total":steps.len(),"steps_invoked":results.len(),"assertions_total":assertions.len(),"assertions_passed":checks.iter().filter(|check|text(check,"status")=="pass").count(),"started_runs":0,"successful_runs":0,"failed_runs":0,"unknown_runs":0}});
    if !runtime {
        value["attestation"] = json!({"id":flow_like_types::create_id(),"mode":"read_only"});
    }
    Ok(value)
}

pub(super) async fn execute(
    session: &NativePlatformSession,
    mut build: Value,
    plan: &Value,
    request_id: &str,
) -> Result<Value, String> {
    if !build["validation_revision"].is_u64()
        || text(&build["structural_evidence"], "status") != "passed"
    {
        return Err("Validate all persisted resources before running acceptance scenarios.".into());
    }
    for scenario in array(plan, "scenarios") {
        let result = run(session, plan, scenario, request_id).await?;
        session.assert_current_profile().await?;
        let latest = super::read(session, text(plan, "app_id"), text(plan, "build_id"))
            .await?
            .ok_or("The app build no longer exists")?;
        if latest["validation_revision"] != build["validation_revision"]
            || latest["spec_fingerprint"] != plan["spec_fingerprint"]
        {
            return Err("Build changed while its acceptance scenario was running. Results were not accepted.".into());
        }
        build = latest;
        let status = match text(&result, "status") {
            "pass" => "passed",
            "fail" => "failed",
            _ => "blocked",
        };
        let issues = array(&result, "issues");
        let evidence = json!({"evidence_id":flow_like_types::create_id(),"scenario_id":scenario["id"],"certification":result["certification"],"status":status,"build_revision":build["validation_revision"],"spec_fingerprint":plan["spec_fingerprint"],"scenario_fingerprint":scenario["desired_fingerprint"],"recorded_at_ms":now(),"message":format!("Host scenario result: {}.",text(&result,"status")),"details":{"schema":"flowpilot.app-scenario-receipt/v1","scenario_id":scenario["id"],"status":result["status"],"certification":result["certification"],"started_at_ms":result["started_at_ms"],"completed_at_ms":result["completed_at_ms"],"metrics":result["metrics"],"result_fingerprint":super::fingerprint("scenario-result",&result),"attestation_id":result.pointer("/attestation/id"),"raw_observations_retained":false,"issue_count":issues.len(),"issues":issues.iter().take(3).map(|issue|json!({"code":text(issue,"code").chars().take(100).collect::<String>(),"message":text(issue,"message").chars().take(240).collect::<String>()})).collect::<Vec<_>>()}});
        let state = &mut build["scenarios"][text(scenario, "id")];
        state["status"] = json!(status);
        state["evidence"] = evidence;
        build = commit(session, build, false).await?;
    }
    Ok(build)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn missing_paths_are_distinct_from_present_null() {
        assert!(assertion(Some(&Value::Null), "exists", &Value::Null));
        assert!(!assertion(None, "not_equals", &json!(4)));
    }
    #[test]
    fn fresh_reference_requires_an_observed_step() {
        assert!(
            resolve(
                &json!({"$scenario_ref":{"source":"step","step_id":"old","path":"/result"}}),
                &json!({}),
                &json!({}),
                &HashMap::new(),
                0
            )
            .is_err()
        );
    }
    #[test]
    fn web_json_comparisons_preserve_numbers_and_utf16_lengths() {
        assert!(assertion(Some(&json!([1.0])), "contains", &json!(1)));
        assert!(assertion(Some(&json!("🌍")), "length_at_least", &json!(2)));
    }
}
