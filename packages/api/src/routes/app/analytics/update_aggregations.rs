use crate::utils::stats_period::StatsPeriod;
use crate::utils::time::utc_midnight;
use crate::{
    entity::{
        app_analytics_daily, embedding_usage_tracking, execution_usage_tracking, feedback,
        llm_usage_tracking,
    },
    error::ApiError,
    state::AppState,
};
use chrono::{Duration, NaiveDate, Utc};
use flow_like_types::{create_id, tokio::try_join};
use sea_orm::{
    ActiveValue::Set,
    ColumnTrait, DbBackend, DbErr, EntityTrait, FromQueryResult, QueryFilter, QueryOrder,
    QueryResult, QuerySelect, Select, SelectModel, Selector,
    sea_query::{Expr, OnConflict},
};
use std::collections::HashMap;

/// Execution aggregates of one window, computed in SQL. `p95_us` is the
/// nearest-rank percentile (`percentile_disc`), the definition every stored
/// `p95LatencyMs` was written with.
#[derive(Debug, Default, FromQueryResult)]
pub(super) struct ExecutionTotals {
    pub total: i64,
    pub failed: i64,
    pub unique_users: i64,
    pub avg_us: Option<f64>,
    pub p95_us: Option<i64>,
}

impl ExecutionTotals {
    pub(super) fn avg_latency_ms(&self) -> Option<f64> {
        self.avg_us.map(|us| us / 1000.0)
    }

    pub(super) fn p95_latency_ms(&self) -> Option<f64> {
        self.p95_us.map(|us| us as f64 / 1000.0)
    }
}

#[derive(Debug, Default, FromQueryResult)]
pub(super) struct FeedbackTotals {
    pub total: i64,
    pub positive: i64,
    pub negative: i64,
    pub avg_rating: Option<f64>,
}

#[derive(Debug, Default, FromQueryResult)]
pub(super) struct LlmTotals {
    pub calls: i64,
    pub tokens_in: i64,
    pub tokens_out: i64,
    pub cost: i64,
}

#[derive(Debug, Default, FromQueryResult)]
pub(super) struct EmbeddingTotals {
    pub calls: i64,
    pub tokens: i64,
    pub cost: i64,
}

/// One `GROUP BY` day row: the UTC day label plus the aggregate columns of `T`.
pub(super) struct DayBucket<T> {
    bucket: String,
    totals: T,
}

impl<T: FromQueryResult> FromQueryResult for DayBucket<T> {
    fn from_query_result(res: &QueryResult, pre: &str) -> Result<Self, DbErr> {
        Ok(Self {
            bucket: res.try_get(pre, "bucket")?,
            totals: T::from_query_result(res, pre)?,
        })
    }
}

// SUM and AVG over BIGINT come back as NUMERIC/DECIMAL, hence the casts.
pub(super) const COUNT_ROWS_SQL: &str = "CAST(COUNT(*) AS BIGINT)";
pub(super) const COUNT_FAILED_EXECUTIONS_SQL: &str =
    r#"CAST(COUNT(*) FILTER (WHERE "status" IN ('ERROR', 'FATAL')) AS BIGINT)"#;
pub(super) const AVG_MICROSECONDS_SQL: &str = r#"CAST(AVG("microseconds") AS DOUBLE PRECISION)"#;

pub(super) fn select_execution_totals(
    query: Select<execution_usage_tracking::Entity>,
) -> Select<execution_usage_tracking::Entity> {
    query
        .select_only()
        .expr_as(Expr::cust(COUNT_ROWS_SQL), "total")
        .expr_as(Expr::cust(COUNT_FAILED_EXECUTIONS_SQL), "failed")
        .expr_as(
            Expr::cust(r#"CAST(COUNT(DISTINCT "userId") AS BIGINT)"#),
            "unique_users",
        )
        .expr_as(Expr::cust(AVG_MICROSECONDS_SQL), "avg_us")
        .expr_as(
            Expr::cust(
                r#"CAST(percentile_disc(0.95::float8) WITHIN GROUP (ORDER BY "microseconds") AS BIGINT)"#,
            ),
            "p95_us",
        )
}

pub(super) fn select_feedback_totals(query: Select<feedback::Entity>) -> Select<feedback::Entity> {
    query
        .select_only()
        .expr_as(Expr::cust(COUNT_ROWS_SQL), "total")
        .expr_as(
            Expr::cust(r#"CAST(COUNT(*) FILTER (WHERE "rating" > 0) AS BIGINT)"#),
            "positive",
        )
        .expr_as(
            Expr::cust(r#"CAST(COUNT(*) FILTER (WHERE "rating" < 0) AS BIGINT)"#),
            "negative",
        )
        .expr_as(
            Expr::cust(r#"CAST(AVG("rating") AS DOUBLE PRECISION)"#),
            "avg_rating",
        )
}

pub(super) fn select_llm_totals(
    query: Select<llm_usage_tracking::Entity>,
) -> Select<llm_usage_tracking::Entity> {
    query
        .select_only()
        .expr_as(Expr::cust(COUNT_ROWS_SQL), "calls")
        .expr_as(
            Expr::cust(r#"CAST(COALESCE(SUM("tokenIn"), 0) AS BIGINT)"#),
            "tokens_in",
        )
        .expr_as(
            Expr::cust(r#"CAST(COALESCE(SUM("tokenOut"), 0) AS BIGINT)"#),
            "tokens_out",
        )
        .expr_as(
            Expr::cust(r#"CAST(COALESCE(SUM("price"), 0) AS BIGINT)"#),
            "cost",
        )
}

pub(super) fn select_embedding_totals(
    query: Select<embedding_usage_tracking::Entity>,
) -> Select<embedding_usage_tracking::Entity> {
    query
        .select_only()
        .expr_as(Expr::cust(COUNT_ROWS_SQL), "calls")
        .expr_as(
            Expr::cust(r#"CAST(COALESCE(SUM("tokenCount"), 0) AS BIGINT)"#),
            "tokens",
        )
        .expr_as(
            Expr::cust(r#"CAST(COALESCE(SUM("price"), 0) AS BIGINT)"#),
            "cost",
        )
}

/// Turn a totals select into one row per UTC day of `createdAt`.
pub(super) fn by_day<E, T>(
    totals: Select<E>,
    backend: DbBackend,
) -> Selector<SelectModel<DayBucket<T>>>
where
    E: EntityTrait,
    T: FromQueryResult,
{
    let bucket = StatsPeriod::Day.bucket_expr(backend, "createdAt");
    totals
        .expr_as(bucket.clone(), "bucket")
        .group_by(bucket)
        .into_model::<DayBucket<T>>()
}

pub(super) fn collect_by_day<T>(rows: Vec<DayBucket<T>>) -> HashMap<NaiveDate, T> {
    rows.into_iter()
        .filter_map(|row| {
            NaiveDate::parse_from_str(&row.bucket, "%Y-%m-%d")
                .ok()
                .map(|day| (day, row.totals))
        })
        .collect()
}

/// Ensures aggregations are up-to-date through yesterday.
/// Finds the latest aggregated date and backfills any missing days up to yesterday.
/// Caps backfill at 90 days to avoid runaway on first load.
pub async fn ensure_aggregations_current(state: &AppState, app_id: &str) -> Result<(), ApiError> {
    let yesterday = Utc::now().date_naive() - Duration::days(1);

    let latest = app_analytics_daily::Entity::find()
        .filter(app_analytics_daily::Column::AppId.eq(app_id))
        .order_by_desc(app_analytics_daily::Column::Date)
        .one(&state.db)
        .await?;

    let start_date = match latest {
        Some(ref row) if row.date >= yesterday => return Ok(()),
        Some(ref row) => row.date + Duration::days(1),
        None => yesterday - Duration::days(89),
    };

    let capped_start = {
        let earliest_allowed = yesterday - Duration::days(89);
        if start_date < earliest_allowed {
            earliest_allowed
        } else {
            start_date
        }
    };

    update_analytics_daily_range(state, app_id, capped_start, yesterday).await
}

/// Recompute every day of `start_date..=end_date` with one grouped aggregate
/// per source table and upsert all day rows, quiet days included, in a single
/// statement.
pub async fn update_analytics_daily_range(
    state: &AppState,
    app_id: &str,
    start_date: NaiveDate,
    end_date: NaiveDate,
) -> Result<(), ApiError> {
    if start_date > end_date {
        return Ok(());
    }

    let backend = state.db.get_database_backend();
    let range_start = utc_midnight(start_date);
    // Exclusive next midnight: every day in the range covers its full 24 hours,
    // so a stored day does not depend on when its backfill ran.
    let range_end = utc_midnight(end_date + chrono::Duration::days(1));

    let (executions, feedbacks, llm_records, embedding_records) = try_join!(
        by_day::<_, ExecutionTotals>(
            select_execution_totals(
                execution_usage_tracking::Entity::find()
                    .filter(execution_usage_tracking::Column::AppId.eq(app_id))
                    .filter(execution_usage_tracking::Column::CreatedAt.gte(range_start))
                    .filter(execution_usage_tracking::Column::CreatedAt.lt(range_end)),
            ),
            backend,
        )
        .all(&state.db),
        by_day::<_, FeedbackTotals>(
            select_feedback_totals(
                feedback::Entity::find()
                    .filter(feedback::Column::AppId.eq(app_id))
                    .filter(feedback::Column::CreatedAt.gte(range_start))
                    .filter(feedback::Column::CreatedAt.lt(range_end)),
            ),
            backend,
        )
        .all(&state.db),
        by_day::<_, LlmTotals>(
            select_llm_totals(
                llm_usage_tracking::Entity::find()
                    .filter(llm_usage_tracking::Column::AppId.eq(app_id))
                    .filter(llm_usage_tracking::Column::CreatedAt.gte(range_start))
                    .filter(llm_usage_tracking::Column::CreatedAt.lt(range_end)),
            ),
            backend,
        )
        .all(&state.db),
        by_day::<_, EmbeddingTotals>(
            select_embedding_totals(
                embedding_usage_tracking::Entity::find()
                    .filter(embedding_usage_tracking::Column::AppId.eq(app_id))
                    .filter(embedding_usage_tracking::Column::CreatedAt.gte(range_start))
                    .filter(embedding_usage_tracking::Column::CreatedAt.lt(range_end)),
            ),
            backend,
        )
        .all(&state.db),
    )?;

    let mut executions_by_day = collect_by_day(executions);
    let mut feedbacks_by_day = collect_by_day(feedbacks);
    let mut llm_by_day = collect_by_day(llm_records);
    let mut embeddings_by_day = collect_by_day(embedding_records);

    let now = Utc::now().fixed_offset();
    let mut rows = Vec::new();
    let mut date = start_date;
    while date <= end_date {
        let executions = executions_by_day.remove(&date).unwrap_or_default();
        let feedbacks = feedbacks_by_day.remove(&date).unwrap_or_default();
        let llm = llm_by_day.remove(&date).unwrap_or_default();
        let embeddings = embeddings_by_day.remove(&date).unwrap_or_default();

        rows.push(app_analytics_daily::ActiveModel {
            id: Set(create_id()),
            app_id: Set(app_id.to_string()),
            date: Set(date),
            total_executions: Set(executions.total),
            successful_executions: Set(executions.total - executions.failed),
            failed_executions: Set(executions.failed),
            unique_users: Set(executions.unique_users),
            feedback_count: Set(feedbacks.total),
            avg_feedback_rating: Set(feedbacks.avg_rating),
            positive_feedback: Set(feedbacks.positive),
            negative_feedback: Set(feedbacks.negative),
            total_llm_calls: Set(llm.calls),
            total_llm_tokens_in: Set(llm.tokens_in),
            total_llm_tokens_out: Set(llm.tokens_out),
            total_llm_cost: Set(llm.cost),
            avg_latency_ms: Set(executions.avg_latency_ms()),
            p95_latency_ms: Set(executions.p95_latency_ms()),
            total_embedding_calls: Set(embeddings.calls),
            total_embedding_tokens: Set(embeddings.tokens),
            total_embedding_cost: Set(embeddings.cost),
            created_at: Set(now),
            updated_at: Set(now),
        });

        date += Duration::days(1);
    }

    app_analytics_daily::Entity::insert_many(rows)
        .on_conflict(
            OnConflict::columns([
                app_analytics_daily::Column::AppId,
                app_analytics_daily::Column::Date,
            ])
            .update_columns([
                app_analytics_daily::Column::TotalExecutions,
                app_analytics_daily::Column::SuccessfulExecutions,
                app_analytics_daily::Column::FailedExecutions,
                app_analytics_daily::Column::UniqueUsers,
                app_analytics_daily::Column::FeedbackCount,
                app_analytics_daily::Column::AvgFeedbackRating,
                app_analytics_daily::Column::PositiveFeedback,
                app_analytics_daily::Column::NegativeFeedback,
                app_analytics_daily::Column::TotalLlmCalls,
                app_analytics_daily::Column::TotalLlmTokensIn,
                app_analytics_daily::Column::TotalLlmTokensOut,
                app_analytics_daily::Column::TotalLlmCost,
                app_analytics_daily::Column::AvgLatencyMs,
                app_analytics_daily::Column::P95LatencyMs,
                app_analytics_daily::Column::TotalEmbeddingCalls,
                app_analytics_daily::Column::TotalEmbeddingTokens,
                app_analytics_daily::Column::TotalEmbeddingCost,
                app_analytics_daily::Column::UpdatedAt,
            ])
            .to_owned(),
        )
        .exec_without_returning(&state.db)
        .await?;

    Ok(())
}
