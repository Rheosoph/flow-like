//! Admin view over captured FlowScript applies that failed, were blocked, or applied with warnings.
//!
//! Unlike the rest of this module the rows are user-attributed board content, not anonymous
//! counters — see `crate::routes::flowscript` for what that buys and what it costs. Both handlers
//! are therefore behind `GlobalPermission::Admin`, the same bar as the cross-tenant issue and trace
//! explorers.
//!
//! Every filter is a column, so the listing pushes its window, facets and pagination into SQL
//! instead of scanning: the sources are up to 64 KB each and folding a month of them in memory
//! would be the one expensive read on this page. The list query never selects the source column;
//! only the detail handler returns it.

use crate::entity::{flow_script_apply_failure, user};
use crate::error::ApiError;
use crate::middleware::jwt::AppUser;
use crate::permission::global_permission::GlobalPermission;
use crate::routes::user::identity::escape_like_pattern;
use crate::state::AppState;
use axum::extract::{Path, Query, State};
use axum::{Extension, Json};
use chrono::{Duration, NaiveDateTime, Utc};
use sea_orm::sea_query::{Alias, Expr, LikeExpr, extension::postgres::PgExpr};
use sea_orm::{
    ColumnTrait, Condition, EntityTrait, FromQueryResult, PaginatorTrait, QueryFilter, QueryOrder,
    QuerySelect, Select,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use utoipa::{IntoParams, ToSchema};

const DEFAULT_HOURS: i64 = 24 * 30;
const MAX_HOURS: i64 = 24 * 90;
const DEFAULT_PAGE_SIZE: u64 = 25;
const MAX_PAGE_SIZE: u64 = 100;
/// Entries kept per breakdown, highest count first.
const TOP_LIST_LIMIT: u64 = 15;

#[derive(Debug, Deserialize, IntoParams)]
pub struct ListFlowScriptFailuresQuery {
    /// Lookback window in hours. Default 720 (30 days), clamped to 1..=2160.
    #[serde(default)]
    pub hours: Option<i64>,
    /// Filter by outcome: "error", "blocked", "partial" or "all" (default).
    #[serde(default)]
    pub outcome: Option<String>,
    /// Filter by where the apply ran: "desktop", "web" or "all" (default).
    #[serde(default)]
    pub source: Option<String>,
    /// Filter by who authored the source: "editor", "agent" or "all" (default).
    #[serde(default)]
    pub origin: Option<String>,
    /// Filter to one reporting user's attempts.
    #[serde(default)]
    pub user_id: Option<String>,
    /// Filter to one app.
    #[serde(default)]
    pub app_id: Option<String>,
    /// Case-insensitive substring match on the cause, the error and the redacted source.
    #[serde(default)]
    pub query: Option<String>,
    #[serde(default)]
    pub page: Option<u64>,
    /// Page size, capped at 100. Default 25.
    #[serde(default)]
    pub page_size: Option<u64>,
}

/// The list projection. Deliberately omits `flowscript`, which only the detail handler returns.
#[derive(Debug, FromQueryResult)]
struct FailureRow {
    id: String,
    user_id: Option<String>,
    app_id: String,
    board_id: String,
    layer_id: Option<String>,
    source: String,
    origin: String,
    outcome: String,
    cause: String,
    error_message: Option<String>,
    diagnostics: Option<serde_json::Value>,
    command_count: i32,
    allow_deletions: bool,
    flowscript_chars: i32,
    dropped_values: i32,
    redacted_literals: i32,
    truncated: bool,
    app_version: Option<String>,
    platform: Option<String>,
    trace_id: Option<String>,
    created_at: NaiveDateTime,
}

#[derive(Debug, FromQueryResult)]
struct FacetRow {
    key: Option<String>,
    count: i64,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct FlowScriptFailureRecord {
    pub id: String,
    pub user_id: Option<String>,
    /// Display name of the reporting user, resolved for the rows on this page.
    pub user_name: Option<String>,
    pub app_id: String,
    pub board_id: String,
    pub layer_id: Option<String>,
    pub source: String,
    pub origin: String,
    pub outcome: String,
    pub cause: String,
    pub error_message: Option<String>,
    pub diagnostics: Vec<String>,
    pub diagnostic_count: usize,
    pub command_count: i32,
    pub allow_deletions: bool,
    pub flowscript_chars: i32,
    /// Declared values dropped and long literals generalized before storage.
    pub dropped_values: i32,
    pub redacted_literals: i32,
    pub truncated: bool,
    pub app_version: Option<String>,
    pub platform: Option<String>,
    /// Trace the apply belonged to, when it carried one. Opens in the trace explorer.
    pub trace_id: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct FlowScriptFailureFacet {
    pub key: String,
    /// Present for user facets, where the id alone is unreadable.
    pub label: Option<String>,
    pub count: i64,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct FlowScriptFailureSummary {
    pub total: i64,
    pub errors: i64,
    pub blocked: i64,
    pub partial: i64,
    pub users: i64,
    pub apps: i64,
    pub by_outcome: Vec<FlowScriptFailureFacet>,
    pub by_source: Vec<FlowScriptFailureFacet>,
    /// Editor edits versus FlowPilot's own applies.
    pub by_origin: Vec<FlowScriptFailureFacet>,
    /// The causes worth fixing first.
    pub by_cause: Vec<FlowScriptFailureFacet>,
    /// Who is hitting this, which is the whole reason the rows are attributed.
    pub by_user: Vec<FlowScriptFailureFacet>,
    pub by_app: Vec<FlowScriptFailureFacet>,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ListFlowScriptFailuresResponse {
    pub items: Vec<FlowScriptFailureRecord>,
    pub total: u64,
    pub page: u64,
    pub page_size: u64,
    pub hours: i64,
    /// Computed over the filtered set, so the tiles always describe what is on screen.
    pub summary: FlowScriptFailureSummary,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct FlowScriptFailureDetailResponse {
    pub record: FlowScriptFailureRecord,
    /// The redacted FlowScript the user submitted: declared values dropped, long literals
    /// generalized, line numbers preserved so a diagnostic's line still lines up.
    pub flowscript: String,
    pub corrections: Vec<String>,
}

fn string_list(value: Option<&serde_json::Value>) -> Vec<String> {
    value
        .and_then(|v| v.as_array())
        .map(|entries| {
            entries
                .iter()
                .filter_map(|entry| entry.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

fn record_from(row: FailureRow, names: &HashMap<String, String>) -> FlowScriptFailureRecord {
    let diagnostics = string_list(row.diagnostics.as_ref());
    FlowScriptFailureRecord {
        user_name: row
            .user_id
            .as_ref()
            .and_then(|id| names.get(id))
            .map(String::to_string),
        user_id: row.user_id,
        id: row.id,
        app_id: row.app_id,
        board_id: row.board_id,
        layer_id: row.layer_id,
        source: row.source,
        origin: row.origin,
        outcome: row.outcome,
        cause: row.cause,
        error_message: row.error_message,
        diagnostic_count: diagnostics.len(),
        diagnostics,
        command_count: row.command_count,
        allow_deletions: row.allow_deletions,
        flowscript_chars: row.flowscript_chars,
        dropped_values: row.dropped_values,
        redacted_literals: row.redacted_literals,
        truncated: row.truncated,
        app_version: row.app_version,
        platform: row.platform,
        trace_id: row.trace_id,
        created_at: row.created_at.and_utc().to_rfc3339(),
    }
}

/// The filtered window every read on this page shares.
fn filtered(
    q: &ListFlowScriptFailuresQuery,
    cutoff: NaiveDateTime,
) -> Select<flow_script_apply_failure::Entity> {
    use flow_script_apply_failure::Column;

    let mut select =
        flow_script_apply_failure::Entity::find().filter(Column::CreatedAt.gte(cutoff));

    if let Some(outcome) = q
        .outcome
        .as_deref()
        .filter(|v| !v.is_empty() && *v != "all")
    {
        select = select.filter(Column::Outcome.eq(outcome));
    }
    if let Some(source) = q.source.as_deref().filter(|v| !v.is_empty() && *v != "all") {
        select = select.filter(Column::Source.eq(source));
    }
    if let Some(origin) = q.origin.as_deref().filter(|v| !v.is_empty() && *v != "all") {
        select = select.filter(Column::Origin.eq(origin));
    }
    if let Some(user_id) = q.user_id.as_deref().filter(|v| !v.is_empty()) {
        select = select.filter(Column::UserId.eq(user_id));
    }
    if let Some(app_id) = q.app_id.as_deref().filter(|v| !v.is_empty()) {
        select = select.filter(Column::AppId.eq(app_id));
    }
    if let Some(query) = q.query.as_deref().map(str::trim).filter(|v| !v.is_empty()) {
        // Escaped, so a `%` typed into the search box matches a literal `%` instead of turning the
        // filter into a full scan of the source column.
        let pattern = format!("%{}%", escape_like_pattern(query));
        let contains =
            |column: Column| Expr::col(column).ilike(LikeExpr::new(pattern.clone()).escape('\\'));
        select = select.filter(
            Condition::any()
                .add(contains(Column::Cause))
                .add(contains(Column::ErrorMessage))
                .add(contains(Column::Flowscript)),
        );
    }
    select
}

/// One grouped breakdown, ordered by count. `labels` names the ids that are not human readable.
async fn facet(
    state: &AppState,
    select: Select<flow_script_apply_failure::Entity>,
    column: flow_script_apply_failure::Column,
    labels: Option<&HashMap<String, String>>,
) -> Result<Vec<FlowScriptFailureFacet>, ApiError> {
    let rows = select
        .select_only()
        .column_as(column, "key")
        .column_as(flow_script_apply_failure::Column::Id.count(), "count")
        .group_by(column)
        .order_by_desc(Expr::col(Alias::new("count")))
        .limit(TOP_LIST_LIMIT)
        .into_model::<FacetRow>()
        .all(&state.db)
        .await?;

    Ok(rows
        .into_iter()
        .filter_map(|row| {
            let key = row.key?;
            Some(FlowScriptFailureFacet {
                label: labels.and_then(|l| l.get(&key)).map(String::to_string),
                key,
                count: row.count,
            })
        })
        .collect())
}

/// Display names for the users on this page and in the user breakdown.
async fn user_names(
    state: &AppState,
    ids: Vec<String>,
) -> Result<HashMap<String, String>, ApiError> {
    if ids.is_empty() {
        return Ok(HashMap::new());
    }
    let rows = user::Entity::find()
        .filter(user::Column::Id.is_in(ids))
        .all(&state.db)
        .await?;
    Ok(rows
        .into_iter()
        .filter_map(|row| {
            let name = row.name.filter(|n| !n.trim().is_empty())?;
            Some((row.id, name))
        })
        .collect())
}

#[utoipa::path(
    get,
    path = "/admin/telemetry/flowscript-failures",
    tag = "admin",
    description = "List FlowScript applies that failed, were blocked, or applied with warnings, with the causes, the people who hit them and the apps they happened in. Sources are stored redacted; this listing never returns them.",
    params(ListFlowScriptFailuresQuery),
    responses(
        (status = 200, description = "Captured FlowScript apply failures for the window", body = ListFlowScriptFailuresResponse),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Platform admin permission required")
    ),
    security(("bearer_auth" = []), ("api_key" = []), ("pat" = []))
)]
#[tracing::instrument(name = "GET /admin/telemetry/flowscript-failures", skip(state, user))]
pub async fn list_flowscript_failures(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Query(q): Query<ListFlowScriptFailuresQuery>,
) -> Result<Json<ListFlowScriptFailuresResponse>, ApiError> {
    user.check_global_permission(&state, GlobalPermission::Admin)
        .await?;

    use flow_script_apply_failure::Column;

    let hours = q.hours.unwrap_or(DEFAULT_HOURS).clamp(1, MAX_HOURS);
    let page = q.page.unwrap_or(0);
    let page_size = q
        .page_size
        .unwrap_or(DEFAULT_PAGE_SIZE)
        .clamp(1, MAX_PAGE_SIZE);
    let cutoff = Utc::now().naive_utc() - Duration::hours(hours);

    let total = filtered(&q, cutoff).count(&state.db).await?;

    let rows = filtered(&q, cutoff)
        .select_only()
        .column_as(Column::Id, "id")
        .column_as(Column::UserId, "user_id")
        .column_as(Column::AppId, "app_id")
        .column_as(Column::BoardId, "board_id")
        .column_as(Column::LayerId, "layer_id")
        .column_as(Column::Source, "source")
        .column_as(Column::Origin, "origin")
        .column_as(Column::Outcome, "outcome")
        .column_as(Column::Cause, "cause")
        .column_as(Column::ErrorMessage, "error_message")
        .column_as(Column::Diagnostics, "diagnostics")
        .column_as(Column::CommandCount, "command_count")
        .column_as(Column::AllowDeletions, "allow_deletions")
        .column_as(Column::FlowscriptChars, "flowscript_chars")
        .column_as(Column::DroppedValues, "dropped_values")
        .column_as(Column::RedactedLiterals, "redacted_literals")
        .column_as(Column::Truncated, "truncated")
        .column_as(Column::AppVersion, "app_version")
        .column_as(Column::Platform, "platform")
        .column_as(Column::TraceId, "trace_id")
        .column_as(Column::CreatedAt, "created_at")
        .order_by_desc(Column::CreatedAt)
        .limit(page_size)
        .offset(page * page_size)
        .into_model::<FailureRow>()
        .all(&state.db)
        .await?;

    let by_outcome = facet(&state, filtered(&q, cutoff), Column::Outcome, None).await?;
    let by_source = facet(&state, filtered(&q, cutoff), Column::Source, None).await?;
    let by_origin = facet(&state, filtered(&q, cutoff), Column::Origin, None).await?;
    let by_cause = facet(&state, filtered(&q, cutoff), Column::Cause, None).await?;
    let by_app = facet(&state, filtered(&q, cutoff), Column::AppId, None).await?;
    let unnamed_users = facet(&state, filtered(&q, cutoff), Column::UserId, None).await?;

    let mut lookup: Vec<String> = rows.iter().filter_map(|row| row.user_id.clone()).collect();
    lookup.extend(unnamed_users.iter().map(|facet| facet.key.clone()));
    lookup.sort();
    lookup.dedup();
    let names = user_names(&state, lookup).await?;

    let by_user = unnamed_users
        .into_iter()
        .map(|facet| FlowScriptFailureFacet {
            label: names.get(&facet.key).map(String::to_string),
            ..facet
        })
        .collect();

    let users = filtered(&q, cutoff)
        .select_only()
        .column(Column::UserId)
        .group_by(Column::UserId)
        .count(&state.db)
        .await? as i64;
    let apps = filtered(&q, cutoff)
        .select_only()
        .column(Column::AppId)
        .group_by(Column::AppId)
        .count(&state.db)
        .await? as i64;

    let outcome_total = |name: &str| {
        by_outcome
            .iter()
            .find(|facet| facet.key == name)
            .map(|facet| facet.count)
            .unwrap_or(0)
    };

    let summary = FlowScriptFailureSummary {
        total: total as i64,
        errors: outcome_total(crate::routes::flowscript::OUTCOME_ERROR),
        blocked: outcome_total(crate::routes::flowscript::OUTCOME_BLOCKED),
        partial: outcome_total(crate::routes::flowscript::OUTCOME_PARTIAL),
        users,
        apps,
        by_outcome,
        by_source,
        by_origin,
        by_cause,
        by_user,
        by_app,
    };

    Ok(Json(ListFlowScriptFailuresResponse {
        items: rows
            .into_iter()
            .map(|row| record_from(row, &names))
            .collect(),
        total,
        page,
        page_size,
        hours,
        summary,
    }))
}

#[utoipa::path(
    get,
    path = "/admin/telemetry/flowscript-failures/{failure_id}",
    tag = "admin",
    description = "Read one captured FlowScript apply failure, including the redacted source the user submitted and every diagnostic it produced.",
    params(("failure_id" = String, Path, description = "Captured failure identifier")),
    responses(
        (status = 200, description = "The captured failure", body = FlowScriptFailureDetailResponse),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Platform admin permission required"),
        (status = 404, description = "No such captured failure")
    ),
    security(("bearer_auth" = []), ("api_key" = []), ("pat" = []))
)]
#[tracing::instrument(
    name = "GET /admin/telemetry/flowscript-failures/{failure_id}",
    skip(state, user)
)]
pub async fn get_flowscript_failure(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(failure_id): Path<String>,
) -> Result<Json<FlowScriptFailureDetailResponse>, ApiError> {
    user.check_global_permission(&state, GlobalPermission::Admin)
        .await?;

    let model = flow_script_apply_failure::Entity::find_by_id(failure_id)
        .one(&state.db)
        .await?
        .ok_or(ApiError::NOT_FOUND)?;

    let names = user_names(&state, model.user_id.clone().into_iter().collect()).await?;
    let corrections = string_list(model.corrections.as_ref());
    let flowscript = model.flowscript.clone();

    let row = FailureRow {
        id: model.id,
        user_id: model.user_id,
        app_id: model.app_id,
        board_id: model.board_id,
        layer_id: model.layer_id,
        source: model.source,
        origin: model.origin,
        outcome: model.outcome,
        cause: model.cause,
        error_message: model.error_message,
        diagnostics: model.diagnostics,
        command_count: model.command_count,
        allow_deletions: model.allow_deletions,
        flowscript_chars: model.flowscript_chars,
        dropped_values: model.dropped_values,
        redacted_literals: model.redacted_literals,
        truncated: model.truncated,
        app_version: model.app_version,
        platform: model.platform,
        trace_id: model.trace_id,
        created_at: model.created_at,
    };

    Ok(Json(FlowScriptFailureDetailResponse {
        record: record_from(row, &names),
        flowscript,
        corrections,
    }))
}
