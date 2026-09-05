use crate::{
    entity::{usage_invocation, usage_limit_audit_log},
    error::ApiError,
    state::AppState,
    usage_limits::check_app_usage_limits_for_user,
};
use chrono::{Duration, Utc};
use flow_like_types::{Value, create_id};
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter,
};
use serde::Serialize;
use utoipa::ToSchema;

pub const STATUS_PENDING: &str = "pending";
pub const STATUS_COMPLETED: &str = "completed";
pub const STATUS_FAILED: &str = "failed";
pub const STATUS_CANCELLED: &str = "cancelled";
pub const STATUS_UNKNOWN_USAGE: &str = "unknown_usage";

#[derive(Clone, Debug)]
pub struct UsageInvocationStart<'a> {
    pub kind: &'a str,
    pub user_id: Option<&'a str>,
    pub technical_user_id: Option<&'a str>,
    pub app_id: Option<&'a str>,
    pub provider: Option<&'a str>,
    pub endpoint: Option<&'a str>,
    pub model_id: Option<&'a str>,
    pub estimated_tokens: i64,
    pub estimated_cost_micro_dollars: i64,
}

#[derive(Clone, Debug, Default)]
pub struct UsageInvocationSettlement {
    pub status: &'static str,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub embedding_tokens: i64,
    pub cost_micro_dollars: i64,
    pub latency_ms: Option<f64>,
    pub provider_request_id: Option<String>,
    pub raw_usage: Option<Value>,
    pub error: Option<String>,
}

#[derive(Clone, Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct UsageReconciliationResult {
    pub older_than_minutes: i64,
    pub marked_unknown_usage: u64,
}

pub async fn start_usage_invocation(
    state: &AppState,
    start: UsageInvocationStart<'_>,
) -> Result<Option<String>, ApiError> {
    start_usage_invocation_with_db(&state.db, state.db_dialect, start).await
}

pub(crate) async fn start_usage_invocation_with_db(
    db: &DatabaseConnection,
    dialect: crate::db::DbDialect,
    start: UsageInvocationStart<'_>,
) -> Result<Option<String>, ApiError> {
    let Some(app_id) = start
        .app_id
        .map(str::trim)
        .filter(|app_id| !app_id.is_empty())
    else {
        return Ok(None);
    };

    let now = Utc::now().fixed_offset();
    let id = create_id();
    let reservation = usage_invocation::ActiveModel {
        id: Set(id.clone()),
        kind: Set(start.kind.to_string()),
        status: Set(STATUS_PENDING.to_string()),
        user_id: Set(start.user_id.map(ToOwned::to_owned)),
        technical_user_id: Set(start.technical_user_id.map(ToOwned::to_owned)),
        app_id: Set(Some(app_id.to_string())),
        provider: Set(start.provider.map(ToOwned::to_owned)),
        endpoint: Set(start.endpoint.map(ToOwned::to_owned)),
        model_id: Set(start.model_id.map(ToOwned::to_owned)),
        provider_request_id: Set(None),
        estimated_tokens: Set(start.estimated_tokens.max(0)),
        estimated_cost_micro_dollars: Set(start.estimated_cost_micro_dollars.max(0)),
        input_tokens: Set(0),
        output_tokens: Set(0),
        embedding_tokens: Set(0),
        cost_micro_dollars: Set(0),
        latency: Set(None),
        raw_usage: Set(None),
        error: Set(None),
        started_at: Set(now),
        completed_at: Set(None),
        created_at: Set(now),
        updated_at: Set(now),
    };
    let app_id = app_id.to_owned();
    let user_id = start.user_id.map(ToOwned::to_owned);
    let technical_user_id = start.technical_user_id.map(ToOwned::to_owned);
    let estimated_tokens = start.estimated_tokens.max(0);
    let estimated_cost = start.estimated_cost_micro_dollars.max(0);
    crate::db::retry_transaction(
        db,
        dialect,
        None,
        &crate::db::RetryPolicy::default(),
        move |txn| {
            let reservation = reservation.clone();
            let app_id = app_id.clone();
            let user_id = user_id.clone();
            let technical_user_id = technical_user_id.clone();
            let id = id.clone();
            Box::pin(async move {
                crate::db::coordination::coordinate(txn, "usage-budget", &[&app_id]).await?;
                if let Some(rejection) = check_app_usage_limits_for_user(
                    txn,
                    Some(&app_id),
                    user_id.as_deref(),
                    technical_user_id.as_deref(),
                    Some(estimated_tokens),
                    Some(estimated_cost),
                )
                .await?
                {
                    return Ok::<_, ApiError>(Err(rejection));
                }
                reservation.insert(txn).await?;
                Ok(Ok(Some(id)))
            })
        },
    )
    .await?
}

pub async fn settle_usage_invocation(
    db: &DatabaseConnection,
    invocation_id: Option<&str>,
    settlement: UsageInvocationSettlement,
) -> Result<(), sea_orm::DbErr> {
    let Some(invocation_id) = invocation_id
        .map(str::trim)
        .filter(|invocation_id| !invocation_id.is_empty())
    else {
        return Ok(());
    };

    let now = Utc::now().fixed_offset();
    usage_invocation::Entity::update_many()
        .set(usage_invocation::ActiveModel {
            status: Set(settlement.status.to_string()),
            input_tokens: Set(settlement.input_tokens.max(0)),
            output_tokens: Set(settlement.output_tokens.max(0)),
            embedding_tokens: Set(settlement.embedding_tokens.max(0)),
            cost_micro_dollars: Set(settlement.cost_micro_dollars.max(0)),
            latency: Set(settlement.latency_ms),
            provider_request_id: Set(settlement.provider_request_id),
            raw_usage: Set(settlement.raw_usage),
            error: Set(settlement.error),
            completed_at: Set(Some(now)),
            updated_at: Set(now),
            ..Default::default()
        })
        .filter(usage_invocation::Column::Id.eq(invocation_id))
        .filter(usage_invocation::Column::Status.eq(STATUS_PENDING))
        .exec(db)
        .await?;

    Ok(())
}

pub async fn reconcile_stale_invocations(
    db: &DatabaseConnection,
    older_than_minutes: i64,
) -> Result<UsageReconciliationResult, sea_orm::DbErr> {
    let cutoff = Utc::now().fixed_offset() - Duration::minutes(older_than_minutes.max(1));
    let stale = usage_invocation::Entity::find()
        .filter(usage_invocation::Column::Status.eq(STATUS_PENDING))
        .filter(usage_invocation::Column::StartedAt.lt(cutoff))
        .all(db)
        .await?;

    let now = Utc::now().fixed_offset();
    let mut marked = 0;
    for row in stale {
        let updated = usage_invocation::Entity::update_many()
            .set(usage_invocation::ActiveModel {
                status: Set(STATUS_UNKNOWN_USAGE.to_string()),
                completed_at: Set(Some(now)),
                updated_at: Set(now),
                ..Default::default()
            })
            .filter(usage_invocation::Column::Id.eq(row.id))
            .filter(usage_invocation::Column::Status.eq(STATUS_PENDING))
            .exec(db)
            .await?;
        marked += updated.rows_affected;
    }

    Ok(UsageReconciliationResult {
        older_than_minutes,
        marked_unknown_usage: marked,
    })
}

pub async fn record_usage_limit_audit(
    db: &DatabaseConnection,
    app_id: Option<&str>,
    user_id: Option<&str>,
    actor_user_id: Option<&str>,
    action: &str,
    before: Option<Value>,
    after: Option<Value>,
) -> Result<(), sea_orm::DbErr> {
    usage_limit_audit_log::ActiveModel {
        id: Set(create_id()),
        app_id: Set(app_id.map(ToOwned::to_owned)),
        user_id: Set(user_id.map(ToOwned::to_owned)),
        actor_user_id: Set(actor_user_id.map(ToOwned::to_owned)),
        action: Set(action.to_string()),
        before: Set(before),
        after: Set(after),
        created_at: Set(Utc::now().fixed_offset()),
    }
    .insert(db)
    .await?;

    Ok(())
}

pub fn estimate_text_tokens(text: &str) -> i64 {
    ((text.len() as i64) / 4).max(1)
}
