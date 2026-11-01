use super::{resources, scenarios};
use crate::functions::ai::native_platform_tools::NativePlatformSession;
use serde_json::{Value, json};

pub(super) fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
pub(super) fn text<'a>(value: &'a Value, key: &str) -> &'a str {
    value.get(key).and_then(Value::as_str).unwrap_or("")
}
pub(super) fn array<'a>(value: &'a Value, key: &str) -> &'a [Value] {
    value
        .get(key)
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[])
}
pub(super) fn invalidate(build: &mut Value) {
    let build = build.as_object_mut().expect("validated build");
    build.remove("validation_revision");
    build.remove("structural_evidence");
    for scenario in build
        .get_mut("scenarios")
        .and_then(Value::as_object_mut)
        .into_iter()
        .flat_map(|v| v.values_mut())
    {
        scenario["status"] = json!("pending");
        scenario.as_object_mut().unwrap().remove("evidence");
    }
    build.insert(
        "promotion".into(),
        json!({"status":"staged","attempts":build["promotion"]["attempts"]}),
    );
}
fn set_validation_snapshot(build: &mut Value) {
    let complete = build["resources"]
        .as_object()
        .is_some_and(|resources| resources.values().all(|r| text(r, "status") == "applied"))
        && build["retired_resources"]
            .as_object()
            .is_none_or(|resources| {
                resources
                    .values()
                    .all(|r| text(r, "cleanup_status") != "required")
            });
    if complete {
        build["validation_revision"] = build["revision"].clone();
    } else {
        build.as_object_mut().unwrap().remove("validation_revision");
    }
}
pub(super) async fn commit(
    session: &NativePlatformSession,
    mut next: Value,
    complete_snapshot: bool,
) -> Result<Value, String> {
    session.assert_current_profile().await?;
    let previous = next["revision"].as_u64().ok_or("Invalid build revision")?;
    next["revision"] = json!(previous.checked_add(1).ok_or("Build revision overflow")?);
    next["updated_at_ms"] = json!(now());
    if complete_snapshot {
        set_validation_snapshot(&mut next);
    }
    super::write(session, next, Some(previous)).await
}
fn keys(build: &Value, statuses: &[&str]) -> Vec<String> {
    build["resources"]
        .as_object()
        .into_iter()
        .flat_map(|v| v.iter())
        .filter(|(_, r)| statuses.contains(&text(r, "status")))
        .map(|(key, _)| key.clone())
        .collect()
}
fn planned<'a>(plan: &'a Value, key: &str) -> Result<&'a Value, String> {
    array(plan, "resources")
        .iter()
        .find(|r| text(r, "key") == key)
        .ok_or_else(|| format!("Unknown compiled resource '{key}'"))
}
fn latest_receipt(resource: &Value) -> Option<&Value> {
    array(resource, "receipts").iter().rev().find(|r| {
        text(r, "status") == "applied"
            && r["physical_id"] == resource["physical_id"]
            && r["desired_fingerprint"] == resource["desired_fingerprint"]
    })
}
fn result(build: &Value, action: &str, resource_keys: Vec<String>) -> Value {
    let mut value = super::summarize(build);
    value["action"] = json!(action);
    value["resource_keys"] = json!(resource_keys);
    value
}

pub(super) async fn execute(
    session: &NativePlatformSession,
    args: &Value,
    mut build: Value,
    plan: Value,
    request_id: &str,
) -> Result<Value, String> {
    session.assert_current_profile().await?;
    let operation = text(args, "operation");
    if text(&build["promotion"], "status") == "promoted" {
        return Ok(result(&build, "idle", vec![]));
    }
    if operation == "advance" {
        let promotion = text(&build["promotion"], "status");
        if matches!(promotion, "partial" | "unknown") {
            return Ok(result(&build, &format!("promotion_{promotion}"), vec![]));
        }
        if promotion == "promoting" {
            return Ok(result(&build, "in_progress", vec![]));
        }
        let active = keys(&build, &["applying"]);
        let expired: Vec<String> = active
            .iter()
            .filter(|key| {
                build["resources"][*key]["active_operation"]["expires_at_ms"]
                    .as_u64()
                    .is_none_or(|expiry| expiry <= now())
            })
            .cloned()
            .collect();
        if !expired.is_empty() {
            invalidate(&mut build);
            for key in &expired {
                let resource = &mut build["resources"][key];
                resource["status"] = json!("unknown");
                resource.as_object_mut().unwrap().remove("active_operation");
                resource["last_error"] = json!(
                    "The host operation lease expired while applying. Inspect this resource before retrying."
                );
            }
            build = commit(session, build, false).await?;
        }
        if !keys(&build, &["applying"]).is_empty() {
            return Ok(result(&build, "in_progress", keys(&build, &["applying"])));
        }
        let uncertain = keys(&build, &["unknown", "partial"]);
        if !uncertain.is_empty() {
            let previous = build.clone();
            invalidate(&mut build);
            for key in &uncertain {
                session.assert_current_profile().await?;
                let compiled = planned(&plan, key)?;
                let observed = resources::read(session, &plan, compiled).await;
                let prior = latest_receipt(&previous["resources"][key]);
                let (inspection_status, status, fingerprint, message) = match observed {
                    Ok(read) if !read.exists => (
                        "missing",
                        "pending",
                        None,
                        "Reserved resource does not exist.".into(),
                    ),
                    Ok(read)
                        if read.issues.is_empty()
                            && read.fingerprint.is_some()
                            && prior.and_then(|r| r["observed_fingerprint"].as_str())
                                == read.fingerprint.as_deref() =>
                    {
                        ("matches", "applied", read.fingerprint, String::new())
                    }
                    Ok(read) => (
                        if prior.is_some() {
                            "drifted"
                        } else {
                            "unknown"
                        },
                        if prior.is_some() {
                            "partial"
                        } else {
                            "unknown"
                        },
                        read.fingerprint,
                        if read.issues.is_empty() {
                            "Resource exists without a matching completed host receipt. Inspect and request a focused repair.".into()
                        } else {
                            read.issues.join("\n")
                        },
                    ),
                    Err(error) => ("unknown", "unknown", None, error),
                };
                let mut inspection = json!({"inspection_id":flow_like_types::create_id(),"status":inspection_status,"physical_id":compiled["physical_id"],"desired_fingerprint":compiled["desired_fingerprint"],"inspected_at_ms":now()});
                if let Some(fingerprint) = fingerprint {
                    inspection["observed_fingerprint"] = json!(fingerprint);
                }
                if !message.is_empty() {
                    inspection["message"] = json!(bounded(&message, 4000));
                }
                let resource = &mut build["resources"][key];
                resource["status"] = json!(status);
                resource["last_inspection"] = inspection;
                resource.as_object_mut().unwrap().remove("last_error");
                if !matches!(status, "pending" | "applied") {
                    resource["last_error"] = json!(bounded(&message, 4000));
                }
            }
            build = commit(session, build, true).await?;
            return Ok(result(&build, "inspected", uncertain));
        }
        if text(&build["promotion"], "status") != "staged" {
            return Ok(result(&build, "idle", vec![]));
        }
        let ready: Vec<String> = array(&plan, "resources")
            .iter()
            .filter(|resource| {
                text(&build["resources"][text(resource, "key")], "status") == "pending"
                    && array(resource, "depends_on").iter().all(|key| {
                        text(&build["resources"][key.as_str().unwrap_or("")], "status") == "applied"
                    })
            })
            .map(|resource| text(resource, "key").to_owned())
            .take(3)
            .collect();
        if ready.is_empty() {
            return Ok(result(
                &build,
                if expired.is_empty() {
                    "idle"
                } else {
                    "recovered"
                },
                vec![],
            ));
        }
        resources::provision_foundation(session, &plan).await?;
        invalidate(&mut build);
        let started = now();
        let operation_id = flow_like_types::create_id();
        for key in &ready {
            let resource = &mut build["resources"][key];
            resource["status"] = json!("applying");
            resource["attempts"] = json!(resource["attempts"].as_u64().unwrap_or(0) + 1);
            resource["active_operation"] = json!({"operation_id":operation_id,"owner_id":request_id,"started_at_ms":started,"expires_at_ms":started+8*60*60*1000+15*60*1000});
            resource.as_object_mut().unwrap().remove("last_error");
        }
        build = commit(session, build, false).await?;
        let outcomes = {
            let plan = &plan;
            let build = &build;
            let operation_id = &operation_id;
            futures::future::join_all(ready.iter().map(|key| async move {
            let compiled = planned(&plan,key)?;
            let state=&build["resources"][key];
            let diagnostics = [state.get("last_error"),state.pointer("/last_inspection/message"),array(state,"receipts").last().and_then(|r|r.get("message"))].into_iter().flatten().filter_map(Value::as_str).map(str::to_owned).collect::<Vec<_>>();
            let resource_request_id = format!("{request_id}:resource:{key}");
            let applied = resources::apply(session,&plan,compiled,&diagnostics,&resource_request_id).await;
            let mut receipt=json!({"receipt_id":flow_like_types::create_id(),"operation_id":operation_id,"physical_id":compiled["physical_id"],"desired_fingerprint":compiled["desired_fingerprint"],"recorded_at_ms":now()});
            match applied {
                Ok(()) => match resources::read(session,&plan,compiled).await {
                    Ok(read) => {
                        receipt["status"]=json!(if read.exists && read.fingerprint.is_some() && read.issues.is_empty(){"applied"}else{"partial"});
                        if let Some(fp)=read.fingerprint{receipt["observed_fingerprint"]=json!(fp);}
                        if text(&receipt,"status")!="applied" {receipt["message"]=json!(bounded(&if read.issues.is_empty(){"Resource was not present on authoritative readback.".into()}else{read.issues.join("\n")},4000));}
                    },
                    Err(error)=>{receipt["status"]=json!("unknown");receipt["message"]=json!(bounded(&error,4000));}
                },
                Err(error)=>{receipt["status"]=json!("unknown");receipt["message"]=json!(bounded(&error,4000));}
            }
            Ok::<_,String>((key.clone(),receipt))
        })).await
        };
        for outcome in outcomes {
            let (key, mut receipt) = outcome?;
            let resource = &mut build["resources"][&key];
            if resource["active_operation"]["expires_at_ms"]
                .as_u64()
                .is_none_or(|expiry| expiry < now())
            {
                receipt["status"] = json!("unknown");
                receipt["message"] =
                    json!("Host result arrived outside its durable operation lease.");
            }
            resource["status"] = receipt["status"].clone();
            if let Some(message) = receipt.get("message") {
                resource["last_error"] = message.clone();
            }
            resource.as_object_mut().unwrap().remove("active_operation");
            let receipts = resource["receipts"]
                .as_array_mut()
                .ok_or("Invalid build receipts")?;
            receipts.push(receipt);
            if receipts.len() > 32 {
                receipts.drain(..receipts.len() - 32);
            }
        }
        build = commit(session, build, true).await?;
        return Ok(result(&build, "applied", ready));
    }
    if !keys(&build, &["applying"]).is_empty() || text(&build["promotion"], "status") == "promoting"
    {
        return Ok(result(&build, "in_progress", keys(&build, &["applying"])));
    }
    if operation == "test" {
        build = scenarios::execute(session, build, &plan, request_id).await?;
        return Ok(result(&build, "tested", vec![]));
    }
    if !build["validation_revision"].is_u64() {
        return Err("Structural validation requires a complete applied resource snapshot.".into());
    }
    let mut issues = Vec::new();
    let mut fingerprints = serde_json::Map::new();
    for resource in array(&plan, "resources") {
        session.assert_current_profile().await?;
        let key = text(resource, "key");
        match resources::read(session, &plan, resource).await {
            Ok(read) => {
                if !read.exists || read.fingerprint.is_none() {
                    issues.push(format!("{key}: missing resource."));
                }
                if let Some(fp) = read.fingerprint {
                    if latest_receipt(&build["resources"][key])
                        .and_then(|r| r["observed_fingerprint"].as_str())
                        != Some(fp.as_str())
                    {
                        issues.push(format!("{key}: changed since generation receipt."));
                    }
                    fingerprints.insert(key.into(), json!(fp));
                }
                issues.extend(
                    read.issues
                        .into_iter()
                        .map(|issue| format!("{key}: {issue}")),
                );
            }
            Err(error) => issues.push(format!("{key}: {error}")),
        }
    }
    match resources::structural_issues(session, &plan).await {
        Ok(extra) => issues.extend(extra),
        Err(error) => issues.push(error),
    }
    issues.truncate(128);
    build["structural_evidence"] = json!({"evidence_id":flow_like_types::create_id(),"status":if issues.is_empty(){"passed"}else{"failed"},"build_revision":build["validation_revision"],"spec_fingerprint":build["spec_fingerprint"],"resource_fingerprints":fingerprints,"recorded_at_ms":now(),"issues":issues.iter().map(|issue|bounded(issue,2000)).collect::<Vec<_>>()});
    build = commit(session, build, false).await?;
    if operation == "promote" {
        // Match the web host: native workflow nodes are not an attested isolated test runtime.
        // An argument or a previously stored receipt cannot supply that missing authority.
        return Err("This host has no attested isolated runtime adapter. Behavioral certification and automatic promotion remain blocked.".into());
    }
    Ok(result(&build, "validated", vec![]))
}

fn bounded(value: &str, max: usize) -> String {
    value.chars().take(max).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn any_resource_change_invalidates_all_prior_acceptance_evidence() {
        let mut build = json!({"validation_revision":7,"structural_evidence":{"status":"passed"},"promotion":{"status":"failed","attempts":2},"scenarios":{"s":{"id":"s","status":"passed","evidence":{"status":"passed"}}}});
        invalidate(&mut build);
        assert!(build.get("validation_revision").is_none());
        assert!(build.get("structural_evidence").is_none());
        assert!(build["scenarios"]["s"].get("evidence").is_none());
        assert_eq!(build["scenarios"]["s"]["status"], "pending");
        assert_eq!(build["promotion"]["attempts"], 2);
    }
    #[test]
    fn uncertain_resources_cannot_acquire_a_validation_snapshot() {
        let mut build = json!({"revision":4,"resources":{"a":{"status":"applied"},"b":{"status":"unknown"}},"retired_resources":{}});
        set_validation_snapshot(&mut build);
        assert!(build.get("validation_revision").is_none());
        build["resources"]["b"]["status"] = json!("applied");
        set_validation_snapshot(&mut build);
        assert_eq!(build["validation_revision"], 4);
    }
}
