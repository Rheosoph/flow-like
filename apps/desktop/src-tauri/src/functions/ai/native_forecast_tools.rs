//! Forecast tool previews and retained results, owned by the native account/profile session.
use super::{
    frontend_tool_bridge::FrontendToolContext,
    native_platform_tools::{NativePlatformSession, segment},
};
use flow_like::bit::{Bit, BitTypes, ForecastingAssetRef};
use reqwest::Method;
use serde_json::{Value, json};
use std::{
    collections::{HashMap, HashSet, VecDeque},
    sync::{Arc, LazyLock},
    time::{Duration, Instant},
};
use tauri::AppHandle;
use tokio::sync::Mutex;

const MAX_CACHE_BYTES: usize = 64 * 1024 * 1024;
const MAX_CACHE_RUNS: usize = 3;
const CACHE_TTL: Duration = Duration::from_secs(30 * 60);

#[derive(Clone)]
struct CompletedRun {
    key: String,
    request: Value,
    result: Value,
    bytes: usize,
    expires: Instant,
    saves: Arc<Mutex<HashMap<String, Value>>>,
}

#[derive(Default)]
struct ForecastCache {
    identity: String,
    runs: VecDeque<CompletedRun>,
}
static CACHE: LazyLock<Mutex<ForecastCache>> = LazyLock::new(Mutex::default);

impl ForecastCache {
    fn bind(&mut self, identity: &str) {
        if self.identity != identity {
            self.runs.clear();
            self.identity = identity.into();
        }
        self.prune();
    }
    fn prune(&mut self) {
        self.runs.retain(|run| run.expires > Instant::now());
        let mut bytes: usize = self.runs.iter().map(|run| run.bytes).sum();
        while self.runs.len() > MAX_CACHE_RUNS || bytes > MAX_CACHE_BYTES {
            if let Some(run) = self.runs.pop_front() {
                bytes -= run.bytes;
            } else {
                break;
            }
        }
    }
}

pub(super) fn supports(tool: &str) -> bool {
    tool == "forecast_tool"
}

fn text<'a>(value: &'a Value, name: &str, maximum: usize) -> Result<&'a str, String> {
    value
        .as_str()
        .filter(|value| {
            !value.trim().is_empty()
                && value.chars().count() <= maximum
                && !value.chars().any(|c| c < ' ')
        })
        .ok_or_else(|| {
            format!("{name} must be a non-empty string of at most {maximum} characters.")
        })
}
fn id<'a>(value: &'a Value, name: &str) -> Result<&'a str, String> {
    let value = text(value, name, 256)?;
    if matches!(value, "." | "..") {
        return Err(format!("Invalid {name}."));
    }
    Ok(value)
}
fn href(app: &str, user: bool, forecast: Option<&str>) -> String {
    let mut query =
        reqwest::Url::parse("https://local/library/config/explore").expect("constant URL");
    query
        .query_pairs_mut()
        .append_pair("id", app)
        .append_pair("view", "forecast");
    if user {
        query.query_pairs_mut().append_pair("scope", "user");
    }
    if let Some(forecast) = forecast {
        query.query_pairs_mut().append_pair("forecast", forecast);
    }
    format!("{}?{}", query.path(), query.query().unwrap_or_default())
}
fn summary(saved: &Value) -> Value {
    let mut value = json!({});
    for key in [
        "id",
        "app_id",
        "name",
        "description",
        "created_by",
        "created_at",
    ] {
        value[key] = saved[key].clone();
    }
    value["model_bit_id"] = saved["request"]["model_bit_id"].clone();
    value["horizon"] = saved["request"]["horizon"].clone();
    value["run_id"] = saved["result"]["run_id"].clone();
    value
}
fn array(value: &Value) -> &[Value] {
    value.as_array().map(Vec::as_slice).unwrap_or_default()
}
fn point_preview(points: &Value, limit: usize) -> Value {
    let points = array(points);
    let preview: Vec<Value> = points
        .iter()
        .take(limit)
        .map(|point| {
            let mut point = point.clone();
            point["quantiles"] = json!(
                array(&point["quantiles"])
                    .iter()
                    .take(21)
                    .cloned()
                    .collect::<Vec<_>>()
            );
            point
        })
        .collect();
    json!({"total":points.len(),"truncated":points.len()>limit,"points":preview})
}
fn request_preview(request: &Value) -> Value {
    if request.to_string().encode_utf16().count() <= 16 * 1024 {
        json!({"request":request,"request_truncated":false})
    } else {
        json!({"request":{"model_bit_id":request["model_bit_id"],"horizon":request["horizon"],"backtest_horizon":request.get("backtest_horizon").unwrap_or(&json!(0)),"backtest_offset":request.get("backtest_offset").unwrap_or(&json!(0))},"request_truncated":true,"request_note":"The complete setup is available in Data Studio. This preview is too large to return in chat."})
    }
}
fn evaluation_windows(result: &Value, limit: usize) -> Value {
    let mut windows: Vec<Value> = Vec::new();
    let mut positions = HashMap::new();
    for point in array(&result["backtest"]["forecasts"]) {
        let key = json!([point["series_id"], point["target"]]).to_string();
        let index = *positions.entry(key).or_insert_with(|| {
            windows.push(json!({"series_id":point["series_id"],"target":point["target"],"cutoff_ms":null,"start_ms":point["timestamp_ms"],"end_ms":point["timestamp_ms"],"horizon":0,"observations":0}));
            windows.len()-1
        });
        let window = &mut windows[index];
        let stamp = point["timestamp_ms"].as_i64().unwrap_or_default();
        window["start_ms"] = json!(window["start_ms"].as_i64().unwrap_or(stamp).min(stamp));
        window["end_ms"] = json!(window["end_ms"].as_i64().unwrap_or(stamp).max(stamp));
        window["horizon"] = json!(window["horizon"].as_u64().unwrap_or_default() + 1);
        if !point["actual"].is_null() {
            window["observations"] = json!(window["observations"].as_u64().unwrap_or_default() + 1);
        }
    }
    for point in array(&result["history"]) {
        let key = json!([point["series_id"], point["target"]]).to_string();
        if let Some(index) = positions.get(&key) {
            let window = &mut windows[*index];
            let stamp = point["timestamp_ms"].as_i64().unwrap_or_default();
            if stamp < window["start_ms"].as_i64().unwrap_or_default()
                && window["cutoff_ms"]
                    .as_i64()
                    .is_none_or(|cutoff| stamp > cutoff)
            {
                window["cutoff_ms"] = json!(stamp);
            }
        }
    }
    json!({"evaluation_windows":windows.iter().take(limit).collect::<Vec<_>>(),"windows_total":windows.len(),"windows_truncated":windows.len()>limit})
}
fn merge(target: &mut Value, value: Value) {
    if let (Some(target), Some(value)) = (target.as_object_mut(), value.as_object()) {
        target.extend(value.clone());
    }
}
fn result_preview(result: &Value, limit: usize) -> Value {
    let mut capabilities = result["capabilities"].clone();
    capabilities["quantile_levels"] = json!(
        array(&capabilities["quantile_levels"])
            .iter()
            .take(21)
            .collect::<Vec<_>>()
    );
    let mut backtest = Value::Null;
    if !result["backtest"].is_null() {
        let metrics = array(&result["backtest"]["metrics"]);
        backtest = json!({"horizon":result["backtest"]["horizon"],"offset":result["backtest"].get("offset").unwrap_or(&json!(0)),"metrics":metrics.iter().take(limit).collect::<Vec<_>>(),"metrics_total":metrics.len(),"metrics_truncated":metrics.len()>limit,"metric_units":"MAE and RMSE use target units. MAPE and WAPE are percentage errors, not accuracy scores; lower is better. Null means unavailable.","forecasts":point_preview(&result["backtest"]["forecasts"],limit)});
        merge(&mut backtest, evaluation_windows(result, limit));
    }
    json!({"run_id":result["run_id"],"model":result["model"],"capabilities":capabilities,"input_rows":result["input_rows"],"horizon":result["horizon"],"point_statistic":result["point_statistic"],"quantile_levels":array(&result["quantile_levels"]).iter().take(21).collect::<Vec<_>>(),"quantile_levels_truncated":array(&result["quantile_levels"]).len()>21,"forecasts":point_preview(&result["forecasts"],limit),"backtest":backtest})
}
fn bounded(value: Value) -> Value {
    if value.to_string().len() <= 64 * 1024 {
        return value;
    }
    let mut compact = json!({"response_truncated":true,"note":"The result exceeds the chat preview limit. Open Data Studio to inspect the full prediction; a completed run can still be saved using run_id."});
    for key in ["status", "app_id", "user_scoped", "run_id", "href"] {
        if let Some(field) = value.get(key) {
            compact[key] = field.clone();
        }
    }
    if value["saved"].is_object() {
        compact["saved"] = json!({"id":value["saved"]["id"],"name":value["saved"]["name"]});
    }
    for key in ["models", "forecasts"] {
        if value[key].is_array() {
            compact[key] = json!(array(&value[key]).iter().take(20).map(|item| json!({"id":item["id"].as_str().unwrap_or_default().chars().take(256).collect::<String>(),"name":item["name"].as_str().unwrap_or_default().chars().take(256).collect::<String>()})).collect::<Vec<_>>());
            compact["truncated"] = json!(true);
        }
    }
    compact
}

async fn models(session: &NativePlatformSession) -> Result<Vec<Bit>, String> {
    let bits = crate::functions::settings::profiles::get_bits_in_current_profile(
        session.app_handle.clone(),
    )
    .await
    .map_err(|e| e.to_string())?;
    session.assert_current_profile().await?;
    let mut seen = HashSet::new();
    Ok(bits
        .into_iter()
        .filter(|bit| {
            if bit.bit_type != BitTypes::Forecasting {
                return false;
            }
            let references: Vec<_> = session
                .profile
                .bits
                .iter()
                .filter(|reference| {
                    reference
                        .rsplit_once(':')
                        .map_or(reference.as_str(), |(_, id)| id)
                        == bit.id
                })
                .collect();
            if references.is_empty()
                || !references.iter().all(|reference| {
                    ForecastingAssetRef {
                        bit: (*reference).clone(),
                        relative_path: String::new(),
                    }
                    .matches_reference(bit, &session.profile.hub)
                    .unwrap_or(false)
                })
            {
                return false;
            }
            seen.insert(format!("{}:{}", bit.hub, bit.id))
        })
        .collect())
}

async fn query(
    session: &NativePlatformSession,
    app: &str,
    user: bool,
    operation: &str,
    payload: Value,
    offset: usize,
    limit: usize,
) -> Result<Value, String> {
    session.assert_current_profile().await?;
    let response = if session.is_local(app).await? {
        let app_handle = session.app_handle.clone();
        let app_id = app.to_owned();
        match operation {
            "run" => serde_json::to_value(
                crate::functions::app::forecasting::query_forecast(
                    app_handle,
                    app_id,
                    serde_json::from_value(payload).map_err(|e| e.to_string())?,
                    Some(user),
                )
                .await
                .map_err(|e| e.to_string())?,
            ),
            "list" => serde_json::to_value(
                crate::functions::app::saved_forecasts::studio_forecast_list(
                    app_handle,
                    app_id,
                    Some(user),
                    Some(offset),
                    Some(limit),
                )
                .await
                .map_err(|e| e.to_string())?,
            ),
            "get" => serde_json::to_value(
                crate::functions::app::saved_forecasts::studio_forecast_get(
                    app_handle,
                    app_id,
                    payload.as_str().ok_or("Missing forecast ID")?.into(),
                    Some(user),
                )
                .await
                .map_err(|e| e.to_string())?,
            ),
            "save" => serde_json::to_value(
                crate::functions::app::saved_forecasts::studio_forecast_create(
                    app_handle,
                    app_id,
                    serde_json::from_value(payload).map_err(|e| e.to_string())?,
                    Some(user),
                )
                .await
                .map_err(|e| e.to_string())?,
            ),
            _ => return Err("Unsupported forecast operation".into()),
        }
        .map_err(|e| e.to_string())?
    } else {
        let base = format!("apps/{}/db", segment(app));
        let (method, mut path, body) = match operation {
            "run" => (
                Method::POST,
                format!("{base}/queries/forecast"),
                Some(payload),
            ),
            "list" => (
                Method::GET,
                format!("{base}/forecasts?limit={limit}&offset={offset}"),
                None,
            ),
            "get" => (
                Method::GET,
                format!(
                    "{base}/forecasts/{}",
                    segment(payload.as_str().ok_or("Missing forecast ID")?)
                ),
                None,
            ),
            "save" => (Method::POST, format!("{base}/forecasts"), Some(payload)),
            _ => return Err("Unsupported forecast operation".into()),
        };
        if user {
            path.push_str(if path.contains('?') {
                "&scope=user"
            } else {
                "?scope=user"
            });
        }
        session.request(method, &path, body.as_ref()).await?
    };
    session.assert_current_profile().await?;
    Ok(response)
}

pub(super) async fn execute(
    app_handle: &AppHandle,
    context: Option<&FrontendToolContext>,
    _tool: &str,
    args: &Value,
) -> Result<Value, String> {
    let session = NativePlatformSession::new(app_handle, context).await?;
    if context.and_then(|context| context.profile_id.as_deref())
        != Some(session.profile.id.as_str())
    {
        return Err("Forecasting requires the requesting session's active profile. Start a new Data Studio request.".into());
    }
    let identity = json!([
        session.hub,
        session.profile.id,
        session.subject,
        super::native_tool_bridge::run_identity(context)
            .and_then(|identity| identity.auth.map(|auth| auth.generation))
    ])
    .to_string();
    CACHE.lock().await.bind(&identity);
    let operation = text(&args["operation"], "operation", 256)?;
    let app = session.app_id(args)?;
    id(&json!(app), "app_id")?;
    session.require_visible(&app)?;
    let user = match args.get("user_scoped") {
        None => false,
        Some(Value::Bool(user)) => *user,
        _ => return Err("user_scoped must be a boolean.".into()),
    };
    let limit = args
        .get("limit")
        .map_or(Some(20), Value::as_u64)
        .filter(|limit| (1..=100).contains(limit))
        .ok_or("limit must be an integer between 1 and 100.")? as usize;
    if args.get("profile_id").is_some() || args.get("result").is_some() {
        return Err("The host supplies the active profile and completed forecast results.".into());
    }
    let mut envelope = json!({"status":"ok","app_id":app,"user_scoped":user});
    match operation {
        "list_models" => {
            let models = models(&session).await?;
            envelope["models"] = json!(models.iter().take(limit).map(|bit| {
                let bit = serde_json::to_value(bit).unwrap_or_default();
                json!({"id":bit["id"],"name":bit["meta"]["en"]["name"].as_str().or_else(||bit["meta"].as_object().and_then(|meta|meta.values().find_map(|meta|meta["name"].as_str()))).unwrap_or(bit["id"].as_str().unwrap_or_default()),"family":bit["parameters"]["model_type"],"hub":bit["hub"],"repository":bit["repository"]})
            }).collect::<Vec<_>>());
            envelope["total"] = json!(models.len());
            envelope["truncated"] = json!(models.len() > limit);
            envelope["note"] = json!(
                "These models are activated in the current profile. Checkpoint capabilities are returned by run. Add or activate a Forecasting model in the model catalog if the list is empty."
            );
        }
        "run" => {
            let request = &args["request"];
            if !request.is_object() || request.get("profile_id").is_some() {
                return Err("run requires a forecast request without profile_id.".into());
            }
            let model = text(&request["model_bit_id"], "request.model_bit_id", 256)?;
            if !models(&session).await?.iter().any(|bit| bit.id == model) {
                return Err(
                    "Choose a Forecasting model returned by list_models for this profile.".into(),
                );
            }
            let mut payload = request.clone();
            payload["profile_id"] = json!(session.profile.id);
            let result = query(&session, &app, user, "run", payload, 0, limit).await?;
            let run = text(&result["run_id"], "run_id", 256)?;
            if result["model"]["bit_id"].as_str() != Some(model) {
                return Err("The forecast response does not match its model request.".into());
            }
            let bytes = json!({"request":request,"result":result}).to_string().len();
            if bytes > MAX_CACHE_BYTES {
                return Err("The completed prediction exceeds the chat snapshot limit. Run it in Data Studio with less history.".into());
            }
            let mut cache = CACHE.lock().await;
            if cache.identity != identity {
                return Err("The forecasting session changed.".into());
            }
            let key = json!([app, user, run]).to_string();
            cache.runs.retain(|run| run.key != key);
            cache.runs.push_back(CompletedRun {
                key,
                request: request.clone(),
                result: result.clone(),
                bytes,
                expires: Instant::now() + CACHE_TTL,
                saves: Arc::new(Mutex::new(HashMap::new())),
            });
            cache.prune();
            merge(&mut envelope, request_preview(request));
            merge(&mut envelope, result_preview(&result, limit));
            envelope["saved"] = json!(false);
            envelope["retention"] = json!(
                "Save using run_id within 30 minutes. This session retains at most three completed runs and 64 MiB; changing account, Hub or profile clears them."
            );
        }
        "list" => {
            let mut forecasts = Vec::new();
            let mut offset = 0;
            while offset < limit + 1 {
                let count = (limit + 1 - offset).min(100);
                let page = query(&session, &app, user, "list", Value::Null, offset, count).await?;
                let page = array(&page);
                for saved in page {
                    if saved["app_id"].as_str() != Some(app.as_str()) {
                        return Err("Saved predictions do not match the requested app.".into());
                    }
                    forecasts.push(saved.clone());
                }
                if page.len() < count {
                    break;
                }
                offset += count;
            }
            envelope["forecasts"] = json!(
                forecasts
                    .iter()
                    .take(limit)
                    .map(|saved| {
                        let mut saved = saved.clone();
                        saved["href"] = json!(href(&app, user, saved["id"].as_str()));
                        saved
                    })
                    .collect::<Vec<_>>()
            );
            envelope["truncated"] = json!(forecasts.len() > limit);
            envelope["href"] = json!(href(&app, user, None));
        }
        "get" => {
            let forecast = id(&args["forecast_id"], "forecast_id")?;
            let saved = query(&session, &app, user, "get", json!(forecast), 0, limit).await?;
            if saved["app_id"].as_str() != Some(app.as_str())
                || saved["id"].as_str() != Some(forecast)
            {
                return Err("The saved prediction does not match its app and ID.".into());
            }
            envelope["saved"] = summary(&saved);
            envelope["href"] = json!(href(&app, user, Some(forecast)));
            merge(&mut envelope, request_preview(&saved["request"]));
            merge(&mut envelope, result_preview(&saved["result"], limit));
        }
        "save" => {
            if args.get("request").is_some() {
                return Err("save accepts run_id, name and description; it uses the exact completed request.".into());
            }
            let run_id = text(&args["run_id"], "run_id", 256)?;
            let name = text(&args["name"], "name", 256)?.trim();
            if args.get("description").is_some_and(|description| {
                description
                    .as_str()
                    .is_none_or(|description| description.encode_utf16().count() > 4000)
            }) {
                return Err("description must be a string of at most 4000 characters.".into());
            }
            let key = json!([app, user, run_id]).to_string();
            let run={let mut cache=CACHE.lock().await;cache.prune();if cache.identity!=identity {return Err("The forecasting session changed.".into());}cache.runs.iter().find(|run|run.key==key).cloned()}.ok_or("This completed run is unavailable in the current app, profile and data scope. Run the forecast again before saving.")?;
            let save_key = json!([name, args["description"]]).to_string();
            let mut saves = run.saves.lock().await;
            let saved = if let Some(saved) = saves.get(&save_key) {
                saved.clone()
            } else {
                if saves.len() >= 32 {
                    return Err("This run has reached its 32 saved-copy limit. Run another prediction to create more copies.".into());
                }
                let saved=query(&session,&app,user,"save",json!({"name":name,"description":args["description"],"request":run.request,"result":run.result}),0,limit).await?;
                if saved["app_id"].as_str() != Some(app.as_str())
                    || saved["result"]["run_id"].as_str() != Some(run_id)
                {
                    return Err("The saved prediction does not match the completed run.".into());
                }
                let saved = summary(&saved);
                saves.insert(save_key, saved.clone());
                saved
            };
            envelope["href"] = json!(href(&app, user, saved["id"].as_str()));
            envelope["saved"] = saved;
        }
        _ => {
            return Err(format!(
                "Unsupported forecast_tool operation '{operation}'."
            ));
        }
    }
    session.assert_current_profile().await?;
    Ok(bounded(envelope))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn completed(key: &str, bytes: usize) -> CompletedRun {
        CompletedRun {
            key: key.into(),
            request: json!({}),
            result: json!({}),
            bytes,
            expires: Instant::now() + CACHE_TTL,
            saves: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    #[test]
    fn retained_forecasts_are_bounded_and_cleared_when_the_login_or_profile_changes() {
        let mut cache = ForecastCache::default();
        cache.bind("profile:login-one");
        for index in 0..4 {
            cache.runs.push_back(completed(&index.to_string(), 1));
        }
        cache.prune();
        assert_eq!(cache.runs.len(), 3);
        assert_eq!(cache.runs.front().unwrap().key, "1");
        cache.runs.push_back(completed("large", MAX_CACHE_BYTES));
        cache.prune();
        assert_eq!(cache.runs.len(), 1);
        cache.bind("profile:login-two");
        assert!(cache.runs.is_empty());
        let mut expired = completed("expired", 1);
        expired.expires = Instant::now();
        cache.runs.push_back(expired);
        cache.prune();
        assert!(cache.runs.is_empty());
    }

    #[test]
    fn evaluation_preview_retains_cutoffs_and_bounds_quantiles() {
        let result = json!({"history":[{"series_id":"one","target":"sales","timestamp_ms":1},{"series_id":"one","target":"sales","timestamp_ms":3}],"backtest":{"forecasts":[{"series_id":"one","target":"sales","timestamp_ms":4,"actual":null},{"series_id":"one","target":"sales","timestamp_ms":5,"actual":9}]}});
        let windows = evaluation_windows(&result, 20);
        assert_eq!(windows["evaluation_windows"][0]["cutoff_ms"], 3);
        assert_eq!(windows["evaluation_windows"][0]["horizon"], 2);
        assert_eq!(windows["evaluation_windows"][0]["observations"], 1);
        let points = json!([{"quantiles":(0..30).collect::<Vec<_>>()},{"quantiles":[]}]);
        let preview = point_preview(&points, 1);
        assert_eq!(
            preview["points"][0]["quantiles"].as_array().unwrap().len(),
            21
        );
        assert_eq!(preview["total"], 2);
        assert_eq!(preview["truncated"], true);
    }

    #[test]
    fn oversized_previews_keep_the_run_id_needed_to_save_the_complete_result() {
        let response = bounded(
            json!({"status":"ok","app_id":"app","run_id":"run","forecasts":[{"id":"forecast","name":"x".repeat(100_000)}]}),
        );
        assert_eq!(response["run_id"], "run");
        assert_eq!(response["response_truncated"], true);
        assert!(response.to_string().len() < 64 * 1024);
    }
}
