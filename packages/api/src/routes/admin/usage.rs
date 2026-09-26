use crate::{
    compute_cost::{ComputeCostModel, compute_cost_micro_dollars, compute_cost_model},
    entity::{
        app, app_usage_limit, embedding_usage_tracking, execution_usage_tracking,
        llm_usage_tracking, meta, technical_user, usage_alert, usage_invocation,
        usage_limit_audit_log, user,
    },
    error::ApiError,
    middleware::jwt::AppUser,
    permission::global_permission::GlobalPermission,
    state::AppState,
    usage_accounting::{
        UsageReconciliationResult, reconcile_hosted_invocations, record_usage_limit_audit,
    },
    usage_limits::{
        AppUsageLimits, MONTHLY, get_app_usage_limits, get_app_usage_limits_for_scope,
        normalize_period, period_start, set_app_usage_limits, set_app_usage_limits_for_scope,
    },
    utils::stats_period::StatsPeriod,
};
use axum::{
    Extension, Json,
    extract::{Path, Query, State},
};
use chrono::{DateTime, Datelike, Duration, FixedOffset, NaiveDate, Utc};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, EntityTrait, FromQueryResult, PaginatorTrait, QueryFilter,
    QueryOrder, QuerySelect, Select, Set,
    sea_query::{Alias, Expr, Func, SimpleExpr},
};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use utoipa::{IntoParams, ToSchema};

#[derive(Clone, Debug, Deserialize, IntoParams)]
pub struct UsageOverviewQuery {
    pub period: Option<String>,
}

#[derive(Clone, Debug, Deserialize, IntoParams)]
pub struct UsageListQuery {
    pub period: Option<String>,
    pub page: Option<u64>,
    pub page_size: Option<u64>,
    pub app_id: Option<String>,
    pub user_id: Option<String>,
    pub technical_user_id: Option<String>,
    pub status: Option<String>,
}

#[derive(Clone, Debug, Deserialize, IntoParams)]
pub struct UsageReconcileQuery {
    pub older_than_minutes: Option<i64>,
}

#[derive(Clone, Debug, Default)]
struct UsageAggregate {
    llm_price: i64,
    embedding_price: i64,
    llm_tokens: i64,
    embedding_tokens: i64,
    llm_invocations: u64,
    embedding_invocations: u64,
    executions: u64,
    execution_microseconds: i64,
}

impl UsageAggregate {
    fn add_llm(&mut self, row: &PeriodAiUsageRow) {
        self.llm_price += row.price;
        self.llm_tokens += row.tokens;
        self.llm_invocations += to_count(row.invocations);
    }

    fn add_embedding(&mut self, row: &PeriodAiUsageRow) {
        self.embedding_price += row.price;
        self.embedding_tokens += row.tokens;
        self.embedding_invocations += to_count(row.invocations);
    }

    fn add_executions(&mut self, row: &PeriodExecutionUsageRow) {
        self.executions += to_count(row.executions);
        self.execution_microseconds += row.microseconds;
    }

    fn total_price(&self) -> i64 {
        self.llm_price + self.embedding_price
    }

    fn total_tokens(&self) -> i64 {
        self.llm_tokens + self.embedding_tokens
    }

    fn compute_cost(&self) -> i64 {
        compute_cost_micro_dollars(self.execution_microseconds, self.executions as i64)
    }

    fn average_execution_ms(&self) -> Option<f64> {
        if self.executions == 0 {
            None
        } else {
            Some(self.execution_microseconds as f64 / self.executions as f64 / 1000.0)
        }
    }
}

#[derive(Clone, Debug, Default)]
struct ModelAggregate {
    price: i64,
    tokens: i64,
    invocations: u64,
    latency_sum: f64,
    latency_count: u64,
}

#[derive(Clone, Debug, Default)]
struct PowerUserAggregate {
    total_price: i64,
    total_tokens: i64,
    ai_invocations: u64,
    executions: u64,
    active_days: HashSet<String>,
    last_seen: Option<DateTime<FixedOffset>>,
}

impl PowerUserAggregate {
    fn total_interactions(&self) -> u64 {
        self.ai_invocations + self.executions
    }

    fn touch(&mut self, day: &str, last_seen: DateTime<FixedOffset>) {
        self.active_days.insert(day.to_string());
        self.last_seen = Some(match self.last_seen {
            Some(current) => current.max(last_seen),
            None => last_seen,
        });
    }
}

#[derive(Clone, Debug, Default)]
struct TrendBucket {
    new_users: u64,
    active_users: HashSet<String>,
    executions: u64,
    ai_invocations: u64,
    tokens: i64,
    cost: i64,
}

impl ModelAggregate {
    fn average_latency_ms(&self) -> Option<f64> {
        if self.latency_count == 0 {
            None
        } else {
            Some(self.latency_sum / self.latency_count as f64)
        }
    }
}

#[derive(Clone, Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct AdminUsageTotals {
    pub llm_price: i64,
    pub embedding_price: i64,
    pub total_price: i64,
    pub llm_tokens: i64,
    pub embedding_tokens: i64,
    pub total_tokens: i64,
    pub llm_invocations: u64,
    pub embedding_invocations: u64,
    pub executions: u64,
    pub execution_microseconds: i64,
    pub average_execution_ms: Option<f64>,
    /// Estimated serverless runtime cost (micro-dollars)
    pub compute_cost: i64,
}

impl From<UsageAggregate> for AdminUsageTotals {
    fn from(value: UsageAggregate) -> Self {
        Self {
            llm_price: value.llm_price,
            embedding_price: value.embedding_price,
            total_price: value.total_price(),
            llm_tokens: value.llm_tokens,
            embedding_tokens: value.embedding_tokens,
            total_tokens: value.total_tokens(),
            llm_invocations: value.llm_invocations,
            embedding_invocations: value.embedding_invocations,
            executions: value.executions,
            execution_microseconds: value.execution_microseconds,
            average_execution_ms: value.average_execution_ms(),
            compute_cost: value.compute_cost(),
        }
    }
}

#[derive(Clone, Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct AdminUserUsage {
    pub user_id: Option<String>,
    pub display_name: Option<String>,
    pub email: Option<String>,
    #[serde(flatten)]
    pub totals: AdminUsageTotals,
}

#[derive(Clone, Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct AdminAppUsage {
    pub app_id: Option<String>,
    pub app_name: Option<String>,
    #[serde(flatten)]
    pub totals: AdminUsageTotals,
    pub limits: Option<AppUsageLimits>,
}

#[derive(Clone, Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct AdminTechnicalUserUsage {
    pub technical_user_id: String,
    pub name: Option<String>,
    pub app_id: Option<String>,
    pub app_name: Option<String>,
    pub creator_user_id: Option<String>,
    pub creator_membership_id: Option<String>,
    pub creator_display_name: Option<String>,
    pub creator_email: Option<String>,
    pub limits: Option<AppUsageLimits>,
    #[serde(flatten)]
    pub totals: AdminUsageTotals,
}

#[derive(Clone, Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct AdminModelUsage {
    pub kind: String,
    pub model_id: String,
    pub provider: Option<String>,
    pub endpoint: Option<String>,
    pub price: i64,
    pub tokens: i64,
    pub invocations: u64,
    pub average_latency_ms: Option<f64>,
}

#[derive(Clone, Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct AdminUserStats {
    pub total_users: u64,
    pub new_users_today: u64,
    pub new_users_weekly: u64,
    pub new_users_monthly: u64,
    pub active_users_daily: u64,
    pub active_users_weekly: u64,
    pub active_users_monthly: u64,
    pub active_apps_daily: u64,
    pub active_apps_weekly: u64,
    pub active_apps_monthly: u64,
    pub ai_users_monthly: u64,
    pub execution_users_monthly: u64,
    pub power_users_weekly: u64,
    pub power_users_monthly: u64,
    pub average_cost_per_active_user: Option<f64>,
}

#[derive(Clone, Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct AdminUsageTrendPoint {
    pub bucket: String,
    pub label: String,
    pub new_users: u64,
    pub active_users: u64,
    pub executions: u64,
    pub ai_invocations: u64,
    pub tokens: i64,
    pub cost: i64,
}

#[derive(Clone, Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct AdminPowerUser {
    pub user_id: String,
    pub display_name: Option<String>,
    pub email: Option<String>,
    pub total_price: i64,
    pub total_tokens: i64,
    pub ai_invocations: u64,
    pub executions: u64,
    pub total_interactions: u64,
    pub active_days: u64,
    pub last_seen: Option<String>,
}

#[derive(Clone, Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct AdminUsageOverview {
    pub period: String,
    pub started_at: String,
    pub totals: AdminUsageTotals,
    pub user_stats: AdminUserStats,
    pub trend: Vec<AdminUsageTrendPoint>,
    pub power_users: Vec<AdminPowerUser>,
    pub users: Vec<AdminUserUsage>,
    pub technical_users: Vec<AdminTechnicalUserUsage>,
    pub apps: Vec<AdminAppUsage>,
    pub models: Vec<AdminModelUsage>,
    /// Rate card behind every `computeCost` in this response
    pub compute_cost_model: ComputeCostModel,
}

#[derive(Clone, Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct AdminUsageInvocation {
    pub id: String,
    pub kind: String,
    pub status: String,
    pub user_id: Option<String>,
    pub technical_user_id: Option<String>,
    pub app_id: Option<String>,
    pub provider: Option<String>,
    pub endpoint: Option<String>,
    pub model_id: Option<String>,
    pub provider_request_id: Option<String>,
    pub estimated_tokens: i64,
    pub estimated_cost_micro_dollars: i64,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub embedding_tokens: i64,
    pub cost_micro_dollars: i64,
    pub latency: Option<f64>,
    pub error: Option<String>,
    pub started_at: String,
    pub completed_at: Option<String>,
}

impl From<usage_invocation::Model> for AdminUsageInvocation {
    fn from(row: usage_invocation::Model) -> Self {
        Self {
            id: row.id,
            kind: row.kind,
            status: row.status,
            user_id: row.user_id,
            technical_user_id: row.technical_user_id,
            app_id: row.app_id,
            provider: row.provider,
            endpoint: row.endpoint,
            model_id: row.model_id,
            provider_request_id: row.provider_request_id,
            estimated_tokens: row.estimated_tokens,
            estimated_cost_micro_dollars: row.estimated_cost_micro_dollars,
            input_tokens: row.input_tokens,
            output_tokens: row.output_tokens,
            embedding_tokens: row.embedding_tokens,
            cost_micro_dollars: row.cost_micro_dollars,
            latency: row.latency,
            error: row.error,
            started_at: row.started_at.to_rfc3339(),
            completed_at: row
                .completed_at
                .map(|completed_at| completed_at.to_rfc3339()),
        }
    }
}

#[derive(Clone, Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct AdminUsageAlert {
    pub id: String,
    pub kind: String,
    pub severity: String,
    pub period: Option<String>,
    pub message: String,
    pub app_id: Option<String>,
    pub user_id: Option<String>,
    pub threshold_percent: Option<i32>,
    pub current_cost_micro_dollars: Option<i64>,
    pub current_tokens: Option<i64>,
    pub acknowledged_at: Option<String>,
    pub created_at: String,
}

impl From<usage_alert::Model> for AdminUsageAlert {
    fn from(row: usage_alert::Model) -> Self {
        Self {
            id: row.id,
            kind: row.kind,
            severity: row.severity,
            period: row.period,
            message: row.message,
            app_id: row.app_id,
            user_id: row.user_id,
            threshold_percent: row.threshold_percent,
            current_cost_micro_dollars: row.current_cost_micro_dollars,
            current_tokens: row.current_tokens,
            acknowledged_at: row
                .acknowledged_at
                .map(|acknowledged_at| acknowledged_at.to_rfc3339()),
            created_at: row.created_at.to_rfc3339(),
        }
    }
}

#[derive(Clone, Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct AdminUsageAuditLog {
    pub id: String,
    pub app_id: Option<String>,
    pub user_id: Option<String>,
    pub actor_user_id: Option<String>,
    pub action: String,
    pub before: Option<flow_like_types::Value>,
    pub after: Option<flow_like_types::Value>,
    pub created_at: String,
}

impl From<usage_limit_audit_log::Model> for AdminUsageAuditLog {
    fn from(row: usage_limit_audit_log::Model) -> Self {
        Self {
            id: row.id,
            app_id: row.app_id,
            user_id: row.user_id,
            actor_user_id: row.actor_user_id,
            action: row.action,
            before: row.before,
            after: row.after,
            created_at: row.created_at.to_rfc3339(),
        }
    }
}

#[derive(Clone, Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct AdminPaginated<T> {
    pub items: Vec<T>,
    pub total: u64,
    pub page: u64,
    pub page_size: u64,
}

#[derive(Debug, FromQueryResult)]
struct PeriodAiUsageRow {
    bucket: String,
    user_id: Option<String>,
    technical_user_id: Option<String>,
    app_id: Option<String>,
    model_id: String,
    provider: Option<String>,
    endpoint: Option<String>,
    invocations: i64,
    price: i64,
    tokens: i64,
    latency_sum: Option<f64>,
    latency_count: i64,
}

#[derive(Debug, FromQueryResult)]
struct PeriodExecutionUsageRow {
    bucket: String,
    user_id: Option<String>,
    technical_user_id: Option<String>,
    app_id: Option<String>,
    executions: i64,
    microseconds: i64,
}

#[derive(Debug, FromQueryResult)]
struct NewUserBucketRow {
    bucket: String,
    new_users: i64,
}

#[derive(Debug, Default, FromQueryResult)]
struct UserCountsRow {
    total_users: i64,
    new_users_today: i64,
    new_users_weekly: i64,
    new_users_monthly: i64,
}

/// Usage of the last 30 days per UTC day, user and app. `interactions` counts
/// the whole group, the daily/weekly variants only its rows inside that window.
#[derive(Debug, FromQueryResult)]
struct RecentUsageRow {
    day: String,
    user_id: Option<String>,
    app_id: Option<String>,
    interactions: i64,
    daily_interactions: i64,
    weekly_interactions: i64,
    price: i64,
    tokens: i64,
    last_seen: DateTime<FixedOffset>,
}

#[derive(Clone, Copy)]
enum ActivityWindow {
    Daily,
    Weekly,
    Monthly,
}

impl RecentUsageRow {
    fn interactions_in(&self, window: ActivityWindow) -> u64 {
        to_count(match window {
            ActivityWindow::Daily => self.daily_interactions,
            ActivityWindow::Weekly => self.weekly_interactions,
            ActivityWindow::Monthly => self.interactions,
        })
    }
}

struct RecentUsage {
    llm: Vec<RecentUsageRow>,
    embedding: Vec<RecentUsageRow>,
    execution: Vec<RecentUsageRow>,
}

impl RecentUsage {
    fn ai_rows(&self) -> impl Iterator<Item = &RecentUsageRow> {
        self.llm.iter().chain(&self.embedding)
    }
}

#[derive(Default)]
struct PeriodUsage {
    totals: UsageAggregate,
    users: HashMap<Option<String>, UsageAggregate>,
    technical_users: HashMap<String, UsageAggregate>,
    apps: HashMap<Option<String>, UsageAggregate>,
    models: HashMap<(String, String, Option<String>, Option<String>), ModelAggregate>,
}

impl PeriodUsage {
    fn record(
        &mut self,
        user_id: &Option<String>,
        technical_user_id: &Option<String>,
        app_id: &Option<String>,
        add: impl Fn(&mut UsageAggregate),
    ) {
        add(&mut self.totals);
        add(self.users.entry(user_id.clone()).or_default());
        if let Some(technical_user_id) = technical_user_id {
            add(self
                .technical_users
                .entry(technical_user_id.clone())
                .or_default());
        }
        add(self.apps.entry(app_id.clone()).or_default());
    }

    fn record_model(&mut self, kind: &str, row: &PeriodAiUsageRow) {
        let model = self
            .models
            .entry((
                kind.to_string(),
                row.model_id.clone(),
                row.provider.clone(),
                row.endpoint.clone(),
            ))
            .or_default();
        model.price += row.price;
        model.tokens += row.tokens;
        model.invocations += to_count(row.invocations);
        model.latency_sum += row.latency_sum.unwrap_or(0.0);
        model.latency_count += to_count(row.latency_count);
    }
}

fn to_count(value: i64) -> u64 {
    u64::try_from(value).unwrap_or(0)
}

fn db_error(err: sea_orm::DbErr) -> ApiError {
    ApiError::internal_error(err.into())
}

#[derive(Clone, Copy)]
struct UsageColumns<C> {
    id: C,
    created_at: C,
    user_id: C,
    technical_user_id: C,
    app_id: C,
}

#[derive(Clone, Copy)]
struct AiUsageColumns<C> {
    usage: UsageColumns<C>,
    model_id: C,
    provider: C,
    endpoint: C,
    price: C,
    latency: C,
}

const LLM_COLUMNS: AiUsageColumns<llm_usage_tracking::Column> = AiUsageColumns {
    usage: UsageColumns {
        id: llm_usage_tracking::Column::Id,
        created_at: llm_usage_tracking::Column::CreatedAt,
        user_id: llm_usage_tracking::Column::UserId,
        technical_user_id: llm_usage_tracking::Column::TechnicalUserId,
        app_id: llm_usage_tracking::Column::AppId,
    },
    model_id: llm_usage_tracking::Column::ModelId,
    provider: llm_usage_tracking::Column::Provider,
    endpoint: llm_usage_tracking::Column::Endpoint,
    price: llm_usage_tracking::Column::Price,
    latency: llm_usage_tracking::Column::Latency,
};

const EMBEDDING_COLUMNS: AiUsageColumns<embedding_usage_tracking::Column> = AiUsageColumns {
    usage: UsageColumns {
        id: embedding_usage_tracking::Column::Id,
        created_at: embedding_usage_tracking::Column::CreatedAt,
        user_id: embedding_usage_tracking::Column::UserId,
        technical_user_id: embedding_usage_tracking::Column::TechnicalUserId,
        app_id: embedding_usage_tracking::Column::AppId,
    },
    model_id: embedding_usage_tracking::Column::ModelId,
    provider: embedding_usage_tracking::Column::Provider,
    endpoint: embedding_usage_tracking::Column::Endpoint,
    price: embedding_usage_tracking::Column::Price,
    latency: embedding_usage_tracking::Column::Latency,
};

const EXECUTION_COLUMNS: UsageColumns<execution_usage_tracking::Column> = UsageColumns {
    id: execution_usage_tracking::Column::Id,
    created_at: execution_usage_tracking::Column::CreatedAt,
    user_id: execution_usage_tracking::Column::UserId,
    technical_user_id: execution_usage_tracking::Column::TechnicalUserId,
    app_id: execution_usage_tracking::Column::AppId,
};

/// `CAST(COALESCE(SUM(<expr>), 0) AS BIGINT)`: `SUM` over a 64 bit column comes
/// back as NUMERIC/DECIMAL, which does not decode into `i64`.
fn sum_bigint(expr: SimpleExpr) -> SimpleExpr {
    use sea_orm::sea_query::ExprTrait;

    Expr::expr(Func::coalesce([
        Expr::from(Func::sum(expr)),
        Expr::val(0i64),
    ]))
    .cast_as(Alias::new("BIGINT"))
}

fn zero_bigint() -> SimpleExpr {
    use sea_orm::sea_query::ExprTrait;

    Expr::val(0i64).cast_as(Alias::new("BIGINT"))
}

fn llm_tokens() -> SimpleExpr {
    use sea_orm::sea_query::ExprTrait;

    Expr::col(llm_usage_tracking::Column::TokenIn)
        .add(Expr::col(llm_usage_tracking::Column::TokenOut))
}

fn embedding_tokens() -> SimpleExpr {
    Expr::col(embedding_usage_tracking::Column::TokenCount)
}

/// `COUNT(CASE WHEN <created_at> >= start THEN <id> END)`.
fn count_since<C: ColumnTrait>(id: C, created_at: C, start: DateTime<FixedOffset>) -> SimpleExpr {
    use sea_orm::sea_query::ExprTrait;

    Expr::expr(Expr::case(
        ColumnTrait::gte(&created_at, start),
        Expr::col(id),
    ))
    .count()
}

/// Usage since `started_at`, grouped by trend bucket, user, technical user and
/// app; callers add their own measures (and any further group keys).
fn period_usage_select<E: EntityTrait>(
    columns: UsageColumns<E::Column>,
    bucket: &SimpleExpr,
    started_at: DateTime<FixedOffset>,
) -> Select<E> {
    E::find()
        .select_only()
        .expr_as(bucket.clone(), "bucket")
        .column_as(columns.user_id, "user_id")
        .column_as(columns.technical_user_id, "technical_user_id")
        .column_as(columns.app_id, "app_id")
        .filter(columns.created_at.gte(started_at))
        .group_by(bucket.clone())
        .group_by(columns.user_id)
        .group_by(columns.technical_user_id)
        .group_by(columns.app_id)
}

fn ai_period_select<E: EntityTrait>(
    columns: AiUsageColumns<E::Column>,
    tokens: SimpleExpr,
    bucket: &SimpleExpr,
    started_at: DateTime<FixedOffset>,
) -> Select<E> {
    use sea_orm::sea_query::ExprTrait;

    period_usage_select::<E>(columns.usage, bucket, started_at)
        .column_as(columns.model_id, "model_id")
        .column_as(columns.provider, "provider")
        .column_as(columns.endpoint, "endpoint")
        .expr_as(Expr::col(columns.usage.id).count(), "invocations")
        .expr_as(sum_bigint(Expr::col(columns.price)), "price")
        .expr_as(sum_bigint(tokens), "tokens")
        .expr_as(Expr::col(columns.latency).sum(), "latency_sum")
        .expr_as(Expr::col(columns.latency).count(), "latency_count")
        .group_by(columns.model_id)
        .group_by(columns.provider)
        .group_by(columns.endpoint)
}

fn execution_period_select(
    bucket: &SimpleExpr,
    started_at: DateTime<FixedOffset>,
) -> Select<execution_usage_tracking::Entity> {
    use sea_orm::sea_query::ExprTrait;

    period_usage_select::<execution_usage_tracking::Entity>(EXECUTION_COLUMNS, bucket, started_at)
        .expr_as(Expr::col(EXECUTION_COLUMNS.id).count(), "executions")
        .expr_as(
            sum_bigint(Expr::col(execution_usage_tracking::Column::Microseconds)),
            "microseconds",
        )
}

fn new_users_select(
    bucket: &SimpleExpr,
    started_at: DateTime<FixedOffset>,
) -> Select<user::Entity> {
    use sea_orm::sea_query::ExprTrait;

    user::Entity::find()
        .select_only()
        .expr_as(bucket.clone(), "bucket")
        .expr_as(Expr::col(user::Column::Id).count(), "new_users")
        .filter(user::Column::CreatedAt.gte(started_at))
        .group_by(bucket.clone())
}

struct ActivityStarts {
    daily: DateTime<FixedOffset>,
    weekly: DateTime<FixedOffset>,
    monthly: DateTime<FixedOffset>,
}

fn user_counts_select(starts: &ActivityStarts) -> Select<user::Entity> {
    use sea_orm::sea_query::ExprTrait;

    let new_users = |start| count_since(user::Column::Id, user::Column::CreatedAt, start);
    user::Entity::find()
        .select_only()
        .expr_as(Expr::col(user::Column::Id).count(), "total_users")
        .expr_as(new_users(starts.daily), "new_users_today")
        .expr_as(new_users(starts.weekly), "new_users_weekly")
        .expr_as(new_users(starts.monthly), "new_users_monthly")
}

/// Usage of the last 30 days grouped by UTC day, user and app.
fn recent_usage_select<E: EntityTrait>(
    columns: UsageColumns<E::Column>,
    price: SimpleExpr,
    tokens: SimpleExpr,
    day: &SimpleExpr,
    starts: &ActivityStarts,
) -> Select<E> {
    use sea_orm::sea_query::ExprTrait;

    E::find()
        .select_only()
        .expr_as(day.clone(), "day")
        .column_as(columns.user_id, "user_id")
        .column_as(columns.app_id, "app_id")
        .expr_as(Expr::col(columns.id).count(), "interactions")
        .expr_as(
            count_since(columns.id, columns.created_at, starts.daily),
            "daily_interactions",
        )
        .expr_as(
            count_since(columns.id, columns.created_at, starts.weekly),
            "weekly_interactions",
        )
        .expr_as(price, "price")
        .expr_as(tokens, "tokens")
        .expr_as(Expr::col(columns.created_at).max(), "last_seen")
        .filter(ColumnTrait::gte(&columns.created_at, starts.monthly))
        .group_by(day.clone())
        .group_by(columns.user_id)
        .group_by(columns.app_id)
}

#[tracing::instrument(name = "GET /admin/usage/overview", skip_all)]
pub async fn overview(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Query(query): Query<UsageOverviewQuery>,
) -> Result<Json<AdminUsageOverview>, ApiError> {
    user.check_global_permission(&state, GlobalPermission::Admin)
        .await?;

    let period = query
        .period
        .as_deref()
        .and_then(normalize_period)
        .unwrap_or_else(|| MONTHLY.to_string());
    let started_at =
        period_start(&period).ok_or_else(|| ApiError::bad_request("Invalid period"))?;
    let now = Utc::now().fixed_offset();
    let starts = ActivityStarts {
        daily: now - Duration::days(1),
        weekly: now - Duration::days(7),
        monthly: now - Duration::days(30),
    };

    let backend = state.db.get_database_backend();
    let monthly_buckets = period == crate::usage_limits::YEARLY;
    let trend_period = if monthly_buckets {
        StatsPeriod::Month
    } else {
        StatsPeriod::Day
    };
    let trend_bucket = trend_period.bucket_expr(backend, "createdAt");
    let day_bucket = StatsPeriod::Day.bucket_expr(backend, "createdAt");

    let (llm_rows, embedding_rows, execution_rows, new_user_rows) = flow_like_types::tokio::join!(
        ai_period_select::<llm_usage_tracking::Entity>(
            LLM_COLUMNS,
            llm_tokens(),
            &trend_bucket,
            started_at,
        )
        .into_model::<PeriodAiUsageRow>()
        .all(&state.db),
        ai_period_select::<embedding_usage_tracking::Entity>(
            EMBEDDING_COLUMNS,
            embedding_tokens(),
            &trend_bucket,
            started_at,
        )
        .into_model::<PeriodAiUsageRow>()
        .all(&state.db),
        execution_period_select(&trend_bucket, started_at)
            .into_model::<PeriodExecutionUsageRow>()
            .all(&state.db),
        new_users_select(&trend_bucket, started_at)
            .into_model::<NewUserBucketRow>()
            .all(&state.db),
    );
    let llm_rows = llm_rows.map_err(db_error)?;
    let embedding_rows = embedding_rows.map_err(db_error)?;
    let execution_rows = execution_rows.map_err(db_error)?;
    let new_user_rows = new_user_rows.map_err(db_error)?;

    let (recent_llm_rows, recent_embedding_rows, recent_execution_rows, user_counts) = flow_like_types::tokio::join!(
        recent_usage_select::<llm_usage_tracking::Entity>(
            LLM_COLUMNS.usage,
            sum_bigint(Expr::col(LLM_COLUMNS.price)),
            sum_bigint(llm_tokens()),
            &day_bucket,
            &starts,
        )
        .into_model::<RecentUsageRow>()
        .all(&state.db),
        recent_usage_select::<embedding_usage_tracking::Entity>(
            EMBEDDING_COLUMNS.usage,
            sum_bigint(Expr::col(EMBEDDING_COLUMNS.price)),
            sum_bigint(embedding_tokens()),
            &day_bucket,
            &starts,
        )
        .into_model::<RecentUsageRow>()
        .all(&state.db),
        recent_usage_select::<execution_usage_tracking::Entity>(
            EXECUTION_COLUMNS,
            zero_bigint(),
            zero_bigint(),
            &day_bucket,
            &starts,
        )
        .into_model::<RecentUsageRow>()
        .all(&state.db),
        user_counts_select(&starts)
            .into_model::<UserCountsRow>()
            .one(&state.db),
    );
    let recent = RecentUsage {
        llm: recent_llm_rows.map_err(db_error)?,
        embedding: recent_embedding_rows.map_err(db_error)?,
        execution: recent_execution_rows.map_err(db_error)?,
    };
    let user_counts = user_counts.map_err(db_error)?.unwrap_or_default();

    let mut usage = PeriodUsage::default();
    for row in &llm_rows {
        usage.record(
            &row.user_id,
            &row.technical_user_id,
            &row.app_id,
            |totals| totals.add_llm(row),
        );
        usage.record_model("llm", row);
    }
    for row in &embedding_rows {
        usage.record(
            &row.user_id,
            &row.technical_user_id,
            &row.app_id,
            |totals| totals.add_embedding(row),
        );
        usage.record_model("embedding", row);
    }
    for row in &execution_rows {
        usage.record(
            &row.user_id,
            &row.technical_user_id,
            &row.app_id,
            |totals| totals.add_executions(row),
        );
    }
    let PeriodUsage {
        totals,
        users,
        technical_users,
        apps,
        models,
    } = usage;

    let user_stats = build_user_stats(&user_counts, &recent);
    let trend = build_usage_trend(
        monthly_buckets,
        started_at,
        now,
        &llm_rows,
        &embedding_rows,
        &execution_rows,
        &new_user_rows,
    );
    let power_users = rank_power_users(build_power_user_aggregates(&recent));

    let mut users: Vec<(Option<String>, UsageAggregate)> = users.into_iter().collect();
    users.sort_by_key(|(_, totals)| std::cmp::Reverse(totals.total_price()));
    users.truncate(10);

    let mut technical_users: Vec<(String, UsageAggregate)> = technical_users.into_iter().collect();
    technical_users.sort_by_key(|(_, totals)| {
        std::cmp::Reverse((
            totals.total_price(),
            totals.total_tokens(),
            totals.executions,
        ))
    });
    technical_users.truncate(10);

    let mut apps: Vec<AdminAppUsage> = apps
        .into_iter()
        .map(|(app_id, totals)| AdminAppUsage {
            app_id,
            app_name: None,
            totals: totals.into(),
            limits: None,
        })
        .collect();
    retain_top_apps_by_either_cost(&mut apps, 10);

    let technical_user_lookup = load_technical_users(
        &state,
        technical_users.iter().map(|(id, _)| id.clone()).collect(),
    )
    .await?;

    let mut user_ids: HashSet<String> = users.iter().filter_map(|(id, _)| id.clone()).collect();
    user_ids.extend(power_users.iter().map(|(id, _)| id.clone()));
    let mut app_ids: HashSet<String> = apps.iter().filter_map(|row| row.app_id.clone()).collect();
    for technical_user in technical_user_lookup.values() {
        if let Some(creator_user_id) = &technical_user.creator_user_id {
            user_ids.insert(creator_user_id.clone());
        }
        app_ids.insert(technical_user.app_id.clone());
    }

    let (user_lookup, app_names, app_limits, technical_user_limits) = flow_like_types::tokio::join!(
        load_users(&state, user_ids),
        load_app_names(&state, app_ids.clone()),
        load_app_limits(&state, app_ids),
        load_scoped_limits(
            &state,
            technical_user_lookup
                .keys()
                .cloned()
                .collect::<HashSet<_>>(),
        ),
    );
    let user_lookup = user_lookup?;
    let app_names = app_names?;
    let app_limits = app_limits?;
    let technical_user_limits = technical_user_limits?;
    let power_users = build_power_users(power_users, &user_lookup);

    let users: Vec<AdminUserUsage> = users
        .into_iter()
        .map(|(user_id, totals)| {
            let user_model = user_id.as_ref().and_then(|id| user_lookup.get(id));
            AdminUserUsage {
                user_id,
                display_name: user_model.and_then(|user| {
                    user.name
                        .clone()
                        .or_else(|| user.preferred_username.clone())
                        .or_else(|| user.username.clone())
                }),
                email: user_model.and_then(|user| user.email.clone()),
                totals: totals.into(),
            }
        })
        .collect();

    let technical_users: Vec<AdminTechnicalUserUsage> = technical_users
        .into_iter()
        .map(|(technical_user_id, totals)| {
            let technical_user = technical_user_lookup.get(&technical_user_id);
            let creator = technical_user
                .and_then(|technical_user| technical_user.creator_user_id.as_ref())
                .and_then(|creator_user_id| user_lookup.get(creator_user_id));
            let app_id = technical_user.map(|technical_user| technical_user.app_id.clone());
            let limits = technical_user_limits.get(&technical_user_id).cloned();
            AdminTechnicalUserUsage {
                technical_user_id,
                name: technical_user.map(|technical_user| technical_user.name.clone()),
                app_name: app_id.as_ref().and_then(|id| app_names.get(id).cloned()),
                app_id,
                creator_user_id: technical_user
                    .and_then(|technical_user| technical_user.creator_user_id.clone()),
                creator_membership_id: technical_user
                    .and_then(|technical_user| technical_user.creator_membership_id.clone()),
                creator_display_name: creator.and_then(|user| {
                    user.name
                        .clone()
                        .or_else(|| user.preferred_username.clone())
                        .or_else(|| user.username.clone())
                }),
                creator_email: creator.and_then(|user| user.email.clone()),
                limits,
                totals: totals.into(),
            }
        })
        .collect();

    for app in &mut apps {
        if let Some(app_id) = &app.app_id {
            app.app_name = app_names.get(app_id).cloned();
            app.limits = app_limits.get(app_id).cloned();
        }
    }

    let mut models: Vec<AdminModelUsage> = models
        .into_iter()
        .map(
            |((kind, model_id, provider, endpoint), totals)| AdminModelUsage {
                kind,
                model_id,
                provider,
                endpoint,
                price: totals.price,
                tokens: totals.tokens,
                invocations: totals.invocations,
                average_latency_ms: totals.average_latency_ms(),
            },
        )
        .collect();
    models.sort_by_key(|row| std::cmp::Reverse(row.price));
    models.truncate(10);

    Ok(Json(AdminUsageOverview {
        period,
        started_at: started_at.to_rfc3339(),
        totals: totals.into(),
        user_stats,
        trend,
        power_users,
        users,
        technical_users,
        apps,
        models,
        compute_cost_model: compute_cost_model(),
    }))
}

/// The dashboard ranks projects twice, by AI price and by estimated runtime
/// cost, so the response has to carry every row either ranking can show: an app
/// that burns compute without ever calling a model never enters the AI top ten,
/// and a model-heavy app never enters the runtime one. Rows come back ordered by
/// the two costs combined; the client re-orders them per ranking.
fn retain_top_apps_by_either_cost(apps: &mut Vec<AdminAppUsage>, limit: usize) {
    if apps.len() <= limit {
        apps.sort_by_key(|row| std::cmp::Reverse(row.totals.total_price + row.totals.compute_cost));
        return;
    }

    let mut keep: HashSet<Option<String>> = HashSet::new();
    for key in [
        |row: &AdminAppUsage| row.totals.total_price,
        |row: &AdminAppUsage| row.totals.compute_cost,
    ] {
        apps.sort_by_key(|row| std::cmp::Reverse(key(row)));
        keep.extend(apps.iter().take(limit).map(|row| row.app_id.clone()));
    }

    apps.retain(|row| keep.contains(&row.app_id));
    apps.sort_by_key(|row| std::cmp::Reverse(row.totals.total_price + row.totals.compute_cost));
}

#[tracing::instrument(name = "GET /admin/usage/apps/{app_id}/limits", skip(state, user))]
pub async fn get_limits(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(app_id): Path<String>,
) -> Result<Json<AppUsageLimits>, ApiError> {
    user.check_global_permission(&state, GlobalPermission::Admin)
        .await?;

    app::Entity::find_by_id(&app_id)
        .one(&state.db)
        .await?
        .ok_or(ApiError::NOT_FOUND)?;

    Ok(Json(
        get_app_usage_limits(&state.db, &app_id)
            .await
            .map_err(|e| ApiError::internal_error(e.into()))?,
    ))
}

#[tracing::instrument(
    name = "PUT /admin/usage/apps/{app_id}/limits",
    skip(state, user, limits)
)]
pub async fn put_limits(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(app_id): Path<String>,
    Json(limits): Json<AppUsageLimits>,
) -> Result<Json<AppUsageLimits>, ApiError> {
    user.check_global_permission(&state, GlobalPermission::Admin)
        .await?;

    app::Entity::find_by_id(&app_id)
        .one(&state.db)
        .await?
        .ok_or(ApiError::NOT_FOUND)?;

    let before = get_app_usage_limits(&state.db, &app_id)
        .await
        .map_err(|e| ApiError::internal_error(e.into()))?;
    let updated = set_app_usage_limits(&state.db, &app_id, limits)
        .await
        .map_err(|e| ApiError::internal_error(e.into()))?;
    let actor_user_id = user.executor_scoped_sub().ok();
    record_usage_limit_audit(
        &state.db,
        Some(&app_id),
        None,
        actor_user_id.as_deref(),
        "set_app_limits",
        flow_like_types::json::to_value(before).ok(),
        flow_like_types::json::to_value(&updated).ok(),
    )
    .await
    .map_err(|e| ApiError::internal_error(e.into()))?;

    Ok(Json(updated))
}

#[tracing::instrument(
    name = "GET /admin/usage/apps/{app_id}/technical-users/{technical_user_id}/limits",
    skip(state, user, technical_user_id)
)]
pub async fn get_technical_user_limits(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((app_id, technical_user_id)): Path<(String, String)>,
) -> Result<Json<AppUsageLimits>, ApiError> {
    user.check_global_permission(&state, GlobalPermission::Admin)
        .await?;

    technical_user::Entity::find_by_id(&technical_user_id)
        .filter(technical_user::Column::AppId.eq(&app_id))
        .one(&state.db)
        .await?
        .ok_or(ApiError::NOT_FOUND)?;

    Ok(Json(
        get_app_usage_limits_for_scope(&state.db, &app_id, &technical_user_id)
            .await
            .map_err(|e| ApiError::internal_error(e.into()))?,
    ))
}

#[tracing::instrument(
    name = "PUT /admin/usage/apps/{app_id}/technical-users/{technical_user_id}/limits",
    skip(state, user, limits, technical_user_id)
)]
pub async fn put_technical_user_limits(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((app_id, technical_user_id)): Path<(String, String)>,
    Json(limits): Json<AppUsageLimits>,
) -> Result<Json<AppUsageLimits>, ApiError> {
    user.check_global_permission(&state, GlobalPermission::Admin)
        .await?;

    technical_user::Entity::find_by_id(&technical_user_id)
        .filter(technical_user::Column::AppId.eq(&app_id))
        .one(&state.db)
        .await?
        .ok_or(ApiError::NOT_FOUND)?;

    let before = get_app_usage_limits_for_scope(&state.db, &app_id, &technical_user_id)
        .await
        .map_err(|e| ApiError::internal_error(e.into()))?;
    let updated = set_app_usage_limits_for_scope(&state.db, &app_id, &technical_user_id, limits)
        .await
        .map_err(|e| ApiError::internal_error(e.into()))?;
    let actor_user_id = user.executor_scoped_sub().ok();
    record_usage_limit_audit(
        &state.db,
        Some(&app_id),
        Some(&technical_user_id),
        actor_user_id.as_deref(),
        "set_technical_user_limits",
        flow_like_types::json::to_value(before).ok(),
        flow_like_types::json::to_value(&updated).ok(),
    )
    .await
    .map_err(|e| ApiError::internal_error(e.into()))?;

    Ok(Json(updated))
}

#[tracing::instrument(name = "GET /admin/usage/invocations", skip_all)]
pub async fn invocations(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Query(query): Query<UsageListQuery>,
) -> Result<Json<AdminPaginated<AdminUsageInvocation>>, ApiError> {
    user.check_global_permission(&state, GlobalPermission::Admin)
        .await?;

    let (page, page_size) = paging(query.page, query.page_size);
    let mut select = usage_invocation::Entity::find();
    if let Some(start) = query
        .period
        .as_deref()
        .and_then(normalize_period)
        .and_then(|period| period_start(&period))
    {
        select = select.filter(usage_invocation::Column::CreatedAt.gte(start));
    }
    if let Some(app_id) = query.app_id.as_deref().filter(|value| !value.is_empty()) {
        select = select.filter(usage_invocation::Column::AppId.eq(app_id));
    }
    if let Some(user_id) = query.user_id.as_deref().filter(|value| !value.is_empty()) {
        select = select.filter(usage_invocation::Column::UserId.eq(user_id));
    }
    if let Some(technical_user_id) = query
        .technical_user_id
        .as_deref()
        .filter(|value| !value.is_empty())
    {
        select = select.filter(usage_invocation::Column::TechnicalUserId.eq(technical_user_id));
    }
    if let Some(status) = query.status.as_deref().filter(|value| !value.is_empty()) {
        select = select.filter(usage_invocation::Column::Status.eq(status));
    }

    let paginator = select
        .order_by_desc(usage_invocation::Column::CreatedAt)
        .paginate(&state.db, page_size);
    let total = paginator
        .num_items()
        .await
        .map_err(|e| ApiError::internal_error(e.into()))?;
    let items = paginator
        .fetch_page(page.saturating_sub(1))
        .await
        .map_err(|e| ApiError::internal_error(e.into()))?
        .into_iter()
        .map(AdminUsageInvocation::from)
        .collect();

    Ok(Json(AdminPaginated {
        items,
        total,
        page,
        page_size,
    }))
}

#[tracing::instrument(name = "POST /admin/usage/reconcile", skip_all)]
pub async fn reconcile(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Query(query): Query<UsageReconcileQuery>,
) -> Result<Json<UsageReconciliationResult>, ApiError> {
    user.check_global_permission(&state, GlobalPermission::Admin)
        .await?;

    Ok(Json(
        reconcile_hosted_invocations(&state, query.older_than_minutes.unwrap_or(30))
            .await
            .map_err(|e| ApiError::internal_error(e.into()))?,
    ))
}

#[tracing::instrument(name = "GET /admin/usage/alerts", skip_all)]
pub async fn alerts(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Query(query): Query<UsageListQuery>,
) -> Result<Json<AdminPaginated<AdminUsageAlert>>, ApiError> {
    user.check_global_permission(&state, GlobalPermission::Admin)
        .await?;

    let (page, page_size) = paging(query.page, query.page_size);
    let mut select = usage_alert::Entity::find();
    if let Some(app_id) = query.app_id.as_deref().filter(|value| !value.is_empty()) {
        select = select.filter(usage_alert::Column::AppId.eq(app_id));
    }
    if let Some(user_id) = query.user_id.as_deref().filter(|value| !value.is_empty()) {
        select = select.filter(usage_alert::Column::UserId.eq(user_id));
    }
    let paginator = select
        .order_by_desc(usage_alert::Column::CreatedAt)
        .paginate(&state.db, page_size);
    let total = paginator
        .num_items()
        .await
        .map_err(|e| ApiError::internal_error(e.into()))?;
    let items = paginator
        .fetch_page(page.saturating_sub(1))
        .await
        .map_err(|e| ApiError::internal_error(e.into()))?
        .into_iter()
        .map(AdminUsageAlert::from)
        .collect();

    Ok(Json(AdminPaginated {
        items,
        total,
        page,
        page_size,
    }))
}

#[tracing::instrument(name = "POST /admin/usage/alerts/{alert_id}/ack", skip(state, user))]
pub async fn acknowledge_alert(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(alert_id): Path<String>,
) -> Result<Json<AdminUsageAlert>, ApiError> {
    user.check_global_permission(&state, GlobalPermission::Admin)
        .await?;

    let existing = usage_alert::Entity::find_by_id(&alert_id)
        .one(&state.db)
        .await?
        .ok_or(ApiError::NOT_FOUND)?;
    let actor_user_id = user.executor_scoped_sub().ok();
    let now = Utc::now().fixed_offset();
    let mut active: usage_alert::ActiveModel = existing.into();
    active.acknowledged_at = Set(Some(now));
    active.acknowledged_by_user_id = Set(actor_user_id);
    active.updated_at = Set(now);
    let updated = active
        .update(&state.db)
        .await
        .map_err(|e| ApiError::internal_error(e.into()))?;

    Ok(Json(updated.into()))
}

#[tracing::instrument(name = "GET /admin/usage/audit", skip_all)]
pub async fn audit(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Query(query): Query<UsageListQuery>,
) -> Result<Json<AdminPaginated<AdminUsageAuditLog>>, ApiError> {
    user.check_global_permission(&state, GlobalPermission::Admin)
        .await?;

    let (page, page_size) = paging(query.page, query.page_size);
    let mut select = usage_limit_audit_log::Entity::find();
    if let Some(app_id) = query.app_id.as_deref().filter(|value| !value.is_empty()) {
        select = select.filter(usage_limit_audit_log::Column::AppId.eq(app_id));
    }
    if let Some(user_id) = query.user_id.as_deref().filter(|value| !value.is_empty()) {
        select = select.filter(usage_limit_audit_log::Column::UserId.eq(user_id));
    }
    let paginator = select
        .order_by_desc(usage_limit_audit_log::Column::CreatedAt)
        .paginate(&state.db, page_size);
    let total = paginator
        .num_items()
        .await
        .map_err(|e| ApiError::internal_error(e.into()))?;
    let items = paginator
        .fetch_page(page.saturating_sub(1))
        .await
        .map_err(|e| ApiError::internal_error(e.into()))?
        .into_iter()
        .map(AdminUsageAuditLog::from)
        .collect();

    Ok(Json(AdminPaginated {
        items,
        total,
        page,
        page_size,
    }))
}

fn paging(page: Option<u64>, page_size: Option<u64>) -> (u64, u64) {
    let page = page.unwrap_or(1).max(1);
    let page_size = page_size.unwrap_or(25).clamp(1, 100);
    (page, page_size)
}

fn build_user_stats(user_counts: &UserCountsRow, recent: &RecentUsage) -> AdminUserStats {
    let daily = build_activity_sets(recent, ActivityWindow::Daily);
    let weekly = build_activity_sets(recent, ActivityWindow::Weekly);
    let monthly = build_activity_sets(recent, ActivityWindow::Monthly);

    let monthly_cost: i64 = recent.ai_rows().map(|row| row.price).sum();
    let average_cost_per_active_user = if monthly.active_users.is_empty() {
        None
    } else {
        Some(monthly_cost as f64 / monthly.active_users.len() as f64 / 1_000_000.0)
    };

    AdminUserStats {
        total_users: to_count(user_counts.total_users),
        new_users_today: to_count(user_counts.new_users_today),
        new_users_weekly: to_count(user_counts.new_users_weekly),
        new_users_monthly: to_count(user_counts.new_users_monthly),
        active_users_daily: daily.active_users.len() as u64,
        active_users_weekly: weekly.active_users.len() as u64,
        active_users_monthly: monthly.active_users.len() as u64,
        active_apps_daily: daily.active_apps.len() as u64,
        active_apps_weekly: weekly.active_apps.len() as u64,
        active_apps_monthly: monthly.active_apps.len() as u64,
        ai_users_monthly: monthly.ai_users.len() as u64,
        execution_users_monthly: monthly.execution_users.len() as u64,
        power_users_weekly: weekly
            .user_interactions
            .values()
            .filter(|count| **count >= 10)
            .count() as u64,
        power_users_monthly: monthly
            .user_interactions
            .values()
            .filter(|count| **count >= 25)
            .count() as u64,
        average_cost_per_active_user,
    }
}

#[derive(Default)]
struct ActivitySets {
    active_users: HashSet<String>,
    active_apps: HashSet<String>,
    ai_users: HashSet<String>,
    execution_users: HashSet<String>,
    user_interactions: HashMap<String, u64>,
}

impl ActivitySets {
    fn record(&mut self, row: &RecentUsageRow, window: ActivityWindow, ai: bool) {
        let interactions = row.interactions_in(window);
        if interactions == 0 {
            return;
        }
        if let Some(user_id) = &row.user_id {
            self.active_users.insert(user_id.clone());
            if ai {
                self.ai_users.insert(user_id.clone());
            } else {
                self.execution_users.insert(user_id.clone());
            }
            *self.user_interactions.entry(user_id.clone()).or_default() += interactions;
        }
        if let Some(app_id) = &row.app_id {
            self.active_apps.insert(app_id.clone());
        }
    }
}

fn build_activity_sets(recent: &RecentUsage, window: ActivityWindow) -> ActivitySets {
    let mut activity = ActivitySets::default();
    for row in recent.ai_rows() {
        activity.record(row, window, true);
    }
    for row in &recent.execution {
        activity.record(row, window, false);
    }
    activity
}

fn build_usage_trend(
    monthly_buckets: bool,
    started_at: DateTime<FixedOffset>,
    now: DateTime<FixedOffset>,
    llm_rows: &[PeriodAiUsageRow],
    embedding_rows: &[PeriodAiUsageRow],
    execution_rows: &[PeriodExecutionUsageRow],
    new_user_rows: &[NewUserBucketRow],
) -> Vec<AdminUsageTrendPoint> {
    let mut buckets =
        seed_trend_buckets(started_at.date_naive(), now.date_naive(), monthly_buckets);

    for row in new_user_rows {
        let key = trend_key(&row.bucket, monthly_buckets);
        buckets.entry(key).or_default().new_users += to_count(row.new_users);
    }

    for row in llm_rows.iter().chain(embedding_rows) {
        let key = trend_key(&row.bucket, monthly_buckets);
        let bucket = buckets.entry(key).or_default();
        bucket.ai_invocations += to_count(row.invocations);
        bucket.tokens += row.tokens;
        bucket.cost += row.price;
        if let Some(user_id) = &row.user_id {
            bucket.active_users.insert(user_id.clone());
        }
    }

    for row in execution_rows {
        let key = trend_key(&row.bucket, monthly_buckets);
        let bucket = buckets.entry(key).or_default();
        bucket.executions += to_count(row.executions);
        if let Some(user_id) = &row.user_id {
            bucket.active_users.insert(user_id.clone());
        }
    }

    let mut points: Vec<_> = buckets
        .into_iter()
        .map(|(bucket, totals)| AdminUsageTrendPoint {
            label: trend_label(&bucket, monthly_buckets),
            bucket,
            new_users: totals.new_users,
            active_users: totals.active_users.len() as u64,
            executions: totals.executions,
            ai_invocations: totals.ai_invocations,
            tokens: totals.tokens,
            cost: totals.cost,
        })
        .collect();
    points.sort_by(|a, b| a.bucket.cmp(&b.bucket));
    points
}

fn seed_trend_buckets(
    start: NaiveDate,
    end: NaiveDate,
    monthly_buckets: bool,
) -> HashMap<String, TrendBucket> {
    let mut buckets = HashMap::new();
    if monthly_buckets {
        let mut year = start.year();
        let mut month = start.month();
        loop {
            let key = format!("{year:04}-{month:02}");
            buckets.entry(key).or_default();
            if year == end.year() && month == end.month() {
                break;
            }
            month += 1;
            if month > 12 {
                month = 1;
                year += 1;
            }
        }
        return buckets;
    }

    let mut day = start;
    while day <= end {
        buckets
            .entry(day.format("%Y-%m-%d").to_string())
            .or_default();
        day += Duration::days(1);
    }
    buckets
}

/// SQL buckets arrive as `YYYY-MM-DD` (the first of the month for monthly
/// buckets); the monthly trend key is the `YYYY-MM` prefix.
fn trend_key(bucket: &str, monthly_buckets: bool) -> String {
    if monthly_buckets {
        bucket.get(..7).unwrap_or(bucket).to_string()
    } else {
        bucket.to_string()
    }
}

fn trend_label(bucket: &str, monthly_buckets: bool) -> String {
    if monthly_buckets {
        return bucket.to_string();
    }
    NaiveDate::parse_from_str(bucket, "%Y-%m-%d")
        .map(|date| date.format("%b %-d").to_string())
        .unwrap_or_else(|_| bucket.to_string())
}

fn build_power_user_aggregates(recent: &RecentUsage) -> HashMap<String, PowerUserAggregate> {
    let mut users = HashMap::<String, PowerUserAggregate>::new();

    for row in recent.ai_rows() {
        let Some(user_id) = &row.user_id else {
            continue;
        };
        let entry = users.entry(user_id.clone()).or_default();
        entry.total_price += row.price;
        entry.total_tokens += row.tokens;
        entry.ai_invocations += to_count(row.interactions);
        entry.touch(&row.day, row.last_seen);
    }

    for row in &recent.execution {
        let Some(user_id) = &row.user_id else {
            continue;
        };
        let entry = users.entry(user_id.clone()).or_default();
        entry.executions += to_count(row.interactions);
        entry.touch(&row.day, row.last_seen);
    }

    users
}

fn rank_power_users(
    aggregates: HashMap<String, PowerUserAggregate>,
) -> Vec<(String, PowerUserAggregate)> {
    let mut ranked: Vec<_> = aggregates.into_iter().collect();
    ranked.sort_by_key(|(_, totals)| {
        std::cmp::Reverse((
            totals.total_interactions(),
            totals.active_days.len() as u64,
            totals.total_tokens,
            totals.total_price,
        ))
    });
    ranked.truncate(8);
    ranked
}

fn build_power_users(
    ranked: Vec<(String, PowerUserAggregate)>,
    user_lookup: &HashMap<String, user::Model>,
) -> Vec<AdminPowerUser> {
    ranked
        .into_iter()
        .map(|(user_id, totals)| {
            let user_model = user_lookup.get(&user_id);
            AdminPowerUser {
                user_id,
                display_name: user_model.and_then(|user| {
                    user.name
                        .clone()
                        .or_else(|| user.preferred_username.clone())
                        .or_else(|| user.username.clone())
                }),
                email: user_model.and_then(|user| user.email.clone()),
                total_price: totals.total_price,
                total_tokens: totals.total_tokens,
                ai_invocations: totals.ai_invocations,
                executions: totals.executions,
                total_interactions: totals.total_interactions(),
                active_days: totals.active_days.len() as u64,
                last_seen: totals.last_seen.map(|last_seen| last_seen.to_rfc3339()),
            }
        })
        .collect()
}

async fn load_users(
    state: &AppState,
    user_ids: HashSet<String>,
) -> Result<HashMap<String, user::Model>, ApiError> {
    if user_ids.is_empty() {
        return Ok(HashMap::new());
    }
    let user_ids: Vec<String> = user_ids.into_iter().collect();

    let rows = user::Entity::find()
        .filter(user::Column::Id.is_in(user_ids))
        .all(&state.db)
        .await
        .map_err(|e| ApiError::internal_error(e.into()))?;
    Ok(rows.into_iter().map(|row| (row.id.clone(), row)).collect())
}

async fn load_technical_users(
    state: &AppState,
    technical_user_ids: HashSet<String>,
) -> Result<HashMap<String, technical_user::Model>, ApiError> {
    if technical_user_ids.is_empty() {
        return Ok(HashMap::new());
    }
    let technical_user_ids: Vec<String> = technical_user_ids.into_iter().collect();

    let rows = technical_user::Entity::find()
        .filter(technical_user::Column::Id.is_in(technical_user_ids))
        .all(&state.db)
        .await
        .map_err(|e| ApiError::internal_error(e.into()))?;
    Ok(rows.into_iter().map(|row| (row.id.clone(), row)).collect())
}

async fn load_app_names(
    state: &AppState,
    app_ids: HashSet<String>,
) -> Result<HashMap<String, String>, ApiError> {
    if app_ids.is_empty() {
        return Ok(HashMap::new());
    }
    let app_ids: Vec<String> = app_ids.into_iter().collect();

    let metas: Vec<(Option<String>, String, String)> = meta::Entity::find()
        .select_only()
        .column(meta::Column::AppId)
        .column(meta::Column::Lang)
        .column(meta::Column::Name)
        .filter(meta::Column::AppId.is_in(app_ids))
        .into_tuple()
        .all(&state.db)
        .await
        .map_err(db_error)?;

    let mut names = HashMap::new();
    for (app_id, lang, name) in metas {
        let Some(app_id) = app_id else {
            continue;
        };
        if lang == "en" || !names.contains_key(&app_id) {
            names.insert(app_id, name);
        }
    }
    Ok(names)
}

async fn load_app_limits(
    state: &AppState,
    app_ids: HashSet<String>,
) -> Result<HashMap<String, AppUsageLimits>, ApiError> {
    if app_ids.is_empty() {
        return Ok(HashMap::new());
    }
    let app_ids: Vec<String> = app_ids.into_iter().collect();

    let rows = app_usage_limit::Entity::find()
        .filter(app_usage_limit::Column::AppId.is_in(app_ids))
        .filter(app_usage_limit::Column::UserId.eq(""))
        .all(&state.db)
        .await
        .map_err(|e| ApiError::internal_error(e.into()))?;

    let mut grouped: HashMap<String, Vec<app_usage_limit::Model>> = HashMap::new();
    for row in rows {
        grouped.entry(row.app_id.clone()).or_default().push(row);
    }

    Ok(grouped
        .into_iter()
        .map(|(app_id, rows)| (app_id, AppUsageLimits::from_rows(rows)))
        .collect())
}

async fn load_scoped_limits(
    state: &AppState,
    scoped_user_ids: HashSet<String>,
) -> Result<HashMap<String, AppUsageLimits>, ApiError> {
    if scoped_user_ids.is_empty() {
        return Ok(HashMap::new());
    }
    let scoped_user_ids: Vec<String> = scoped_user_ids.into_iter().collect();

    let rows = app_usage_limit::Entity::find()
        .filter(app_usage_limit::Column::UserId.is_in(scoped_user_ids))
        .all(&state.db)
        .await
        .map_err(|e| ApiError::internal_error(e.into()))?;

    let mut grouped: HashMap<String, Vec<app_usage_limit::Model>> = HashMap::new();
    for row in rows {
        grouped.entry(row.user_id.clone()).or_default().push(row);
    }

    Ok(grouped
        .into_iter()
        .map(|(user_id, rows)| (user_id, AppUsageLimits::from_rows(rows)))
        .collect())
}
