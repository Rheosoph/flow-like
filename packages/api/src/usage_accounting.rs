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
    QueryOrder, QuerySelect,
};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

pub const STATUS_PENDING: &str = "pending";
pub const STATUS_COMPLETED: &str = "completed";
pub const STATUS_FAILED: &str = "failed";
pub const STATUS_CANCELLED: &str = "cancelled";
pub const STATUS_UNKNOWN_USAGE: &str = "unknown_usage";

/// A published rate is copied into the reservation so a later price or FX change
/// cannot change the amount charged for an already admitted request.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct HostedRateSnapshot {
    pub version: String,
    /// Missing Bit prices allow inference, but cannot establish a token-based cost.
    #[serde(default = "default_provider_pricing_available")]
    pub provider_pricing_available: bool,
    pub input_micro_usd_per_million_tokens: i64,
    /// An explicitly estimated internal embedding tariff per UTF-8 input byte.
    /// Internal gateways that report word counts cannot supply tokenizer usage.
    #[serde(default)]
    pub input_micro_usd_per_million_bytes: Option<i64>,
    #[serde(default)]
    pub max_input_bytes: Option<i64>,
    pub output_micro_usd_per_million_tokens: i64,
    #[serde(default)]
    pub request_micro_usd: i64,
    pub context_tokens: i64,
    #[serde(default = "default_usd_micro_per_eur")]
    pub usd_micro_per_eur: i64,
    #[serde(default)]
    pub funding_basis_points: i64,
    #[serde(default = "default_api_rate")]
    pub api_micro_usd_per_million_ms: i64,
    #[serde(default)]
    pub serving_request_micro_usd: i64,
    #[serde(default = "default_request_timeout_ms")]
    pub max_request_ms: i64,
}

fn default_provider_pricing_available() -> bool {
    true
}

fn default_usd_micro_per_eur() -> i64 {
    1_159_200
}
fn default_api_rate() -> i64 {
    20_001
}
fn default_request_timeout_ms() -> i64 {
    240_000
}

fn rounded_ratio(numerator: i128, denominator: i64) -> i64 {
    ((numerator.max(0) + i128::from(denominator.max(1)) - 1) / i128::from(denominator.max(1)))
        .min(i128::from(i64::MAX)) as i64
}

impl HostedRateSnapshot {
    pub fn validate(&self) -> Result<(), ApiError> {
        if self.version.trim().is_empty()
            || self.input_micro_usd_per_million_tokens < 0
            || self
                .input_micro_usd_per_million_bytes
                .is_some_and(|rate| rate < 0)
            || (self.input_micro_usd_per_million_bytes.is_some()
                && self.max_input_bytes.is_none_or(|limit| limit <= 0))
            || self.output_micro_usd_per_million_tokens < 0
            || self.request_micro_usd < 0
            || self.context_tokens <= 0
            || self.usd_micro_per_eur <= 0
            || !(0..=10_000).contains(&self.funding_basis_points)
            || self.api_micro_usd_per_million_ms < 0
            || self.serving_request_micro_usd < 0
            || !(1..=840_000).contains(&self.max_request_ms)
        {
            return Err(ApiError::internal(
                "Hosted model has an invalid accounting rate",
            ));
        }
        Ok(())
    }

    pub fn provider_cost(&self, input_tokens: i64, output_tokens: i64) -> i64 {
        rounded_ratio(
            i128::from(input_tokens.max(0)) * i128::from(self.input_micro_usd_per_million_tokens)
                + i128::from(output_tokens.max(0))
                    * i128::from(self.output_micro_usd_per_million_tokens),
            1_000_000,
        )
        .saturating_add(self.request_micro_usd)
    }

    pub fn known_provider_cost(&self, input_tokens: i64, output_tokens: i64) -> Option<i64> {
        self.provider_pricing_available
            .then(|| self.provider_cost(input_tokens, output_tokens))
    }

    pub fn provider_cost_bytes(&self, input_bytes: i64) -> Result<i64, ApiError> {
        let rate = self.input_micro_usd_per_million_bytes.ok_or_else(|| ApiError::internal(
            "Internal embedding pricing requires input_micro_usd_per_million_bytes; its reported word counts are not tokenizer usage",
        ))?;
        Ok(
            rounded_ratio(i128::from(input_bytes.max(0)) * i128::from(rate), 1_000_000)
                .saturating_add(self.request_micro_usd),
        )
    }

    pub fn serving_cost(&self, latency_ms: i64) -> i64 {
        rounded_ratio(
            i128::from(latency_ms.max(0)) * i128::from(self.api_micro_usd_per_million_ms),
            1_000_000,
        )
        .saturating_add(self.serving_request_micro_usd)
    }

    pub fn all_in_micro_eur(&self, provider_micro_usd: i64, latency_ms: i64) -> i64 {
        let funded = rounded_ratio(
            i128::from(provider_micro_usd.max(0)) * i128::from(10_000 + self.funding_basis_points),
            10_000,
        );
        rounded_ratio(
            i128::from(funded.saturating_add(self.serving_cost(latency_ms))) * 1_000_000,
            self.usd_micro_per_eur,
        )
    }
}

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
    pub rate: Option<HostedRateSnapshot>,
}

#[derive(Clone, Debug, Default, Serialize)]
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
    pub provider_usage_recovered: u64,
}

pub async fn start_usage_invocation(
    state: &AppState,
    start: UsageInvocationStart<'_>,
) -> Result<Option<String>, ApiError> {
    let rate = start
        .rate
        .clone()
        .ok_or_else(|| ApiError::internal("Hosted AI accounting rate is unavailable"))?;
    rate.validate()?;
    let payer = crate::quota::resolve_payer(state, start.user_id, start.app_id).await?;
    let mut request = crate::quota::QuotaRequest {
        operation_id: String::new(),
        payer_id: payer.clone(),
        actor_id: start.user_id.map(ToOwned::to_owned),
        app_id: start.app_id.map(ToOwned::to_owned),
        model_id: start.model_id.map(ToOwned::to_owned),
        provider: start.provider.map(ToOwned::to_owned),
        kind: start.kind.to_owned(),
        funding_class: "hosted".to_owned(),
        execution_mode: "hosted_ai".to_owned(),
        amounts: crate::quota::QuotaAmounts {
            ai_cost_micros: rate
                .all_in_micro_eur(start.estimated_cost_micro_dollars, rate.max_request_ms),
            ai_calls: 1,
            ..Default::default()
        },
        deadline: Utc::now() + Duration::milliseconds(rate.max_request_ms),
    };
    let id = start_usage_invocation_with_db(&state.db, state.db_dialect, start)
        .await?
        .ok_or_else(|| ApiError::internal("Failed to reserve hosted AI invocation"))?;
    let original = usage_invocation::Entity::find_by_id(&id)
        .one(&state.db)
        .await?
        .ok_or_else(|| ApiError::internal("Hosted AI admission is missing"))?;
    if original.status != STATUS_PENDING {
        return Err(ApiError::conflict(
            "Hosted AI admission has already expired",
        ));
    }
    // Recovery can release an account-less admission after fifteen minutes.
    // A resumed process keeps this original deadline, and mark_started checks it
    // again after the quota transaction, so it cannot revive the released work.
    request.deadline = request
        .deadline
        .min(original.created_at.with_timezone(&Utc) + Duration::milliseconds(rate.max_request_ms));
    if let Err(error) = crate::quota::reserve(
        state,
        crate::quota::QuotaRequest {
            operation_id: id.clone(),
            ..request
        },
    )
    .await
    {
        settle_usage_invocation_with_dialect(
            &state.db,
            state.db_dialect,
            Some(&id),
            UsageInvocationSettlement {
                status: STATUS_FAILED,
                error: Some("Account plan rejected the request before provider dispatch".into()),
                ..Default::default()
            },
        )
        .await?;
        return Err(error);
    }
    crate::compute_attempts::associate_current(&state.db, &id, &payer, "ai_relay", "ai_serving")
        .await?;
    Ok(Some(id))
}

pub(crate) async fn start_usage_invocation_with_db(
    db: &DatabaseConnection,
    dialect: crate::db::DbDialect,
    start: UsageInvocationStart<'_>,
) -> Result<Option<String>, ApiError> {
    let app_id = start
        .app_id
        .map(str::trim)
        .filter(|app_id| !app_id.is_empty());

    let now = Utc::now().fixed_offset();
    let id = create_id();
    let reservation = usage_invocation::ActiveModel {
        id: Set(id.clone()),
        kind: Set(start.kind.to_string()),
        status: Set(STATUS_PENDING.to_string()),
        user_id: Set(start.user_id.map(ToOwned::to_owned)),
        technical_user_id: Set(start.technical_user_id.map(ToOwned::to_owned)),
        app_id: Set(app_id.map(ToOwned::to_owned)),
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
        raw_usage: Set(start
            .rate
            .as_ref()
            .map(|rate| serde_json::json!({"accounting": rate}))),
        error: Set(None),
        started_at: Set(now),
        completed_at: Set(None),
        created_at: Set(now),
        updated_at: Set(now),
    };
    let app_id = app_id.map(ToOwned::to_owned);
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
                if let Some(app_id) = app_id.as_deref() {
                    crate::db::coordination::coordinate(txn, "usage-budget", &[app_id]).await?;
                }
                if let Some(rejection) = check_app_usage_limits_for_user(
                    txn,
                    app_id.as_deref(),
                    user_id.as_deref(),
                    technical_user_id.as_deref(),
                    Some(estimated_tokens),
                    Some(estimated_cost),
                )
                .await?
                {
                    return Ok::<_, ApiError>(Err(rejection));
                }
                let row = reservation.insert(txn).await?;
                crate::rolling_usage::sync_invocation(txn, &row).await?;
                Ok(Ok(Some(id)))
            })
        },
    )
    .await?
}

#[cfg(test)]
pub async fn settle_usage_invocation(
    db: &DatabaseConnection,
    invocation_id: Option<&str>,
    settlement: UsageInvocationSettlement,
) -> Result<(), sea_orm::DbErr> {
    settle_usage_invocation_with_dialect(
        db,
        crate::db::DbDialect::Postgres,
        invocation_id,
        settlement,
    )
    .await
}

pub(crate) async fn release_unstarted_hosted(
    db: &DatabaseConnection,
    dialect: crate::db::DbDialect,
    invocation_id: &str,
) -> Result<(), sea_orm::DbErr> {
    settle_usage_invocation_with_dialect(
        db,
        dialect,
        Some(invocation_id),
        UsageInvocationSettlement {
            status: STATUS_FAILED,
            error: Some("Released before provider dispatch".into()),
            ..Default::default()
        },
    )
    .await
}

async fn settle_usage_invocation_with_dialect(
    db: &DatabaseConnection,
    dialect: crate::db::DbDialect,
    invocation_id: Option<&str>,
    settlement: UsageInvocationSettlement,
) -> Result<(), sea_orm::DbErr> {
    let Some(invocation_id) = invocation_id
        .map(str::trim)
        .filter(|invocation_id| !invocation_id.is_empty())
    else {
        return Ok(());
    };

    let id = invocation_id.to_owned();
    crate::db::retry_transaction(
        db,
        dialect,
        None,
        &crate::db::RetryPolicy::default(),
        move |txn| {
            let id = id.clone();
            let settlement = settlement.clone();
            Box::pin(async move {
                let Some(row) = usage_invocation::Entity::find_by_id(&id).one(txn).await? else {
                    return Ok::<_, sea_orm::DbErr>(());
                };
                if let Some(app) = row.app_id.as_deref() {
                    crate::db::coordination::coordinate(txn, "usage-budget", &[app]).await?;
                }
                let Some(row) = usage_invocation::Entity::find_by_id(&id).one(txn).await? else {
                    return Ok(());
                };
                if !matches!(row.status.as_str(), STATUS_PENDING | STATUS_UNKNOWN_USAGE) {
                    return Ok(());
                }
                let now = Utc::now().fixed_offset();
                let active: usage_invocation::ActiveModel = row.into();
                let row = usage_invocation::ActiveModel {
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
                    ..active
                }
                .update(txn)
                .await?;
                crate::rolling_usage::sync_invocation(txn, &row).await?;
                Ok::<_, sea_orm::DbErr>(())
            })
        },
    )
    .await
}

/// Account settlement is durable before the legacy detail row is finalized.
/// Repeating either write is safe. An interrupted response retains its money
/// reservation until the provider reports authoritative usage.
pub async fn settle_hosted_usage_invocation(
    state: &AppState,
    invocation_id: Option<&str>,
    mut settlement: UsageInvocationSettlement,
) -> Result<(), sea_orm::DbErr> {
    let Some(id) = invocation_id else {
        return Ok(());
    };
    // A provider response must outlive a transient SQL failure. Keep the private
    // receipt until maintenance replays it or normal payload cleanup expires it.
    if let Err(error) =
        crate::routes::chat::hosted_worker::store_usage_receipt(state, id, &settlement).await
    {
        tracing::warn!(operation_id = id, %error, "Could not retain hosted usage receipt; attempting database settlement");
    }
    let Some(row) = usage_invocation::Entity::find_by_id(id)
        .one(&state.db)
        .await?
    else {
        return Err(sea_orm::DbErr::Custom(
            "Hosted AI reservation is missing".to_owned(),
        ));
    };
    if !matches!(row.status.as_str(), STATUS_PENDING | STATUS_UNKNOWN_USAGE) {
        return Ok(());
    }
    if settlement.provider_request_id.is_none() {
        settlement.provider_request_id = row.provider_request_id.clone();
    }
    if settlement.latency_ms.is_none() {
        settlement.latency_ms = row.latency;
    }
    if settlement.status == STATUS_UNKNOWN_USAGE {
        settlement.input_tokens = settlement.input_tokens.max(row.input_tokens);
        settlement.output_tokens = settlement.output_tokens.max(row.output_tokens);
        settlement.embedding_tokens = settlement.embedding_tokens.max(row.embedding_tokens);
        settlement.cost_micro_dollars = settlement.cost_micro_dollars.max(row.cost_micro_dollars);
        if settlement.raw_usage.is_none() {
            settlement.raw_usage = row
                .raw_usage
                .as_ref()
                .and_then(|value| value.get("providerUsage"))
                .filter(|value| !value.is_null())
                .cloned();
        }
    }
    let accounting = row
        .raw_usage
        .as_ref()
        .and_then(|value| value.get("accounting"))
        .cloned()
        .ok_or_else(|| {
            sea_orm::DbErr::Custom("Hosted AI reservation rate is missing".to_owned())
        })?;
    let rate: HostedRateSnapshot = serde_json::from_value(accounting.clone())
        .map_err(|_| sea_orm::DbErr::Custom("Hosted AI reservation rate is invalid".to_owned()))?;
    let latency_ms = settlement
        .latency_ms
        .filter(|value| value.is_finite() && *value >= 0.0)
        .map(|value| value.ceil().min(i64::MAX as f64) as i64)
        .unwrap_or(0);
    let finalized = matches!(settlement.status, STATUS_COMPLETED | STATUS_FAILED);
    let cost_micro_eur = rate.all_in_micro_eur(settlement.cost_micro_dollars, latency_ms);
    let detail = serde_json::json!({
        "accounting": accounting,
        "providerUsage": settlement.raw_usage,
        "providerRequestId": settlement.provider_request_id,
        "providerCostMicroUsd": settlement.cost_micro_dollars,
        "servingCostMicroUsd": rate.serving_cost(latency_ms),
        "costMicroEur": cost_micro_eur,
        "inputTokens": settlement.input_tokens,
        "outputTokens": settlement.output_tokens,
        "embeddingTokens": settlement.embedding_tokens,
        "status": settlement.status,
    });
    let revision = format!(
        "provider-{}-{}-{}",
        settlement.status, settlement.cost_micro_dollars, latency_ms
    );
    crate::quota::settle(
        state,
        id,
        &revision,
        crate::quota::QuotaAmounts {
            ai_cost_micros: cost_micro_eur,
            ai_calls: 1,
            ..Default::default()
        },
        finalized,
        detail.clone(),
    )
    .await
    .map_err(|error| sea_orm::DbErr::Custom(error.to_string()))?;
    settlement.raw_usage = Some(detail);
    settle_usage_invocation_with_dialect(&state.db, state.db_dialect, Some(id), settlement).await
}

pub async fn record_provider_request_id(
    state: &AppState,
    invocation_id: Option<&str>,
    provider_request_id: &str,
) -> Result<(), sea_orm::DbErr> {
    let Some(id) = invocation_id else {
        return Ok(());
    };
    usage_invocation::Entity::update_many()
        .set(usage_invocation::ActiveModel {
            provider_request_id: Set(Some(provider_request_id.to_owned())),
            updated_at: Set(Utc::now().fixed_offset()),
            ..Default::default()
        })
        .filter(usage_invocation::Column::Id.eq(id))
        .filter(usage_invocation::Column::Status.is_in([STATUS_PENDING, STATUS_UNKNOWN_USAGE]))
        .exec(&state.db)
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
        .order_by_asc(usage_invocation::Column::StartedAt)
        .limit(500)
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
        provider_usage_recovered: 0,
    })
}

/// Provider recovery is bounded per pass and never re-dispatches inference.
/// Requests without a provider receipt retain their reservation for review.
pub async fn reconcile_hosted_invocations(
    state: &AppState,
    older_than_minutes: i64,
) -> Result<UsageReconciliationResult, sea_orm::DbErr> {
    use flow_like_secrets::{ExposeSecret, SecretRef};
    let mut result = reconcile_stale_invocations(&state.db, older_than_minutes).await?;
    let candidates = usage_invocation::Entity::find()
        .filter(usage_invocation::Column::Status.eq(STATUS_UNKNOWN_USAGE))
        .filter(usage_invocation::Column::Provider.eq("openrouter"))
        .filter(usage_invocation::Column::ProviderRequestId.is_not_null())
        .order_by_asc(usage_invocation::Column::UpdatedAt)
        .limit(50)
        .all(&state.db)
        .await?;
    if candidates.is_empty() {
        return Ok(result);
    }
    let Ok(key) = state
        .secrets
        .get_secret_string(&SecretRef::new("OPENROUTER_API_KEY"))
        .await
    else {
        return Ok(result);
    };
    let client = flow_like_types::reqwest::Client::new();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(25);
    for row in candidates {
        if std::time::Instant::now() >= deadline {
            break;
        }
        let Some(provider_id) = row.provider_request_id.as_deref() else {
            continue;
        };
        let response = client
            .get("https://openrouter.ai/api/v1/generation")
            .bearer_auth(key.expose_secret())
            .query(&[("id", provider_id)])
            .timeout(std::time::Duration::from_secs(4))
            .send()
            .await;
        let recovered = match response {
            Ok(response) if response.status().is_success() => response.json::<Value>().await.ok(),
            _ => None,
        };
        let usage = recovered.as_ref().and_then(|value| value.get("data"));
        if let Some(usage) = usage
            && let Some(cost) = usage
                .get("total_cost")
                .and_then(Value::as_f64)
                .filter(|value| value.is_finite() && *value >= 0.0)
            && usage.get("id").and_then(Value::as_str) == Some(provider_id)
            && usage
                .get("finish_reason")
                .is_some_and(|value| !value.is_null())
        {
            // Missing relay timing remains pending even when the provider bill
            // is known. A provider's latency is not the API Lambda's duration.
            let final_status = if row.latency.is_some() {
                STATUS_COMPLETED
            } else {
                STATUS_UNKNOWN_USAGE
            };
            settle_hosted_usage_invocation(
                state,
                Some(&row.id),
                UsageInvocationSettlement {
                    status: final_status,
                    input_tokens: usage
                        .get("native_tokens_prompt")
                        .or_else(|| usage.get("tokens_prompt"))
                        .and_then(Value::as_i64)
                        .unwrap_or(0),
                    output_tokens: usage
                        .get("native_tokens_completion")
                        .or_else(|| usage.get("tokens_completion"))
                        .and_then(Value::as_i64)
                        .unwrap_or(0),
                    cost_micro_dollars: (cost * 1_000_000.0).ceil().min(i64::MAX as f64) as i64,
                    latency_ms: row.latency,
                    provider_request_id: Some(provider_id.to_owned()),
                    raw_usage: Some(usage.clone()),
                    ..Default::default()
                },
            )
            .await?;
            result.provider_usage_recovered += 1;
        }
        usage_invocation::Entity::update_many()
            .set(usage_invocation::ActiveModel {
                updated_at: Set(Utc::now().fixed_offset()),
                ..Default::default()
            })
            .filter(usage_invocation::Column::Id.eq(row.id))
            .exec(&state.db)
            .await?;
    }
    Ok(result)
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

#[cfg(test)]
mod tests {
    use super::*;

    fn rate() -> HostedRateSnapshot {
        HostedRateSnapshot {
            version: "test-rate".into(),
            provider_pricing_available: true,
            input_micro_usd_per_million_tokens: 1_000_000,
            input_micro_usd_per_million_bytes: None,
            max_input_bytes: None,
            output_micro_usd_per_million_tokens: 4_000_000,
            request_micro_usd: 0,
            context_tokens: 32_000,
            usd_micro_per_eur: 1_159_200,
            funding_basis_points: 550,
            api_micro_usd_per_million_ms: 20_001,
            serving_request_micro_usd: 0,
            max_request_ms: 240_000,
        }
    }

    #[test]
    fn all_in_budget_includes_funding_and_serving_before_converting_currency() {
        let rate = rate();
        assert_eq!(rate.provider_cost(1_000, 250), 2_000);
        assert_eq!(rate.serving_cost(10_000), 201);
        assert_eq!(rate.all_in_micro_eur(2_000, 10_000), 1_994);
    }

    #[test]
    fn free_inference_still_has_hosted_serving_cost() {
        let rate = rate();
        assert_eq!(rate.all_in_micro_eur(0, 10_000), 174);
    }

    #[test]
    fn missing_price_keeps_token_only_provider_cost_unknown() {
        let mut rate = rate();
        rate.provider_pricing_available = false;
        rate.input_micro_usd_per_million_tokens = 0;
        rate.output_micro_usd_per_million_tokens = 0;
        assert_eq!(rate.known_provider_cost(1_000, 250), None);

        rate.provider_pricing_available = true;
        assert_eq!(rate.known_provider_cost(1_000, 250), Some(0));
    }

    #[test]
    fn reservation_retains_missing_pricing_marker() {
        let mut rate = rate();
        rate.provider_pricing_available = false;
        let saved = serde_json::to_value(&rate).unwrap();
        let restored: HostedRateSnapshot = serde_json::from_value(saved).unwrap();
        assert!(!restored.provider_pricing_available);
        assert_eq!(restored.known_provider_cost(1_000, 250), None);
    }

    #[test]
    fn existing_reservations_keep_their_known_prices() {
        let mut saved = serde_json::to_value(rate()).unwrap();
        saved
            .as_object_mut()
            .unwrap()
            .remove("provider_pricing_available");
        let restored: HostedRateSnapshot = serde_json::from_value(saved).unwrap();
        assert!(restored.provider_pricing_available);
        assert_eq!(restored.known_provider_cost(1_000, 250), Some(2_000));
    }

    #[test]
    fn fractional_token_cost_rounds_up_and_never_becomes_zero() {
        let mut rate = rate();
        rate.input_micro_usd_per_million_tokens = 20_000;
        assert_eq!(rate.provider_cost(1, 0), 1);
    }

    #[test]
    fn internal_byte_tariff_is_explicit_and_does_not_bill_reported_word_counts() {
        let mut rate = rate();
        assert!(rate.provider_cost_bytes(10_000).is_err());
        rate.input_micro_usd_per_million_bytes = Some(100_000);
        rate.max_input_bytes = Some(100_000);
        assert!(rate.validate().is_ok());
        assert_eq!(rate.provider_cost_bytes(10_000).unwrap(), 1_000);
        assert_eq!(rate.provider_cost_bytes(1).unwrap(), 1);
    }

    #[test]
    fn invalid_rates_are_rejected_before_external_work() {
        let mut rate = rate();
        rate.usd_micro_per_eur = 0;
        assert!(rate.validate().is_err());
        rate.usd_micro_per_eur = 1_159_200;
        rate.max_request_ms = 900_000;
        assert!(rate.validate().is_err());
        rate.max_request_ms = 240_000;
        rate.input_micro_usd_per_million_tokens = -1;
        assert!(rate.validate().is_err());
    }

    #[test]
    fn settlement_uses_the_serialized_rate_even_after_the_original_changes() {
        let mut original = rate();
        let saved = serde_json::to_value(&original).unwrap();
        original.usd_micro_per_eur = 2_000_000;
        let saved: HostedRateSnapshot = serde_json::from_value(saved).unwrap();
        assert_eq!(saved.all_in_micro_eur(2_000, 10_000), 1_994);
        assert_ne!(original.all_in_micro_eur(2_000, 10_000), 1_994);
    }
}
