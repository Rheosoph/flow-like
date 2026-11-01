use super::*;
use crate::state::TauriSettingsState;
use icu_collator::{Collator, CollatorBorrowed};
use reqwest::Method;
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};
use tauri::Emitter;
use tokio::io::AsyncWriteExt;

static CONTRACT: LazyLock<Value> = LazyLock::new(|| {
    serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../packages/ui/components/global-chat/tools/home/generated/native-contracts.json"
    )))
    .expect("Bundled Home contracts are valid JSON")
});
static COLLATOR: LazyLock<CollatorBorrowed<'static>> = LazyLock::new(|| {
    Collator::try_new(Default::default(), Default::default())
        .expect("Bundled collation data is valid")
});
#[derive(Clone)]
struct CapturedHome {
    owner: String,
    observed: Instant,
    value: Value,
}
static CAPTURED: LazyLock<DashMap<String, CapturedHome>> = LazyLock::new(DashMap::new);
static STAGED: LazyLock<DashMap<String, Value>> = LazyLock::new(DashMap::new);
fn identity(session: &NativePlatformSession) -> String {
    json!([session.profile.id, session.hub, session.subject]).to_string()
}
pub(super) fn staged_receipt(request_id: &str) -> Option<Value> {
    STAGED.remove(request_id).map(|(_, value)| value)
}
#[tauri::command]
pub async fn flowpilot_capture_home_snapshot(
    app_handle: AppHandle,
    webview_window: tauri::WebviewWindow,
    snapshot: Option<Value>,
    expected_owner: Option<Value>,
) -> Result<(), String> {
    let session = NativePlatformSession::new(&app_handle, None).await?;
    let key = identity(&session);
    let owner = webview_window.label().to_owned();
    CAPTURED.retain(|_, capture| capture.observed.elapsed() < Duration::from_secs(6));
    let Some(snapshot) = snapshot else {
        CAPTURED.remove_if(&key, |_, capture| capture.owner == owner);
        return Ok(());
    };
    validate_snapshot_owner(&session, expected_owner.as_ref())?;
    if snapshot["profileId"] != session.profile.id {
        return Err("The Home snapshot belongs to another profile.".into());
    }
    for field in ["layout", "baseLayout", "defaultLayout"] {
        flow_like::profile::validate_home_layout(&snapshot[field])?;
    }
    if snapshot["candidateFingerprint"] != fingerprint(&snapshot["layout"])
        || snapshot["baseFingerprint"] != fingerprint(&snapshot["baseLayout"])
    {
        return Err("The Home snapshot fingerprint is invalid.".into());
    }
    session.assert_current_profile().await?;
    if CAPTURED.get(&key).is_some_and(|capture| {
        capture.owner != owner
            && capture.value["candidateFingerprint"] != snapshot["candidateFingerprint"]
    }) {
        return Err(
            "Another Home editor has a different draft. Close one editor before running FlowPilot."
                .into(),
        );
    }
    CAPTURED.insert(
        key,
        CapturedHome {
            owner,
            observed: Instant::now(),
            value: snapshot,
        },
    );
    Ok(())
}
#[tauri::command]
pub async fn flowpilot_get_home_proposal(app_handle: AppHandle) -> Result<Option<Value>, String> {
    let session = NativePlatformSession::new(&app_handle, None).await?;
    let current = snapshot(&session).await?;
    let bytes = match tokio::fs::read(proposal_path(&session)?).await {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.to_string()),
    };
    if bytes.len() > 160 * 1024 {
        return Err("The Home proposal exceeds its size limit.".into());
    }
    let proposal: Value = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    if proposal["profile_id"] != session.profile.id
        || proposal["base_fingerprint"] != fingerprint(&current.base)
    {
        return Ok(None);
    }
    session.assert_current_profile().await?;
    Ok(Some(proposal))
}
#[tauri::command]
pub async fn flowpilot_resolve_home_proposal(
    app_handle: AppHandle,
    profile_id: String,
    candidate_fingerprint: String,
) -> Result<(), String> {
    let session = NativePlatformSession::new(&app_handle, None).await?;
    if session.profile.id != profile_id {
        return Err("The Home proposal belongs to another profile.".into());
    }
    let _guard = target_lock(format!("{}:{}:home", session.profile.id, session.hub)).await;
    let path = proposal_path(&session)?;
    let bytes = match tokio::fs::read(&path).await {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.to_string()),
    };
    let proposal: Value = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    if fingerprint(&proposal["layout"]) != candidate_fingerprint {
        return Err("The retained Home proposal changed.".into());
    }
    session.assert_current_profile().await?;
    tokio::fs::remove_file(path)
        .await
        .map_err(|e| e.to_string())
}
pub(super) fn catalog() -> &'static Value {
    &CONTRACT["catalog"]
}
pub(super) fn contracts() -> &'static Value {
    &catalog()["widget_types"]
}
pub(super) fn condition_matches(condition: &Value, config: &Value, fields: &Value) -> bool {
    if condition.is_null() {
        return true;
    }
    if condition["all"]
        .as_array()
        .is_some_and(|all| all.iter().any(|c| !condition_matches(c, config, fields)))
    {
        return false;
    }
    let Some(field) = condition["field"].as_str() else {
        return true;
    };
    let actual = config
        .get(field)
        .filter(|v| !v.is_null())
        .unwrap_or(&fields[field]["default"]);
    if let Some(expected) = condition.get("equals") {
        return actual == expected;
    }
    if let Some(expected) = condition["in"].as_array() {
        return expected.contains(actual);
    }
    if let Some(expected) = condition.get("not_equals") {
        return actual != expected;
    }
    if let Some(expected) = condition["not_in"].as_array() {
        return !expected.contains(actual);
    }
    true
}
pub(super) fn supports(tool: &str) -> bool {
    matches!(
        tool,
        "get_home_context"
            | "get_home_widget_catalog"
            | "list_home_data_sources"
            | "validate_home_layout"
            | "apply_home_layout"
    )
}
fn fail(code: &str, message: &str) -> Value {
    json!({"status":"error","code":code,"message":message})
}

// Match the browser's stable JSON key collation and versioned Home change detector.
pub(super) fn compare_keys(left: &str, right: &str) -> std::cmp::Ordering {
    COLLATOR.compare(left, right)
}
pub(super) fn fingerprint(value: &Value) -> String {
    fingerprint_for("home-layout-v1", value)
}
pub(super) fn fingerprint_for(namespace: &str, value: &Value) -> String {
    fn canonical(value: &Value, out: &mut String) {
        match value {
            Value::Object(map) => {
                out.push('{');
                let mut entries = map.iter().collect::<Vec<_>>();
                entries.sort_by(|(a, _), (b, _)| COLLATOR.compare(a, b));
                // JavaScript enumerates array-index property names before other object keys.
                entries.sort_by(|(a, _), (b, _)| {
                    let index = |key: &str| {
                        key.parse::<u32>()
                            .ok()
                            .filter(|&n| n < u32::MAX && n.to_string() == key)
                    };
                    match (index(a), index(b)) {
                        (Some(a), Some(b)) => a.cmp(&b),
                        (Some(_), None) => std::cmp::Ordering::Less,
                        (None, Some(_)) => std::cmp::Ordering::Greater,
                        _ => std::cmp::Ordering::Equal,
                    }
                });
                for (i, (key, value)) in entries.into_iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    out.push_str(&serde_json::to_string(key).unwrap());
                    out.push(':');
                    canonical(value, out);
                }
                out.push('}');
            }
            Value::Array(items) => {
                out.push('[');
                for (i, item) in items.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    canonical(item, out);
                }
                out.push(']');
            }
            Value::Number(number) => {
                let n = number.as_f64().unwrap_or(0.0);
                if n == 0.0 {
                    out.push('0')
                } else if n.fract() == 0.0 && n.abs() < 1e21 {
                    out.push_str(&format!("{n:.0}"));
                } else {
                    let text = number.to_string();
                    if let Some((base, exponent)) = text.split_once('e') {
                        let exp = exponent.parse::<i32>().unwrap_or(0);
                        if (-6..0).contains(&exp) {
                            out.push_str(&format!(
                                "{n:.precision$}",
                                precision =
                                    (-exp + base.replace('.', "").len() as i32 - 1).max(0) as usize
                            ));
                        } else {
                            out.push_str(base);
                            out.push('e');
                            if exp >= 0 {
                                out.push('+')
                            }
                            out.push_str(&exp.to_string());
                        }
                    } else {
                        out.push_str(&text)
                    }
                }
            }
            _ => out.push_str(&value.to_string()),
        }
    }
    let mut text = String::new();
    canonical(value, &mut text);
    let hash = |prefix: &str, offset: u64| {
        format!("{prefix}\0{namespace}\0{text}")
            .bytes()
            .fold(offset, |hash, byte| {
                (hash ^ u64::from(byte)).wrapping_mul(0x100000001b3)
            })
    };
    format!(
        "fp1:{:016x}{:016x}",
        hash("a", 0xcbf29ce484222325),
        hash("b", 0x84222325cbf29ce4)
    )
}
fn proposal_path(session: &NativePlatformSession) -> Result<PathBuf, String> {
    let scope = json!([session.profile.id, session.hub, session.subject]).to_string();
    Ok(session
        .app_handle
        .path()
        .app_data_dir()
        .map_err(|e| e.to_string())?
        .join("flowpilot-home-proposals")
        .join(format!("{}.json", blake3::hash(scope.as_bytes()).to_hex())))
}
struct Snapshot {
    layout: Value,
    base: Value,
    default: Option<Value>,
    source: String,
    profile: flow_like::profile::Profile,
    editing: bool,
}
async fn snapshot(session: &NativePlatformSession) -> Result<Snapshot, String> {
    session.assert_current_profile().await?;
    let profile = TauriSettingsState::current_profile(&session.app_handle)
        .await
        .map_err(|e| e.to_string())?
        .hub_profile;
    if let Some(capture) = CAPTURED
        .get(&identity(session))
        .filter(|capture| capture.observed.elapsed() < Duration::from_secs(6))
    {
        let value = &capture.value;
        return Ok(Snapshot {
            layout: value["layout"].clone(),
            base: value["baseLayout"].clone(),
            default: Some(value["defaultLayout"].clone()),
            source: value["source"].as_str().unwrap_or("personal").to_owned(),
            profile,
            editing: value["editing"].as_bool().unwrap_or(false),
        });
    }
    let personal = profile
        .home_layout
        .as_ref()
        .and_then(super::home_validation::normalize_layout);
    let defaults = if session.subject.is_some() {
        let path = format!(
            "info/home-defaults{}",
            profile
                .home_default_id
                .as_ref()
                .map(|id| format!("?default_id={}", segment(id)))
                .unwrap_or_default()
        );
        match session.request(Method::GET, &path, None).await {
            Ok(value) => Some(value),
            Err(error) if personal.is_some() => {
                tracing::debug!("Inherited Home layout unavailable: {error}");
                None
            }
            Err(error) => return Err(error),
        }
    } else {
        Some(json!({}))
    };
    let inherited = defaults.as_ref().map(|defaults| {
        if let Some(layout) =
            super::home_validation::normalize_layout(&defaults["profile"]["layout"])
        {
            (layout, "profile")
        } else if let Some(layout) =
            super::home_validation::normalize_layout(&defaults["main"]["layout"])
        {
            (layout, "main")
        } else {
            (CONTRACT["default_layout"].clone(), "bundled")
        }
    });
    let (base, source) = if let Some(personal) = personal {
        (personal, "personal".to_owned())
    } else {
        let (layout, source) = inherited
            .clone()
            .ok_or("The inherited Home layout is unavailable.")?;
        (layout, source.to_owned())
    };
    let mut layout = base.clone();
    let mut editing = false;
    match tokio::fs::read(proposal_path(session)?).await {
        Ok(bytes) => {
            if bytes.len() <= 160 * 1024 {
                if let Ok(proposal) = serde_json::from_slice::<Value>(&bytes) {
                    if proposal["base_fingerprint"] == fingerprint(&base)
                        && proposal["profile_id"] == profile.id
                    {
                        if let Some(candidate) =
                            super::home_validation::normalize_layout(&proposal["layout"])
                        {
                            layout = candidate;
                            editing = true;
                        }
                    }
                }
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(format!(
                "The retained Home proposal could not be read: {error}"
            ));
        }
    }
    session.assert_current_profile().await?;
    Ok(Snapshot {
        layout,
        base,
        default: inherited.map(|(layout, _)| layout),
        source,
        profile,
        editing,
    })
}
fn context(snapshot: &Snapshot, comparisons: bool) -> Value {
    let current_fp = fingerprint(&snapshot.layout);
    let base = fingerprint(&snapshot.base);
    let mut result = json!({"status":"ok","profile_id":snapshot.profile.id,"profile":{"id":snapshot.profile.id,"name":snapshot.profile.name,"description":snapshot.profile.description,"interests":snapshot.profile.interests,"tags":snapshot.profile.tags},"source":snapshot.source,"current_layout":snapshot.layout,"editing":snapshot.editing,"dirty":snapshot.layout!=snapshot.base,"surface_available":false,"can_stage":true,"fingerprint":current_fp,"base_fingerprint":base,"guards":{"expected_profile_id":snapshot.profile.id,"expected_fingerprint":current_fp},"note":"The native host can stage a Home proposal for review. Save or Cancel in the Home editor to resolve it.","route":"/"});
    if comparisons && current_fp != base {
        result["base_layout"] = snapshot.base.clone();
    }
    if let Some(default) = &snapshot.default {
        let fp = fingerprint(default);
        result["default_fingerprint"] = json!(fp);
        if comparisons && fp != current_fp {
            result["default_layout"] = default.clone();
        }
    } else {
        result["default_layout_available"] = json!(false);
        result["default_layout_note"] = json!(
            "The personal layout is available, but its inherited default could not be read for comparison."
        );
    }
    result
}
fn filtered_catalog(args: &Value) -> Value {
    let category = string(args, "category", "category")
        .unwrap_or("")
        .to_lowercase();
    let kind = string(args, "type", "type").unwrap_or("").to_lowercase();
    let query = string(args, "query", "query").unwrap_or("").to_lowercase();
    if !category.is_empty()
        && !catalog()["categories"]
            .as_array()
            .is_some_and(|categories| categories.contains(&json!(category)))
    {
        let mut error = fail(
            "home_widget_category_invalid",
            &format!("Unknown category '{category}'."),
        );
        error["categories"] = catalog()["categories"].clone();
        return error;
    }
    let mut result = catalog().clone();
    let presets = catalog()["presets"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|preset| {
            (category.is_empty() || preset["category"] == category)
                && (kind.is_empty() || preset["widget"]["type"] == kind)
                && (query.is_empty()
                    || format!(
                        "{} {} {} {}",
                        preset["preset_id"].as_str().unwrap_or(""),
                        preset["name"].as_str().unwrap_or(""),
                        preset["description"].as_str().unwrap_or(""),
                        preset["widget"]["type"].as_str().unwrap_or("")
                    )
                    .to_lowercase()
                    .contains(&query))
        })
        .cloned()
        .collect::<Vec<_>>();
    let kinds = presets
        .iter()
        .filter_map(|p| p["widget"]["type"].as_str())
        .collect::<std::collections::BTreeSet<_>>();
    result["widget_types"] = Value::Object(
        kinds
            .iter()
            .map(|kind| ((*kind).to_owned(), contracts()[*kind].clone()))
            .collect(),
    );
    result["widget_type_total"] = json!(kinds.len());
    result["total"] = json!(presets.len());
    result["presets"] = json!(presets);
    result
}
async fn retain_proposal(
    session: &NativePlatformSession,
    base: &Value,
    layout: &Value,
    expected: &str,
) -> Result<(), String> {
    let path = proposal_path(session)?;
    let parent = path.parent().ok_or("Invalid Home proposal path.")?;
    tokio::fs::create_dir_all(parent)
        .await
        .map_err(|e| e.to_string())?;
    let temporary = parent.join(format!(".{}.tmp", uuid::Uuid::new_v4()));
    let mut options = tokio::fs::OpenOptions::new();
    options.create_new(true).write(true);
    #[cfg(unix)]
    options.mode(0o600);
    let result=async {let mut file=options.open(&temporary).await.map_err(|e|e.to_string())?;let bytes=serde_json::to_vec(&json!({"version":1,"owner":snapshot_owner(session),"profile_id":session.profile.id,"base_fingerprint":fingerprint(base),"expected_fingerprint":expected,"candidate_fingerprint":fingerprint(layout),"layout":layout})).map_err(|e|e.to_string())?;file.write_all(&bytes).await.map_err(|e|e.to_string())?;file.sync_all().await.map_err(|e|e.to_string())?;session.assert_current_profile().await?;tokio::fs::rename(&temporary,&path).await.map_err(|e|e.to_string())}.await;
    if result.is_err() {
        let _ = tokio::fs::remove_file(&temporary).await;
    }
    result
}
pub(super) async fn execute(
    session: &NativePlatformSession,
    tool: &str,
    args: &Value,
) -> Result<Value, String> {
    if tool == "get_home_widget_catalog" {
        return Ok(filtered_catalog(args));
    }
    if tool == "list_home_data_sources" {
        return super::home_references::data_sources(session, args).await;
    }
    let _guard = target_lock(format!("{}:{}:home", session.profile.id, session.hub)).await;
    let initial = snapshot(session).await?;
    if tool == "get_home_context" {
        return Ok(context(
            &initial,
            args["include_comparisons"]
                .as_bool()
                .or_else(|| args["includeComparisons"].as_bool())
                .unwrap_or(false),
        ));
    }
    let expected_profile = string(args, "expected_profile_id", "expectedProfileId");
    let expected = string(args, "expected_fingerprint", "expectedFingerprint");
    if tool == "apply_home_layout" && (expected_profile.is_none() || expected.is_none()) {
        return Ok(fail(
            "home_apply_guard_required",
            "apply_home_layout requires expected_profile_id and expected_fingerprint from the latest Home context or validation result.",
        ));
    }
    let mut validation = super::home_validation::validate(&args["layout"], &initial.layout);
    if validation["canonical_layout"].is_object() {
        let references =
            super::home_references::validate_references(session, &validation["canonical_layout"])
                .await?;
        validation["issues"]
            .as_array_mut()
            .unwrap()
            .extend(references);
    }
    let latest = snapshot(session).await?;
    let fp = fingerprint(&latest.layout);
    let issues = validation["issues"].as_array_mut().unwrap();
    if expected_profile.is_some_and(|profile| profile != latest.profile.id) {
        issues.push(super::home_validation::issue(
            "error",
            "home_profile_changed",
            "$.expected_profile_id",
            "The active profile changed.",
        ));
    }
    if expected.is_some_and(|expected| expected != fp) {
        issues.push(super::home_validation::issue(
            "error",
            "home_layout_changed",
            "$.expected_fingerprint",
            "The Home draft changed. Merge the latest layout before applying.",
        ));
    }
    if fingerprint(&initial.layout) != fp {
        issues.push(super::home_validation::issue(
            "error",
            "home_layout_changed_during_validation",
            "$",
            "The Home draft changed while references were checked. Validate again.",
        ));
    }
    let valid = validation["canonical_layout"].is_object()
        && !validation["issues"]
            .as_array()
            .unwrap()
            .iter()
            .any(|issue| issue["severity"] == "error");
    validation["valid"] = json!(valid);
    validation["status"] = json!(if valid { "ok" } else { "validation_error" });
    validation["profile_id"] = json!(latest.profile.id);
    validation["current_fingerprint"] = json!(fp);
    if valid {
        validation["guards"] =
            json!({"expected_profile_id":latest.profile.id,"expected_fingerprint":fp});
    }
    if tool == "validate_home_layout" || !valid {
        return Ok(validation);
    }
    let candidate = &validation["canonical_layout"];
    let changed = *candidate != latest.layout;
    retain_proposal(session, &latest.base, candidate, &fp).await?;
    let candidate_fp = fingerprint(candidate);
    let response = json!({"status":"staged","owner":snapshot_owner(session),"changed":changed,"profileId":latest.profile.id,"profile_id":latest.profile.id,"baseFingerprint":fingerprint(&latest.base),"base_fingerprint":fingerprint(&latest.base),"candidateFingerprint":candidate_fp,"candidate_fingerprint":candidate_fp,"fingerprint":candidate_fp,"layout":candidate,"expected_fingerprint":fp,"guards":{"expected_profile_id":latest.profile.id,"expected_fingerprint":candidate_fp},"note":"The Home proposal is retained for review. The user must choose Save to persist it or Cancel to discard it."});
    session.assert_current_profile().await?;
    if let Some(mut capture) = CAPTURED.get_mut(&identity(session)) {
        capture.value["layout"] = candidate.clone();
        capture.value["candidateFingerprint"] = json!(candidate_fp);
        capture.value["editing"] = json!(true);
        capture.value["dirty"] = json!(true);
    }
    if let Some(parent) = session
        .context
        .as_ref()
        .and_then(|c| c.parent_request_id.as_ref())
    {
        STAGED.insert(parent.clone(), response.clone());
    }
    let _ = session
        .app_handle
        .emit("flowpilot:home-layout-staged", &response);
    Ok(response)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fingerprint_matches_browser_default() {
        assert_eq!(
            fingerprint(&CONTRACT["default_layout"]),
            CONTRACT["default_fingerprint"].as_str().unwrap()
        );
    }
    #[test]
    fn catalog_filter_selects_exact_types() {
        let result = filtered_catalog(&json!({"type":"app-embed"}));
        assert!(result["total"].as_u64().unwrap() > 0);
        assert_eq!(result["widget_type_total"], 1);
    }
}
