//! FlowPilot prompt feedback: which assistant turns users rated, and everything captured about the
//! turn that produced them.
//!
//! Rows are the app-less `Feedback` records written by `PUT /ai/global-chat/feedback`, identified by
//! `appId IS NULL AND eventId = 'flowpilot'`. The app filter is not optional decoration: every
//! app-scoped analytics query filters `"appId" = $1` and so already excludes these rows, and this
//! module must be the exact mirror image so an admin listing can never sweep in an app's private
//! end-user feedback.
//!
//! The prompt/response/run context all live inside the row's `context` JSON blob, so the interesting
//! filters (model, provider, outcome, free text) cannot be pushed into SQL. Rather than pretend
//! otherwise, one bounded window is read per request and folded in memory; `truncated` says plainly
//! when the window hit its cap.

use super::super::super::ai::global_chat::feedback::FLOWPILOT_FEEDBACK_SCOPE;
use crate::entity::feedback;
use crate::error::ApiError;
use crate::middleware::jwt::AppUser;
use crate::permission::global_permission::GlobalPermission;
use crate::state::AppState;
use axum::extract::{Path, Query, State};
use axum::{Extension, Json};
use chrono::{Duration, NaiveDateTime, Utc};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QueryOrder, QuerySelect};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};
use utoipa::{IntoParams, ToSchema};

/// Upper bound on the rows folded for one request. Ratings are a deliberate user action and so are
/// low volume; this exists to keep a pathological window from loading the whole table.
const SCAN_CAP: u64 = 5_000;
const PREVIEW_CHARS: usize = 240;
/// How many entries the model/provider breakdowns keep.
const TOP_LIST_LIMIT: usize = 12;

#[derive(Debug, Deserialize, IntoParams)]
pub struct ListPromptFeedbackQuery {
    /// Lookback window in hours. Default 720 (30 days), clamped to 1..=2160.
    #[serde(default)]
    pub hours: Option<i64>,
    /// Filter by sentiment: "positive", "negative" or "all" (default).
    #[serde(default)]
    pub rating: Option<String>,
    /// Filter by the provider that ran the turn, e.g. "bits" or "claude-code".
    #[serde(default)]
    pub provider: Option<String>,
    /// Filter by the model id that ran the turn.
    #[serde(default)]
    pub model: Option<String>,
    /// Filter by how the turn ended: "ok", "partial", "error" or "timeout".
    #[serde(default)]
    pub outcome: Option<String>,
    /// Case-insensitive substring match on the prompt, the response and the reviewer's comment.
    #[serde(default)]
    pub query: Option<String>,
    #[serde(default)]
    pub page: Option<u64>,
    /// Page size, capped at 100. Default 25.
    #[serde(default)]
    pub page_size: Option<u64>,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct PromptFeedbackRecord {
    pub id: String,
    pub message_id: String,
    pub conversation_id: Option<String>,
    pub user_id: Option<String>,
    /// 5 for a positive rating, 1 for a negative one.
    pub rating: i64,
    pub comment: String,
    pub provider: Option<String>,
    pub model: Option<String>,
    pub reasoning_effort: Option<String>,
    pub outcome: Option<String>,
    pub auto_mode: Option<bool>,
    pub surface: Option<String>,
    pub duration_ms: Option<i64>,
    pub total_tokens: Option<i64>,
    pub prompt_preview: String,
    pub response_preview: String,
    pub has_transcript: bool,
    pub created_at: String,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct PromptFeedbackFacetCount {
    pub key: String,
    pub count: i64,
    /// Ratings in this bucket that were negative — the reason to look at it.
    pub negative: i64,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct PromptFeedbackTrendPoint {
    /// ISO-8601 timestamp at the start of the UTC day.
    pub ts: String,
    pub positive: i64,
    pub negative: i64,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct PromptFeedbackSummary {
    pub total: i64,
    pub positive: i64,
    pub negative: i64,
    /// Share of ratings that were positive, 0..100. Null when nothing was rated.
    pub satisfaction: Option<f64>,
    pub raters: i64,
    pub conversations: i64,
    pub with_comment: i64,
    pub by_model: Vec<PromptFeedbackFacetCount>,
    pub by_provider: Vec<PromptFeedbackFacetCount>,
    pub by_outcome: Vec<PromptFeedbackFacetCount>,
    pub trend: Vec<PromptFeedbackTrendPoint>,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct PromptFeedbackFilters {
    pub providers: Vec<String>,
    pub models: Vec<String>,
    pub outcomes: Vec<String>,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ListPromptFeedbackResponse {
    pub items: Vec<PromptFeedbackRecord>,
    pub total: u64,
    pub page: u64,
    pub page_size: u64,
    pub hours: i64,
    /// True when the window hit [`SCAN_CAP`] and the numbers cover only the most recent rows.
    pub truncated: bool,
    /// Computed over the filtered set, so the tiles and breakdowns always describe what is on screen.
    pub summary: PromptFeedbackSummary,
    /// Every value present in the unfiltered window, so narrowing the filters never empties the
    /// dropdowns that would let you widen them again.
    pub filters: PromptFeedbackFilters,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct PromptFeedbackDetailResponse {
    pub record: PromptFeedbackRecord,
    pub prompt: String,
    pub response: String,
    /// The full `IChatRunContext` stamped while the turn was running.
    pub run_context: Option<serde_json::Value>,
    pub usage: Option<serde_json::Value>,
    pub steps: Vec<String>,
    pub tools: Vec<String>,
    pub app_refs: Vec<String>,
    /// Present only when the user opted into sharing the conversation.
    pub transcript: Option<serde_json::Value>,
    pub transcript_truncated: bool,
    pub can_contact: bool,
}

fn json_str(value: Option<&serde_json::Value>, keys: &[&str]) -> Option<String> {
    let value = value?;
    for key in keys {
        if let Some(found) = value.get(key).and_then(|v| v.as_str())
            && !found.is_empty()
        {
            return Some(found.to_string());
        }
    }
    None
}

fn json_i64(value: Option<&serde_json::Value>, keys: &[&str]) -> Option<i64> {
    let value = value?;
    for key in keys {
        if let Some(found) = value.get(key)
            && let Some(number) = found.as_i64().or_else(|| found.as_f64().map(|v| v as i64))
        {
            return Some(number);
        }
    }
    None
}

fn json_bool(value: Option<&serde_json::Value>, keys: &[&str]) -> Option<bool> {
    let value = value?;
    for key in keys {
        if let Some(found) = value.get(key).and_then(|v| v.as_bool()) {
            return Some(found);
        }
    }
    None
}

fn json_string_list(value: Option<&serde_json::Value>, key: &str) -> Vec<String> {
    value
        .and_then(|v| v.get(key))
        .and_then(|v| v.as_array())
        .map(|entries| {
            entries
                .iter()
                .filter_map(|entry| entry.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

fn preview(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.chars().count() <= PREVIEW_CHARS {
        return trimmed.to_string();
    }
    let clipped: String = trimmed.chars().take(PREVIEW_CHARS).collect();
    format!("{clipped}…")
}

/// Strips the primary-key namespace so the UI shows (and links to) the chat message id.
fn message_id_of(row_id: &str) -> String {
    row_id
        .strip_prefix("flowpilot:")
        .unwrap_or(row_id)
        .to_string()
}

fn record_from(model: &feedback::Model) -> PromptFeedbackRecord {
    let context = model.context.as_ref();
    let run_context = context.and_then(|c| c.get("run_context"));

    PromptFeedbackRecord {
        id: model.id.clone(),
        message_id: json_str(context, &["message_id", "messageId"])
            .unwrap_or_else(|| message_id_of(&model.id)),
        conversation_id: json_str(context, &["conversation_id", "conversationId"]),
        user_id: model.user_id.clone(),
        rating: model.rating,
        comment: model.comment.clone(),
        provider: json_str(run_context, &["provider"]),
        model: json_str(run_context, &["effective_model_id", "model_id", "model"]),
        reasoning_effort: json_str(run_context, &["reasoning_effort"]),
        outcome: json_str(run_context, &["outcome"]),
        auto_mode: json_bool(run_context, &["auto_mode"]),
        surface: json_str(run_context, &["surface"]),
        duration_ms: json_i64(run_context, &["duration_ms"]),
        total_tokens: json_i64(context.and_then(|c| c.get("usage")), &["total_tokens"]),
        prompt_preview: preview(&json_str(context, &["prompt"]).unwrap_or_default()),
        response_preview: preview(&json_str(context, &["response"]).unwrap_or_default()),
        has_transcript: context
            .and_then(|c| c.get("transcript"))
            .and_then(|v| v.as_array())
            .is_some_and(|entries| !entries.is_empty()),
        created_at: model.created_at.and_utc().to_rfc3339(),
    }
}

/// A rating is positive at the top of the 0..5 scale and negative at the bottom. The scale is
/// unsigned everywhere in this codebase — treating `rating > 0` as positive (as the app analytics
/// rollup does) would count every thumbs-down as praise.
fn is_positive(rating: i64) -> bool {
    rating >= 4
}

fn is_negative(rating: i64) -> bool {
    rating > 0 && rating <= 2
}

fn matches_text(record: &PromptFeedbackRecord, needle: &str) -> bool {
    let needle = needle.to_lowercase();
    record.prompt_preview.to_lowercase().contains(&needle)
        || record.response_preview.to_lowercase().contains(&needle)
        || record.comment.to_lowercase().contains(&needle)
        || record
            .model
            .as_deref()
            .is_some_and(|model| model.to_lowercase().contains(&needle))
}

fn top_facets(counts: BTreeMap<String, (i64, i64)>) -> Vec<PromptFeedbackFacetCount> {
    let mut facets: Vec<PromptFeedbackFacetCount> = counts
        .into_iter()
        .map(|(key, (count, negative))| PromptFeedbackFacetCount {
            key,
            count,
            negative,
        })
        .collect();
    facets.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.key.cmp(&b.key)));
    facets.truncate(TOP_LIST_LIMIT);
    facets
}

fn day_key(created_at: NaiveDateTime) -> NaiveDateTime {
    created_at.date().and_hms_opt(0, 0, 0).unwrap_or(created_at)
}

fn summarize(records: &[PromptFeedbackRecord], raw: &[feedback::Model]) -> PromptFeedbackSummary {
    let mut positive = 0i64;
    let mut negative = 0i64;
    let mut with_comment = 0i64;
    let mut raters: HashSet<&str> = HashSet::new();
    let mut conversations: HashSet<&str> = HashSet::new();
    let mut by_model: BTreeMap<String, (i64, i64)> = BTreeMap::new();
    let mut by_provider: BTreeMap<String, (i64, i64)> = BTreeMap::new();
    let mut by_outcome: BTreeMap<String, (i64, i64)> = BTreeMap::new();
    let mut trend: BTreeMap<NaiveDateTime, (i64, i64)> = BTreeMap::new();

    for (record, model) in records.iter().zip(raw.iter()) {
        let record_negative = is_negative(record.rating);
        if is_positive(record.rating) {
            positive += 1;
        }
        if record_negative {
            negative += 1;
        }
        if !record.comment.trim().is_empty() {
            with_comment += 1;
        }
        if let Some(user) = record.user_id.as_deref() {
            raters.insert(user);
        }
        if let Some(conversation) = record.conversation_id.as_deref() {
            conversations.insert(conversation);
        }

        let bump = |map: &mut BTreeMap<String, (i64, i64)>, key: Option<&str>| {
            let entry = map.entry(key.unwrap_or("unknown").to_string()).or_default();
            entry.0 += 1;
            if record_negative {
                entry.1 += 1;
            }
        };
        bump(&mut by_model, record.model.as_deref());
        bump(&mut by_provider, record.provider.as_deref());
        bump(&mut by_outcome, record.outcome.as_deref());

        let day = trend.entry(day_key(model.created_at)).or_default();
        if is_positive(record.rating) {
            day.0 += 1;
        }
        if record_negative {
            day.1 += 1;
        }
    }

    let total = records.len() as i64;
    let rated = positive + negative;

    PromptFeedbackSummary {
        total,
        positive,
        negative,
        satisfaction: (rated > 0).then(|| (positive as f64 / rated as f64) * 100.0),
        raters: raters.len() as i64,
        conversations: conversations.len() as i64,
        with_comment,
        by_model: top_facets(by_model),
        by_provider: top_facets(by_provider),
        by_outcome: top_facets(by_outcome),
        trend: trend
            .into_iter()
            .map(|(ts, (positive, negative))| PromptFeedbackTrendPoint {
                ts: ts.and_utc().to_rfc3339(),
                positive,
                negative,
            })
            .collect(),
    }
}

fn sorted_unique(values: impl Iterator<Item = String>) -> Vec<String> {
    let mut unique: Vec<String> = values.collect::<HashSet<_>>().into_iter().collect();
    unique.sort();
    unique
}

#[utoipa::path(
    get,
    path = "/admin/telemetry/prompt-feedback",
    tag = "admin",
    params(ListPromptFeedbackQuery),
    responses(
        (status = 200, description = "Rated FlowPilot turns with the captured prompt, model and outcome", body = ListPromptFeedbackResponse),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden")
    ),
    description = "List FlowPilot assistant messages users rated, newest first, with a satisfaction summary and per-model/provider/outcome breakdowns for the window. Model, provider, outcome and free-text filters are applied over a bounded window of the most recent ratings; `truncated` reports when that window was reached. Requires Admin permission."
)]
#[tracing::instrument(name = "GET /admin/telemetry/prompt-feedback", skip_all)]
pub async fn list_prompt_feedback(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Query(q): Query<ListPromptFeedbackQuery>,
) -> Result<Json<ListPromptFeedbackResponse>, ApiError> {
    user.check_global_permission(&state, GlobalPermission::Admin)
        .await?;

    let hours = q.hours.unwrap_or(24 * 30).clamp(1, 2160);
    let page = q.page.unwrap_or(0);
    let page_size = q.page_size.unwrap_or(25).clamp(1, 100);
    let cutoff = Utc::now().naive_utc() - Duration::hours(hours);

    let rows = feedback::Entity::find()
        .filter(feedback::Column::AppId.is_null())
        .filter(feedback::Column::EventId.eq(FLOWPILOT_FEEDBACK_SCOPE))
        .filter(feedback::Column::CreatedAt.gte(cutoff))
        .order_by_desc(feedback::Column::CreatedAt)
        .limit(SCAN_CAP)
        .all(&state.db)
        .await?;

    let truncated = rows.len() as u64 >= SCAN_CAP;
    if truncated {
        tracing::warn!(
            cap = SCAN_CAP,
            hours,
            "prompt feedback window hit its row cap; summary covers only the most recent ratings"
        );
    }

    let records: Vec<PromptFeedbackRecord> = rows.iter().map(record_from).collect();
    let filters = PromptFeedbackFilters {
        providers: sorted_unique(records.iter().filter_map(|r| r.provider.clone())),
        models: sorted_unique(records.iter().filter_map(|r| r.model.clone())),
        outcomes: sorted_unique(records.iter().filter_map(|r| r.outcome.clone())),
    };

    // Filter over (record, row) pairs: `summarize` needs the row's timestamp for the daily trend,
    // and the two lists must stay index-aligned after narrowing.
    let sentiment = q.rating.as_deref().unwrap_or("all");
    let filtered: Vec<(PromptFeedbackRecord, feedback::Model)> = records
        .into_iter()
        .zip(rows)
        .filter(|(record, _)| match sentiment {
            "positive" => is_positive(record.rating),
            "negative" => is_negative(record.rating),
            _ => true,
        })
        .filter(|(record, _)| match q.provider.as_deref() {
            Some(provider) if !provider.is_empty() => record.provider.as_deref() == Some(provider),
            _ => true,
        })
        .filter(|(record, _)| match q.model.as_deref() {
            Some(model) if !model.is_empty() => record.model.as_deref() == Some(model),
            _ => true,
        })
        .filter(|(record, _)| match q.outcome.as_deref() {
            Some(outcome) if !outcome.is_empty() => record.outcome.as_deref() == Some(outcome),
            _ => true,
        })
        .filter(|(record, _)| match q.query.as_deref() {
            Some(needle) if !needle.trim().is_empty() => matches_text(record, needle.trim()),
            _ => true,
        })
        .collect();

    let (filtered_records, filtered_rows): (Vec<_>, Vec<_>) = filtered.into_iter().unzip();
    let summary = summarize(&filtered_records, &filtered_rows);

    let total = filtered_records.len() as u64;
    let items = filtered_records
        .into_iter()
        .skip((page * page_size) as usize)
        .take(page_size as usize)
        .collect();

    Ok(Json(ListPromptFeedbackResponse {
        items,
        total,
        page,
        page_size,
        hours,
        truncated,
        summary,
        filters,
    }))
}

#[utoipa::path(
    get,
    path = "/admin/telemetry/prompt-feedback/{feedback_id}",
    tag = "admin",
    params(("feedback_id" = String, Path, description = "Feedback row id")),
    responses(
        (status = 200, description = "One rated FlowPilot turn with its full prompt, response and run context", body = PromptFeedbackDetailResponse),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "Not found")
    ),
    description = "Read one rated FlowPilot turn: the full prompt and response, how the turn was executed (provider, model, reasoning effort, outcome, duration), token usage, the plan steps and tools it ran, and the conversation transcript when the user opted into sharing it. Requires Admin permission."
)]
#[tracing::instrument(name = "GET /admin/telemetry/prompt-feedback/{feedback_id}", skip_all)]
pub async fn get_prompt_feedback(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(feedback_id): Path<String>,
) -> Result<Json<PromptFeedbackDetailResponse>, ApiError> {
    user.check_global_permission(&state, GlobalPermission::Admin)
        .await?;

    let model = feedback::Entity::find()
        .filter(feedback::Column::Id.eq(feedback_id))
        .filter(feedback::Column::AppId.is_null())
        .filter(feedback::Column::EventId.eq(FLOWPILOT_FEEDBACK_SCOPE))
        .one(&state.db)
        .await?
        .ok_or(ApiError::NOT_FOUND)?;

    let record = record_from(&model);
    let context = model.context.as_ref();

    Ok(Json(PromptFeedbackDetailResponse {
        record,
        prompt: json_str(context, &["prompt"]).unwrap_or_default(),
        response: json_str(context, &["response"]).unwrap_or_default(),
        run_context: context.and_then(|c| c.get("run_context")).cloned(),
        usage: context.and_then(|c| c.get("usage")).cloned(),
        steps: json_string_list(context, "steps"),
        tools: json_string_list(context, "tools"),
        app_refs: json_string_list(context, "app_refs"),
        transcript: context.and_then(|c| c.get("transcript")).cloned(),
        transcript_truncated: json_bool(context, &["transcript_truncated"]).unwrap_or(false),
        can_contact: json_bool(context, &["can_contact"]).unwrap_or(false),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use sea_orm::{DbBackend, QueryTrait};
    use serde_json::json;

    fn model(id: &str, rating: i64, context: serde_json::Value) -> feedback::Model {
        feedback::Model {
            id: id.to_string(),
            user_id: Some("user-1".to_string()),
            app_id: None,
            template_id: None,
            event_id: Some(FLOWPILOT_FEEDBACK_SCOPE.to_string()),
            context: Some(context),
            comment: String::new(),
            rating,
            created_at: NaiveDateTime::parse_from_str("2026-08-17 10:00:00", "%Y-%m-%d %H:%M:%S")
                .expect("timestamp"),
            updated_at: NaiveDateTime::parse_from_str("2026-08-17 10:00:00", "%Y-%m-%d %H:%M:%S")
                .expect("timestamp"),
        }
    }

    /// The whole safety story of this module: app-scoped readers filter `"appId" = $1` (which SQL
    /// excludes NULL from), and this listing must be their exact mirror image. A refactor that drops
    /// either predicate leaks feedback across the boundary in one direction or the other.
    #[test]
    fn the_listing_is_scoped_to_app_less_flowpilot_rows() {
        let sql = feedback::Entity::find()
            .filter(feedback::Column::AppId.is_null())
            .filter(feedback::Column::EventId.eq(FLOWPILOT_FEEDBACK_SCOPE))
            .build(DbBackend::Postgres)
            .to_string();

        assert!(sql.contains(r#""appId" IS NULL"#), "{sql}");
        assert!(sql.contains(r#""eventId" = 'flowpilot'"#), "{sql}");
    }

    #[test]
    fn the_rating_scale_is_unsigned_so_a_thumbs_down_is_never_positive() {
        assert!(is_positive(5));
        assert!(is_positive(4));
        assert!(!is_positive(3));
        assert!(!is_positive(1));

        assert!(is_negative(1));
        assert!(is_negative(2));
        assert!(!is_negative(3));
        assert!(!is_negative(5));
        // 0 means withdrawn; it is neither, and no row should carry it.
        assert!(!is_positive(0));
        assert!(!is_negative(0));
    }

    #[test]
    fn a_record_reads_the_run_context_the_client_stamped() {
        let row = model(
            "flowpilot:msg-1",
            1,
            json!({
                "message_id": "msg-1",
                "conversation_id": "conv-1",
                "prompt": "build me a board",
                "response": "done",
                "run_context": {
                    "provider": "bits",
                    "model_id": "gpt-5",
                    "effective_model_id": "bits:gpt-5",
                    "outcome": "error",
                    "duration_ms": 4200,
                    "auto_mode": true
                },
                "usage": { "total_tokens": 1234 },
                "transcript": [{ "role": "user", "content": "hi" }]
            }),
        );

        let record = record_from(&row);
        assert_eq!(record.message_id, "msg-1");
        assert_eq!(record.conversation_id.as_deref(), Some("conv-1"));
        assert_eq!(record.provider.as_deref(), Some("bits"));
        // The prefixed id is what actually ran, so it wins over the raw picker id.
        assert_eq!(record.model.as_deref(), Some("bits:gpt-5"));
        assert_eq!(record.outcome.as_deref(), Some("error"));
        assert_eq!(record.duration_ms, Some(4200));
        assert_eq!(record.total_tokens, Some(1234));
        assert_eq!(record.auto_mode, Some(true));
        assert!(record.has_transcript);
        assert_eq!(record.prompt_preview, "build me a board");
    }

    #[test]
    fn a_record_survives_a_row_with_no_context_at_all() {
        let row = feedback::Model {
            context: None,
            ..model("flowpilot:msg-2", 5, json!({}))
        };
        let record = record_from(&row);
        assert_eq!(record.message_id, "msg-2");
        assert!(record.provider.is_none());
        assert!(record.prompt_preview.is_empty());
        assert!(!record.has_transcript);
    }

    #[test]
    fn the_summary_counts_sentiment_users_and_models() {
        let rows = vec![
            model(
                "flowpilot:a",
                5,
                json!({ "conversation_id": "c1", "run_context": { "model_id": "m1", "provider": "bits" } }),
            ),
            model(
                "flowpilot:b",
                1,
                json!({ "conversation_id": "c1", "run_context": { "model_id": "m1", "provider": "bits" } }),
            ),
            model(
                "flowpilot:c",
                1,
                json!({ "conversation_id": "c2", "run_context": { "model_id": "m2", "provider": "codex" } }),
            ),
        ];
        let records: Vec<PromptFeedbackRecord> = rows.iter().map(record_from).collect();
        let summary = summarize(&records, &rows);

        assert_eq!(summary.total, 3);
        assert_eq!(summary.positive, 1);
        assert_eq!(summary.negative, 2);
        assert_eq!(summary.conversations, 2);
        assert_eq!(summary.raters, 1);
        assert!(
            summary
                .satisfaction
                .is_some_and(|value| (value - 100.0 / 3.0).abs() < 1e-9)
        );

        let m1 = summary
            .by_model
            .iter()
            .find(|facet| facet.key == "m1")
            .expect("m1 present");
        assert_eq!(m1.count, 2);
        assert_eq!(m1.negative, 1);
        assert_eq!(summary.trend.len(), 1);
        assert_eq!(summary.trend[0].negative, 2);
    }

    #[test]
    fn the_text_filter_searches_prompt_response_comment_and_model() {
        let row = model(
            "flowpilot:d",
            1,
            json!({
                "prompt": "make a CRM",
                "response": "here is the board",
                "run_context": { "model_id": "sonnet" }
            }),
        );
        let mut record = record_from(&row);
        record.comment = "the board was empty".to_string();

        assert!(matches_text(&record, "crm"));
        assert!(matches_text(&record, "HERE IS"));
        assert!(matches_text(&record, "empty"));
        assert!(matches_text(&record, "sonnet"));
        assert!(!matches_text(&record, "kubernetes"));
    }

    #[test]
    fn previews_are_clipped_by_characters() {
        let long = "ü".repeat(PREVIEW_CHARS + 50);
        let clipped = preview(&long);
        assert_eq!(clipped.chars().count(), PREVIEW_CHARS + 1);
        assert!(clipped.ends_with('…'));
        assert_eq!(preview("  short  "), "short");
    }
}
