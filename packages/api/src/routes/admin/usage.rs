use crate::{
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
        UsageReconciliationResult, reconcile_stale_invocations, record_usage_limit_audit,
    },
    usage_limits::{
        AppUsageLimits, MONTHLY, get_app_usage_limits, get_app_usage_limits_for_scope,
        normalize_period, period_start, set_app_usage_limits, set_app_usage_limits_for_scope,
    },
};
use axum::{
    Extension, Json,
    extract::{Path, Query, State},
};
use chrono::{Datelike, Duration, NaiveDate, NaiveDateTime, Utc};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, EntityTrait, PaginatorTrait, QueryFilter, QueryOrder, Set,
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
    fn total_price(&self) -> i64 {
        self.llm_price + self.embedding_price
    }

    fn total_tokens(&self) -> i64 {
        self.llm_tokens + self.embedding_tokens
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
    last_seen: Option<NaiveDateTime>,
}

impl PowerUserAggregate {
    fn total_interactions(&self) -> u64 {
        self.ai_invocations + self.executions
    }

    fn touch(&mut self, created_at: NaiveDateTime) {
        self.active_days
            .insert(created_at.date().format("%Y-%m-%d").to_string());
        self.last_seen = Some(match self.last_seen {
            Some(last_seen) => last_seen.max(created_at),
            None => created_at,
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
            started_at: row.started_at.and_utc().to_rfc3339(),
            completed_at: row
                .completed_at
                .map(|completed_at| completed_at.and_utc().to_rfc3339()),
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
                .map(|acknowledged_at| acknowledged_at.and_utc().to_rfc3339()),
            created_at: row.created_at.and_utc().to_rfc3339(),
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
            created_at: row.created_at.and_utc().to_rfc3339(),
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
    let now = Utc::now().naive_utc();
    let daily_start = now - Duration::days(1);
    let weekly_start = now - Duration::days(7);
    let monthly_start = now - Duration::days(30);

    let llm_rows = llm_usage_tracking::Entity::find()
        .filter(llm_usage_tracking::Column::CreatedAt.gte(started_at))
        .all(&state.db)
        .await
        .map_err(|e| ApiError::internal_error(e.into()))?;
    let embedding_rows = embedding_usage_tracking::Entity::find()
        .filter(embedding_usage_tracking::Column::CreatedAt.gte(started_at))
        .all(&state.db)
        .await
        .map_err(|e| ApiError::internal_error(e.into()))?;
    let execution_rows = execution_usage_tracking::Entity::find()
        .filter(execution_usage_tracking::Column::CreatedAt.gte(started_at))
        .all(&state.db)
        .await
        .map_err(|e| ApiError::internal_error(e.into()))?;
    let new_user_rows = user::Entity::find()
        .filter(user::Column::CreatedAt.gte(started_at))
        .all(&state.db)
        .await
        .map_err(|e| ApiError::internal_error(e.into()))?;

    let recent_llm_rows = llm_usage_tracking::Entity::find()
        .filter(llm_usage_tracking::Column::CreatedAt.gte(monthly_start))
        .all(&state.db)
        .await
        .map_err(|e| ApiError::internal_error(e.into()))?;
    let recent_embedding_rows = embedding_usage_tracking::Entity::find()
        .filter(embedding_usage_tracking::Column::CreatedAt.gte(monthly_start))
        .all(&state.db)
        .await
        .map_err(|e| ApiError::internal_error(e.into()))?;
    let recent_execution_rows = execution_usage_tracking::Entity::find()
        .filter(execution_usage_tracking::Column::CreatedAt.gte(monthly_start))
        .all(&state.db)
        .await
        .map_err(|e| ApiError::internal_error(e.into()))?;

    let mut totals = UsageAggregate::default();
    let mut users: HashMap<Option<String>, UsageAggregate> = HashMap::new();
    let mut technical_users: HashMap<String, UsageAggregate> = HashMap::new();
    let mut apps: HashMap<Option<String>, UsageAggregate> = HashMap::new();
    let mut models: HashMap<(String, String, Option<String>, Option<String>), ModelAggregate> =
        HashMap::new();
    let mut user_ids = HashSet::new();
    let mut technical_user_ids = HashSet::new();
    let mut app_ids = HashSet::new();

    for row in &llm_rows {
        let tokens = row.token_in + row.token_out;
        totals.llm_price += row.price;
        totals.llm_tokens += tokens;
        totals.llm_invocations += 1;

        if let Some(user_id) = &row.user_id {
            user_ids.insert(user_id.clone());
        }
        if let Some(app_id) = &row.app_id {
            app_ids.insert(app_id.clone());
        }

        let user_total = users.entry(row.user_id.clone()).or_default();
        user_total.llm_price += row.price;
        user_total.llm_tokens += tokens;
        user_total.llm_invocations += 1;

        if let Some(technical_user_id) = &row.technical_user_id {
            technical_user_ids.insert(technical_user_id.clone());
            let technical_total = technical_users
                .entry(technical_user_id.clone())
                .or_default();
            technical_total.llm_price += row.price;
            technical_total.llm_tokens += tokens;
            technical_total.llm_invocations += 1;
        }

        let app_total = apps.entry(row.app_id.clone()).or_default();
        app_total.llm_price += row.price;
        app_total.llm_tokens += tokens;
        app_total.llm_invocations += 1;

        let model_total = models
            .entry((
                "llm".to_string(),
                row.model_id.clone(),
                row.provider.clone(),
                row.endpoint.clone(),
            ))
            .or_default();
        model_total.price += row.price;
        model_total.tokens += tokens;
        model_total.invocations += 1;
        if let Some(latency) = row.latency {
            model_total.latency_sum += latency;
            model_total.latency_count += 1;
        }
    }

    for row in &embedding_rows {
        totals.embedding_price += row.price;
        totals.embedding_tokens += row.token_count;
        totals.embedding_invocations += 1;

        if let Some(user_id) = &row.user_id {
            user_ids.insert(user_id.clone());
        }
        if let Some(app_id) = &row.app_id {
            app_ids.insert(app_id.clone());
        }

        let user_total = users.entry(row.user_id.clone()).or_default();
        user_total.embedding_price += row.price;
        user_total.embedding_tokens += row.token_count;
        user_total.embedding_invocations += 1;

        if let Some(technical_user_id) = &row.technical_user_id {
            technical_user_ids.insert(technical_user_id.clone());
            let technical_total = technical_users
                .entry(technical_user_id.clone())
                .or_default();
            technical_total.embedding_price += row.price;
            technical_total.embedding_tokens += row.token_count;
            technical_total.embedding_invocations += 1;
        }

        let app_total = apps.entry(row.app_id.clone()).or_default();
        app_total.embedding_price += row.price;
        app_total.embedding_tokens += row.token_count;
        app_total.embedding_invocations += 1;

        let model_total = models
            .entry((
                "embedding".to_string(),
                row.model_id.clone(),
                row.provider.clone(),
                row.endpoint.clone(),
            ))
            .or_default();
        model_total.price += row.price;
        model_total.tokens += row.token_count;
        model_total.invocations += 1;
        if let Some(latency) = row.latency {
            model_total.latency_sum += latency;
            model_total.latency_count += 1;
        }
    }

    for row in &execution_rows {
        totals.executions += 1;
        totals.execution_microseconds += row.microseconds;

        if let Some(user_id) = &row.user_id {
            user_ids.insert(user_id.clone());
        }
        if let Some(app_id) = &row.app_id {
            app_ids.insert(app_id.clone());
        }

        let user_total = users.entry(row.user_id.clone()).or_default();
        user_total.executions += 1;
        user_total.execution_microseconds += row.microseconds;

        if let Some(technical_user_id) = &row.technical_user_id {
            technical_user_ids.insert(technical_user_id.clone());
            let technical_total = technical_users
                .entry(technical_user_id.clone())
                .or_default();
            technical_total.executions += 1;
            technical_total.execution_microseconds += row.microseconds;
        }

        let app_total = apps.entry(row.app_id.clone()).or_default();
        app_total.executions += 1;
        app_total.execution_microseconds += row.microseconds;
    }

    let user_stats = build_user_stats(
        &state,
        &recent_llm_rows,
        &recent_embedding_rows,
        &recent_execution_rows,
        daily_start,
        weekly_start,
        monthly_start,
    )
    .await?;
    let trend = build_usage_trend(
        &period,
        started_at,
        now,
        &llm_rows,
        &embedding_rows,
        &execution_rows,
        &new_user_rows,
    );
    let power_aggregates = build_power_user_aggregates(
        &recent_llm_rows,
        &recent_embedding_rows,
        &recent_execution_rows,
        monthly_start,
    );
    for user_id in power_aggregates.keys() {
        user_ids.insert(user_id.clone());
    }

    let technical_user_lookup = load_technical_users(&state, technical_user_ids).await?;
    for technical_user in technical_user_lookup.values() {
        if let Some(creator_user_id) = &technical_user.creator_user_id {
            user_ids.insert(creator_user_id.clone());
        }
        app_ids.insert(technical_user.app_id.clone());
    }

    let user_lookup = load_users(&state, user_ids).await?;
    let app_names = load_app_names(&state, app_ids.clone()).await?;
    let app_limits = load_app_limits(&state, app_ids).await?;
    let technical_user_limits = load_scoped_limits(
        &state,
        technical_user_lookup
            .keys()
            .cloned()
            .collect::<HashSet<_>>(),
    )
    .await?;
    let power_users = build_power_users(power_aggregates, &user_lookup);

    let mut users: Vec<AdminUserUsage> = users
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
    users.sort_by_key(|row| std::cmp::Reverse(row.totals.total_price));
    users.truncate(10);

    let mut technical_users: Vec<AdminTechnicalUserUsage> = technical_users
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
    technical_users.sort_by_key(|row| {
        std::cmp::Reverse((
            row.totals.total_price,
            row.totals.total_tokens,
            row.totals.executions,
        ))
    });
    technical_users.truncate(10);

    let mut apps: Vec<AdminAppUsage> = apps
        .into_iter()
        .map(|(app_id, totals)| AdminAppUsage {
            app_name: app_id.as_ref().and_then(|id| app_names.get(id).cloned()),
            limits: app_id.as_ref().and_then(|id| app_limits.get(id).cloned()),
            app_id,
            totals: totals.into(),
        })
        .collect();
    apps.sort_by_key(|row| std::cmp::Reverse(row.totals.total_price));
    apps.truncate(10);

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
        started_at: started_at.and_utc().to_rfc3339(),
        totals: totals.into(),
        user_stats,
        trend,
        power_users,
        users,
        technical_users,
        apps,
        models,
    }))
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
        reconcile_stale_invocations(&state.db, query.older_than_minutes.unwrap_or(30))
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
    let now = Utc::now().naive_utc();
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

async fn build_user_stats(
    state: &AppState,
    llm_rows: &[llm_usage_tracking::Model],
    embedding_rows: &[embedding_usage_tracking::Model],
    execution_rows: &[execution_usage_tracking::Model],
    daily_start: NaiveDateTime,
    weekly_start: NaiveDateTime,
    monthly_start: NaiveDateTime,
) -> Result<AdminUserStats, ApiError> {
    let total_users = user::Entity::find()
        .count(&state.db)
        .await
        .map_err(|e| ApiError::internal_error(e.into()))?;
    let new_users_today = count_new_users_since(state, daily_start).await?;
    let new_users_weekly = count_new_users_since(state, weekly_start).await?;
    let new_users_monthly = count_new_users_since(state, monthly_start).await?;

    let daily = build_activity_sets(llm_rows, embedding_rows, execution_rows, daily_start);
    let weekly = build_activity_sets(llm_rows, embedding_rows, execution_rows, weekly_start);
    let monthly = build_activity_sets(llm_rows, embedding_rows, execution_rows, monthly_start);

    let average_cost_per_active_user = if monthly.active_users.is_empty() {
        None
    } else {
        Some(monthly.cost as f64 / monthly.active_users.len() as f64 / 1_000_000.0)
    };

    Ok(AdminUserStats {
        total_users,
        new_users_today,
        new_users_weekly,
        new_users_monthly,
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
    })
}

async fn count_new_users_since(state: &AppState, start: NaiveDateTime) -> Result<u64, ApiError> {
    user::Entity::find()
        .filter(user::Column::CreatedAt.gte(start))
        .count(&state.db)
        .await
        .map_err(|e| ApiError::internal_error(e.into()))
}

#[derive(Default)]
struct ActivitySets {
    active_users: HashSet<String>,
    active_apps: HashSet<String>,
    ai_users: HashSet<String>,
    execution_users: HashSet<String>,
    user_interactions: HashMap<String, u64>,
    cost: i64,
}

fn build_activity_sets(
    llm_rows: &[llm_usage_tracking::Model],
    embedding_rows: &[embedding_usage_tracking::Model],
    execution_rows: &[execution_usage_tracking::Model],
    start: NaiveDateTime,
) -> ActivitySets {
    let mut activity = ActivitySets::default();

    for row in llm_rows.iter().filter(|row| row.created_at >= start) {
        activity.cost += row.price;
        if let Some(user_id) = &row.user_id {
            activity.active_users.insert(user_id.clone());
            activity.ai_users.insert(user_id.clone());
            *activity
                .user_interactions
                .entry(user_id.clone())
                .or_default() += 1;
        }
        if let Some(app_id) = &row.app_id {
            activity.active_apps.insert(app_id.clone());
        }
    }

    for row in embedding_rows.iter().filter(|row| row.created_at >= start) {
        activity.cost += row.price;
        if let Some(user_id) = &row.user_id {
            activity.active_users.insert(user_id.clone());
            activity.ai_users.insert(user_id.clone());
            *activity
                .user_interactions
                .entry(user_id.clone())
                .or_default() += 1;
        }
        if let Some(app_id) = &row.app_id {
            activity.active_apps.insert(app_id.clone());
        }
    }

    for row in execution_rows.iter().filter(|row| row.created_at >= start) {
        if let Some(user_id) = &row.user_id {
            activity.active_users.insert(user_id.clone());
            activity.execution_users.insert(user_id.clone());
            *activity
                .user_interactions
                .entry(user_id.clone())
                .or_default() += 1;
        }
        if let Some(app_id) = &row.app_id {
            activity.active_apps.insert(app_id.clone());
        }
    }

    activity
}

fn build_usage_trend(
    period: &str,
    started_at: NaiveDateTime,
    now: NaiveDateTime,
    llm_rows: &[llm_usage_tracking::Model],
    embedding_rows: &[embedding_usage_tracking::Model],
    execution_rows: &[execution_usage_tracking::Model],
    new_user_rows: &[user::Model],
) -> Vec<AdminUsageTrendPoint> {
    let monthly_buckets = period == crate::usage_limits::YEARLY;
    let mut buckets = seed_trend_buckets(started_at.date(), now.date(), monthly_buckets);

    for row in new_user_rows {
        let key = trend_key(row.created_at, monthly_buckets);
        buckets.entry(key).or_default().new_users += 1;
    }

    for row in llm_rows {
        let key = trend_key(row.created_at, monthly_buckets);
        let bucket = buckets.entry(key).or_default();
        bucket.ai_invocations += 1;
        bucket.tokens += row.token_in + row.token_out;
        bucket.cost += row.price;
        if let Some(user_id) = &row.user_id {
            bucket.active_users.insert(user_id.clone());
        }
    }

    for row in embedding_rows {
        let key = trend_key(row.created_at, monthly_buckets);
        let bucket = buckets.entry(key).or_default();
        bucket.ai_invocations += 1;
        bucket.tokens += row.token_count;
        bucket.cost += row.price;
        if let Some(user_id) = &row.user_id {
            bucket.active_users.insert(user_id.clone());
        }
    }

    for row in execution_rows {
        let key = trend_key(row.created_at, monthly_buckets);
        let bucket = buckets.entry(key).or_default();
        bucket.executions += 1;
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

fn trend_key(created_at: NaiveDateTime, monthly_buckets: bool) -> String {
    if monthly_buckets {
        created_at.format("%Y-%m").to_string()
    } else {
        created_at.format("%Y-%m-%d").to_string()
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

fn build_power_user_aggregates(
    llm_rows: &[llm_usage_tracking::Model],
    embedding_rows: &[embedding_usage_tracking::Model],
    execution_rows: &[execution_usage_tracking::Model],
    start: NaiveDateTime,
) -> HashMap<String, PowerUserAggregate> {
    let mut users = HashMap::<String, PowerUserAggregate>::new();

    for row in llm_rows.iter().filter(|row| row.created_at >= start) {
        let Some(user_id) = &row.user_id else {
            continue;
        };
        let entry = users.entry(user_id.clone()).or_default();
        entry.total_price += row.price;
        entry.total_tokens += row.token_in + row.token_out;
        entry.ai_invocations += 1;
        entry.touch(row.created_at);
    }

    for row in embedding_rows.iter().filter(|row| row.created_at >= start) {
        let Some(user_id) = &row.user_id else {
            continue;
        };
        let entry = users.entry(user_id.clone()).or_default();
        entry.total_price += row.price;
        entry.total_tokens += row.token_count;
        entry.ai_invocations += 1;
        entry.touch(row.created_at);
    }

    for row in execution_rows.iter().filter(|row| row.created_at >= start) {
        let Some(user_id) = &row.user_id else {
            continue;
        };
        let entry = users.entry(user_id.clone()).or_default();
        entry.executions += 1;
        entry.touch(row.created_at);
    }

    users
}

fn build_power_users(
    aggregates: HashMap<String, PowerUserAggregate>,
    user_lookup: &HashMap<String, user::Model>,
) -> Vec<AdminPowerUser> {
    let mut power_users: Vec<_> = aggregates
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
                last_seen: totals
                    .last_seen
                    .map(|last_seen| last_seen.and_utc().to_rfc3339()),
            }
        })
        .collect();
    power_users.sort_by_key(|user| {
        std::cmp::Reverse((
            user.total_interactions,
            user.active_days,
            user.total_tokens,
            user.total_price,
        ))
    });
    power_users.truncate(8);
    power_users
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

    let metas = meta::Entity::find()
        .filter(meta::Column::AppId.is_in(app_ids))
        .all(&state.db)
        .await
        .map_err(|e| ApiError::internal_error(e.into()))?;

    let mut names = HashMap::new();
    for row in metas {
        let Some(app_id) = row.app_id else {
            continue;
        };
        if row.lang == "en" || !names.contains_key(&app_id) {
            names.insert(app_id, row.name);
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
