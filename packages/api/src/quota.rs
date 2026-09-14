//! Account quotas use retained counters and operation identities. External work
//! starts only after reservation commits; uncertain outcomes retain capacity.

use crate::{
    db::{DbDialect, RetryPolicy, coordination::coordinate, retry_transaction},
    entity::user,
    error::{ApiError, QuotaLimitDetails},
    state::AppState,
};
use chrono::{DateTime, Datelike, Months, TimeZone, Utc};
use flow_like::hub::UserTier;
use flow_like_storage::object_store::ObjectStoreExt;
use sea_orm::{
    ConnectionTrait, DatabaseConnection, DatabaseTransaction, EntityTrait, FromQueryResult,
    Statement,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", default)]
pub struct QuotaAmounts {
    pub runtime_ms: i64,
    pub ai_cost_micros: i64,
    pub ai_calls: i64,
    pub cloud_starts: i64,
}

impl QuotaAmounts {
    fn values(self) -> [i64; 4] {
        [
            self.runtime_ms,
            self.ai_cost_micros,
            self.ai_calls,
            self.cloud_starts,
        ]
    }

    fn from_values(v: [i64; 4]) -> Self {
        Self {
            runtime_ms: v[0],
            ai_cost_micros: v[1],
            ai_calls: v[2],
            cloud_starts: v[3],
        }
    }

    fn checked_add(self, other: Self) -> Result<Self, ApiError> {
        let a = self.values();
        let b = other.values();
        let mut result = [0; 4];
        for i in 0..4 {
            result[i] = a[i]
                .checked_add(b[i])
                .ok_or_else(|| ApiError::internal("quota counter overflow"))?;
            if result[i] < 0 {
                return Err(ApiError::internal("quota counter underflow"));
            }
        }
        Ok(Self::from_values(result))
    }

    fn delta(self, previous: Self) -> Self {
        let a = self.values();
        let b = previous.values();
        Self::from_values(std::array::from_fn(|i| a[i] - b[i]))
    }

    fn bounded(self, maximum: Self) -> Self {
        let a = self.values();
        let b = maximum.values();
        Self::from_values(std::array::from_fn(|i| a[i].clamp(0, b[i])))
    }

    fn remaining(self, spent: Self) -> Self {
        let a = self.values();
        let b = spent.values();
        Self::from_values(std::array::from_fn(|i| (a[i] - b[i]).max(0)))
    }
}

const RESOURCES: [&str; 4] = [
    "cloud_runtime_ms",
    "hosted_ai_cost_micros",
    "hosted_ai_calls",
    "cloud_starts",
];
const UNITS: [&str; 4] = ["milliseconds", "eur_micros", "operations", "starts"];

#[derive(Clone, Debug)]
pub struct QuotaRequest {
    pub operation_id: String,
    pub payer_id: String,
    pub actor_id: Option<String>,
    pub app_id: Option<String>,
    pub model_id: Option<String>,
    pub provider: Option<String>,
    pub kind: String,
    pub funding_class: String,
    pub execution_mode: String,
    pub amounts: QuotaAmounts,
    pub deadline: DateTime<Utc>,
}

#[derive(Clone, Debug, FromQueryResult)]
struct PeriodRow {
    id: String,
    #[sea_orm(from_alias = "payerId")]
    payer_id: String,
    #[sea_orm(from_alias = "periodStart")]
    period_start: i64,
    #[sea_orm(from_alias = "periodEnd")]
    period_end: i64,
    used: String,
    reserved: String,
    #[sea_orm(from_alias = "trackingStartedAt")]
    tracking_started_at: i64,
}

#[derive(Clone, Debug, FromQueryResult)]
struct OperationRow {
    id: String,
    #[sea_orm(from_alias = "actorId")]
    actor_id: Option<String>,
    #[sea_orm(from_alias = "payerId")]
    payer_id: String,
    #[sea_orm(from_alias = "periodId")]
    period_id: String,
    #[sea_orm(from_alias = "appId")]
    app_id: Option<String>,
    #[sea_orm(from_alias = "modelId")]
    model_id: Option<String>,
    provider: Option<String>,
    #[sea_orm(from_alias = "fundingClass")]
    funding_class: String,
    #[sea_orm(from_alias = "executionMode")]
    execution_mode: String,
    kind: String,
    status: String,
    ceiling: String,
    used: String,
    reserved: String,
    #[sea_orm(from_alias = "ownerId")]
    owner_id: Option<String>,
    #[sea_orm(from_alias = "createdAt")]
    created_at: i64,
}

fn sql(statement: &str, values: Vec<sea_orm::Value>) -> Statement {
    Statement::from_sql_and_values(sea_orm::DatabaseBackend::Postgres, statement, values)
}

fn decode(value: &str) -> Result<QuotaAmounts, ApiError> {
    serde_json::from_str(value)
        .map_err(|e| ApiError::internal(format!("invalid quota counter: {e}")))
}

fn encode(value: QuotaAmounts) -> String {
    serde_json::to_string(&value).expect("integer quota serialization")
}

fn stamp(value: i64) -> String {
    DateTime::from_timestamp_millis(value)
        .unwrap_or(DateTime::<Utc>::UNIX_EPOCH)
        .to_rfc3339()
}

pub async fn resolve_payer(
    state: &AppState,
    actor_id: Option<&str>,
    app_id: Option<&str>,
) -> Result<String, ApiError> {
    if let Some(app_id) = app_id.filter(|id| !id.is_empty()) {
        return crate::capacity::payer_for_app(&state.db, app_id)
            .await?
            .ok_or_else(|| ApiError::forbidden("The app has no billing owner"));
    }
    actor_id
        .filter(|id| !id.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| ApiError::forbidden("A billing owner is required"))
}

pub async fn payer_plan(state: &AppState, payer_id: &str) -> Result<(String, UserTier), ApiError> {
    let payer = user::Entity::find_by_id(payer_id)
        .one(&state.db)
        .await?
        .ok_or(ApiError::NOT_FOUND)?;
    let plan = serde_json::to_value(&payer.tier)?
        .as_str()
        .ok_or_else(|| ApiError::internal("invalid account tier"))?
        .to_uppercase();
    let tier = state
        .platform_config
        .tiers
        .get(&plan)
        .cloned()
        .ok_or_else(|| ApiError::internal(format!("missing entitlement for {plan}")))?;
    Ok((plan, tier))
}

/// Month boundaries retain the original anniversary, including after February.
pub fn monthly_period(
    now: DateTime<Utc>,
    anchor: Option<DateTime<Utc>>,
) -> (DateTime<Utc>, DateTime<Utc>) {
    let Some(anchor) = anchor.filter(|anchor| *anchor <= now) else {
        let start = Utc
            .with_ymd_and_hms(now.year(), now.month(), 1, 0, 0, 0)
            .unwrap();
        return (start, start.checked_add_months(Months::new(1)).unwrap());
    };
    let mut months = ((now.year() - anchor.year()) * 12 + now.month() as i32
        - anchor.month() as i32)
        .max(0) as u32;
    let anniversary = |months| anchor.checked_add_months(Months::new(months)).unwrap();
    if anniversary(months) > now {
        months = months.saturating_sub(1);
    }
    (anniversary(months), anniversary(months + 1))
}

fn limits(tier: &UserTier) -> QuotaAmounts {
    QuotaAmounts {
        runtime_ms: tier.max_runtime_ms,
        ai_cost_micros: tier.max_ai_cost_micros,
        ai_calls: tier.max_llm_calls.map(i64::from).unwrap_or(-1),
        cloud_starts: i64::from(tier.max_remote_executions),
    }
}

async fn period<C: ConnectionTrait>(db: &C, id: &str) -> Result<PeriodRow, ApiError> {
    let row = db
        .query_one_raw(sql(
            "SELECT * FROM \"QuotaPeriod\" WHERE id = $1",
            vec![id.into()],
        ))
        .await?
        .ok_or_else(|| ApiError::internal("quota period missing"))?;
    Ok(PeriodRow::from_query_result(&row, "")?)
}

async fn operation<C: ConnectionTrait>(db: &C, id: &str) -> Result<Option<OperationRow>, ApiError> {
    db.query_one_raw(sql(
        "SELECT * FROM \"QuotaOperation\" WHERE id = $1",
        vec![id.into()],
    ))
    .await?
    .map(|r| OperationRow::from_query_result(&r, "").map_err(ApiError::from))
    .transpose()
}

pub async fn operation_payer(state: &AppState, id: &str) -> Result<String, ApiError> {
    Ok(operation(&state.db, id)
        .await?
        .ok_or(ApiError::NOT_FOUND)?
        .payer_id)
}

async fn active_period(
    txn: &DatabaseTransaction,
    payer: &str,
    now: DateTime<Utc>,
    anchor: Option<DateTime<Utc>>,
) -> Result<PeriodRow, ApiError> {
    if let Some(row) = txn.query_one_raw(sql(
        "SELECT * FROM \"QuotaPeriod\" WHERE \"payerId\" = $1 AND \"periodEnd\" > $2 AND \"periodStart\" <= $2 ORDER BY \"periodStart\" DESC LIMIT 1",
        vec![payer.into(), now.timestamp_millis().into()],
    )).await? { return Ok(PeriodRow::from_query_result(&row, "")?); }
    let (mut start, end) = monthly_period(now, anchor);
    // A new paid anchor must not reopen time that was already counted on Free.
    if let Some(row) = txn.query_one_raw(sql("SELECT \"periodEnd\" FROM \"QuotaPeriod\" WHERE \"payerId\" = $1 ORDER BY \"periodEnd\" DESC LIMIT 1", vec![payer.into()])).await? {
        let previous: i64 = row.try_get("", "periodEnd")?;
        if previous > start.timestamp_millis() && previous <= now.timestamp_millis() {
            start = DateTime::from_timestamp_millis(previous).ok_or_else(|| ApiError::internal("invalid quota boundary"))?;
        }
    }
    let id = format!("{payer}:{}", start.timestamp_millis());
    let zero = encode(QuotaAmounts::default());
    txn.execute_raw(sql("INSERT INTO \"QuotaPeriod\" (id, \"payerId\", \"periodStart\", \"periodEnd\", used, reserved, \"updatedAt\",\"trackingStartedAt\") VALUES ($1,$2,$3,$4,$5,$5,$6,$6)", vec![id.clone().into(), payer.into(), start.timestamp_millis().into(), end.timestamp_millis().into(), zero.into(), now.timestamp_millis().into()])).await?;
    period(txn, &id).await
}

pub async fn reserve(state: &AppState, request: QuotaRequest) -> Result<(), ApiError> {
    if request.amounts.values().iter().any(|v| *v < 0) || request.operation_id.is_empty() {
        return Err(ApiError::bad_request("Invalid quota reservation"));
    }
    if request.funding_class != "hosted"
        && (request.amounts.ai_calls != 0 || request.amounts.ai_cost_micros != 0)
    {
        return Err(ApiError::internal(
            "customer-funded operations cannot consume hosted AI",
        ));
    }
    let tiers = state.platform_config.tiers.clone();
    let enforce = enforcing();
    retry_transaction(&state.db, state.db_dialect, None, &RetryPolicy::idempotent(), move |txn| {
        let request = request.clone(); let tiers = tiers.clone();
        Box::pin(async move {
            if let Some(app_id)=request.app_id.as_deref() {
                flow_like_db::coordination::app_capacity(txn,app_id).await?;
                if crate::capacity::payer_for_app(txn,app_id).await?.as_deref()!=Some(request.payer_id.as_str()) {
                    return Err(ApiError::conflict("App billing ownership changed. Retry the operation."));
                }
            }
            coordinate(txn, "account-quota", &[&request.payer_id]).await?;
            if let Some(existing) = operation(txn, &request.operation_id).await? {
                if existing.payer_id != request.payer_id || existing.app_id != request.app_id || existing.kind != request.kind || existing.actor_id != request.actor_id || existing.model_id != request.model_id || existing.provider != request.provider || existing.funding_class != request.funding_class || existing.execution_mode != request.execution_mode || decode(&existing.ceiling)? != request.amounts {
                    return Err(ApiError::conflict("Operation identity already has a different reservation"));
                }
                return Ok(());
            }
            // Billing webhooks use this account lock too. Read the entitlement
            // after acquiring it so a concurrent downgrade cannot admit old caps.
            let payer = user::Entity::find_by_id(&request.payer_id).one(txn).await?.ok_or(ApiError::NOT_FOUND)?;
            if payer.status != crate::entity::sea_orm_active_enums::UserStatus::Active {
                return Err(ApiError::forbidden("Billing account is not active"));
            }
            let plan=serde_json::to_value(&payer.tier)?.as_str().ok_or_else(||ApiError::internal("invalid account tier"))?.to_uppercase();
            let tier=tiers.get(&plan).cloned().ok_or_else(||ApiError::internal(format!("missing entitlement for {plan}")))?;
            let anchor=payer.billing_period_anchor.map(|d|d.with_timezone(&Utc));
            let now = Utc::now();
            if request.deadline <= now { return Err(ApiError::bad_request("Reservation deadline has expired")); }
            let period = active_period(txn, &request.payer_id, now, anchor).await?;
            let used = decode(&period.used)?;
            let reserved = decode(&period.reserved)?;
            let next = used.checked_add(reserved)?.checked_add(request.amounts)?;
            for (i, limit) in limits(&tier).values().into_iter().enumerate() {
                if enforce && limit >= 0 && next.values()[i] > limit {
                    return Err(ApiError::quota_exceeded(QuotaLimitDetails {
                        resource: RESOURCES[i].into(), scope: "account".into(), payer_id: request.payer_id.clone(), plan,
                        required_model_tier: None,
                        used: used.values()[i], reserved: reserved.values()[i], requested: request.amounts.values()[i], limit,
                        unit: UNITS[i].into(), period_end: Some(stamp(period.period_end)),
                        actions: vec!["view_usage".into(), "upgrade".into(), if i == 1 || i == 2 { "use_own_model" } else { "run_locally" }.into()],
                    }));
                }
            }
            txn.execute_raw(sql("INSERT INTO \"QuotaAccount\" (\"payerId\", active) VALUES ($1,0) ON CONFLICT (\"payerId\") DO NOTHING", vec![request.payer_id.clone().into()])).await?;
            if request.amounts.cloud_starts > 0 {
                let row = txn.query_one_raw(sql("SELECT active FROM \"QuotaAccount\" WHERE \"payerId\"=$1", vec![request.payer_id.clone().into()])).await?.ok_or_else(|| ApiError::internal("quota account missing"))?;
                let active: i64 = row.try_get("", "active")?;
                if enforce && tier.max_concurrent_executions >= 0 && active >= i64::from(tier.max_concurrent_executions) {
                    return Err(ApiError::quota_exceeded(QuotaLimitDetails {
                        resource: "concurrent_cloud_executions".into(), scope: "account".into(), payer_id: request.payer_id.clone(), plan,
                        required_model_tier: None,
                        used: active, reserved: 0, requested: 1, limit: tier.max_concurrent_executions.into(), unit: "executions".into(), period_end: None,
                        actions: vec!["wait".into(), "view_usage".into(), "upgrade".into(), "run_locally".into()],
                    }));
                }
                txn.execute_raw(sql("UPDATE \"QuotaAccount\" SET active=active+1 WHERE \"payerId\"=$1", vec![request.payer_id.clone().into()])).await?;
            }
            let amounts = encode(request.amounts);
            let entitlement_version=blake3::hash(&serde_json::to_vec(&tier)?).to_hex().to_string();
            txn.execute_raw(sql("INSERT INTO \"QuotaOperation\" (id,\"payerId\",\"periodId\",\"actorId\",\"appId\",\"modelId\",provider,kind,\"fundingClass\",\"executionMode\",status,ceiling,used,reserved,deadline,\"createdAt\",\"updatedAt\",generation,plan,\"entitlementVersion\") VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,'reserved',$11,$12,$11,$13,$14,$14,0,$15,$16)", vec![
                request.operation_id.into(), request.payer_id.into(), period.id.clone().into(), request.actor_id.into(), request.app_id.into(), request.model_id.into(), request.provider.into(), request.kind.into(), request.funding_class.into(), request.execution_mode.into(), amounts.into(), encode(QuotaAmounts::default()).into(), request.deadline.timestamp_millis().into(), now.timestamp_millis().into(),
                plan.into(),entitlement_version.into(),
            ])).await?;
            txn.execute_raw(sql("UPDATE \"QuotaPeriod\" SET reserved=$2,\"updatedAt\"=$3 WHERE id=$1", vec![period.id.into(), encode(reserved.checked_add(request.amounts)?).into(), now.timestamp_millis().into()])).await?;
            Ok(())
        })
    }).await
}

pub async fn mark_started(state: &AppState, operation_id: &str) -> Result<bool, ApiError> {
    let result = state.db.execute_raw(sql("UPDATE \"QuotaOperation\" SET status='running',\"ownerId\"=$1,generation=generation+1,\"updatedAt\"=$2 WHERE id=$1 AND status='reserved' AND deadline>$2 AND \"cancelRequested\"=FALSE", vec![operation_id.into(), Utc::now().timestamp_millis().into()])).await?;
    if result.rows_affected() == 0 {
        let cancelled=state.db.query_one_raw(sql("SELECT id FROM \"QuotaOperation\" WHERE id=$1 AND status='reserved' AND \"cancelRequested\"=TRUE",vec![operation_id.into()])).await?.is_some();
        if cancelled {
            release_unstarted(state, operation_id, "Cancelled before work started").await?;
        }
    }
    Ok(result.rows_affected() == 1)
}

pub async fn reserve_cloud(
    state: &AppState,
    request: &crate::execution::DispatchRequest,
    mode: &str,
) -> Result<u64, ApiError> {
    if let Some(existing) = operation(&state.db, &request.run_id).await? {
        if existing.app_id.as_deref() != Some(&request.app_id)
            || existing.kind != "workflow"
            || existing.actor_id.as_deref() != Some(&request.user_id)
            || existing.execution_mode != mode
        {
            return Err(ApiError::forbidden(
                "Execution reservation does not match this app",
            ));
        }
        if existing.status != "reserved" {
            return Err(ApiError::conflict(
                "This cloud run was already started or completed",
            ));
        }
        return Ok(decode(&existing.ceiling)?.runtime_ms as u64);
    }
    let payer_id = resolve_payer(state, Some(&request.user_id), Some(&request.app_id)).await?;
    let (_, tier) = payer_plan(state, &payer_id).await?;
    let now = Utc::now();
    let row = state.db.query_one_raw(sql("SELECT used,reserved FROM \"QuotaPeriod\" WHERE \"payerId\"=$1 AND \"periodStart\"<=$2 AND \"periodEnd\">$2 ORDER BY \"periodStart\" DESC LIMIT 1", vec![payer_id.clone().into(), now.timestamp_millis().into()])).await?;
    let occupied = match row {
        Some(row) => decode(&row.try_get::<String>("", "used")?)?
            .runtime_ms
            .saturating_add(decode(&row.try_get::<String>("", "reserved")?)?.runtime_ms),
        None => 0,
    };
    let runtime_ms = if !enforcing() || tier.max_runtime_ms < 0 {
        900_000
    } else {
        tier.max_runtime_ms
            .saturating_sub(occupied)
            .clamp(1, 900_000)
    };
    reserve(
        state,
        QuotaRequest {
            operation_id: request.run_id.clone(),
            payer_id: payer_id.clone(),
            actor_id: Some(request.user_id.clone()),
            app_id: Some(request.app_id.clone()),
            model_id: None,
            provider: None,
            kind: "workflow".into(),
            funding_class: "cloud".into(),
            execution_mode: mode.into(),
            amounts: QuotaAmounts {
                runtime_ms,
                cloud_starts: 1,
                ..Default::default()
            },
            deadline: now + chrono::Duration::hours(24),
        },
    )
    .await?;
    crate::compute_attempts::associate_current(
        &state.db,
        &request.run_id,
        &payer_id,
        "workflow_api",
        "workflow_compute",
    )
    .await?;
    Ok(runtime_ms as u64)
}

pub fn enforcing() -> bool {
    std::env::var("FLOW_LIKE_QUOTA_MODE").as_deref() != Ok("shadow")
}

pub fn receipt_path(operation_id: &str) -> flow_like_storage::Path {
    flow_like_storage::Path::from(format!(
        "system/quota/{}/terminal.json",
        blake3::hash(operation_id.as_bytes()).to_hex()
    ))
}

pub async fn receipt_ttl(
    state: &AppState,
    operation_id: &str,
) -> Result<std::time::Duration, ApiError> {
    let seconds = match operation(&state.db, operation_id).await? {
        Some(op) => {
            op.created_at
                .saturating_add(86_400_000)
                .saturating_sub(Utc::now().timestamp_millis())
                / 1000
        }
        None => 86_400,
    };
    if seconds <= 0 {
        return Err(ApiError::conflict("Cloud dispatch admission has expired"));
    }
    Ok(std::time::Duration::from_secs(seconds as u64))
}

pub(crate) async fn runtime_receipt_candidates(
    db: &DatabaseConnection,
    now: i64,
) -> Result<Vec<String>, ApiError> {
    db.query_all_raw(sql("SELECT id FROM \"QuotaOperation\" WHERE status IN ('running','unknown') AND kind='workflow' AND \"nextReceiptCheckAt\"<=$1 ORDER BY \"nextReceiptCheckAt\",id LIMIT 50", vec![now.into()])).await?.into_iter().map(|row|row.try_get("","id").map_err(ApiError::from)).collect()
}

pub(crate) async fn schedule_runtime_receipt_retry(
    db: &DatabaseConnection,
    id: &str,
    now: i64,
) -> Result<bool, ApiError> {
    Ok(db.execute_raw(sql("UPDATE \"QuotaOperation\" SET \"nextReceiptCheckAt\"=$2 WHERE id=$1 AND status IN ('running','unknown') AND \"nextReceiptCheckAt\"<=$3", vec![id.into(),now.saturating_add(600_000).into(),now.into()])).await?.rows_affected()>0)
}

/// Replay retained worker measurements when its SQL callback was unavailable.
pub async fn reconcile_runtime_receipts(state: &AppState) -> Result<u64, ApiError> {
    let now = Utc::now().timestamp_millis();
    let ids = runtime_receipt_candidates(&state.db, now).await?;
    let started = std::time::Instant::now();
    let mut reconciled = 0;
    for id in ids {
        if started.elapsed() > std::time::Duration::from_secs(20) {
            break;
        }
        // Advance before reading, including missing or invalid receipts, so an
        // unresolved older run cannot starve newer terminal measurements.
        if !schedule_runtime_receipt_retry(&state.db, &id, now).await? {
            continue;
        }
        let read = tokio::time::timeout(std::time::Duration::from_secs(2), async {
            let result = match state.meta_bucket.as_generic().get(&receipt_path(&id)).await {
                Ok(value) => value,
                Err(flow_like_storage::object_store::Error::NotFound { .. }) => return Ok(None),
                Err(error) => return Err(ApiError::from(error)),
            };
            if result.meta.size > 16_384 {
                return Err(ApiError::internal("Runtime receipt exceeds its size limit"));
            }
            let value: Value = serde_json::from_slice(&result.bytes().await?)?;
            Ok::<_, ApiError>(Some(value))
        })
        .await;
        let value = match read {
            Ok(Ok(Some(value))) => value,
            Ok(Ok(None)) => continue,
            error => {
                tracing::warn!(operation_id=%id,?error,"Cannot read runtime receipt; retry scheduled");
                continue;
            }
        };
        if let (Some(attempt), Some(runtime)) =
            (value["attempt_id"].as_str(), value["runtime_ms"].as_u64())
        {
            match finish_cloud(
                state,
                &id,
                attempt,
                runtime,
                value["status"].as_str().unwrap_or("unknown"),
            )
            .await
            {
                Ok(()) => reconciled += 1,
                Err(error) => {
                    tracing::warn!(operation_id=%id,?error,"Cannot reconcile runtime receipt; retry scheduled")
                }
            }
        } else {
            tracing::warn!(operation_id=%id,"Invalid runtime receipt; retry scheduled");
        }
    }
    Ok(reconciled)
}

pub async fn claim_cloud(
    state: &AppState,
    run_id: &str,
    attempt_id: &str,
    limit_ms: u64,
) -> Result<bool, ApiError> {
    let Some(op) = operation(&state.db, run_id).await? else {
        return Err(ApiError::forbidden("Execution has no quota reservation"));
    };
    if op.kind != "workflow" || decode(&op.ceiling)?.runtime_ms != limit_ms as i64 {
        return Err(ApiError::forbidden(
            "Execution quota does not match its signed limit",
        ));
    }
    state.db.execute_raw(sql("UPDATE \"QuotaOperation\" SET status='running',\"ownerId\"=$2,generation=generation+1,deadline=$3,\"updatedAt\"=$4 WHERE id=$1 AND status='reserved' AND deadline>$4 AND \"cancelRequested\"=FALSE", vec![run_id.into(),attempt_id.into(),(Utc::now().timestamp_millis().saturating_add(limit_ms as i64).saturating_add(120_000)).into(),Utc::now().timestamp_millis().into()])).await?;
    let row = state
        .db
        .query_one_raw(sql(
            "SELECT status,\"ownerId\" FROM \"QuotaOperation\" WHERE id=$1",
            vec![run_id.into()],
        ))
        .await?
        .ok_or(ApiError::NOT_FOUND)?;
    Ok(row.try_get::<String>("", "status")? == "running"
        && row.try_get::<Option<String>>("", "ownerId")?.as_deref() == Some(attempt_id))
}

pub async fn finish_cloud(
    state: &AppState,
    run_id: &str,
    attempt_id: &str,
    runtime_ms: u64,
    status: &str,
) -> Result<(), ApiError> {
    let row = state
        .db
        .query_one_raw(sql(
            "SELECT \"ownerId\",\"executionMode\" FROM \"QuotaOperation\" WHERE id=$1",
            vec![run_id.into()],
        ))
        .await?
        .ok_or(ApiError::NOT_FOUND)?;
    if row.try_get::<Option<String>>("", "ownerId")?.as_deref() != Some(attempt_id) {
        return Err(ApiError::forbidden(
            "Execution quota belongs to another worker",
        ));
    }
    if row.try_get::<String>("", "executionMode")? == "realtime" {
        crate::execution::update_run_on_completion_with_runtime(
            &crate::audit::ExecutionAuditContext::from(state),
            run_id,
            crate::execution::completed_run_status(Some(status)),
            if status == "completed" { 0 } else { 4 },
            Some(runtime_ms),
        )
        .await?;
    }
    settle(
        state,
        run_id,
        "workflow-terminal",
        QuotaAmounts {
            runtime_ms: i64::try_from(runtime_ms).unwrap_or(i64::MAX),
            cloud_starts: 1,
            ..Default::default()
        },
        true,
        json!({"status":status,"measuredRuntimeMs":runtime_ms,"workerOwnerId":attempt_id}),
    )
    .await
}

/// A stop request does not imply that the worker has stopped or that its cost is zero.
pub async fn request_cloud_cancellation(state: &AppState, run_id: &str) -> Result<bool, ApiError> {
    let result = state.db.execute_raw(sql("UPDATE \"QuotaOperation\" SET \"cancelRequested\"=TRUE,\"updatedAt\"=$2 WHERE id=$1 AND kind='workflow' AND status IN ('reserved','running','unknown')", vec![run_id.into(),Utc::now().timestamp_millis().into()])).await?;
    Ok(result.rows_affected() > 0)
}

pub async fn cloud_should_continue(
    state: &AppState,
    run_id: &str,
    attempt_id: &str,
) -> Result<bool, ApiError> {
    let row = state.db.query_one_raw(sql("SELECT status,\"ownerId\",\"cancelRequested\",deadline FROM \"QuotaOperation\" WHERE id=$1 AND kind='workflow'",vec![run_id.into()])).await?.ok_or(ApiError::NOT_FOUND)?;
    Ok(
        row.try_get::<Option<String>>("", "ownerId")?.as_deref() == Some(attempt_id)
            && matches!(
                row.try_get::<String>("", "status")?.as_str(),
                "running" | "unknown"
            )
            && !row.try_get::<bool>("", "cancelRequested")?
            && row.try_get::<i64>("", "deadline")? > Utc::now().timestamp_millis(),
    )
}

pub async fn settle(
    state: &AppState,
    operation_id: &str,
    revision: &str,
    actual: QuotaAmounts,
    finalized: bool,
    detail: Value,
) -> Result<(), ApiError> {
    settle_with_db(
        &state.db,
        state.db_dialect,
        operation_id,
        revision,
        actual,
        finalized,
        detail,
    )
    .await
}

pub async fn settle_with_db(
    db: &DatabaseConnection,
    dialect: DbDialect,
    operation_id: &str,
    revision: &str,
    actual: QuotaAmounts,
    finalized: bool,
    detail: Value,
) -> Result<(), ApiError> {
    if actual.values().iter().any(|v| *v < 0) {
        return Err(ApiError::bad_request("Usage cannot be negative"));
    }
    let Some(initial) = operation(db, operation_id).await? else {
        return Ok(());
    };
    let operation_id = operation_id.to_owned();
    let event_id = format!("{operation_id}:{revision}");
    retry_transaction(db, dialect, None, &RetryPolicy::idempotent(), move |txn| {
        let operation_id = operation_id.clone(); let event_id = event_id.clone(); let payer = initial.payer_id.clone(); let detail = detail.clone();
        Box::pin(async move {
            coordinate(txn, "account-quota", &[&payer]).await?;
            if txn.query_one_raw(sql("SELECT id FROM \"QuotaEvent\" WHERE id=$1", vec![event_id.clone().into()])).await?.is_some() { return Ok(()); }
            let op = operation(txn, &operation_id).await?.ok_or_else(|| ApiError::internal("quota operation missing"))?;
            if let Some(owner) = detail.get("workerOwnerId").and_then(Value::as_str) {
                if op.owner_id.as_deref() != Some(owner) { return Err(ApiError::forbidden("A reconciled operation rejects stale worker settlement")); }
            }
            if op.status == "finalized" && !finalized { return Ok(()); }
            let period = period(txn, &op.period_id).await?;
            let used_before = decode(&op.used)?;
            let reserved_before = decode(&op.reserved)?;
            let ceiling = decode(&op.ceiling)?;
            // Platform deadline overruns are recorded internally, never silently
            // charged beyond the capacity authorized before execution.
            let used = actual.bounded(ceiling);
            // Ordinary reports are cumulative. Only an explicit internal adjustment
            // may reduce a prior debit; a delayed callback must never issue a refund.
            if detail.get("adjustment").and_then(Value::as_bool) != Some(true)
                && used.values().iter().zip(used_before.values()).any(|(new, old)| *new < old)
            {
                return Err(ApiError::conflict("Cumulative usage cannot decrease without an explicit adjustment"));
            }
            let is_final = finalized || op.status == "finalized";
            let reserved = if is_final { QuotaAmounts::default() } else { ceiling.remaining(used) };
            let delta = used.delta(used_before);
            let period_used = decode(&period.used)?.checked_add(delta)?;
            let period_reserved = decode(&period.reserved)?.checked_add(reserved.delta(reserved_before))?;
            let now = Utc::now().timestamp_millis();
            txn.execute_raw(sql("UPDATE \"QuotaPeriod\" SET used=$2,reserved=$3,\"updatedAt\"=$4 WHERE id=$1", vec![period.id.into(), encode(period_used).into(), encode(period_reserved).into(), now.into()])).await?;
            txn.execute_raw(sql("UPDATE \"QuotaOperation\" SET used=$2,reserved=$3,status=$4,\"updatedAt\"=$5 WHERE id=$1", vec![operation_id.clone().into(), encode(used).into(), encode(reserved).into(), if is_final { "finalized" } else { "unknown" }.into(), now.into()])).await?;
            if detail.get("administrative").and_then(Value::as_bool)==Some(true) {
                txn.execute_raw(sql("UPDATE \"QuotaOperation\" SET \"ownerId\"=$2,generation=generation+1 WHERE id=$1",vec![operation_id.clone().into(),format!("adjusted:{event_id}").into()])).await?;
            }
            if is_final && op.status != "finalized" && ceiling.cloud_starts > 0 {
                txn.execute_raw(sql("UPDATE \"QuotaAccount\" SET active=active-1 WHERE \"payerId\"=$1 AND active>0", vec![payer.clone().into()])).await?;
            }
            txn.execute_raw(sql("INSERT INTO \"QuotaEvent\" (id,\"operationId\",\"payerId\",actual,detail,\"createdAt\") VALUES ($1,$2,$3,$4,$5,$6)", vec![event_id.into(), operation_id.into(), payer.clone().into(), encode(actual).into(), detail.to_string().into(), now.into()])).await?;
            let day = stamp(op.created_at)[..10].to_owned();
            let dimensions = json!([payer,day,op.app_id,op.model_id,op.provider,op.funding_class,op.execution_mode]);
            let rollup_id = blake3::hash(dimensions.to_string().as_bytes()).to_hex().to_string();
            let old = txn.query_one_raw(sql("SELECT used FROM \"QuotaDailyUsage\" WHERE id=$1", vec![rollup_id.clone().into()])).await?;
            let new = match old { Some(row) => decode(&row.try_get::<String>("", "used")?)?.checked_add(delta)?, None => QuotaAmounts::default().checked_add(delta)? };
            txn.execute_raw(sql("INSERT INTO \"QuotaDailyUsage\" (id,\"payerId\",day,\"appId\",\"modelId\",provider,\"fundingClass\",\"executionMode\",used,\"updatedAt\") VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10) ON CONFLICT (id) DO UPDATE SET used=EXCLUDED.used,\"updatedAt\"=EXCLUDED.\"updatedAt\"", vec![rollup_id.into(), payer.into(), day.into(), op.app_id.into(), op.model_id.into(), op.provider.into(), op.funding_class.into(), op.execution_mode.into(), encode(new).into(), now.into()])).await?;
            Ok(())
        })
    }).await
}

pub async fn release_unstarted(
    state: &AppState,
    operation_id: &str,
    reason: &str,
) -> Result<(), ApiError> {
    release_unstarted_with_db(&state.db, state.db_dialect, operation_id, reason).await
}

/// A published delivery intent may already be accepted by Lambda. Preserve it.
pub async fn release_failed_dispatch(
    state: &AppState,
    operation_id: &str,
    reason: &str,
) -> Result<(), ApiError> {
    if state
        .db
        .query_one_raw(sql(
            "SELECT id FROM \"CloudDispatchIntent\" WHERE id=$1",
            vec![operation_id.into()],
        ))
        .await?
        .is_none()
    {
        release_unstarted(state, operation_id, reason).await?;
    }
    Ok(())
}

pub async fn release_unstarted_with_db(
    db: &DatabaseConnection,
    dialect: DbDialect,
    operation_id: &str,
    reason: &str,
) -> Result<(), ApiError> {
    // The durable releasing state fences workers and lets a later retry complete
    // settlement if the process stops between these transactions.
    let changed = db.execute_raw(sql("UPDATE \"QuotaOperation\" SET status='releasing' WHERE id=$1 AND (status IN ('reserved','releasing') OR (status='unknown' AND \"ownerId\" IS NULL AND deadline<$2))", vec![operation_id.into(),Utc::now().timestamp_millis().into()])).await?;
    if changed.rows_affected() == 1 {
        settle_with_db(
            db,
            dialect,
            operation_id,
            "unstarted",
            QuotaAmounts::default(),
            true,
            json!({"reason":reason,"notStarted":true}),
        )
        .await?;
        if let Some(row) = db
            .query_one_raw(sql(
                "SELECT kind FROM \"QuotaOperation\" WHERE id=$1",
                vec![operation_id.into()],
            ))
            .await?
        {
            if matches!(
                row.try_get::<String>("", "kind")?.as_str(),
                "llm" | "embedding"
            ) {
                crate::usage_accounting::release_unstarted_hosted(db, dialect, operation_id)
                    .await?;
            }
        }
    }
    Ok(())
}

pub async fn recover_unstarted_releases(
    db: &DatabaseConnection,
    dialect: DbDialect,
    limit: u64,
) -> Result<u64, ApiError> {
    let rows = db.query_all_raw(sql("SELECT id FROM \"QuotaOperation\" WHERE status='releasing' OR (status='unknown' AND \"ownerId\" IS NULL AND deadline<$2) ORDER BY deadline LIMIT $1", vec![(limit.clamp(1,500) as i64).into(),Utc::now().timestamp_millis().into()])).await?;
    let mut recovered = 0;
    for row in rows {
        let id: String = row.try_get("", "id")?;
        release_unstarted_with_db(db, dialect, &id, "recovered interrupted release").await?;
        recovered += 1;
    }
    Ok(recovered)
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResourceUsage {
    pub resource: String,
    pub used: i64,
    pub reserved: i64,
    pub limit: i64,
    pub remaining: Option<i64>,
    pub unit: String,
    pub threshold: u8,
}

fn resource_usage(
    resource: &str,
    used: i64,
    reserved: i64,
    limit: i64,
    unit: &str,
) -> ResourceUsage {
    let threshold = if limit < 0 {
        0
    } else if used >= limit {
        100
    } else if i128::from(used) * 100 >= i128::from(limit) * 90 {
        90
    } else if i128::from(used) * 100 >= i128::from(limit) * 75 {
        75
    } else {
        0
    };
    ResourceUsage {
        resource: resource.into(),
        used,
        reserved,
        limit,
        remaining: (limit >= 0).then(|| limit.saturating_sub(used).saturating_sub(reserved).max(0)),
        unit: unit.into(),
        threshold,
    }
}

pub async fn summary(state: &AppState, payer_id: &str, days: i64) -> Result<Value, ApiError> {
    let (plan, tier) = payer_plan(state, payer_id).await?;
    let payer = user::Entity::find_by_id(payer_id)
        .one(&state.db)
        .await?
        .ok_or(ApiError::NOT_FOUND)?;
    let anchor = payer.billing_period_anchor.map(|d| d.with_timezone(&Utc));
    let id = payer_id.to_owned();
    let row = retry_transaction(
        &state.db,
        state.db_dialect,
        None,
        &RetryPolicy::idempotent(),
        move |txn| {
            let id = id.clone();
            Box::pin(async move {
                coordinate(txn, "account-quota", &[&id]).await?;
                active_period(txn, &id, Utc::now(), anchor).await
            })
        },
    )
    .await?;
    let used = decode(&row.used)?.values();
    let reserved = decode(&row.reserved)?.values();
    let caps = limits(&tier).values();
    let mut resources: Vec<_> = (0..4)
        .map(|i| resource_usage(RESOURCES[i], used[i], reserved[i], caps[i], UNITS[i]))
        .collect();
    let capacity = crate::capacity::usage(state, payer_id).await?;
    resources.push(resource_usage(
        "storage_bytes",
        capacity.storage_bytes,
        capacity.reserved_storage_bytes,
        tier.max_total_size,
        "bytes",
    ));
    resources.push(resource_usage(
        "projects",
        capacity.project_count,
        0,
        tier.max_non_visible_projects.into(),
        "projects",
    ));
    let active = state
        .db
        .query_one_raw(sql(
            "SELECT active FROM \"QuotaAccount\" WHERE \"payerId\"=$1",
            vec![payer_id.into()],
        ))
        .await?
        .map(|r| r.try_get::<i64>("", "active"))
        .transpose()?
        .unwrap_or(0);
    resources.push(resource_usage(
        "concurrent_cloud_executions",
        active,
        0,
        tier.max_concurrent_executions.into(),
        "executions",
    ));
    let warnings = crate::quota_warnings::record_warnings(
        &state.db,
        state.db_dialect,
        payer_id,
        &plan,
        &row.id,
        &stamp(row.period_end),
        &resources,
    )
    .await?;
    let since = (Utc::now() - chrono::Duration::days(days.clamp(1, 90)))
        .format("%Y-%m-%d")
        .to_string();
    let rows = state.db.query_all_raw(sql("SELECT * FROM \"QuotaDailyUsage\" WHERE \"payerId\"=$1 AND day>=$2 ORDER BY day DESC,id DESC LIMIT 1001", vec![payer_id.into(), since.into()])).await?;
    let truncated = rows.len() > 1000;
    let mut usage = Vec::new();
    for r in rows.into_iter().take(1000) {
        let amounts = decode(&r.try_get::<String>("", "used")?)?;
        usage.push(json!({"day":r.try_get::<String>("","day")?,"appId":r.try_get::<Option<String>>("","appId")?,"modelId":r.try_get::<Option<String>>("","modelId")?,"provider":r.try_get::<Option<String>>("","provider")?,"fundingClass":r.try_get::<String>("","fundingClass")?,"executionMode":r.try_get::<String>("","executionMode")?,"runtimeMs":amounts.runtime_ms,"aiCostMicros":amounts.ai_cost_micros,"aiCalls":amounts.ai_calls,"cloudStarts":amounts.cloud_starts}));
    }
    Ok(
        json!({"plan":plan,"payerId":payer_id,"periodStart":stamp(row.period_start),"periodEnd":stamp(row.period_end),"trackingSince":stamp(row.tracking_started_at),"resources":resources,"warnings":warnings,"usage":usage,"usageTruncated":truncated,"updatedAt":Utc::now().to_rfc3339()}),
    )
}

pub async fn operations(
    state: &AppState,
    payer_id: &str,
    cursor: Option<String>,
    limit: u64,
) -> Result<Value, ApiError> {
    let limit = limit.clamp(1, 100);
    let rows = state.db.query_all_raw(sql("SELECT * FROM \"QuotaOperation\" WHERE \"payerId\"=$1 AND ($2::text IS NULL OR id<$2) ORDER BY id DESC LIMIT $3", vec![payer_id.into(), cursor.into(), (limit as i64 + 1).into()])).await?;
    let has_more = rows.len() > limit as usize;
    let mut items = Vec::new();
    for r in rows.into_iter().take(limit as usize) {
        let op = OperationRow::from_query_result(&r, "")?;
        items.push(json!({"id":op.id,"appId":op.app_id,"modelId":op.model_id,"provider":op.provider,"kind":op.kind,"fundingClass":op.funding_class,"executionMode":op.execution_mode,"status":op.status,"used":decode(&op.used)?,"reserved":decode(&op.reserved)?,"createdAt":stamp(op.created_at)}));
    }
    let next = if has_more {
        items.last().and_then(|v| v.get("id")).cloned()
    } else {
        None
    };
    Ok(json!({"items":items,"nextCursor":next}))
}

/// Expiry is evidence that work needs reconciliation, never evidence of no cost.
pub async fn flag_stale(db: &DatabaseConnection, limit: u64) -> Result<u64, ApiError> {
    let rows = db.query_all_raw(sql("SELECT id FROM \"QuotaOperation\" WHERE status IN ('reserved','running') AND deadline<$1 ORDER BY deadline LIMIT $2", vec![Utc::now().timestamp_millis().into(), (limit.clamp(1,500) as i64).into()])).await?;
    let mut count = 0;
    for row in rows {
        count += db.execute_raw(sql("UPDATE \"QuotaOperation\" SET status='unknown',\"updatedAt\"=$2 WHERE id=$1 AND status IN ('reserved','running')", vec![row.try_get::<String>("","id")?.into(), Utc::now().timestamp_millis().into()])).await?.rows_affected();
    }
    Ok(count)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn anniversaries_keep_original_day_after_short_months() {
        let anchor = Utc.with_ymd_and_hms(2024, 1, 31, 12, 30, 0).unwrap();
        assert_eq!(
            monthly_period(
                Utc.with_ymd_and_hms(2024, 2, 29, 13, 0, 0).unwrap(),
                Some(anchor)
            ),
            (
                Utc.with_ymd_and_hms(2024, 2, 29, 12, 30, 0).unwrap(),
                Utc.with_ymd_and_hms(2024, 3, 31, 12, 30, 0).unwrap()
            )
        );
        assert_eq!(
            monthly_period(
                Utc.with_ymd_and_hms(2025, 2, 28, 12, 29, 0).unwrap(),
                Some(anchor)
            )
            .0,
            Utc.with_ymd_and_hms(2025, 1, 31, 12, 30, 0).unwrap()
        );
    }

    #[test]
    fn reservations_do_not_generate_false_consumed_warnings() {
        assert_eq!(resource_usage("runtime", 20, 80, 100, "ms").threshold, 0);
        assert_eq!(resource_usage("runtime", 75, 20, 100, "ms").threshold, 75);
        assert_eq!(resource_usage("runtime", 90, 10, 100, "ms").threshold, 90);
        assert_eq!(resource_usage("runtime", 100, 0, 100, "ms").threshold, 100);
        assert_eq!(resource_usage("runtime", 100, 0, -1, "ms").threshold, 0);
    }

    #[test]
    fn settlement_revisions_preserve_unresolved_capacity_and_bound_overruns() {
        let ceiling = QuotaAmounts {
            runtime_ms: 1000,
            ai_cost_micros: 100,
            ..Default::default()
        };
        let first = QuotaAmounts {
            runtime_ms: 400,
            ai_cost_micros: 50,
            ..Default::default()
        };
        assert_eq!(ceiling.remaining(first).ai_cost_micros, 50);
        let final_usage = QuotaAmounts {
            runtime_ms: 1300,
            ai_cost_micros: 80,
            ..Default::default()
        }
        .bounded(ceiling);
        assert_eq!(final_usage.runtime_ms, 1000);
        assert_eq!(
            first.checked_add(final_usage.delta(first)).unwrap(),
            final_usage
        );
        assert!(
            QuotaAmounts::default()
                .checked_add(QuotaAmounts {
                    runtime_ms: -1,
                    ..Default::default()
                })
                .is_err()
        );
    }
}
