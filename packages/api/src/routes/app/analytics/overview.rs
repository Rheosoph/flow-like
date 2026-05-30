use crate::{
    ensure_permission,
    entity::{
        app_analytics_daily, embedding_usage_tracking, execution_usage_tracking, feedback,
        llm_usage_tracking,
    },
    error::ApiError,
    middleware::jwt::AppUser,
    permission::role_permission::RolePermissions,
    state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, Query, State},
};
use chrono::{Duration, NaiveDate, Utc};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QueryOrder, QuerySelect};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use super::update_aggregations::ensure_aggregations_current;

#[derive(Debug, Deserialize, ToSchema)]
pub struct AnalyticsStatsQuery {
    /// Start date (YYYY-MM-DD)
    pub start_date: Option<String>,
    /// End date (YYYY-MM-DD)
    pub end_date: Option<String>,
    /// Aggregation period: "day", "week", "month"
    #[serde(default = "default_period")]
    pub period: String,
}

fn default_period() -> String {
    "day".to_string()
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct AnalyticsOverview {
    /// Total executions (all time)
    pub total_executions: i64,
    /// Successful executions (all time)
    pub successful_executions: i64,
    /// Failed executions (all time)
    pub failed_executions: i64,
    /// Total unique users (all time)
    pub unique_users: i64,
    /// Average feedback rating
    pub avg_feedback_rating: Option<f64>,
    /// Total feedback entries
    pub total_feedback: i64,
    /// Positive feedback count
    pub positive_feedback: i64,
    /// Negative feedback count
    pub negative_feedback: i64,
    /// Total LLM cost (micro-dollars)
    pub total_llm_cost: i64,
    /// Total embedding cost (micro-dollars)
    pub total_embedding_cost: i64,
    /// Average latency (ms)
    pub avg_latency_ms: Option<f64>,
    /// Executions in the current period
    pub period_executions: i64,
    /// Unique users in the current period
    pub period_unique_users: i64,
    /// Execution change vs previous period (%)
    pub executions_change_percent: Option<f64>,
    /// User change vs previous period (%)
    pub users_change_percent: Option<f64>,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct DailyAnalyticsStat {
    pub date: String,
    pub executions: i64,
    pub successful_executions: i64,
    pub failed_executions: i64,
    pub unique_users: i64,
    pub feedback_count: i64,
    pub avg_rating: Option<f64>,
    pub llm_cost: i64,
    pub embedding_cost: i64,
    pub avg_latency: Option<f64>,
    pub p95_latency: Option<f64>,
    pub positive_feedback: i64,
    pub negative_feedback: i64,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct AnalyticsStats {
    pub daily_stats: Vec<DailyAnalyticsStat>,
    pub summary: AnalyticsOverview,
}

/// GET /apps/{app_id}/analytics - Analytics overview
#[utoipa::path(
    get,
    path = "/apps/{app_id}/analytics",
    tag = "analytics",
    description = "Get analytics overview for an app.",
    params(
        ("app_id" = String, Path, description = "Application ID")
    ),
    responses(
        (status = 200, description = "Analytics overview", body = AnalyticsOverview),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "Not found")
    ),
    security(
        ("bearer_auth" = []),
        ("api_key" = []),
        ("pat" = [])
    )
)]
#[tracing::instrument(name = "GET /apps/{app_id}/analytics", skip(state, user))]
pub async fn get_analytics_overview(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(app_id): Path<String>,
) -> Result<Json<AnalyticsOverview>, ApiError> {
    ensure_permission!(user, &app_id, &state, RolePermissions::ReadAnalytics);

    // Auto-backfill any missing days through yesterday
    ensure_aggregations_current(&state, &app_id).await?;

    let now = Utc::now().date_naive();
    let thirty_days_ago = now - Duration::days(29);
    let sixty_days_ago = now - Duration::days(59);

    let all_daily = app_analytics_daily::Entity::find()
        .filter(app_analytics_daily::Column::AppId.eq(&app_id))
        .all(&state.db)
        .await?;

    // Compute today's live stats from raw tables
    let today_stat = compute_today_live(&state, &app_id).await?;

    if all_daily.is_empty() && today_stat.is_none() {
        return Ok(Json(
            compute_overview_from_raw(&state, &app_id, thirty_days_ago, sixty_days_ago).await?,
        ));
    }

    let total_executions: i64 = all_daily.iter().map(|d| d.total_executions).sum::<i64>()
        + today_stat.as_ref().map_or(0, |t| t.executions);
    let successful_executions: i64 = all_daily
        .iter()
        .map(|d| d.successful_executions)
        .sum::<i64>()
        + today_stat
            .as_ref()
            .map_or(0, |t| t.executions - t.failed_executions);
    let failed_executions: i64 = all_daily.iter().map(|d| d.failed_executions).sum::<i64>()
        + today_stat.as_ref().map_or(0, |t| t.failed_executions);
    let unique_users = count_unique_users(&state, &app_id, None, None).await?;
    let total_feedback: i64 = all_daily.iter().map(|d| d.feedback_count).sum::<i64>()
        + today_stat.as_ref().map_or(0, |t| t.feedback_count);
    let positive_feedback: i64 = all_daily.iter().map(|d| d.positive_feedback).sum::<i64>()
        + today_stat.as_ref().map_or(0, |t| t.positive_feedback);
    let negative_feedback: i64 = all_daily.iter().map(|d| d.negative_feedback).sum::<i64>()
        + today_stat.as_ref().map_or(0, |t| t.negative_feedback);
    let total_llm_cost: i64 = all_daily.iter().map(|d| d.total_llm_cost).sum::<i64>()
        + today_stat.as_ref().map_or(0, |t| t.llm_cost);
    let total_embedding_cost: i64 = all_daily
        .iter()
        .map(|d| d.total_embedding_cost)
        .sum::<i64>()
        + today_stat.as_ref().map_or(0, |t| t.embedding_cost);

    let avg_feedback_rating = {
        let mut sum: f64 = all_daily
            .iter()
            .filter_map(|d| {
                d.avg_feedback_rating
                    .map(|rating| rating * d.feedback_count as f64)
            })
            .sum();
        let mut count: i64 = all_daily.iter().map(|d| d.feedback_count).sum();
        if let Some(ref t) = today_stat
            && let Some(r) = t.avg_rating
        {
            sum += r * t.feedback_count as f64;
            count += t.feedback_count;
        }
        if count == 0 {
            None
        } else {
            Some(sum / count as f64)
        }
    };

    let avg_latency_ms = {
        let mut sum: f64 = all_daily
            .iter()
            .filter_map(|d| {
                d.avg_latency_ms
                    .map(|latency| latency * d.total_executions as f64)
            })
            .sum();
        let mut count: i64 = all_daily
            .iter()
            .filter(|d| d.avg_latency_ms.is_some())
            .map(|d| d.total_executions)
            .sum();
        if let Some(ref t) = today_stat
            && let Some(l) = t.avg_latency
        {
            sum += l * t.executions as f64;
            count += t.executions;
        }
        if count == 0 {
            None
        } else {
            Some(sum / count as f64)
        }
    };

    let current_period: Vec<_> = all_daily
        .iter()
        .filter(|d| d.date >= thirty_days_ago)
        .collect();
    let period_executions: i64 = current_period
        .iter()
        .map(|d| d.total_executions)
        .sum::<i64>()
        + today_stat.as_ref().map_or(0, |t| t.executions);
    let period_unique_users =
        count_unique_users(&state, &app_id, Some(thirty_days_ago), None).await?;

    let prev_period: Vec<_> = all_daily
        .iter()
        .filter(|d| d.date >= sixty_days_ago && d.date < thirty_days_ago)
        .collect();
    let prev_executions: i64 = prev_period.iter().map(|d| d.total_executions).sum();
    let prev_users =
        count_unique_users(&state, &app_id, Some(sixty_days_ago), Some(thirty_days_ago)).await?;

    let executions_change_percent = compute_change_percent(period_executions, prev_executions);
    let users_change_percent = compute_change_percent(period_unique_users, prev_users);

    Ok(Json(AnalyticsOverview {
        total_executions,
        successful_executions,
        failed_executions,
        unique_users,
        avg_feedback_rating,
        total_feedback,
        positive_feedback,
        negative_feedback,
        total_llm_cost,
        total_embedding_cost,
        avg_latency_ms,
        period_executions,
        period_unique_users,
        executions_change_percent,
        users_change_percent,
    }))
}

/// GET /apps/{app_id}/analytics/stats - Detailed analytics with daily breakdown
#[utoipa::path(
    get,
    path = "/apps/{app_id}/analytics/stats",
    tag = "analytics",
    description = "Get analytics statistics with daily breakdown.",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        ("start_date" = Option<String>, Query, description = "Start date (YYYY-MM-DD)"),
        ("end_date" = Option<String>, Query, description = "End date (YYYY-MM-DD)"),
        ("period" = String, Query, description = "Aggregation period: day, week, month")
    ),
    responses(
        (status = 200, description = "Analytics stats", body = AnalyticsStats),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "Not found")
    ),
    security(
        ("bearer_auth" = []),
        ("api_key" = []),
        ("pat" = [])
    )
)]
#[tracing::instrument(name = "GET /apps/{app_id}/analytics/stats", skip(state, user))]
pub async fn get_analytics_stats(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(app_id): Path<String>,
    Query(query): Query<AnalyticsStatsQuery>,
) -> Result<Json<AnalyticsStats>, ApiError> {
    ensure_permission!(user, &app_id, &state, RolePermissions::ReadAnalytics);

    // Auto-backfill any missing days through yesterday
    ensure_aggregations_current(&state, &app_id).await?;

    let today = Utc::now().date_naive();

    let end_date = query
        .end_date
        .as_ref()
        .and_then(|d| NaiveDate::parse_from_str(d, "%Y-%m-%d").ok())
        .unwrap_or(today);

    let start_date = query
        .start_date
        .as_ref()
        .and_then(|d| NaiveDate::parse_from_str(d, "%Y-%m-%d").ok())
        .unwrap_or_else(|| end_date - Duration::days(29));

    let daily_aggregates = app_analytics_daily::Entity::find()
        .filter(app_analytics_daily::Column::AppId.eq(&app_id))
        .filter(app_analytics_daily::Column::Date.gte(start_date))
        .filter(app_analytics_daily::Column::Date.lte(end_date))
        .order_by_asc(app_analytics_daily::Column::Date)
        .all(&state.db)
        .await?;

    let mut daily_stats: Vec<DailyAnalyticsStat> = if daily_aggregates.is_empty() {
        compute_daily_stats_from_raw(&state, &app_id, start_date, end_date).await?
    } else {
        daily_aggregates
            .into_iter()
            .map(|d| DailyAnalyticsStat {
                date: d.date.format("%Y-%m-%d").to_string(),
                executions: d.total_executions,
                successful_executions: d.successful_executions,
                failed_executions: d.failed_executions,
                unique_users: d.unique_users,
                feedback_count: d.feedback_count,
                avg_rating: d.avg_feedback_rating,
                llm_cost: d.total_llm_cost,
                embedding_cost: d.total_embedding_cost,
                avg_latency: d.avg_latency_ms,
                p95_latency: d.p95_latency_ms,
                positive_feedback: d.positive_feedback,
                negative_feedback: d.negative_feedback,
            })
            .collect()
    };

    // Append today's live data if the requested range includes today
    if start_date <= today
        && end_date >= today
        && let Some(live) = compute_today_live(&state, &app_id).await?
    {
        daily_stats.push(live.to_daily_stat());
    }

    let total_executions: i64 = daily_stats.iter().map(|d| d.executions).sum();
    let successful_executions: i64 = daily_stats.iter().map(|d| d.successful_executions).sum();
    let failed_executions: i64 = daily_stats.iter().map(|d| d.failed_executions).sum();
    let unique_users = count_unique_users(
        &state,
        &app_id,
        Some(start_date),
        Some(end_date + Duration::days(1)),
    )
    .await?;
    let total_feedback: i64 = daily_stats.iter().map(|d| d.feedback_count).sum();
    let positive_feedback: i64 = daily_stats.iter().map(|d| d.positive_feedback).sum();
    let negative_feedback: i64 = daily_stats.iter().map(|d| d.negative_feedback).sum();
    let total_llm_cost: i64 = daily_stats.iter().map(|d| d.llm_cost).sum();
    let total_embedding_cost: i64 = daily_stats.iter().map(|d| d.embedding_cost).sum();

    let avg_feedback_rating = weighted_average_by_count(
        daily_stats
            .iter()
            .filter_map(|d| d.avg_rating.map(|rating| (rating, d.feedback_count))),
    );

    let avg_latency_ms = weighted_average_by_count(
        daily_stats
            .iter()
            .filter_map(|d| d.avg_latency.map(|latency| (latency, d.executions))),
    );

    Ok(Json(AnalyticsStats {
        daily_stats,
        summary: AnalyticsOverview {
            total_executions,
            successful_executions,
            failed_executions,
            unique_users,
            avg_feedback_rating,
            total_feedback,
            positive_feedback,
            negative_feedback,
            total_llm_cost,
            total_embedding_cost,
            avg_latency_ms,
            period_executions: total_executions,
            period_unique_users: unique_users,
            executions_change_percent: None,
            users_change_percent: None,
        },
    }))
}

async fn count_unique_users(
    state: &AppState,
    app_id: &str,
    start_date: Option<NaiveDate>,
    end_date_exclusive: Option<NaiveDate>,
) -> Result<i64, ApiError> {
    let mut query = execution_usage_tracking::Entity::find()
        .filter(execution_usage_tracking::Column::AppId.eq(app_id))
        .filter(execution_usage_tracking::Column::UserId.is_not_null())
        .select_only()
        .column(execution_usage_tracking::Column::UserId)
        .distinct();

    if let Some(start_date) = start_date {
        query = query.filter(
            execution_usage_tracking::Column::CreatedAt
                .gte(start_date.and_hms_opt(0, 0, 0).unwrap()),
        );
    }

    if let Some(end_date_exclusive) = end_date_exclusive {
        query = query.filter(
            execution_usage_tracking::Column::CreatedAt
                .lt(end_date_exclusive.and_hms_opt(0, 0, 0).unwrap()),
        );
    }

    let users: Vec<Option<String>> = query.into_tuple().all(&state.db).await?;
    Ok(users.into_iter().flatten().count() as i64)
}

fn weighted_average_by_count(values: impl Iterator<Item = (f64, i64)>) -> Option<f64> {
    let mut weighted_sum = 0.0;
    let mut count = 0;

    for (value, value_count) in values {
        if value_count <= 0 {
            continue;
        }
        weighted_sum += value * value_count as f64;
        count += value_count;
    }

    if count == 0 {
        None
    } else {
        Some(weighted_sum / count as f64)
    }
}

fn latency_stats_from_microseconds(
    latencies_us: impl Iterator<Item = i64>,
) -> (Option<f64>, Option<f64>) {
    let mut latencies_us: Vec<i64> = latencies_us.collect();
    if latencies_us.is_empty() {
        return (None, None);
    }

    let avg_latency_ms =
        latencies_us.iter().sum::<i64>() as f64 / latencies_us.len() as f64 / 1000.0;

    latencies_us.sort();
    let idx = ((latencies_us.len() as f64) * 0.95).ceil() as usize;
    let idx = idx.min(latencies_us.len()) - 1;
    let p95_latency_ms = latencies_us[idx] as f64 / 1000.0;

    (Some(avg_latency_ms), Some(p95_latency_ms))
}

fn compute_change_percent(current: i64, previous: i64) -> Option<f64> {
    if previous > 0 {
        Some(((current - previous) as f64 / previous as f64) * 100.0)
    } else if current > 0 {
        Some(100.0)
    } else {
        None
    }
}

async fn compute_overview_from_raw(
    state: &AppState,
    app_id: &str,
    thirty_days_ago: NaiveDate,
    sixty_days_ago: NaiveDate,
) -> Result<AnalyticsOverview, ApiError> {
    use crate::entity::sea_orm_active_enums::ExecutionStatus;
    use std::collections::HashSet;

    let executions = execution_usage_tracking::Entity::find()
        .filter(execution_usage_tracking::Column::AppId.eq(app_id))
        .all(&state.db)
        .await?;

    let total_executions = executions.len() as i64;
    let failed_executions = executions
        .iter()
        .filter(|e| matches!(e.status, ExecutionStatus::Error | ExecutionStatus::Fatal))
        .count() as i64;
    let successful_executions = total_executions - failed_executions;

    let all_user_ids: HashSet<_> = executions
        .iter()
        .filter_map(|e| e.user_id.as_ref())
        .collect();
    let unique_users = all_user_ids.len() as i64;

    let feedbacks = feedback::Entity::find()
        .filter(feedback::Column::AppId.eq(app_id))
        .all(&state.db)
        .await?;

    let total_feedback = feedbacks.len() as i64;
    let positive_feedback = feedbacks.iter().filter(|f| f.rating > 0).count() as i64;
    let negative_feedback = feedbacks.iter().filter(|f| f.rating < 0).count() as i64;
    let avg_feedback_rating = if feedbacks.is_empty() {
        None
    } else {
        Some(feedbacks.iter().map(|f| f.rating as f64).sum::<f64>() / feedbacks.len() as f64)
    };

    let llm_records = llm_usage_tracking::Entity::find()
        .filter(llm_usage_tracking::Column::AppId.eq(app_id))
        .all(&state.db)
        .await?;
    let total_llm_cost: i64 = llm_records.iter().map(|r| r.price).sum();
    let (avg_latency_ms, _) =
        latency_stats_from_microseconds(executions.iter().map(|e| e.microseconds));

    let embedding_records = embedding_usage_tracking::Entity::find()
        .filter(embedding_usage_tracking::Column::AppId.eq(app_id))
        .all(&state.db)
        .await?;
    let total_embedding_cost: i64 = embedding_records.iter().map(|r| r.price).sum();

    let current_start = thirty_days_ago.and_hms_opt(0, 0, 0).unwrap();
    let period_execs: Vec<_> = executions
        .iter()
        .filter(|e| e.created_at >= current_start)
        .collect();
    let period_executions = period_execs.len() as i64;
    let period_user_ids: HashSet<_> = period_execs
        .iter()
        .filter_map(|e| e.user_id.as_ref())
        .collect();
    let period_unique_users = period_user_ids.len() as i64;

    let prev_start = sixty_days_ago.and_hms_opt(0, 0, 0).unwrap();
    let prev_execs: Vec<_> = executions
        .iter()
        .filter(|e| e.created_at >= prev_start && e.created_at < current_start)
        .collect();
    let prev_executions = prev_execs.len() as i64;
    let prev_user_ids: HashSet<_> = prev_execs
        .iter()
        .filter_map(|e| e.user_id.as_ref())
        .collect();
    let prev_users = prev_user_ids.len() as i64;

    Ok(AnalyticsOverview {
        total_executions,
        successful_executions,
        failed_executions,
        unique_users,
        avg_feedback_rating,
        total_feedback,
        positive_feedback,
        negative_feedback,
        total_llm_cost,
        total_embedding_cost,
        avg_latency_ms,
        period_executions,
        period_unique_users,
        executions_change_percent: compute_change_percent(period_executions, prev_executions),
        users_change_percent: compute_change_percent(period_unique_users, prev_users),
    })
}

async fn compute_daily_stats_from_raw(
    state: &AppState,
    app_id: &str,
    start_date: NaiveDate,
    end_date: NaiveDate,
) -> Result<Vec<DailyAnalyticsStat>, ApiError> {
    use crate::entity::sea_orm_active_enums::ExecutionStatus;
    use std::collections::HashMap;

    let start_dt = start_date.and_hms_opt(0, 0, 0).unwrap();
    let end_dt = end_date.and_hms_opt(23, 59, 59).unwrap();

    let executions = execution_usage_tracking::Entity::find()
        .filter(execution_usage_tracking::Column::AppId.eq(app_id))
        .filter(execution_usage_tracking::Column::CreatedAt.gte(start_dt))
        .filter(execution_usage_tracking::Column::CreatedAt.lte(end_dt))
        .all(&state.db)
        .await?;

    let feedbacks = feedback::Entity::find()
        .filter(feedback::Column::AppId.eq(app_id))
        .filter(feedback::Column::CreatedAt.gte(start_dt))
        .filter(feedback::Column::CreatedAt.lte(end_dt))
        .all(&state.db)
        .await?;

    let llm_records = llm_usage_tracking::Entity::find()
        .filter(llm_usage_tracking::Column::AppId.eq(app_id))
        .filter(llm_usage_tracking::Column::CreatedAt.gte(start_dt))
        .filter(llm_usage_tracking::Column::CreatedAt.lte(end_dt))
        .all(&state.db)
        .await?;

    let embedding_records = embedding_usage_tracking::Entity::find()
        .filter(embedding_usage_tracking::Column::AppId.eq(app_id))
        .filter(embedding_usage_tracking::Column::CreatedAt.gte(start_dt))
        .filter(embedding_usage_tracking::Column::CreatedAt.lte(end_dt))
        .all(&state.db)
        .await?;

    let mut exec_by_day: HashMap<NaiveDate, Vec<&execution_usage_tracking::Model>> = HashMap::new();
    for e in &executions {
        exec_by_day.entry(e.created_at.date()).or_default().push(e);
    }

    let mut feedback_by_day: HashMap<NaiveDate, Vec<&feedback::Model>> = HashMap::new();
    for f in &feedbacks {
        feedback_by_day
            .entry(f.created_at.date())
            .or_default()
            .push(f);
    }

    let mut llm_by_day: HashMap<NaiveDate, Vec<&llm_usage_tracking::Model>> = HashMap::new();
    for l in &llm_records {
        llm_by_day.entry(l.created_at.date()).or_default().push(l);
    }

    let mut embedding_by_day: HashMap<NaiveDate, Vec<&embedding_usage_tracking::Model>> =
        HashMap::new();
    for e in &embedding_records {
        embedding_by_day
            .entry(e.created_at.date())
            .or_default()
            .push(e);
    }

    let mut stats = Vec::new();
    let mut current = start_date;
    while current <= end_date {
        let day_execs = exec_by_day
            .get(&current)
            .map(|v| v.as_slice())
            .unwrap_or(&[]);
        let day_feedback = feedback_by_day
            .get(&current)
            .map(|v| v.as_slice())
            .unwrap_or(&[]);
        let day_llm = llm_by_day
            .get(&current)
            .map(|v| v.as_slice())
            .unwrap_or(&[]);
        let day_embeddings = embedding_by_day
            .get(&current)
            .map(|v| v.as_slice())
            .unwrap_or(&[]);

        let user_ids: std::collections::HashSet<_> = day_execs
            .iter()
            .filter_map(|e| e.user_id.as_ref())
            .collect();

        let failed_executions = day_execs
            .iter()
            .filter(|e| matches!(e.status, ExecutionStatus::Error | ExecutionStatus::Fatal))
            .count() as i64;
        let successful_executions = day_execs.len() as i64 - failed_executions;
        let (avg_latency, p95_latency) =
            latency_stats_from_microseconds(day_execs.iter().map(|e| e.microseconds));

        let avg_rating = if day_feedback.is_empty() {
            None
        } else {
            Some(
                day_feedback.iter().map(|f| f.rating as f64).sum::<f64>()
                    / day_feedback.len() as f64,
            )
        };

        stats.push(DailyAnalyticsStat {
            date: current.format("%Y-%m-%d").to_string(),
            executions: day_execs.len() as i64,
            successful_executions,
            failed_executions,
            unique_users: user_ids.len() as i64,
            feedback_count: day_feedback.len() as i64,
            avg_rating,
            llm_cost: day_llm.iter().map(|l| l.price).sum(),
            embedding_cost: day_embeddings.iter().map(|e| e.price).sum(),
            avg_latency,
            p95_latency,
            positive_feedback: day_feedback.iter().filter(|f| f.rating > 0).count() as i64,
            negative_feedback: day_feedback.iter().filter(|f| f.rating < 0).count() as i64,
        });

        current += Duration::days(1);
    }

    Ok(stats)
}

/// Internal representation of today's live data, richer than DailyAnalyticsStat.
struct TodayLiveData {
    executions: i64,
    failed_executions: i64,
    unique_users: i64,
    feedback_count: i64,
    positive_feedback: i64,
    negative_feedback: i64,
    avg_rating: Option<f64>,
    llm_cost: i64,
    embedding_cost: i64,
    avg_latency: Option<f64>,
    p95_latency: Option<f64>,
}

/// Compute today's analytics from raw tracking tables (not yet aggregated).
/// Returns None if there is zero activity today.
async fn compute_today_live(
    state: &AppState,
    app_id: &str,
) -> Result<Option<TodayLiveData>, ApiError> {
    use crate::entity::sea_orm_active_enums::ExecutionStatus;
    use std::collections::HashSet;

    let today = Utc::now().date_naive();
    let start_of_day = today.and_hms_opt(0, 0, 0).unwrap();

    let executions = execution_usage_tracking::Entity::find()
        .filter(execution_usage_tracking::Column::AppId.eq(app_id))
        .filter(execution_usage_tracking::Column::CreatedAt.gte(start_of_day))
        .all(&state.db)
        .await?;

    let feedbacks = feedback::Entity::find()
        .filter(feedback::Column::AppId.eq(app_id))
        .filter(feedback::Column::CreatedAt.gte(start_of_day))
        .all(&state.db)
        .await?;

    let llm_records = llm_usage_tracking::Entity::find()
        .filter(llm_usage_tracking::Column::AppId.eq(app_id))
        .filter(llm_usage_tracking::Column::CreatedAt.gte(start_of_day))
        .all(&state.db)
        .await?;

    let embedding_records = embedding_usage_tracking::Entity::find()
        .filter(embedding_usage_tracking::Column::AppId.eq(app_id))
        .filter(embedding_usage_tracking::Column::CreatedAt.gte(start_of_day))
        .all(&state.db)
        .await?;

    if executions.is_empty()
        && feedbacks.is_empty()
        && llm_records.is_empty()
        && embedding_records.is_empty()
    {
        return Ok(None);
    }

    let total = executions.len() as i64;
    let failed = executions
        .iter()
        .filter(|e| matches!(e.status, ExecutionStatus::Error | ExecutionStatus::Fatal))
        .count() as i64;
    let user_ids: HashSet<_> = executions
        .iter()
        .filter_map(|e| e.user_id.as_ref())
        .collect();

    let (avg_latency, p95_latency) =
        latency_stats_from_microseconds(executions.iter().map(|e| e.microseconds));

    let avg_rating = if feedbacks.is_empty() {
        None
    } else {
        Some(feedbacks.iter().map(|f| f.rating as f64).sum::<f64>() / feedbacks.len() as f64)
    };

    Ok(Some(TodayLiveData {
        executions: total,
        failed_executions: failed,
        unique_users: user_ids.len() as i64,
        feedback_count: feedbacks.len() as i64,
        positive_feedback: feedbacks.iter().filter(|f| f.rating > 0).count() as i64,
        negative_feedback: feedbacks.iter().filter(|f| f.rating < 0).count() as i64,
        avg_rating,
        llm_cost: llm_records.iter().map(|r| r.price).sum(),
        embedding_cost: embedding_records.iter().map(|r| r.price).sum(),
        avg_latency,
        p95_latency,
    }))
}

impl TodayLiveData {
    fn to_daily_stat(&self) -> DailyAnalyticsStat {
        let today = Utc::now().date_naive();
        DailyAnalyticsStat {
            date: today.format("%Y-%m-%d").to_string(),
            executions: self.executions,
            successful_executions: self.executions - self.failed_executions,
            failed_executions: self.failed_executions,
            unique_users: self.unique_users,
            feedback_count: self.feedback_count,
            avg_rating: self.avg_rating,
            llm_cost: self.llm_cost,
            embedding_cost: self.embedding_cost,
            avg_latency: self.avg_latency,
            p95_latency: self.p95_latency,
            positive_feedback: self.positive_feedback,
            negative_feedback: self.negative_feedback,
        }
    }
}
