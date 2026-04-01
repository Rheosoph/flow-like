use crate::{
    entity::{
        app_analytics_daily, embedding_usage_tracking, execution_usage_tracking, feedback,
        llm_usage_tracking, sea_orm_active_enums::ExecutionStatus,
    },
    error::ApiError,
    state::AppState,
};
use chrono::{Duration, NaiveDate, Utc};
use flow_like_types::create_id;
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, EntityTrait, QueryFilter, QueryOrder,
};
use std::collections::HashSet;

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

    let mut date = capped_start;
    while date <= yesterday {
        update_analytics_daily(state, app_id, date).await?;
        date += Duration::days(1);
    }

    Ok(())
}

pub async fn update_analytics_daily(
    state: &AppState,
    app_id: &str,
    date: NaiveDate,
) -> Result<(), ApiError> {
    let start_of_day = date.and_hms_opt(0, 0, 0).unwrap();
    let end_of_day = date.and_hms_opt(23, 59, 59).unwrap();

    let executions = execution_usage_tracking::Entity::find()
        .filter(execution_usage_tracking::Column::AppId.eq(app_id))
        .filter(execution_usage_tracking::Column::CreatedAt.gte(start_of_day))
        .filter(execution_usage_tracking::Column::CreatedAt.lte(end_of_day))
        .all(&state.db)
        .await?;

    let total_executions = executions.len() as i64;
    let failed_executions = executions
        .iter()
        .filter(|e| matches!(e.status, ExecutionStatus::Error | ExecutionStatus::Fatal))
        .count() as i64;
    let successful_executions = total_executions - failed_executions;

    let unique_user_ids: HashSet<_> = executions
        .iter()
        .filter_map(|e| e.user_id.as_ref())
        .collect();
    let unique_users = unique_user_ids.len() as i64;

    let latencies_us: Vec<i64> = executions.iter().map(|e| e.microseconds).collect();
    let avg_latency_ms = if latencies_us.is_empty() {
        None
    } else {
        Some(latencies_us.iter().sum::<i64>() as f64 / latencies_us.len() as f64 / 1000.0)
    };

    let p95_latency_ms = if latencies_us.is_empty() {
        None
    } else {
        let mut sorted = latencies_us.clone();
        sorted.sort();
        let idx = ((sorted.len() as f64) * 0.95).ceil() as usize;
        let idx = idx.min(sorted.len()) - 1;
        Some(sorted[idx] as f64 / 1000.0)
    };

    let feedbacks = feedback::Entity::find()
        .filter(feedback::Column::AppId.eq(app_id))
        .filter(feedback::Column::CreatedAt.gte(start_of_day))
        .filter(feedback::Column::CreatedAt.lte(end_of_day))
        .all(&state.db)
        .await?;

    let feedback_count = feedbacks.len() as i64;
    let positive_feedback = feedbacks.iter().filter(|f| f.rating > 0).count() as i64;
    let negative_feedback = feedbacks.iter().filter(|f| f.rating < 0).count() as i64;
    let avg_feedback_rating = if feedbacks.is_empty() {
        None
    } else {
        Some(feedbacks.iter().map(|f| f.rating as f64).sum::<f64>() / feedbacks.len() as f64)
    };

    let llm_records = llm_usage_tracking::Entity::find()
        .filter(llm_usage_tracking::Column::AppId.eq(app_id))
        .filter(llm_usage_tracking::Column::CreatedAt.gte(start_of_day))
        .filter(llm_usage_tracking::Column::CreatedAt.lte(end_of_day))
        .all(&state.db)
        .await?;

    let total_llm_calls = llm_records.len() as i64;
    let total_llm_tokens_in: i64 = llm_records.iter().map(|r| r.token_in).sum();
    let total_llm_tokens_out: i64 = llm_records.iter().map(|r| r.token_out).sum();
    let total_llm_cost: i64 = llm_records.iter().map(|r| r.price).sum();

    let embedding_records = embedding_usage_tracking::Entity::find()
        .filter(embedding_usage_tracking::Column::AppId.eq(app_id))
        .filter(embedding_usage_tracking::Column::CreatedAt.gte(start_of_day))
        .filter(embedding_usage_tracking::Column::CreatedAt.lte(end_of_day))
        .all(&state.db)
        .await?;

    let total_embedding_calls = embedding_records.len() as i64;
    let total_embedding_tokens: i64 = embedding_records.iter().map(|r| r.token_count).sum();
    let total_embedding_cost: i64 = embedding_records.iter().map(|r| r.price).sum();

    let existing = app_analytics_daily::Entity::find()
        .filter(app_analytics_daily::Column::AppId.eq(app_id))
        .filter(app_analytics_daily::Column::Date.eq(date))
        .one(&state.db)
        .await?;

    let now = Utc::now().naive_utc();

    if let Some(existing) = existing {
        let mut active: app_analytics_daily::ActiveModel = existing.into();
        active.total_executions = Set(total_executions);
        active.successful_executions = Set(successful_executions);
        active.failed_executions = Set(failed_executions);
        active.unique_users = Set(unique_users);
        active.feedback_count = Set(feedback_count);
        active.avg_feedback_rating = Set(avg_feedback_rating);
        active.positive_feedback = Set(positive_feedback);
        active.negative_feedback = Set(negative_feedback);
        active.total_llm_calls = Set(total_llm_calls);
        active.total_llm_tokens_in = Set(total_llm_tokens_in);
        active.total_llm_tokens_out = Set(total_llm_tokens_out);
        active.total_llm_cost = Set(total_llm_cost);
        active.avg_latency_ms = Set(avg_latency_ms);
        active.p95_latency_ms = Set(p95_latency_ms);
        active.total_embedding_calls = Set(total_embedding_calls);
        active.total_embedding_tokens = Set(total_embedding_tokens);
        active.total_embedding_cost = Set(total_embedding_cost);
        active.updated_at = Set(now);
        active.update(&state.db).await?;
    } else {
        let new_agg = app_analytics_daily::ActiveModel {
            id: Set(create_id()),
            app_id: Set(app_id.to_string()),
            date: Set(date),
            total_executions: Set(total_executions),
            successful_executions: Set(successful_executions),
            failed_executions: Set(failed_executions),
            unique_users: Set(unique_users),
            feedback_count: Set(feedback_count),
            avg_feedback_rating: Set(avg_feedback_rating),
            positive_feedback: Set(positive_feedback),
            negative_feedback: Set(negative_feedback),
            total_llm_calls: Set(total_llm_calls),
            total_llm_tokens_in: Set(total_llm_tokens_in),
            total_llm_tokens_out: Set(total_llm_tokens_out),
            total_llm_cost: Set(total_llm_cost),
            avg_latency_ms: Set(avg_latency_ms),
            p95_latency_ms: Set(p95_latency_ms),
            total_embedding_calls: Set(total_embedding_calls),
            total_embedding_tokens: Set(total_embedding_tokens),
            total_embedding_cost: Set(total_embedding_cost),
            created_at: Set(now),
            updated_at: Set(now),
        };
        new_agg.insert(&state.db).await?;
    }

    Ok(())
}
