//! Alert rules and the in-app alert inbox.
//!
//! Rules are evaluated by `telemetry::alerts`; these endpoints only manage the
//! rule definitions and the events the engine appends. Every alert is a row in
//! the inbox first; a rule may additionally mirror its transitions out of band,
//! to the platform alerting mailbox and to the platform admins.

use crate::db::{DEFAULT_WRITE_CHUNK, delete_in_batches};
use crate::entity::{telemetry_alert_event, telemetry_alert_rule};
use crate::error::ApiError;
use crate::middleware::jwt::AppUser;
use crate::permission::global_permission::GlobalPermission;
use crate::routes::telemetry::errors::SOURCES;
use crate::state::AppState;
use crate::telemetry::alerts::{
    ALERT_COMPARATORS, ALERT_METRICS, ALERT_MODE_ANOMALY, ALERT_MODE_THRESHOLD, ALERT_MODES,
    ALERT_STATUS_RESOLVED, ALERT_STATUS_TRIGGERED, DEFAULT_MIN_SAMPLES, DEFAULT_SENSITIVITY,
    DEFAULT_WINDOW_MINUTES, MAX_MIN_SAMPLES, MAX_WINDOW_MINUTES, MIN_MIN_SAMPLES,
    MIN_WINDOW_MINUTES, TelemetryAlertConfig, evaluate_once,
};
use axum::extract::{Path, Query, State};
use axum::{Extension, Json};
use chrono::{Duration, Utc};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, Condition, EntityTrait, IntoActiveModel, PaginatorTrait,
    QueryFilter, QueryOrder, QuerySelect, Set,
};
use serde::{Deserialize, Deserializer, Serialize};
use utoipa::{IntoParams, ToSchema};

const ALERT_STATUSES: [&str; 2] = [ALERT_STATUS_TRIGGERED, ALERT_STATUS_RESOLVED];
/// Sources that reach the product-event table, which `event_count` counts and
/// `error_rate` divides by. `POST /telemetry/events` only accepts these four —
/// a rule on any other source would divide by zero samples forever, so a "gt"
/// rule could never fire and an "lt" rule would fire on the first evaluation
/// and never recover.
const EVENT_SOURCES: [&str; 4] = ["desktop", "desktop_core", "web", "backend"];
/// Metrics whose value is read from the product-event table.
const EVENT_METRICS: [&str; 2] = ["event_count", "error_rate"];
const DEFAULT_EVENT_HOURS: i64 = 24 * 7;
const MAX_EVENT_HOURS: i64 = 24 * 90;
const DEFAULT_PAGE_SIZE: u64 = 25;
const MAX_PAGE_SIZE: u64 = 100;
const RULE_LIST_LIMIT: u64 = 200;

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct TelemetryAlertRuleRecord {
    pub id: String,
    pub name: String,
    /// Watched metric: "error_rate", "latency_p95", "crash_free_rate", "event_count", "span_error_rate" or "llm_error_rate".
    pub metric: String,
    /// Optional source filter: "desktop", "desktop_core", "desktop_native", "web" or "backend". "desktop_native" never reaches "event_count" or "error_rate".
    pub source: Option<String>,
    /// "gt" or "lt".
    pub comparator: String,
    pub threshold: Option<f64>,
    /// "threshold" or "anomaly".
    pub mode: String,
    pub window_minutes: i32,
    /// Standard deviations tolerated in anomaly mode.
    pub sensitivity: Option<f64>,
    /// Baseline windows an anomaly rule needs before it may fire.
    pub min_samples: i32,
    pub enabled: bool,
    /// Whether firing and recovering also send a mail to the platform alerting mailbox.
    pub notify_email: bool,
    /// Whether firing and recovering also push a notification to every platform admin.
    pub notify_push: bool,
    pub last_evaluated_at: Option<String>,
    pub last_triggered_at: Option<String>,
    pub last_value: Option<f64>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ListTelemetryAlertRulesResponse {
    pub rules: Vec<TelemetryAlertRuleRecord>,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct TelemetryAlertEventRecord {
    pub id: String,
    pub rule_id: String,
    pub rule_name: String,
    /// "triggered" or "resolved".
    pub status: String,
    pub value: f64,
    pub threshold: Option<f64>,
    pub message: String,
    pub acknowledged_at: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ListTelemetryAlertEventsResponse {
    pub events: Vec<TelemetryAlertEventRecord>,
    pub total: u64,
    pub page: u64,
    pub page_size: u64,
    /// Triggered events in the window nobody has acknowledged yet.
    pub unacknowledged: u64,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct DeleteTelemetryAlertRuleResponse {
    pub id: String,
    pub events_deleted: u64,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct EvaluateTelemetryAlertsResponse {
    pub evaluated: u64,
    pub triggered: u64,
    pub resolved: u64,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateTelemetryAlertRulePayload {
    pub name: String,
    /// Watched metric: "error_rate", "latency_p95", "crash_free_rate", "event_count", "span_error_rate" or "llm_error_rate".
    pub metric: String,
    /// Optional source filter: "desktop", "desktop_core", "desktop_native", "web" or "backend". Omit to watch every source. "desktop_native" is rejected for the event-table metrics "event_count" and "error_rate", which it can never reach.
    #[serde(default)]
    pub source: Option<String>,
    /// "gt" or "lt".
    pub comparator: String,
    /// Required in "threshold" mode.
    #[serde(default)]
    pub threshold: Option<f64>,
    /// "threshold" (default) or "anomaly".
    #[serde(default)]
    pub mode: Option<String>,
    /// Length of the evaluated window in minutes. Default 60, capped at 1440.
    #[serde(default)]
    pub window_minutes: Option<i32>,
    /// Standard deviations tolerated in anomaly mode. Default 3.
    #[serde(default)]
    pub sensitivity: Option<f64>,
    /// Baseline windows an anomaly rule needs before it may fire. Default 5.
    #[serde(default)]
    pub min_samples: Option<i32>,
    #[serde(default)]
    pub enabled: Option<bool>,
    /// Also mail the platform alerting mailbox when the rule fires and when it recovers. There are no per-rule recipients. Default false.
    #[serde(default)]
    pub notify_email: Option<bool>,
    /// Also push a notification to every user holding the Admin permission when the rule fires and when it recovers. Default false.
    #[serde(default)]
    pub notify_push: Option<bool>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct UpdateTelemetryAlertRulePayload {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub metric: Option<String>,
    /// Source filter: "desktop", "desktop_core", "desktop_native", "web" or "backend". Send null to watch every source. "desktop_native" is rejected for the event-table metrics "event_count" and "error_rate", which it can never reach.
    #[serde(default, deserialize_with = "double_option")]
    #[schema(value_type = Option<String>)]
    pub source: Option<Option<String>>,
    #[serde(default)]
    pub comparator: Option<String>,
    /// Threshold. Send null to clear it, which only a rule in "anomaly" mode may do.
    #[serde(default, deserialize_with = "double_option")]
    #[schema(value_type = Option<f64>)]
    pub threshold: Option<Option<f64>>,
    #[serde(default)]
    pub mode: Option<String>,
    #[serde(default)]
    pub window_minutes: Option<i32>,
    #[serde(default)]
    pub sensitivity: Option<f64>,
    #[serde(default)]
    pub min_samples: Option<i32>,
    #[serde(default)]
    pub enabled: Option<bool>,
    /// Mail the platform alerting mailbox on every transition. Omit to leave the channel unchanged.
    #[serde(default)]
    pub notify_email: Option<bool>,
    /// Push a notification to every platform admin on every transition. Omit to leave the channel unchanged.
    #[serde(default)]
    pub notify_push: Option<bool>,
}

#[derive(Debug, Deserialize, IntoParams)]
pub struct ListTelemetryAlertEventsQuery {
    /// Lookback window in hours. Default 168 (7 days).
    #[serde(default)]
    pub hours: Option<i64>,
    /// Filter by status: "triggered" or "resolved".
    #[serde(default)]
    pub status: Option<String>,
    /// Filter by the rule that produced the events.
    #[serde(default)]
    pub rule_id: Option<String>,
    #[serde(default)]
    pub page: Option<u64>,
    /// Page size, capped at 100. Default 25.
    #[serde(default)]
    pub page_size: Option<u64>,
}

/// Distinguishes an absent field (`None`) from an explicit `null` (`Some(None)`).
fn double_option<'de, D, T>(deserializer: D) -> Result<Option<Option<T>>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    Deserialize::deserialize(deserializer).map(Some)
}

/// The mode-dependent fields after defaults are applied, as the engine expects
/// to read them back.
#[derive(Clone, Copy, Debug, PartialEq)]
struct ResolvedModeFields {
    threshold: Option<f64>,
    sensitivity: Option<f64>,
    min_samples: i32,
}

fn validate_vocab(field: &str, value: &str, allowed: &[&str]) -> Result<(), ApiError> {
    if allowed.contains(&value) {
        return Ok(());
    }
    Err(ApiError::bad_request(format!(
        "Unknown {} '{}', expected one of {}",
        field,
        value,
        allowed.join(", ")
    )))
}

fn require_name(value: &str) -> Result<String, ApiError> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(ApiError::bad_request("'name' must not be empty"));
    }
    Ok(trimmed.to_string())
}

fn normalize_source(source: Option<String>) -> Option<String> {
    source
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

/// The sources that can actually appear in the table `metric` reads.
fn sources_for_metric(metric: &str) -> &'static [&'static str] {
    if EVENT_METRICS.contains(&metric) {
        return &EVENT_SOURCES;
    }
    &SOURCES
}

/// A rule may only watch a source the ingest accepts, and only one that can
/// reach the table its metric reads — otherwise the rule is silently dead in
/// one direction and permanently firing in the other.
fn validate_source_for_metric(metric: &str, source: Option<&str>) -> Result<(), ApiError> {
    let Some(source) = source else {
        return Ok(());
    };

    validate_vocab("source", source, &SOURCES)?;

    let allowed = sources_for_metric(metric);
    if !allowed.contains(&source) {
        return Err(ApiError::bad_request(format!(
            "Source '{}' never reaches the '{}' metric, expected one of {}",
            source,
            metric,
            allowed.join(", ")
        )));
    }
    Ok(())
}

fn clamp_window(minutes: Option<i32>) -> i32 {
    minutes
        .unwrap_or(DEFAULT_WINDOW_MINUTES)
        .clamp(MIN_WINDOW_MINUTES, MAX_WINDOW_MINUTES)
}

/// Applies the mode defaults and rejects combinations the engine cannot
/// evaluate: a threshold rule without a threshold would never fire, and an
/// anomaly rule must persist a sensitivity and a baseline size.
fn resolve_mode_fields(
    mode: &str,
    threshold: Option<f64>,
    sensitivity: Option<f64>,
    min_samples: Option<i32>,
) -> Result<ResolvedModeFields, ApiError> {
    if let Some(threshold) = threshold
        && !threshold.is_finite()
    {
        return Err(ApiError::bad_request("'threshold' must be a finite number"));
    }

    let min_samples = match min_samples {
        Some(value) if !(MIN_MIN_SAMPLES..=MAX_MIN_SAMPLES).contains(&value) => {
            return Err(ApiError::bad_request(format!(
                "'min_samples' must be between {} and {}",
                MIN_MIN_SAMPLES, MAX_MIN_SAMPLES
            )));
        }
        Some(value) => value,
        None => DEFAULT_MIN_SAMPLES,
    };

    if mode == ALERT_MODE_ANOMALY {
        let sensitivity = sensitivity.unwrap_or(DEFAULT_SENSITIVITY);
        if !sensitivity.is_finite() || sensitivity <= 0.0 {
            return Err(ApiError::bad_request(
                "'sensitivity' must be a positive number",
            ));
        }
        return Ok(ResolvedModeFields {
            threshold,
            sensitivity: Some(sensitivity),
            min_samples,
        });
    }

    let Some(threshold) = threshold else {
        return Err(ApiError::bad_request(
            "'threshold' is required for rules in 'threshold' mode",
        ));
    };

    Ok(ResolvedModeFields {
        threshold: Some(threshold),
        sensitivity,
        min_samples,
    })
}

fn alert_rule_record(model: telemetry_alert_rule::Model) -> TelemetryAlertRuleRecord {
    TelemetryAlertRuleRecord {
        id: model.id,
        name: model.name,
        metric: model.metric,
        source: model.source,
        comparator: model.comparator,
        threshold: model.threshold,
        mode: model.mode,
        window_minutes: model.window_minutes,
        sensitivity: model.sensitivity,
        min_samples: model.min_samples,
        enabled: model.enabled,
        notify_email: model.notify_email,
        notify_push: model.notify_push,
        last_evaluated_at: model.last_evaluated_at.map(|ts| ts.and_utc().to_rfc3339()),
        last_triggered_at: model.last_triggered_at.map(|ts| ts.and_utc().to_rfc3339()),
        last_value: model.last_value,
        created_at: model.created_at.and_utc().to_rfc3339(),
        updated_at: model.updated_at.and_utc().to_rfc3339(),
    }
}

fn alert_event_record(model: telemetry_alert_event::Model) -> TelemetryAlertEventRecord {
    TelemetryAlertEventRecord {
        id: model.id,
        rule_id: model.rule_id,
        rule_name: model.rule_name,
        status: model.status,
        value: model.value,
        threshold: model.threshold,
        message: model.message,
        acknowledged_at: model.acknowledged_at.map(|ts| ts.and_utc().to_rfc3339()),
        created_at: model.created_at.and_utc().to_rfc3339(),
    }
}

#[utoipa::path(
    get,
    path = "/admin/telemetry/alerts",
    tag = "admin",
    responses(
        (status = 200, description = "The configured alert rules, newest first", body = ListTelemetryAlertRulesResponse),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden")
    ),
    description = "List the alert rules watching the anonymous telemetry metrics, with the value and time of their last evaluation and the delivery channels each rule uses. Requires Admin permission."
)]
#[tracing::instrument(name = "GET /admin/telemetry/alerts", skip(state, user))]
pub async fn list_telemetry_alert_rules(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
) -> Result<Json<ListTelemetryAlertRulesResponse>, ApiError> {
    user.check_global_permission(&state, GlobalPermission::Admin)
        .await?;

    let rules = telemetry_alert_rule::Entity::find()
        .order_by_desc(telemetry_alert_rule::Column::CreatedAt)
        .limit(RULE_LIST_LIMIT)
        .all(&state.db)
        .await?
        .into_iter()
        .map(alert_rule_record)
        .collect();

    Ok(Json(ListTelemetryAlertRulesResponse { rules }))
}

#[utoipa::path(
    post,
    path = "/admin/telemetry/alerts",
    tag = "admin",
    request_body = CreateTelemetryAlertRulePayload,
    responses(
        (status = 200, description = "The created alert rule", body = TelemetryAlertRuleRecord),
        (status = 400, description = "Unknown metric, source, comparator or mode, a source the chosen metric can never see, or a mode without its required fields"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden")
    ),
    description = "Create an alert rule that watches a telemetry metric over a time window, either against a fixed threshold or against its own recent baseline. Alerts always appear in the in-app inbox. Enable 'notify_email' to also mail the platform alerting mailbox, and 'notify_push' to also push a notification to every platform admin; both fire on the firing and the recovery transition, and both default to off. Requires Admin permission."
)]
#[tracing::instrument(name = "POST /admin/telemetry/alerts", skip(state, user, payload))]
pub async fn create_telemetry_alert_rule(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Json(payload): Json<CreateTelemetryAlertRulePayload>,
) -> Result<Json<TelemetryAlertRuleRecord>, ApiError> {
    user.check_global_permission(&state, GlobalPermission::Admin)
        .await?;

    let name = require_name(&payload.name)?;
    validate_vocab("metric", &payload.metric, &ALERT_METRICS)?;
    validate_vocab("comparator", &payload.comparator, &ALERT_COMPARATORS)?;

    let mode = payload
        .mode
        .unwrap_or_else(|| ALERT_MODE_THRESHOLD.to_string());
    validate_vocab("mode", &mode, &ALERT_MODES)?;

    let source = normalize_source(payload.source);
    validate_source_for_metric(&payload.metric, source.as_deref())?;

    let resolved = resolve_mode_fields(
        &mode,
        payload.threshold,
        payload.sensitivity,
        payload.min_samples,
    )?;
    let now = Utc::now().naive_utc();

    let model = telemetry_alert_rule::ActiveModel {
        id: Set(flow_like_types::create_id()),
        name: Set(name),
        metric: Set(payload.metric),
        source: Set(source),
        comparator: Set(payload.comparator),
        threshold: Set(resolved.threshold),
        mode: Set(mode),
        window_minutes: Set(clamp_window(payload.window_minutes)),
        sensitivity: Set(resolved.sensitivity),
        min_samples: Set(resolved.min_samples),
        enabled: Set(payload.enabled.unwrap_or(true)),
        // The schema default never reaches the entity, so both channels are set explicitly.
        notify_email: Set(payload.notify_email.unwrap_or(false)),
        notify_push: Set(payload.notify_push.unwrap_or(false)),
        last_evaluated_at: Set(None),
        last_triggered_at: Set(None),
        last_value: Set(None),
        created_at: Set(now),
        updated_at: Set(now),
    }
    .insert(&state.db)
    .await?;

    Ok(Json(alert_rule_record(model)))
}

#[utoipa::path(
    patch,
    path = "/admin/telemetry/alerts/{rule_id}",
    tag = "admin",
    params(("rule_id" = String, Path, description = "Alert rule identifier")),
    request_body = UpdateTelemetryAlertRulePayload,
    responses(
        (status = 200, description = "The updated alert rule", body = TelemetryAlertRuleRecord),
        (status = 400, description = "Unknown metric, source, comparator or mode, a source the chosen metric can never see, or a mode without its required fields"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "Alert rule not found")
    ),
    description = "Change an alert rule: its watched metric, window, threshold, anomaly sensitivity, whether it is enabled, and where its alerts are delivered — 'notify_email' mails the platform alerting mailbox, 'notify_push' pushes to every platform admin. Omitted fields keep their current value. Requires Admin permission."
)]
#[tracing::instrument(
    name = "PATCH /admin/telemetry/alerts/{rule_id}",
    skip(state, user, payload)
)]
pub async fn update_telemetry_alert_rule(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(rule_id): Path<String>,
    Json(payload): Json<UpdateTelemetryAlertRulePayload>,
) -> Result<Json<TelemetryAlertRuleRecord>, ApiError> {
    user.check_global_permission(&state, GlobalPermission::Admin)
        .await?;

    let model = telemetry_alert_rule::Entity::find_by_id(&rule_id)
        .one(&state.db)
        .await?
        .ok_or(ApiError::NOT_FOUND)?;

    let mode = payload.mode.clone().unwrap_or_else(|| model.mode.clone());
    validate_vocab("mode", &mode, &ALERT_MODES)?;

    if let Some(metric) = &payload.metric {
        validate_vocab("metric", metric, &ALERT_METRICS)?;
    }
    if let Some(comparator) = &payload.comparator {
        validate_vocab("comparator", comparator, &ALERT_COMPARATORS)?;
    }

    // The pair has to hold after the patch, not just for the fields it carries:
    // switching only the metric can strand an already stored source.
    let metric = payload
        .metric
        .clone()
        .unwrap_or_else(|| model.metric.clone());
    let source = match payload.source.clone() {
        Some(source) => normalize_source(source),
        None => model.source.clone(),
    };
    validate_source_for_metric(&metric, source.as_deref())?;

    let threshold = payload.threshold.unwrap_or(model.threshold);
    let sensitivity = payload.sensitivity.or(model.sensitivity);
    let min_samples = payload.min_samples.or(Some(
        model.min_samples.clamp(MIN_MIN_SAMPLES, MAX_MIN_SAMPLES),
    ));
    let resolved = resolve_mode_fields(&mode, threshold, sensitivity, min_samples)?;

    let mut active = model.into_active_model();

    if let Some(name) = payload.name {
        active.name = Set(require_name(&name)?);
    }
    if let Some(metric) = payload.metric {
        active.metric = Set(metric);
    }
    if payload.source.is_some() {
        active.source = Set(source);
    }
    if let Some(comparator) = payload.comparator {
        active.comparator = Set(comparator);
    }
    if let Some(window_minutes) = payload.window_minutes {
        active.window_minutes = Set(clamp_window(Some(window_minutes)));
    }
    if let Some(enabled) = payload.enabled {
        active.enabled = Set(enabled);
    }
    if let Some(notify_email) = payload.notify_email {
        active.notify_email = Set(notify_email);
    }
    if let Some(notify_push) = payload.notify_push {
        active.notify_push = Set(notify_push);
    }

    active.mode = Set(mode);
    active.threshold = Set(resolved.threshold);
    active.sensitivity = Set(resolved.sensitivity);
    active.min_samples = Set(resolved.min_samples);
    active.updated_at = Set(Utc::now().naive_utc());

    let model = active.update(&state.db).await?;
    Ok(Json(alert_rule_record(model)))
}

#[utoipa::path(
    delete,
    path = "/admin/telemetry/alerts/{rule_id}",
    tag = "admin",
    params(("rule_id" = String, Path, description = "Alert rule identifier")),
    responses(
        (status = 200, description = "The rule and its inbox entries were removed", body = DeleteTelemetryAlertRuleResponse),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "Alert rule not found")
    ),
    description = "Delete an alert rule together with the alerts it produced. Requires Admin permission."
)]
#[tracing::instrument(name = "DELETE /admin/telemetry/alerts/{rule_id}", skip(state, user))]
pub async fn delete_telemetry_alert_rule(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(rule_id): Path<String>,
) -> Result<Json<DeleteTelemetryAlertRuleResponse>, ApiError> {
    user.check_global_permission(&state, GlobalPermission::Admin)
        .await?;

    let model = telemetry_alert_rule::Entity::find_by_id(&rule_id)
        .one(&state.db)
        .await?
        .ok_or(ApiError::NOT_FOUND)?;

    let events = delete_in_batches::<telemetry_alert_event::Entity>(
        &state.db,
        state.db_dialect,
        Condition::all().add(telemetry_alert_event::Column::RuleId.eq(&rule_id)),
        DEFAULT_WRITE_CHUNK,
        None,
    )
    .await?;

    telemetry_alert_rule::Entity::delete_by_id(&model.id)
        .exec(&state.db)
        .await?;

    Ok(Json(DeleteTelemetryAlertRuleResponse {
        id: rule_id,
        events_deleted: events.rows,
    }))
}

#[utoipa::path(
    get,
    path = "/admin/telemetry/alerts/events",
    tag = "admin",
    params(ListTelemetryAlertEventsQuery),
    responses(
        (status = 200, description = "Paginated alert inbox, newest first", body = ListTelemetryAlertEventsResponse),
        (status = 400, description = "Unknown status"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden")
    ),
    description = "Read the alert inbox: the alerts that fired and recovered in the selected window, with how many are still unacknowledged. Requires Admin permission."
)]
#[tracing::instrument(name = "GET /admin/telemetry/alerts/events", skip_all)]
pub async fn list_telemetry_alert_events(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Query(q): Query<ListTelemetryAlertEventsQuery>,
) -> Result<Json<ListTelemetryAlertEventsResponse>, ApiError> {
    user.check_global_permission(&state, GlobalPermission::Admin)
        .await?;

    let hours = q
        .hours
        .unwrap_or(DEFAULT_EVENT_HOURS)
        .clamp(1, MAX_EVENT_HOURS);
    let page = q.page.unwrap_or(0);
    let page_size = q
        .page_size
        .unwrap_or(DEFAULT_PAGE_SIZE)
        .clamp(1, MAX_PAGE_SIZE);
    let cutoff = Utc::now().naive_utc() - Duration::hours(hours);
    let rule_id = q.rule_id.as_deref().filter(|value| !value.is_empty());

    let mut select = telemetry_alert_event::Entity::find()
        .filter(telemetry_alert_event::Column::CreatedAt.gte(cutoff));
    let mut open = telemetry_alert_event::Entity::find()
        .filter(telemetry_alert_event::Column::CreatedAt.gte(cutoff))
        .filter(telemetry_alert_event::Column::Status.eq(ALERT_STATUS_TRIGGERED))
        .filter(telemetry_alert_event::Column::AcknowledgedAt.is_null());

    if let Some(status) = &q.status
        && !status.is_empty()
    {
        validate_vocab("alert status", status, &ALERT_STATUSES)?;
        select = select.filter(telemetry_alert_event::Column::Status.eq(status));
    }

    if let Some(rule_id) = rule_id {
        select = select.filter(telemetry_alert_event::Column::RuleId.eq(rule_id));
        open = open.filter(telemetry_alert_event::Column::RuleId.eq(rule_id));
    }

    let total = select.clone().count(&state.db).await?;
    let unacknowledged = open.count(&state.db).await?;

    let events = select
        .order_by_desc(telemetry_alert_event::Column::CreatedAt)
        .order_by_desc(telemetry_alert_event::Column::Id)
        .paginate(&state.db, page_size)
        .fetch_page(page)
        .await?
        .into_iter()
        .map(alert_event_record)
        .collect();

    Ok(Json(ListTelemetryAlertEventsResponse {
        events,
        total,
        page,
        page_size,
        unacknowledged,
    }))
}

#[utoipa::path(
    post,
    path = "/admin/telemetry/alerts/{event_id}/ack",
    tag = "admin",
    params(("event_id" = String, Path, description = "Alert inbox entry identifier")),
    responses(
        (status = 200, description = "The acknowledged alert", body = TelemetryAlertEventRecord),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "Alert not found")
    ),
    description = "Acknowledge an alert so it stops counting towards the unacknowledged badge in the inbox. Requires Admin permission."
)]
#[tracing::instrument(
    name = "POST /admin/telemetry/alerts/{event_id}/ack",
    skip(state, user)
)]
pub async fn acknowledge_telemetry_alert_event(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(event_id): Path<String>,
) -> Result<Json<TelemetryAlertEventRecord>, ApiError> {
    user.check_global_permission(&state, GlobalPermission::Admin)
        .await?;

    let model = telemetry_alert_event::Entity::find_by_id(&event_id)
        .one(&state.db)
        .await?
        .ok_or(ApiError::NOT_FOUND)?;

    if model.acknowledged_at.is_some() {
        return Ok(Json(alert_event_record(model)));
    }

    let mut active = model.into_active_model();
    active.acknowledged_at = Set(Some(Utc::now().naive_utc()));

    let model = active.update(&state.db).await?;
    Ok(Json(alert_event_record(model)))
}

#[utoipa::path(
    post,
    path = "/admin/telemetry/alerts/evaluate",
    tag = "admin",
    responses(
        (status = 200, description = "Evaluation completed, returns the rules evaluated and the state changes", body = EvaluateTelemetryAlertsResponse),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden")
    ),
    description = "Manually evaluate every enabled alert rule once, append the resulting alerts to the inbox and deliver them on the channels each rule enables — the platform alerting mailbox and the platform admins. Long-running deployments use an in-process timer; serverless schedulers use the service-authenticated maintenance endpoint. Requires Admin permission."
)]
#[tracing::instrument(name = "POST /admin/telemetry/alerts/evaluate", skip(state, user))]
pub async fn evaluate_telemetry_alerts(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
) -> Result<Json<EvaluateTelemetryAlertsResponse>, ApiError> {
    user.check_global_permission(&state, GlobalPermission::Admin)
        .await?;

    let config = TelemetryAlertConfig::from_env();
    let result = evaluate_once(&state, &config).await.map_err(|e| {
        tracing::error!(error = %e, "Admin telemetry alert evaluation failed");
        ApiError::internal_error(flow_like_types::anyhow!(
            "Telemetry alert evaluation failed: {}",
            e
        ))
    })?;

    if !result.is_empty() {
        tracing::info!(
            evaluated = result.evaluated,
            triggered = result.triggered,
            resolved = result.resolved,
            "Admin telemetry alert evaluation changed rule states"
        );
    }

    Ok(Json(EvaluateTelemetryAlertsResponse {
        evaluated: result.evaluated,
        triggered: result.triggered,
        resolved: result.resolved,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::telemetry::alerts::{is_valid_comparator, is_valid_metric, is_valid_mode};
    use axum::Router;
    use axum::routing::{get, post};

    #[test]
    fn only_known_vocabulary_is_accepted() {
        for metric in ALERT_METRICS {
            assert!(validate_vocab("metric", metric, &ALERT_METRICS).is_ok());
            assert!(is_valid_metric(metric));
        }
        for comparator in ALERT_COMPARATORS {
            assert!(is_valid_comparator(comparator));
        }
        for mode in ALERT_MODES {
            assert!(is_valid_mode(mode));
        }

        assert!(validate_vocab("metric", "cpu", &ALERT_METRICS).is_err());
        assert!(validate_vocab("comparator", "gte", &ALERT_COMPARATORS).is_err());
        assert!(validate_vocab("mode", "ml", &ALERT_MODES).is_err());
        assert!(validate_vocab("alert status", "open", &ALERT_STATUSES).is_err());
    }

    #[test]
    fn names_are_trimmed_and_must_not_be_blank() {
        assert_eq!(require_name("  Error rate ").unwrap(), "Error rate");
        assert!(require_name("   ").is_err());
        assert!(require_name("").is_err());
    }

    #[test]
    fn sources_are_trimmed_and_blank_means_every_source() {
        assert_eq!(
            normalize_source(Some(" desktop ".to_string())),
            Some("desktop".to_string())
        );
        assert_eq!(normalize_source(Some("  ".to_string())), None);
        assert_eq!(normalize_source(None), None);
    }

    #[test]
    fn unknown_sources_are_rejected_for_every_metric() {
        for metric in ALERT_METRICS {
            assert!(validate_source_for_metric(metric, None).is_ok());
            assert!(validate_source_for_metric(metric, Some("mobile")).is_err());
            assert!(validate_source_for_metric(metric, Some("Desktop")).is_err());
            assert!(validate_source_for_metric(metric, Some("")).is_err());
            assert!(validate_source_for_metric(metric, Some("desktop")).is_ok());
        }
    }

    #[test]
    fn event_metrics_reject_a_source_the_event_ingest_never_accepts() {
        for metric in EVENT_METRICS {
            assert!(validate_source_for_metric(metric, Some("desktop_native")).is_err());
            assert!(validate_source_for_metric(metric, Some("web_server")).is_err());
            for source in EVENT_SOURCES {
                assert!(
                    validate_source_for_metric(metric, Some(source)).is_ok(),
                    "expected '{}' to be valid for '{}'",
                    source,
                    metric
                );
            }
        }

        for metric in [
            "crash_free_rate",
            "span_error_rate",
            "llm_error_rate",
            "latency_p95",
        ] {
            assert!(validate_source_for_metric(metric, Some("desktop_native")).is_ok());
        }
    }

    #[test]
    fn the_alert_source_vocabulary_is_the_ingest_vocabulary() {
        for source in EVENT_SOURCES {
            assert!(
                SOURCES.contains(&source),
                "'{}' must be an ingest source",
                source
            );
        }
        // `desktop_native` and `web_server` report crashes only and never reach
        // the product-event table, so they are ingest sources without being
        // event sources.
        assert_eq!(SOURCES.len(), EVENT_SOURCES.len() + 2);
        for metric in ALERT_METRICS {
            let allowed = sources_for_metric(metric);
            assert!(allowed.iter().all(|source| SOURCES.contains(source)));
        }
    }

    #[test]
    fn windows_default_to_an_hour_and_stay_inside_the_bounds() {
        assert_eq!(clamp_window(None), DEFAULT_WINDOW_MINUTES);
        assert_eq!(clamp_window(Some(0)), MIN_WINDOW_MINUTES);
        assert_eq!(clamp_window(Some(-5)), MIN_WINDOW_MINUTES);
        assert_eq!(clamp_window(Some(15)), 15);
        assert_eq!(clamp_window(Some(100_000)), MAX_WINDOW_MINUTES);
    }

    #[test]
    fn threshold_rules_need_a_finite_threshold() {
        assert!(resolve_mode_fields(ALERT_MODE_THRESHOLD, None, None, None).is_err());
        assert!(resolve_mode_fields(ALERT_MODE_THRESHOLD, Some(f64::NAN), None, None).is_err());
        assert!(
            resolve_mode_fields(ALERT_MODE_THRESHOLD, Some(f64::INFINITY), None, None).is_err()
        );

        let resolved = resolve_mode_fields(ALERT_MODE_THRESHOLD, Some(0.05), None, None).unwrap();
        assert_eq!(resolved.threshold, Some(0.05));
        assert_eq!(resolved.sensitivity, None);
        assert_eq!(resolved.min_samples, DEFAULT_MIN_SAMPLES);
    }

    #[test]
    fn anomaly_rules_persist_the_sensitivity_and_baseline_defaults() {
        let resolved = resolve_mode_fields(ALERT_MODE_ANOMALY, None, None, None).unwrap();
        assert_eq!(resolved.sensitivity, Some(DEFAULT_SENSITIVITY));
        assert_eq!(resolved.min_samples, DEFAULT_MIN_SAMPLES);
        assert_eq!(resolved.threshold, None);

        let explicit = resolve_mode_fields(ALERT_MODE_ANOMALY, None, Some(2.5), Some(12)).unwrap();
        assert_eq!(explicit.sensitivity, Some(2.5));
        assert_eq!(explicit.min_samples, 12);
    }

    #[test]
    fn anomaly_rules_reject_a_sensitivity_that_can_never_fire() {
        assert!(resolve_mode_fields(ALERT_MODE_ANOMALY, None, Some(0.0), None).is_err());
        assert!(resolve_mode_fields(ALERT_MODE_ANOMALY, None, Some(-1.0), None).is_err());
        assert!(resolve_mode_fields(ALERT_MODE_ANOMALY, None, Some(f64::NAN), None).is_err());
    }

    #[test]
    fn baseline_sizes_outside_the_bounds_are_rejected() {
        assert!(
            resolve_mode_fields(ALERT_MODE_ANOMALY, None, None, Some(MIN_MIN_SAMPLES - 1)).is_err()
        );
        assert!(
            resolve_mode_fields(ALERT_MODE_ANOMALY, None, None, Some(MAX_MIN_SAMPLES + 1)).is_err()
        );
        assert!(resolve_mode_fields(ALERT_MODE_ANOMALY, None, None, Some(MIN_MIN_SAMPLES)).is_ok());
        assert!(resolve_mode_fields(ALERT_MODE_ANOMALY, None, None, Some(MAX_MIN_SAMPLES)).is_ok());
    }

    #[test]
    fn patch_payloads_tell_an_absent_field_from_an_explicit_null() {
        let absent: UpdateTelemetryAlertRulePayload =
            serde_json::from_str(r#"{"enabled":false}"#).unwrap();
        assert_eq!(absent.threshold, None);
        assert_eq!(absent.source, None);

        let cleared: UpdateTelemetryAlertRulePayload =
            serde_json::from_str(r#"{"source":null,"threshold":null}"#).unwrap();
        assert_eq!(cleared.source, Some(None));
        assert_eq!(cleared.threshold, Some(None));

        let set: UpdateTelemetryAlertRulePayload =
            serde_json::from_str(r#"{"source":"web","threshold":0.2}"#).unwrap();
        assert_eq!(set.source, Some(Some("web".to_string())));
        assert_eq!(set.threshold, Some(Some(0.2)));
    }

    /// A rule that silently mailed the operators or paged every admin would be
    /// a surprise, so an unspecified channel is off, never on.
    #[test]
    fn create_payloads_default_both_notification_channels_to_off() {
        let payload: CreateTelemetryAlertRulePayload = serde_json::from_str(
            r#"{"name":"Error rate","metric":"error_rate","comparator":"gt","threshold":0.05}"#,
        )
        .unwrap();
        assert_eq!(payload.notify_email, None);
        assert_eq!(payload.notify_push, None);
        assert!(!payload.notify_email.unwrap_or(false));
        assert!(!payload.notify_push.unwrap_or(false));

        let explicit: CreateTelemetryAlertRulePayload = serde_json::from_str(
            r#"{"name":"Error rate","metric":"error_rate","comparator":"gt","threshold":0.05,"notify_email":true,"notify_push":false}"#,
        )
        .unwrap();
        assert_eq!(explicit.notify_email, Some(true));
        assert_eq!(explicit.notify_push, Some(false));
    }

    /// Toggling a rule from the list view sends `enabled` alone; that patch must
    /// not silence a channel the rule already delivers on.
    #[test]
    fn patch_payloads_leave_an_absent_notification_channel_unchanged() {
        let absent: UpdateTelemetryAlertRulePayload =
            serde_json::from_str(r#"{"enabled":true}"#).unwrap();
        assert_eq!(absent.notify_email, None);
        assert_eq!(absent.notify_push, None);

        let toggled: UpdateTelemetryAlertRulePayload =
            serde_json::from_str(r#"{"notify_email":true,"notify_push":false}"#).unwrap();
        assert_eq!(toggled.notify_email, Some(true));
        assert_eq!(toggled.notify_push, Some(false));
    }

    #[test]
    fn query_parameters_stay_snake_case() {
        let uri: axum::http::Uri =
            "/admin/telemetry/alerts/events?hours=24&status=triggered&rule_id=r1&page_size=10"
                .parse()
                .unwrap();
        let Query(q) = Query::<ListTelemetryAlertEventsQuery>::try_from_uri(&uri).unwrap();

        assert_eq!(q.hours, Some(24));
        assert_eq!(q.status.as_deref(), Some("triggered"));
        assert_eq!(q.rule_id.as_deref(), Some("r1"));
        assert_eq!(q.page_size, Some(10));
        assert_eq!(q.page, None);
    }

    /// The inbox routes mix static and dynamic segments at the same depth; make
    /// sure the router the Integrator wires up actually builds.
    #[test]
    fn the_alert_routes_do_not_collide() {
        let _router: Router = Router::new()
            .route("/telemetry/alerts", get(|| async {}).post(|| async {}))
            .route("/telemetry/alerts/events", get(|| async {}))
            .route("/telemetry/alerts/evaluate", post(|| async {}))
            .route(
                "/telemetry/alerts/{rule_id}",
                axum::routing::patch(|| async {}).delete(|| async {}),
            )
            .route("/telemetry/alerts/{event_id}/ack", post(|| async {}));
    }
}
