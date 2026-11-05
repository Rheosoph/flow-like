//! Native durable app-build contracts and orchestration.
use super::{
    frontend_tool_bridge::FrontendToolContext,
    native_platform_tools::{NativePlatformSession, segment},
};
use reqwest::Method;
use serde_json::{Value, json};
use std::{future::Future, pin::Pin, sync::LazyLock};
use tauri::AppHandle;
mod capabilities;
mod compiler;
mod engine;
mod resources;
mod scenarios;
pub(super) use compiler::compile;
static CONTRACT: LazyLock<Value> = LazyLock::new(|| {
    serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../packages/ui/lib/flowpilot/generated/app-build-contracts.json"
    )))
    .expect("Bundled AppBuild contracts are valid")
});
pub(super) fn contract() -> &'static Value {
    &CONTRACT
}
pub(super) fn fingerprint(namespace: &str, value: &Value) -> String {
    super::native_specialist_tools::contract_fingerprint(namespace, value)
}
fn compare(left: &str, right: &str) -> std::cmp::Ordering {
    super::native_specialist_tools::contract_key_order(left, right)
}
fn text<'a>(value: &'a Value, key: &str) -> &'a str {
    value[key].as_str().unwrap_or("")
}
fn array<'a>(value: &'a Value, key: &str) -> &'a [Value] {
    value[key].as_array().map(Vec::as_slice).unwrap_or(&[])
}
fn values(value: &Value) -> impl Iterator<Item = &Value> {
    value.as_object().into_iter().flat_map(|o| o.values())
}
fn safe_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}
pub(super) fn supports(tool: &str) -> bool {
    tool == "app_build"
}
pub(super) fn execute<'a>(
    app: &'a AppHandle,
    tool: &'a str,
    args: &'a Value,
    context: Option<&'a FrontendToolContext>,
    request_id: &'a str,
) -> Pin<Box<dyn Future<Output = Option<Result<Value, String>>> + Send + 'a>> {
    Box::pin(async move {
        if !supports(tool) {
            return None;
        }
        Some(execute_inner(app, args, context, request_id).await)
    })
}
async fn execute_inner(
    app: &AppHandle,
    args: &Value,
    context: Option<&FrontendToolContext>,
    request_id: &str,
) -> Result<Value, String> {
    let session = NativePlatformSession::new(app, context).await?;
    let operation = text(args, "operation");
    if operation == "schema" {
        let mut guide = contract()["guide"].clone();
        guide["host_capabilities"] = contract()["host_capabilities"].clone();
        return Ok(guide);
    }
    if operation == "capabilities" {
        return Ok(json!({"status":"ok","capabilities":contract()["capabilities"]}));
    }
    if operation == "recipe" {
        return Ok(
            json!({"status":"ok","fragment":capabilities::instantiate(text(args,"capability_id"),text(args,"capability_version"),args.get("parameters").unwrap_or(&json!({})))?}),
        );
    }
    let app_id = session.app_id(args)?;
    let build_id = text(args, "build_id");
    if !safe_id(&app_id) || !safe_id(build_id) {
        return Err("app_build requires exact safe app_id and build_id.".into());
    }
    session.require_visible(&app_id)?;
    if operation == "begin" {
        let plan = compile(&args["spec"], &app_id, build_id)?;
        let original = context.and_then(|c| c.source_user_prompt.as_deref());
        let candidate = create(&plan, original);
        if let Some(existing) = read(&session, &app_id, build_id).await? {
            if existing["spec_fingerprint"] != candidate["spec_fingerprint"]
                || existing["original_request_fingerprint"]
                    != candidate["original_request_fingerprint"]
            {
                return Err("This build_id belongs to a different app contract. Resume it without replacing its requirements.".into());
            }
            if existing["revision"] == 0 {
                resources::begin_staging(&session, &app_id).await?;
            }
            let mut result = summarize(&existing);
            result["resumed"] = json!(true);
            return Ok(result);
        }
        let persisted = write(&session, candidate, None).await?;
        resources::begin_staging(&session, &app_id).await?;
        return Ok(summarize(&persisted));
    }
    let build = read(&session, &app_id, build_id)
        .await?
        .ok_or("The app build does not exist.")?;
    let plan = validate_state(&build)?;
    match operation {
        "status" => Ok(summarize(&build)),
        "repair" => {
            let keys = args["resource_keys"]
                .as_array()
                .filter(|keys| !keys.is_empty() && keys.iter().all(Value::is_string))
                .ok_or("repair requires nonempty exact resource_keys.")?;
            let next = repair(&build, &plan, keys)?;
            let previous = build["revision"].as_u64().unwrap();
            let saved = write(&session, next, Some(previous)).await?;
            Ok(summarize(&saved))
        }
        "advance" | "validate" | "test" | "promote" => {
            Box::pin(engine::execute(&session, args, build, plan, request_id)).await
        }
        _ => Err(format!("Unknown app_build operation '{operation}'.")),
    }
}
fn create(plan: &Value, original: Option<&str>) -> Value {
    let now = engine::now();
    let mut record = json!({"schema_version":1,"build_id":plan["build_id"],"app_id":plan["app_id"],"revision":0,"spec":plan["spec"],"spec_fingerprint":plan["spec_fingerprint"],"resources":Value::Object(array(plan,"resources").iter().map(|r|(text(r,"key").to_owned(),json!({"key":r["key"],"kind":r["kind"],"physical_id":r["physical_id"],"desired_fingerprint":r["desired_fingerprint"],"status":"pending","attempts":0,"receipts":[]}))).collect()),"retired_resources":{},"scenarios":Value::Object(array(plan,"scenarios").iter().map(|s|(text(s,"id").to_owned(),json!({"id":s["id"],"desired_fingerprint":s["desired_fingerprint"],"status":"pending"}))).collect()),"promotion":{"status":"staged","attempts":0},"created_at_ms":now,"updated_at_ms":now});
    if let Some(original) = original.filter(|s| !s.is_empty()) {
        record["original_request"] = json!(original);
        record["original_request_fingerprint"] =
            json!(fingerprint("original-request", &json!(original)));
    }
    record
}
pub(super) async fn read(
    session: &NativePlatformSession,
    app_id: &str,
    build_id: &str,
) -> Result<Option<Value>, String> {
    session.assert_current_profile().await?;
    session.require_visible(app_id)?;
    let value = if session.is_local(app_id).await? {
        crate::functions::app::flowpilot_builds::read_app_build(
            session.app_handle.clone(),
            app_id.into(),
            build_id.into(),
        )
        .await
        .map_err(|e| e.to_string())?
    } else {
        match session
            .request(
                Method::GET,
                &format!(
                    "apps/{}/flowpilot-builds/{}",
                    segment(app_id),
                    segment(build_id)
                ),
                None,
            )
            .await
        {
            Ok(value) => Some(value),
            Err(error) if error == "The hub returned HTTP 404 for this tool request." => None,
            Err(error) => return Err(error),
        }
    };
    if let Some(value) = &value {
        if value["app_id"] != app_id || value["build_id"] != build_id {
            return Err("The stored build belongs to another app or build.".into());
        }
        validate_state(value)?;
    }
    session.assert_current_profile().await?;
    Ok(value)
}
pub(super) async fn write(
    session: &NativePlatformSession,
    record: Value,
    expected_revision: Option<u64>,
) -> Result<Value, String> {
    session.assert_current_profile().await?;
    validate_state(&record)?;
    let app_id = text(&record, "app_id").to_owned();
    let build_id = text(&record, "build_id").to_owned();
    session.require_visible(&app_id)?;
    let expected_next = expected_revision
        .map(|n| n.checked_add(1).ok_or("Build revision overflow."))
        .transpose()?
        .unwrap_or(0);
    if record["revision"] != expected_next {
        return Err("The build revision must advance exactly once.".into());
    }
    if session.is_local(&app_id).await? {
        crate::functions::app::flowpilot_builds::write_app_build(
            session.app_handle.clone(),
            app_id,
            build_id,
            record,
            expected_revision,
        )
        .await
        .map_err(|e| e.to_string())
    } else {
        session
            .request(
                Method::PUT,
                &format!(
                    "apps/{}/flowpilot-builds/{}",
                    segment(&app_id),
                    segment(&build_id)
                ),
                Some(&json!({"record":record,"expected_revision":expected_revision})),
            )
            .await?;
        let persisted = read(session, &app_id, &build_id)
            .await?
            .ok_or("The persisted build is missing after write.")?;
        if persisted != record {
            return Err(
                "The build changed after its conditional write. Reload before continuing.".into(),
            );
        }
        Ok(persisted)
    }
}
fn latest_receipt(resource: &Value) -> Option<&Value> {
    array(resource, "receipts")
        .iter()
        .rev()
        .find(|r| r["status"] == "applied")
}
pub(super) fn validate_state(build: &Value) -> Result<Value, String> {
    if serde_json::to_vec(build).map_err(|e| e.to_string())?.len() > 1_000_000 {
        return Err("AppBuildState exceeds its durable checkpoint limit.".into());
    }
    let validator =
        jsonschema::validator_for(&contract()["state_schema"]).map_err(|e| e.to_string())?;
    if let Some(error) = validator.iter_errors(build).next() {
        return Err(format!("Invalid AppBuildState: {error}"));
    }
    let plan = compile(
        &build["spec"],
        text(build, "app_id"),
        text(build, "build_id"),
    )?;
    if build["spec_fingerprint"] != plan["spec_fingerprint"] {
        return Err("Stored spec fingerprint does not match its contract.".into());
    }
    if build.get("original_request").is_some()
        != build.get("original_request_fingerprint").is_some()
        || build.get("original_request").is_some_and(|request| {
            build["original_request_fingerprint"] != fingerprint("original-request", request)
        })
    {
        return Err("The immutable original request fingerprint is invalid.".into());
    }
    if build["resources"].as_object().unwrap().len() != array(&plan, "resources").len()
        || build["scenarios"].as_object().unwrap().len() != array(&plan, "scenarios").len()
    {
        return Err("The build has extra or missing resource/scenario bindings.".into());
    }
    let mut lease: Option<&Value> = None;
    for compiled in array(&plan, "resources") {
        let resource = &build["resources"][text(compiled, "key")];
        for key in ["key", "kind", "physical_id", "desired_fingerprint"] {
            if resource[key] != compiled[key] {
                return Err("A resource binding differs from its compiled reservation.".into());
            }
        }
        let applying = resource["status"] == "applying";
        if applying != resource.get("active_operation").is_some() {
            return Err("Only applying resources must hold an operation lease.".into());
        }
        if let Some(active) = resource.get("active_operation") {
            if active["expires_at_ms"].as_u64() <= active["started_at_ms"].as_u64()
                || lease.is_some_and(|held| held != active)
            {
                return Err("Applying resources must hold the same valid operation lease.".into());
            }
            lease = Some(active);
            if array(compiled, "depends_on").iter().any(|dependency| {
                build["resources"][dependency.as_str().unwrap()]["status"] != "applied"
            }) {
                return Err("An applying resource has an unapplied dependency.".into());
            }
        }
        if resource["status"] == "applied"
            && !latest_receipt(resource).is_some_and(|receipt| {
                receipt["physical_id"] == resource["physical_id"]
                    && receipt["desired_fingerprint"] == resource["desired_fingerprint"]
                    && receipt["observed_fingerprint"]
                        .as_str()
                        .is_some_and(|s| !s.is_empty())
            })
        {
            return Err("An applied resource has no exact authoritative receipt.".into());
        }
    }
    let promotion = &build["promotion"];
    if (promotion["status"] == "promoting") != promotion.get("active_operation").is_some()
        || lease.is_some() && promotion.get("active_operation").is_some()
    {
        return Err("The build has conflicting operation leases.".into());
    }
    if let Some(active) = promotion.get("active_operation") {
        if active["expires_at_ms"].as_u64() <= active["started_at_ms"].as_u64() {
            return Err("The promotion lease is invalid.".into());
        }
    }
    if build.get("validation_revision").is_some()
        && (values(&build["resources"]).any(|r| r["status"] != "applied")
            || values(&build["retired_resources"]).any(|r| r["cleanup_status"] == "required")
            || build["validation_revision"].as_u64() > build["revision"].as_u64())
    {
        return Err("The validation snapshot is incomplete or newer than the build.".into());
    }
    if let Some(evidence) = build.get("structural_evidence") {
        if evidence["spec_fingerprint"] != build["spec_fingerprint"]
            || evidence["build_revision"] != build["validation_revision"]
        {
            return Err("Structural evidence belongs to another validation snapshot.".into());
        }
        if evidence["status"] == "passed" {
            for resource in values(&build["resources"]) {
                if latest_receipt(resource).is_none_or(|receipt| {
                    receipt["observed_fingerprint"]
                        != evidence["resource_fingerprints"][text(resource, "key")]
                }) {
                    return Err(
                        "Structural evidence differs from the latest resource receipt.".into(),
                    );
                }
            }
        }
    }
    for compiled in array(&plan, "scenarios") {
        let scenario = &build["scenarios"][text(compiled, "id")];
        if scenario["id"] != compiled["id"]
            || scenario["desired_fingerprint"] != compiled["desired_fingerprint"]
        {
            return Err("A scenario binding differs from its compiled contract.".into());
        }
        if let Some(evidence) = scenario.get("evidence") {
            if evidence["status"] != scenario["status"]
                || evidence["scenario_id"] != compiled["id"]
                || evidence["scenario_fingerprint"] != compiled["desired_fingerprint"]
                || evidence["spec_fingerprint"] != build["spec_fingerprint"]
                || evidence["build_revision"] != build["validation_revision"]
            {
                return Err("Scenario evidence belongs to another validation snapshot.".into());
            }
        } else if scenario["status"] != "pending" {
            return Err("A completed scenario has no host evidence.".into());
        }
    }
    let promotion_status = text(promotion, "status");
    if matches!(promotion_status, "promoted" | "partial" | "failed")
        || promotion_status == "unknown" && promotion.get("receipt").is_some()
    {
        let receipt = &promotion["receipt"];
        if receipt["status"]
            != if promotion_status == "promoted" {
                "applied"
            } else {
                promotion_status
            }
            || receipt["build_revision"] != promotion["requested_build_revision"]
            || receipt["spec_fingerprint"] != build["spec_fingerprint"]
        {
            return Err("Promotion state lacks an exact matching host receipt.".into());
        }
    }
    if build["updated_at_ms"].as_u64() < build["created_at_ms"].as_u64() {
        return Err("Build timestamps are out of order.".into());
    }
    Ok(plan)
}
pub(super) fn readiness(build: &Value) -> Value {
    let plan = match validate_state(build) {
        Ok(plan) => plan,
        Err(error) => {
            return json!({"level":"invalid","structural_preview":false,"behaviorally_ready":false,"can_promote":false,"blockers":[{"code":"invalid_build","message":error}]});
        }
    };
    let mut blockers = Vec::new();
    for resource in values(&build["resources"]) {
        if resource["status"] != "applied" {
            blockers.push(json!({"code":"resource_not_applied","resource_key":resource["key"],"message":format!("Resource '{}' is {}.",text(resource,"key"),text(resource,"status"))}));
        }
    }
    for resource in values(&build["retired_resources"]) {
        if resource["cleanup_status"] == "required" {
            blockers.push(json!({"code":"retired_resource_cleanup_required","resource_key":resource["key"],"message":"Retired resource requires cleanup or an explicit retain decision."}));
        }
    }
    if build.get("validation_revision").is_none() {
        blockers.push(json!({"code":"validation_snapshot_missing","message":"No complete resource revision is available for validation."}));
    }
    let structural = build["structural_evidence"]["status"] == "passed";
    if !structural {
        blockers.push(json!({"code":if build.get("structural_evidence").is_none(){"structural_evidence_missing"}else{"structural_evidence_failed"},"message":"Host structural validation has not passed for this build revision."}));
    }
    let mut behavioral = !array(&plan, "scenarios").is_empty();
    if !behavioral {
        blockers.push(json!({"code":"scenario_contract_missing","message":"The contract has no behavioral scenario. Structural evidence supports preview only."}));
    }
    for scenario in values(&build["scenarios"]) {
        let evidence = &scenario["evidence"];
        let code = if evidence.is_null() {
            Some("scenario_evidence_missing")
        } else if evidence["status"] == "passed" && evidence["certification"] != "behavioral" {
            Some("scenario_contract_only")
        } else if evidence["status"] == "blocked" {
            Some("scenario_blocked")
        } else if evidence["status"] != "passed" {
            Some("scenario_failed")
        } else {
            None
        };
        if let Some(code) = code {
            behavioral = false;
            blockers.push(json!({"code":code,"scenario_id":scenario["id"],"message":evidence["message"].as_str().unwrap_or("The scenario has no passed isolated behavioral evidence.")}));
        }
    }
    for coverage in array(&plan, "requirement_coverage") {
        if !array(coverage, "scenario_ids").iter().any(|id| {
            let evidence = &build["scenarios"][id.as_str().unwrap()]["evidence"];
            evidence["status"] == "passed" && evidence["certification"] == "behavioral"
        }) {
            behavioral = false;
            blockers.push(json!({"code":"requirement_behavioral_evidence_missing","requirement_id":coverage["requirement_id"],"message":"This requirement has no passed isolated behavioral scenario."}));
        }
    }
    let promotion = text(&build["promotion"], "status");
    if matches!(promotion, "partial" | "unknown" | "promoting") {
        blockers.push(json!({"code":"promotion_in_doubt","message":"Reconcile the existing promotion before another attempt."}));
    }
    let ready = structural && behavioral;
    let promoted = promotion == "promoted";
    let level = if promoted {
        "promoted"
    } else if ready {
        "ready"
    } else if structural {
        "structural_preview"
    } else if values(&build["resources"]).all(|r| r["status"] == "applied") {
        "awaiting_validation"
    } else {
        "building"
    };
    let mut result = json!({"level":level,"structural_preview":structural,"behaviorally_ready":ready,"can_promote":ready&&!promoted&&matches!(promotion,"staged"|"failed")&&!values(&build["retired_resources"]).any(|r|r["cleanup_status"]=="required"),"blockers":blockers});
    if let Some(revision) = build.get("validation_revision") {
        result["validation_revision"] = revision.clone();
    }
    result
}
pub(super) fn ready_waves(build: &Value) -> Value {
    if build["promotion"]["status"] != "staged"
        || values(&build["resources"]).any(|r| r["status"] == "applying")
    {
        return json!([]);
    }
    let Ok(plan) = validate_state(build) else {
        return json!([]);
    };
    let ready = array(&plan, "resources")
        .iter()
        .filter(|r| {
            build["resources"][text(r, "key")]["status"] == "pending"
                && array(r, "depends_on").iter().all(|dependency| {
                    build["resources"][dependency.as_str().unwrap()]["status"] == "applied"
                })
        })
        .map(|r| text(r, "key"))
        .collect::<Vec<_>>();
    if ready.is_empty() {
        json!([])
    } else {
        json!([ready])
    }
}
pub(super) fn summarize(build: &Value) -> Value {
    json!({"status":"ok","app_id":build["app_id"],"build_id":build["build_id"],"revision":build["revision"],"readiness":readiness(build),"ready_waves":ready_waves(build),"resources":values(&build["resources"]).map(|r|{let mut row=json!({"key":r["key"],"kind":r["kind"],"id":r["physical_id"],"status":r["status"],"attempts":r["attempts"]});if let Some(message)=r.get("last_error"){row["message"]=message.clone();}row}).collect::<Vec<_>>(),"scenarios":values(&build["scenarios"]).map(|s|{let mut row=json!({"id":s["id"],"status":s["status"]});if let Some(message)=s["evidence"].get("message"){row["message"]=message.clone();}row}).collect::<Vec<_>>(),"promotion":build["promotion"]})
}
fn repair(build: &Value, plan: &Value, keys: &[Value]) -> Result<Value, String> {
    if build["promotion"]["status"] == "promoted"
        || build["promotion"]["status"] == "promoting"
        || values(&build["resources"]).any(|r| r["status"] == "applying")
    {
        return Err(
            "Cannot repair an immutable build or a build with an active operation lease.".into(),
        );
    }
    let mut affected = keys
        .iter()
        .map(|key| key.as_str().unwrap().to_owned())
        .collect::<std::collections::HashSet<_>>();
    if affected
        .iter()
        .any(|key| build["resources"].get(key).is_none())
    {
        return Err("Unknown repair resource key.".into());
    }
    loop {
        let before = affected.len();
        for resource in array(plan, "resources") {
            if array(resource, "depends_on")
                .iter()
                .any(|dependency| affected.contains(dependency.as_str().unwrap()))
            {
                affected.insert(text(resource, "key").into());
            }
        }
        if before == affected.len() {
            break;
        }
    }
    let mut next = build.clone();
    engine::invalidate(&mut next);
    for key in affected {
        next["resources"][&key]["status"] = json!("pending");
        next["resources"][&key]["last_error"] = json!(
            "Focused repair requested through the app build host; original requirements retained."
        );
    }
    next["revision"] = json!(
        build["revision"]
            .as_u64()
            .unwrap()
            .checked_add(1)
            .ok_or("Build revision overflow.")?
    );
    next["updated_at_ms"] = json!(engine::now());
    Ok(next)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn checkpoint() -> (Value, Value) {
        let plan = compile(&contract()["compiler_vectors"][2]["input"], "app", "build").unwrap();
        let build = create(&plan, Some("Keep every requested requirement."));
        (plan, build)
    }
    #[test]
    fn persisted_build_rejects_foreign_ids_or_missing_applied_evidence() {
        let (plan, build) = checkpoint();
        assert!(validate_state(&build).is_ok());
        let key = text(&plan["resources"][0], "key");
        let mut foreign = build.clone();
        foreign["resources"][key]["physical_id"] = json!("another-app-artifact");
        assert!(validate_state(&foreign).is_err());
        let mut missing = build;
        missing["resources"][key]["status"] = json!("applied");
        assert!(validate_state(&missing).is_err());
    }
    #[test]
    fn persisted_build_rejects_mutated_original_request() {
        let (_, mut build) = checkpoint();
        build["original_request"] = json!("Drop the original requirements.");
        assert!(validate_state(&build).is_err());
    }
    #[test]
    fn repair_preserves_contract_and_resets_dependents() {
        let (plan, build) = checkpoint();
        let key = text(&plan["resources"][0], "key");
        let repaired = repair(&build, &plan, &[json!(key)]).unwrap();
        assert_eq!(repaired["spec"], build["spec"]);
        assert_eq!(repaired["original_request"], build["original_request"]);
        assert_eq!(repaired["revision"], 1);
        assert!(validate_state(&repaired).is_ok());
        assert_eq!(readiness(&repaired)["can_promote"], false);
    }
}
