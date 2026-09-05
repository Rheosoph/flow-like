use crate::{
    entity::{app_usage_limit, usage_alert},
    error::ApiError,
    state::AppState,
};
use chrono::{DateTime, Duration, FixedOffset, Utc};
use flow_like_types::create_id;
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, Condition, ConnectionTrait,
    DatabaseConnection, DbBackend, EntityTrait, FromQueryResult, QueryFilter, Statement,
};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

pub const WEEKLY: &str = "weekly";
pub const MONTHLY: &str = "monthly";
pub const YEARLY: &str = "yearly";
pub const PERIODS: [&str; 3] = [WEEKLY, MONTHLY, YEARLY];

#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct AppUsageLimitWindow {
    pub cost_micro_dollars: Option<i64>,
    pub token_limit: Option<i64>,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_true")]
    pub hard: bool,
    pub warning_threshold_percent: Option<i32>,
}

impl Default for AppUsageLimitWindow {
    fn default() -> Self {
        Self {
            cost_micro_dollars: None,
            token_limit: None,
            enabled: true,
            hard: true,
            warning_threshold_percent: None,
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct AppUsageLimits {
    pub weekly: AppUsageLimitWindow,
    pub monthly: AppUsageLimitWindow,
    pub yearly: AppUsageLimitWindow,
}

impl AppUsageLimits {
    pub fn from_rows(rows: Vec<app_usage_limit::Model>) -> Self {
        let mut limits = AppUsageLimits::default();
        for row in rows {
            let window = AppUsageLimitWindow {
                cost_micro_dollars: row.cost_micro_dollars,
                token_limit: row.token_limit,
                enabled: row.enabled,
                hard: row.hard,
                warning_threshold_percent: row.warning_threshold_percent,
            };
            match normalize_period(&row.period).as_deref() {
                Some(WEEKLY) => limits.weekly = window,
                Some(MONTHLY) => limits.monthly = window,
                Some(YEARLY) => limits.yearly = window,
                _ => {}
            }
        }
        limits
    }

    pub fn iter(&self) -> [(&'static str, &AppUsageLimitWindow); 3] {
        [
            (WEEKLY, &self.weekly),
            (MONTHLY, &self.monthly),
            (YEARLY, &self.yearly),
        ]
    }
}

fn default_true() -> bool {
    true
}

pub fn normalize_period(period: &str) -> Option<String> {
    let normalized = period.trim().to_ascii_lowercase();
    if PERIODS.contains(&normalized.as_str()) {
        Some(normalized)
    } else {
        None
    }
}

pub fn period_start(period: &str) -> Option<DateTime<FixedOffset>> {
    let now = Utc::now().fixed_offset();
    match normalize_period(period).as_deref() {
        Some(WEEKLY) => Some(now - Duration::days(7)),
        Some(MONTHLY) => Some(now - Duration::days(30)),
        Some(YEARLY) => Some(now - Duration::days(365)),
        _ => None,
    }
}

pub async fn get_app_usage_limits(
    db: &DatabaseConnection,
    app_id: &str,
) -> Result<AppUsageLimits, sea_orm::DbErr> {
    get_app_usage_limits_for_scope(db, app_id, "").await
}

pub async fn get_app_usage_limits_for_scope(
    db: &DatabaseConnection,
    app_id: &str,
    scoped_user_id: &str,
) -> Result<AppUsageLimits, sea_orm::DbErr> {
    let rows = app_usage_limit::Entity::find()
        .filter(app_usage_limit::Column::AppId.eq(app_id))
        .filter(app_usage_limit::Column::UserId.eq(scoped_user_id))
        .all(db)
        .await?;
    Ok(AppUsageLimits::from_rows(rows))
}

pub async fn set_app_usage_limits(
    db: &DatabaseConnection,
    app_id: &str,
    limits: AppUsageLimits,
) -> Result<AppUsageLimits, sea_orm::DbErr> {
    set_app_usage_limits_for_scope(db, app_id, "", limits).await
}

pub async fn set_app_usage_limits_for_scope(
    db: &DatabaseConnection,
    app_id: &str,
    scoped_user_id: &str,
    limits: AppUsageLimits,
) -> Result<AppUsageLimits, sea_orm::DbErr> {
    let now = Utc::now().fixed_offset();

    for (period, window) in limits.iter() {
        if window.cost_micro_dollars.is_none() && window.token_limit.is_none() {
            app_usage_limit::Entity::delete_many()
                .filter(app_usage_limit::Column::AppId.eq(app_id))
                .filter(app_usage_limit::Column::UserId.eq(scoped_user_id))
                .filter(app_usage_limit::Column::Period.eq(period))
                .exec(db)
                .await?;
            continue;
        }

        let existing = app_usage_limit::Entity::find()
            .filter(app_usage_limit::Column::AppId.eq(app_id))
            .filter(app_usage_limit::Column::UserId.eq(scoped_user_id))
            .filter(app_usage_limit::Column::Period.eq(period))
            .one(db)
            .await?;

        if let Some(existing) = existing {
            let created_at = existing.created_at;
            let mut active: app_usage_limit::ActiveModel = existing.into();
            active.cost_micro_dollars = Set(window.cost_micro_dollars);
            active.token_limit = Set(window.token_limit);
            active.enabled = Set(window.enabled);
            active.hard = Set(window.hard);
            active.warning_threshold_percent = Set(window.warning_threshold_percent);
            active.created_at = Set(created_at);
            active.updated_at = Set(now);
            active.update(db).await?;
        } else {
            app_usage_limit::ActiveModel {
                id: Set(create_id()),
                app_id: Set(app_id.to_string()),
                user_id: Set(scoped_user_id.to_string()),
                period: Set(period.to_string()),
                cost_micro_dollars: Set(window.cost_micro_dollars),
                token_limit: Set(window.token_limit),
                enabled: Set(window.enabled),
                hard: Set(window.hard),
                warning_threshold_percent: Set(window.warning_threshold_percent),
                created_at: Set(now),
                updated_at: Set(now),
            }
            .insert(db)
            .await?;
        }
    }

    get_app_usage_limits_for_scope(db, app_id, scoped_user_id).await
}

pub async fn enforce_app_usage_limits(
    state: &AppState,
    app_id: Option<&str>,
    token_delta: Option<i64>,
    cost_delta: Option<i64>,
) -> Result<(), ApiError> {
    enforce_app_usage_limits_for_user(state, app_id, None, None, token_delta, cost_delta).await
}

pub async fn enforce_app_usage_limits_for_user(
    state: &AppState,
    app_id: Option<&str>,
    user_id: Option<&str>,
    technical_user_id: Option<&str>,
    token_delta: Option<i64>,
    cost_delta: Option<i64>,
) -> Result<(), ApiError> {
    match check_app_usage_limits_for_user(
        &state.db,
        app_id,
        user_id,
        technical_user_id,
        token_delta,
        cost_delta,
    )
    .await?
    {
        Some(rejection) => Err(rejection),
        None => Ok(()),
    }
}

/// Returns a policy rejection as a value so a reservation transaction can
/// commit its alert while omitting the rejected reservation.
pub(crate) async fn check_app_usage_limits_for_user<C: ConnectionTrait>(
    db: &C,
    app_id: Option<&str>,
    user_id: Option<&str>,
    technical_user_id: Option<&str>,
    token_delta: Option<i64>,
    cost_delta: Option<i64>,
) -> Result<Option<ApiError>, ApiError> {
    let Some(app_id) = app_id.map(str::trim).filter(|app_id| !app_id.is_empty()) else {
        return Ok(None);
    };

    let limits = app_usage_limit::Entity::find()
        .filter(app_usage_limit::Column::AppId.eq(app_id))
        .filter(app_usage_limit::Column::Enabled.eq(true))
        .filter(
            Condition::any()
                .add(app_usage_limit::Column::UserId.eq(""))
                .add_option(
                    technical_user_id.map(|user_id| app_usage_limit::Column::UserId.eq(user_id)),
                )
                .add_option(user_id.map(|user_id| app_usage_limit::Column::UserId.eq(user_id))),
        )
        .all(db)
        .await
        .map_err(ApiError::from)?;

    if limits.is_empty() {
        return Ok(None);
    }

    for limit in limits {
        let Some(start) = period_start(&limit.period) else {
            continue;
        };
        if limit.cost_micro_dollars.is_none() && limit.token_limit.is_none() {
            continue;
        }

        let scoped_user_id = if limit.user_id.is_empty() {
            None
        } else {
            Some(limit.user_id.as_str())
        };
        let current = query_usage_totals(db, app_id, scoped_user_id, start)
            .await
            .map_err(ApiError::from)?;
        let used_tokens = current
            .tokens
            .saturating_add(token_delta.unwrap_or(0).max(0));
        let used_cost = current
            .cost_micro_dollars
            .saturating_add(cost_delta.unwrap_or(0).max(0));

        maybe_emit_threshold_alert(
            db,
            &limit,
            used_tokens,
            used_cost,
            limit.warning_threshold_percent,
        )
        .await
        .map_err(ApiError::from)?;

        if let Some(cost_limit) = limit.cost_micro_dollars
            && used_cost > cost_limit
        {
            emit_limit_exceeded_alert(db, &limit, used_tokens, used_cost)
                .await
                .map_err(ApiError::from)?;
            if limit.hard {
                return Ok(Some(ApiError::too_many_requests(format!(
                    "App usage cost limit exceeded for {}",
                    limit.period
                ))));
            }
        }

        if let Some(token_limit) = limit.token_limit
            && used_tokens > token_limit
        {
            emit_limit_exceeded_alert(db, &limit, used_tokens, used_cost)
                .await
                .map_err(ApiError::from)?;
            if limit.hard {
                return Ok(Some(ApiError::too_many_requests(format!(
                    "App usage token limit exceeded for {}",
                    limit.period
                ))));
            }
        }
    }

    Ok(None)
}

#[derive(Clone, Debug, Default, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct UsageLimitTotals {
    pub cost_micro_dollars: i64,
    pub tokens: i64,
    pub invocations: i64,
}

#[derive(Debug, FromQueryResult)]
struct UsageSqlRow {
    cost_micro_dollars: i64,
    tokens: i64,
    invocations: i64,
}

pub async fn query_usage_totals<C: ConnectionTrait>(
    db: &C,
    app_id: &str,
    user_id: Option<&str>,
    start: DateTime<FixedOffset>,
) -> Result<UsageLimitTotals, sea_orm::DbErr> {
    let backend = db.get_database_backend();
    let user_id = user_id.unwrap_or("");
    // Tracking is inserted before an invocation is settled. Reading all three
    // sources in one statement prevents a settlement between reads from making
    // both the tracking row and its pending reservation disappear from the total.
    // A linked tracking row replaces its estimate even before settlement finishes.
    let sources = [
        UsageTotalTable::Llm,
        UsageTotalTable::Embedding,
        UsageTotalTable::Pending,
    ]
    .map(|table| usage_total_sql(backend, table))
    .join(" UNION ALL ");
    let cast = if backend == DbBackend::Postgres {
        "::BIGINT"
    } else {
        ""
    };
    let sql = format!(
        "SELECT SUM(cost_micro_dollars){cast} AS cost_micro_dollars, \
         SUM(tokens){cast} AS tokens, SUM(invocations){cast} AS invocations \
         FROM ({sources}) AS usage_totals"
    );
    let values = match backend {
        DbBackend::Postgres => vec![start.into(), app_id.into(), user_id.into()],
        _ => (0..3)
            .flat_map(|_| {
                [
                    start.into(),
                    app_id.into(),
                    user_id.into(),
                    user_id.into(),
                    user_id.into(),
                ]
            })
            .collect(),
    };
    let row = UsageSqlRow::find_by_statement(Statement::from_sql_and_values(backend, sql, values))
        .one(db)
        .await?;
    Ok(row
        .map(|row| UsageLimitTotals {
            cost_micro_dollars: row.cost_micro_dollars,
            tokens: row.tokens,
            invocations: row.invocations,
        })
        .unwrap_or_default())
}

#[derive(Copy, Clone)]
enum UsageTotalTable {
    Llm,
    Embedding,
    Pending,
}

fn usage_total_sql(backend: DbBackend, table: UsageTotalTable) -> &'static str {
    match (backend, table) {
        (DbBackend::Postgres, UsageTotalTable::Llm) => {
            r#"SELECT COALESCE(SUM("price"), 0)::BIGINT AS cost_micro_dollars,
COALESCE(SUM("tokenIn" + "tokenOut"), 0)::BIGINT AS tokens,
COUNT(*)::BIGINT AS invocations
FROM "LLMUsageTracking"
WHERE "createdAt" >= $1 AND "appId" = $2 AND ($3 = '' OR "userId" = $3 OR "technicalUserId" = $3)"#
        }
        (DbBackend::Postgres, UsageTotalTable::Embedding) => {
            r#"SELECT COALESCE(SUM("price"), 0)::BIGINT AS cost_micro_dollars,
COALESCE(SUM("tokenCount"), 0)::BIGINT AS tokens,
COUNT(*)::BIGINT AS invocations
FROM "EmbeddingUsageTracking"
WHERE "createdAt" >= $1 AND "appId" = $2 AND ($3 = '' OR "userId" = $3 OR "technicalUserId" = $3)"#
        }
        (DbBackend::Postgres, UsageTotalTable::Pending) => {
            r#"SELECT COALESCE(SUM("estimatedCostMicroDollars"), 0)::BIGINT AS cost_micro_dollars,
COALESCE(SUM("estimatedTokens"), 0)::BIGINT AS tokens,
COUNT(*)::BIGINT AS invocations
FROM "UsageInvocation"
WHERE "startedAt" >= $1 AND "status" = 'pending' AND "appId" = $2 AND ($3 = '' OR "userId" = $3 OR "technicalUserId" = $3)
AND NOT EXISTS (SELECT 1 FROM "LLMUsageTracking" tracked WHERE tracked."invocationId" = "UsageInvocation"."id")
AND NOT EXISTS (SELECT 1 FROM "EmbeddingUsageTracking" tracked WHERE tracked."invocationId" = "UsageInvocation"."id")"#
        }
        (_, UsageTotalTable::Llm) => {
            r#"SELECT COALESCE(SUM("price"), 0) AS cost_micro_dollars,
COALESCE(SUM("tokenIn" + "tokenOut"), 0) AS tokens,
COUNT(*) AS invocations
FROM "LLMUsageTracking"
WHERE "createdAt" >= ? AND "appId" = ? AND (? = '' OR "userId" = ? OR "technicalUserId" = ?)"#
        }
        (_, UsageTotalTable::Embedding) => {
            r#"SELECT COALESCE(SUM("price"), 0) AS cost_micro_dollars,
COALESCE(SUM("tokenCount"), 0) AS tokens,
COUNT(*) AS invocations
FROM "EmbeddingUsageTracking"
WHERE "createdAt" >= ? AND "appId" = ? AND (? = '' OR "userId" = ? OR "technicalUserId" = ?)"#
        }
        (_, UsageTotalTable::Pending) => {
            r#"SELECT COALESCE(SUM("estimatedCostMicroDollars"), 0) AS cost_micro_dollars,
COALESCE(SUM("estimatedTokens"), 0) AS tokens,
COUNT(*) AS invocations
FROM "UsageInvocation"
WHERE "startedAt" >= ? AND "status" = 'pending' AND "appId" = ? AND (? = '' OR "userId" = ? OR "technicalUserId" = ?)
AND NOT EXISTS (SELECT 1 FROM "LLMUsageTracking" tracked WHERE tracked."invocationId" = "UsageInvocation"."id")
AND NOT EXISTS (SELECT 1 FROM "EmbeddingUsageTracking" tracked WHERE tracked."invocationId" = "UsageInvocation"."id")"#
        }
    }
}

async fn maybe_emit_threshold_alert<C: ConnectionTrait>(
    db: &C,
    limit: &app_usage_limit::Model,
    used_tokens: i64,
    used_cost: i64,
    threshold_percent: Option<i32>,
) -> Result<(), sea_orm::DbErr> {
    let Some(threshold_percent) = threshold_percent.filter(|value| *value > 0) else {
        return Ok(());
    };

    let cost_crossed = limit
        .cost_micro_dollars
        .map(|limit| {
            i128::from(used_cost) * 100 >= i128::from(limit) * i128::from(threshold_percent)
        })
        .unwrap_or(false);
    let token_crossed = limit
        .token_limit
        .map(|limit| {
            i128::from(used_tokens) * 100 >= i128::from(limit) * i128::from(threshold_percent)
        })
        .unwrap_or(false);

    if cost_crossed || token_crossed {
        emit_usage_alert(
            db,
            limit,
            "limit_warning",
            "warning",
            Some(threshold_percent),
            used_tokens,
            used_cost,
        )
        .await?;
    }

    Ok(())
}

async fn emit_limit_exceeded_alert<C: ConnectionTrait>(
    db: &C,
    limit: &app_usage_limit::Model,
    used_tokens: i64,
    used_cost: i64,
) -> Result<(), sea_orm::DbErr> {
    emit_usage_alert(
        db,
        limit,
        "limit_exceeded",
        if limit.hard { "critical" } else { "warning" },
        None,
        used_tokens,
        used_cost,
    )
    .await
}

async fn emit_usage_alert<C: ConnectionTrait>(
    db: &C,
    limit: &app_usage_limit::Model,
    kind: &str,
    severity: &str,
    threshold_percent: Option<i32>,
    used_tokens: i64,
    used_cost: i64,
) -> Result<(), sea_orm::DbErr> {
    let existing = usage_alert::Entity::find()
        .filter(usage_alert::Column::Kind.eq(kind))
        .filter(usage_alert::Column::AppId.eq(Some(limit.app_id.clone())))
        .filter(usage_alert::Column::UserId.eq(if limit.user_id.is_empty() {
            None
        } else {
            Some(limit.user_id.clone())
        }))
        .filter(usage_alert::Column::Period.eq(Some(limit.period.clone())))
        .filter(usage_alert::Column::AcknowledgedAt.is_null())
        .one(db)
        .await?;
    if existing.is_some() {
        return Ok(());
    }

    let now = Utc::now().fixed_offset();
    let message = if limit.user_id.is_empty() {
        format!("App usage {} for {}", kind.replace('_', " "), limit.period)
    } else {
        format!(
            "User app usage {} for {}",
            kind.replace('_', " "),
            limit.period
        )
    };
    usage_alert::ActiveModel {
        id: Set(create_id()),
        kind: Set(kind.to_string()),
        severity: Set(severity.to_string()),
        period: Set(Some(limit.period.clone())),
        message: Set(message),
        app_id: Set(Some(limit.app_id.clone())),
        user_id: Set(if limit.user_id.is_empty() {
            None
        } else {
            Some(limit.user_id.clone())
        }),
        threshold_percent: Set(threshold_percent),
        current_cost_micro_dollars: Set(Some(used_cost)),
        current_tokens: Set(Some(used_tokens)),
        acknowledged_at: Set(None),
        acknowledged_by_user_id: Set(None),
        created_at: Set(now),
        updated_at: Set(now),
    }
    .insert(db)
    .await?;

    Ok(())
}
