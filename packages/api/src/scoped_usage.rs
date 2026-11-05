//! Monthly budgets retain admission capacity until the corresponding quota operation settles.

use crate::{
    db::{RetryPolicy, coordination::coordinate, retry_transaction},
    error::{ApiError, QuotaLimitDetails},
    quota::{QuotaAmounts, QuotaRequest},
    state::AppState,
};
use chrono::Utc;
use sea_orm::{ConnectionTrait, DatabaseTransaction, FromQueryResult, Statement};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MonthlyUsageLimits {
    pub ai_cost_micros: Option<i64>,
    pub ai_calls: Option<i64>,
    pub runtime_ms: Option<i64>,
}

impl MonthlyUsageLimits {
    pub fn validate(&self) -> Result<(), ApiError> {
        if [self.ai_cost_micros, self.ai_calls, self.runtime_ms]
            .into_iter()
            .flatten()
            .any(|value| !(0..=9_007_199_254_740_991).contains(&value))
        {
            return Err(ApiError::bad_request(
                "Monthly limits must be nonnegative safe integers",
            ));
        }
        Ok(())
    }

    fn within(&self, ceiling: &Self) -> bool {
        [
            (self.ai_cost_micros, ceiling.ai_cost_micros),
            (self.ai_calls, ceiling.ai_calls),
            (self.runtime_ms, ceiling.runtime_ms),
        ]
        .into_iter()
        .all(|(limit, cap)| limit.is_none() || cap.is_none() || limit <= cap)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, FromQueryResult)]
#[serde(rename_all = "camelCase")]
pub struct BudgetScope {
    pub scope: String,
    #[sea_orm(from_alias = "subjectId")]
    pub subject_id: String,
    #[sea_orm(from_alias = "groupId")]
    pub group_id: Option<String>,
}

impl BudgetScope {
    pub fn app(id: &str) -> Self {
        Self::new("app", id, None)
    }
    pub fn bot(id: &str) -> Self {
        Self::new("bot", id, None)
    }
    pub fn bot_group(bot: &str, group: &str) -> Self {
        Self::new("bot_group", bot, Some(group))
    }
    pub fn default_app() -> Self {
        Self::new("default_app", "", None)
    }
    pub fn default_bot() -> Self {
        Self::new("default_bot", "", None)
    }
    fn new(scope: &str, subject: &str, group: Option<&str>) -> Self {
        Self {
            scope: scope.into(),
            subject_id: subject.into(),
            group_id: group.map(str::to_owned),
        }
    }
    pub fn id(&self) -> String {
        blake3::hash(
            serde_json::json!([self.scope, self.subject_id, self.group_id])
                .to_string()
                .as_bytes(),
        )
        .to_hex()
        .to_string()
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BudgetPolicy {
    pub owner_limits: MonthlyUsageLimits,
    pub admin_limits: Option<MonthlyUsageLimits>,
}

fn sql(text: &str, values: Vec<sea_orm::Value>) -> Statement {
    Statement::from_sql_and_values(sea_orm::DbBackend::Postgres, text, values)
}

pub async fn get_policy<C: ConnectionTrait>(
    db: &C,
    scope: &BudgetScope,
) -> Result<BudgetPolicy, ApiError> {
    let Some(row) = db
        .query_one_raw(sql(
            r#"SELECT "ownerLimits","adminLimits" FROM "UsageBudgetPolicy" WHERE id=$1"#,
            vec![scope.id().into()],
        ))
        .await?
    else {
        return Ok(BudgetPolicy::default());
    };
    Ok(BudgetPolicy {
        owner_limits: row
            .try_get::<Option<String>>("", "ownerLimits")?
            .map(|value| serde_json::from_str(&value))
            .transpose()?
            .unwrap_or_default(),
        admin_limits: row
            .try_get::<Option<String>>("", "adminLimits")?
            .map(|value| serde_json::from_str(&value))
            .transpose()?,
    })
}

pub async fn default_limits<C: ConnectionTrait>(
    db: &C,
    scope: &BudgetScope,
) -> Result<MonthlyUsageLimits, ApiError> {
    if scope.scope == "bot_group" {
        let defaults = get_policy(db, &BudgetScope::default_bot())
            .await?
            .admin_limits
            .unwrap_or_default();
        let bot = get_policy(db, &BudgetScope::bot(&scope.subject_id)).await?;
        return Ok(effective_limits(&bot, &defaults));
    }
    let default = match scope.scope.as_str() {
        "app" => BudgetScope::default_app(),
        "bot" => BudgetScope::default_bot(),
        _ => return Ok(MonthlyUsageLimits::default()),
    };
    Ok(get_policy(db, &default)
        .await?
        .admin_limits
        .unwrap_or_default())
}

pub fn effective_limits(policy: &BudgetPolicy, default: &MonthlyUsageLimits) -> MonthlyUsageLimits {
    let cap = policy.admin_limits.as_ref().unwrap_or(default);
    let minimum = |owner: Option<i64>, admin: Option<i64>| match (owner, admin) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (a, b) => a.or(b),
    };
    MonthlyUsageLimits {
        ai_cost_micros: minimum(policy.owner_limits.ai_cost_micros, cap.ai_cost_micros),
        ai_calls: minimum(policy.owner_limits.ai_calls, cap.ai_calls),
        runtime_ms: minimum(policy.owner_limits.runtime_ms, cap.runtime_ms),
    }
}

pub async fn save_policy(
    state: &AppState,
    scope: &BudgetScope,
    limits: Option<MonthlyUsageLimits>,
    admin: bool,
    actor_id: &str,
) -> Result<BudgetPolicy, ApiError> {
    if let Some(limits) = &limits {
        limits.validate()?;
    }
    let scope = scope.clone();
    let actor = actor_id.to_owned();
    retry_transaction(
        &state.db,
        state.db_dialect,
        None,
        &RetryPolicy::idempotent(),
        move |txn| {
            let scope = scope.clone();
            let limits = limits.clone();
            let actor = actor.clone();
            Box::pin(async move {
                lock_scopes(txn, vec![scope.id()]).await?;
                write_policy(txn, &scope, limits, admin, &actor).await
            })
        },
    )
    .await
}

pub async fn save_defaults(
    state: &AppState,
    apps: MonthlyUsageLimits,
    bots: MonthlyUsageLimits,
    actor_id: &str,
) -> Result<(), ApiError> {
    apps.validate()?;
    bots.validate()?;
    let actor = actor_id.to_owned();
    retry_transaction(
        &state.db,
        state.db_dialect,
        None,
        &RetryPolicy::idempotent(),
        move |txn| {
            let apps = apps.clone();
            let bots = bots.clone();
            let actor = actor.clone();
            Box::pin(async move {
                lock_scopes(
                    txn,
                    vec![
                        BudgetScope::default_app().id(),
                        BudgetScope::default_bot().id(),
                    ],
                )
                .await?;
                write_policy(txn, &BudgetScope::default_app(), Some(apps), true, &actor).await?;
                write_policy(txn, &BudgetScope::default_bot(), Some(bots), true, &actor).await?;
                Ok(())
            })
        },
    )
    .await
}

async fn write_policy(
    txn: &DatabaseTransaction,
    scope: &BudgetScope,
    limits: Option<MonthlyUsageLimits>,
    admin: bool,
    actor: &str,
) -> Result<BudgetPolicy, ApiError> {
    let before = get_policy(txn, scope).await?;
    let mut after = before.clone();
    if admin {
        after.admin_limits = limits;
    } else {
        if !matches!(scope.scope.as_str(), "app" | "bot" | "bot_group") {
            return Err(ApiError::forbidden(
                "Only enterprise administrators can change defaults",
            ));
        }
        let defaults = default_limits(txn, scope).await?;
        let ceiling = before.admin_limits.as_ref().unwrap_or(&defaults);
        let limits = limits.unwrap_or_default();
        if !limits.within(ceiling) {
            return Err(ApiError::forbidden(
                "An enterprise administrator must approve a higher monthly limit",
            ));
        }
        after.owner_limits = limits;
    }
    txn.execute_raw(sql(r#"INSERT INTO "UsageBudgetPolicy" (id,scope,"subjectId","groupId","ownerLimits","adminLimits","updatedAt") VALUES ($1,$2,$3,$4,$5,$6,$7) ON CONFLICT (id) DO UPDATE SET "ownerLimits"=EXCLUDED."ownerLimits","adminLimits"=EXCLUDED."adminLimits","updatedAt"=EXCLUDED."updatedAt""#,
        vec![scope.id().into(), scope.scope.clone().into(), scope.subject_id.clone().into(), scope.group_id.clone().into(), serde_json::to_string(&after.owner_limits)?.into(), after.admin_limits.as_ref().map(serde_json::to_string).transpose()?.into(), Utc::now().timestamp_millis().into()])).await?;
    let action = if scope.scope.starts_with("default_") {
        "set_enterprise_defaults"
    } else if admin {
        "set_monthly_admin_limits"
    } else {
        "set_monthly_owner_limits"
    };
    txn.execute_raw(sql(r#"INSERT INTO "UsageLimitAuditLog" (id,"appId","actorUserId",action,before,after,"createdAt") VALUES ($1,$2,$3,$4,$5,$6,$7)"#,
        vec![flow_like_types::create_id().into(), (scope.scope == "app").then(|| scope.subject_id.clone()).into(), actor.into(), action.into(), serde_json::json!({"scope":scope,"policy":before}).into(), serde_json::json!({"scope":scope,"policy":after}).into(), Utc::now().fixed_offset().into()])).await?;
    Ok(after)
}

pub async fn current_usage<C: ConnectionTrait>(
    db: &C,
    scope: &BudgetScope,
) -> Result<(QuotaAmounts, QuotaAmounts), ApiError> {
    let (start, _) = crate::quota::monthly_period(Utc::now(), None);
    let id = format!("{}:{}", scope.id(), start.timestamp_millis());
    usage_for_period(db, &id).await
}

async fn usage_for_period<C: ConnectionTrait>(
    db: &C,
    id: &str,
) -> Result<(QuotaAmounts, QuotaAmounts), ApiError> {
    let Some(row) = db
        .query_one_raw(sql(
            r#"SELECT used,reserved FROM "UsageBudgetPeriod" WHERE id=$1"#,
            vec![id.into()],
        ))
        .await?
    else {
        return Ok((QuotaAmounts::default(), QuotaAmounts::default()));
    };
    Ok((
        serde_json::from_str(&row.try_get::<String>("", "used")?)?,
        serde_json::from_str(&row.try_get::<String>("", "reserved")?)?,
    ))
}

pub async fn list_scopes<C: ConnectionTrait>(
    db: &C,
    scope: Option<&str>,
    limit: u64,
) -> Result<Vec<BudgetScope>, ApiError> {
    let (start, _) = crate::quota::monthly_period(Utc::now(), None);
    let rows = db.query_all_raw(sql(r#"SELECT scope,"subjectId","groupId" FROM (SELECT scope,"subjectId","groupId" FROM "UsageBudgetPolicy" UNION SELECT scope,"subjectId","groupId" FROM "UsageBudgetPeriod" WHERE "periodStart"=$1) s WHERE scope IN ('app','bot','bot_group') AND ($2::text IS NULL OR scope=$2) ORDER BY scope,"subjectId","groupId" LIMIT $3"#,
        vec![start.timestamp_millis().into(), scope.map(str::to_owned).into(), (limit.min(1000) as i64).into()])).await?;
    rows.iter()
        .map(|row| BudgetScope::from_query_result(row, "").map_err(ApiError::from))
        .collect()
}

fn scopes(request: &QuotaRequest) -> Vec<BudgetScope> {
    let mut scopes = Vec::new();
    if let Some(app) = &request.app_id {
        scopes.push(BudgetScope::app(app));
    }
    if let Some(bot) = &request.bot_id {
        scopes.push(BudgetScope::bot(bot));
        if let Some(group) = &request.conversation_id {
            scopes.push(BudgetScope::bot_group(bot, group));
        }
    }
    scopes
}

async fn lock_scopes(txn: &DatabaseTransaction, mut ids: Vec<String>) -> Result<(), ApiError> {
    ids.sort();
    ids.dedup();
    for id in ids {
        coordinate(txn, "usage-budget", &[&id]).await?;
    }
    Ok(())
}

pub(crate) async fn reserve(
    txn: &DatabaseTransaction,
    request: &mut QuotaRequest,
    clamp_runtime: bool,
) -> Result<(), ApiError> {
    let scopes = scopes(request);
    if scopes.is_empty() {
        return Ok(());
    }
    lock_scopes(txn, scopes.iter().map(BudgetScope::id).collect()).await?;
    let (start, end) = crate::quota::monthly_period(Utc::now(), None);
    let mut budgets = Vec::new();
    for scope in scopes {
        let id = format!("{}:{}", scope.id(), start.timestamp_millis());
        let policy = get_policy(txn, &scope).await?;
        let limits = effective_limits(&policy, &default_limits(txn, &scope).await?);
        let (used, reserved) = usage_for_period(txn, &id).await?;
        if clamp_runtime && let Some(limit) = limits.runtime_ms {
            let remaining = limit
                .saturating_sub(used.runtime_ms)
                .saturating_sub(reserved.runtime_ms);
            request.amounts.runtime_ms = request.amounts.runtime_ms.min(remaining.max(1));
        }
        budgets.push((scope, id, limits, used, reserved));
    }
    for (scope, id, limits, used, reserved) in budgets {
        let next = used.checked_add(reserved)?.checked_add(request.amounts)?;
        for (resource, unit, limit, consumed, held, requested, total) in [
            (
                "hosted_ai_cost_micros",
                "eur_micros",
                limits.ai_cost_micros,
                used.ai_cost_micros,
                reserved.ai_cost_micros,
                request.amounts.ai_cost_micros,
                next.ai_cost_micros,
            ),
            (
                "hosted_ai_calls",
                "operations",
                limits.ai_calls,
                used.ai_calls,
                reserved.ai_calls,
                request.amounts.ai_calls,
                next.ai_calls,
            ),
            (
                "cloud_runtime_ms",
                "milliseconds",
                limits.runtime_ms,
                used.runtime_ms,
                reserved.runtime_ms,
                request.amounts.runtime_ms,
                next.runtime_ms,
            ),
        ] {
            if let Some(limit) = limit.filter(|limit| total > *limit) {
                return Err(ApiError::quota_exceeded(QuotaLimitDetails {
                    resource: resource.into(),
                    scope: scope.scope.clone(),
                    payer_id: request.payer_id.clone(),
                    plan: "enterprise_policy".into(),
                    required_model_tier: None,
                    used: consumed,
                    reserved: held,
                    requested,
                    limit,
                    unit: unit.into(),
                    period_end: Some(end.to_rfc3339()),
                    actions: vec!["view_usage".into(), "contact_admin".into()],
                }));
            }
        }
        txn.execute_raw(sql(r#"INSERT INTO "UsageBudgetPeriod" (id,"scopeId",scope,"subjectId","groupId","periodStart","periodEnd",used,reserved,"updatedAt") VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10) ON CONFLICT (id) DO UPDATE SET reserved=EXCLUDED.reserved,"updatedAt"=EXCLUDED."updatedAt""#,
            vec![id.clone().into(),scope.id().into(),scope.scope.into(),scope.subject_id.into(),scope.group_id.into(),start.timestamp_millis().into(),end.timestamp_millis().into(),serde_json::to_string(&used)?.into(),serde_json::to_string(&reserved.checked_add(request.amounts)?)?.into(),Utc::now().timestamp_millis().into()])).await?;
        let reservation_id = blake3::hash(
            serde_json::json!([request.operation_id, id])
                .to_string()
                .as_bytes(),
        )
        .to_hex()
        .to_string();
        txn.execute_raw(sql(r#"INSERT INTO "UsageBudgetReservation" (id,"operationId","periodId") VALUES ($1,$2,$3)"#,vec![reservation_id.into(),request.operation_id.clone().into(),id.into()])).await?;
    }
    Ok(())
}

pub(crate) async fn settle(
    txn: &DatabaseTransaction,
    operation: &str,
    used_delta: QuotaAmounts,
    reserved_delta: QuotaAmounts,
) -> Result<(), ApiError> {
    let links = txn.query_all_raw(sql(r#"SELECT p."scopeId" FROM "UsageBudgetReservation" r JOIN "UsageBudgetPeriod" p ON p.id=r."periodId" WHERE r."operationId"=$1"#, vec![operation.into()])).await?;
    if links.is_empty() {
        return Ok(());
    }
    let ids = links
        .iter()
        .map(|row| row.try_get::<String>("", "scopeId"))
        .collect::<Result<Vec<_>, _>>()?;
    lock_scopes(txn, ids).await?;
    let rows = txn.query_all_raw(sql(r#"SELECT p.id,p.used,p.reserved FROM "UsageBudgetReservation" r JOIN "UsageBudgetPeriod" p ON p.id=r."periodId" WHERE r."operationId"=$1"#, vec![operation.into()])).await?;
    for row in rows {
        let used: QuotaAmounts = serde_json::from_str(&row.try_get::<String>("", "used")?)?;
        let reserved: QuotaAmounts = serde_json::from_str(&row.try_get::<String>("", "reserved")?)?;
        txn.execute_raw(sql(
            r#"UPDATE "UsageBudgetPeriod" SET used=$2,reserved=$3,"updatedAt"=$4 WHERE id=$1"#,
            vec![
                row.try_get::<String>("", "id")?.into(),
                serde_json::to_string(&used.checked_add(used_delta)?)?.into(),
                serde_json::to_string(&reserved.checked_add(reserved_delta)?)?.into(),
                Utc::now().timestamp_millis().into(),
            ],
        ))
        .await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use sea_orm::TransactionTrait;

    #[flow_like_types::tokio::test]
    #[ignore = "requires a disposable PostgreSQL database"]
    async fn owner_changes_require_admin_ceilings_and_write_atomic_audit_records() {
        let db = crate::quota_integration_tests::fixture().await;
        let bot = BudgetScope::bot("bot");
        let group = BudgetScope::bot_group("bot", "group");
        let limit = |calls| MonthlyUsageLimits {
            ai_calls: Some(calls),
            ..Default::default()
        };
        let txn = db.begin().await.unwrap();
        write_policy(
            &txn,
            &BudgetScope::default_bot(),
            Some(limit(10)),
            true,
            "admin",
        )
        .await
        .unwrap();
        txn.commit().await.unwrap();

        let txn = db.begin().await.unwrap();
        let error = write_policy(&txn, &bot, Some(limit(11)), false, "owner")
            .await
            .unwrap_err();
        assert_eq!(error.status(), axum::http::StatusCode::FORBIDDEN);
        txn.rollback().await.unwrap();
        assert_eq!(
            get_policy(&db, &bot).await.unwrap().owner_limits,
            MonthlyUsageLimits::default()
        );

        let txn = db.begin().await.unwrap();
        write_policy(&txn, &bot, Some(limit(20)), true, "admin")
            .await
            .unwrap();
        write_policy(&txn, &bot, Some(limit(15)), false, "owner")
            .await
            .unwrap();
        write_policy(&txn, &group, Some(limit(7)), false, "owner")
            .await
            .unwrap();
        txn.commit().await.unwrap();
        assert_eq!(
            default_limits(&db, &group).await.unwrap().ai_calls,
            Some(15)
        );
        let group_policy = get_policy(&db, &group).await.unwrap();
        assert_eq!(
            effective_limits(&group_policy, &default_limits(&db, &group).await.unwrap()).ai_calls,
            Some(7)
        );

        let txn = db.begin().await.unwrap();
        assert!(
            write_policy(&txn, &group, Some(limit(16)), false, "owner")
                .await
                .is_err()
        );
        txn.rollback().await.unwrap();
        let row = db
            .query_one_raw(sql(
                r#"SELECT COUNT(*) AS total FROM "UsageLimitAuditLog""#,
                vec![],
            ))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(row.try_get::<i64>("", "total").unwrap(), 4);

        // Removing the approval restores the current enterprise ceiling, even for older owner limits.
        let txn = db.begin().await.unwrap();
        write_policy(&txn, &bot, None, true, "admin").await.unwrap();
        txn.commit().await.unwrap();
        let policy = get_policy(&db, &bot).await.unwrap();
        assert_eq!(
            effective_limits(&policy, &default_limits(&db, &bot).await.unwrap()).ai_calls,
            Some(10)
        );
        db.close().await.unwrap();
    }

    #[test]
    fn calendar_months_roll_over_at_midnight_utc() {
        let (start, end) = crate::quota::monthly_period(
            Utc.with_ymd_and_hms(2028, 2, 29, 23, 59, 59).unwrap(),
            None,
        );
        assert_eq!(start, Utc.with_ymd_and_hms(2028, 2, 1, 0, 0, 0).unwrap());
        assert_eq!(end, Utc.with_ymd_and_hms(2028, 3, 1, 0, 0, 0).unwrap());
        assert_eq!(crate::quota::monthly_period(end, None).0, end);
    }

    #[test]
    fn owner_limits_cannot_raise_the_enterprise_ceiling() {
        let default = MonthlyUsageLimits {
            ai_cost_micros: Some(100),
            ai_calls: Some(10),
            runtime_ms: None,
        };
        let mut policy = BudgetPolicy {
            owner_limits: MonthlyUsageLimits {
                ai_cost_micros: Some(200),
                ai_calls: Some(3),
                runtime_ms: Some(0),
            },
            admin_limits: None,
        };
        assert!(!policy.owner_limits.within(&default));
        assert_eq!(
            effective_limits(&policy, &default),
            MonthlyUsageLimits {
                ai_cost_micros: Some(100),
                ai_calls: Some(3),
                runtime_ms: Some(0)
            }
        );
        policy.admin_limits = Some(MonthlyUsageLimits {
            ai_cost_micros: Some(300),
            ..Default::default()
        });
        assert_eq!(
            effective_limits(&policy, &default).ai_cost_micros,
            Some(200)
        );
        assert!(MonthlyUsageLimits::default().within(&default));
    }

    #[test]
    fn explicit_unlimited_admin_override_and_reset_are_distinct() {
        let default = MonthlyUsageLimits {
            ai_calls: Some(1),
            ..Default::default()
        };
        let mut policy = BudgetPolicy {
            admin_limits: Some(MonthlyUsageLimits::default()),
            ..Default::default()
        };
        assert_eq!(effective_limits(&policy, &default).ai_calls, None);
        policy.admin_limits = None;
        assert_eq!(effective_limits(&policy, &default).ai_calls, Some(1));
    }

    #[test]
    fn identifiers_are_unambiguous_and_limits_are_validated() {
        assert_ne!(
            BudgetScope::bot_group("a:b", "c").id(),
            BudgetScope::bot_group("a", "b:c").id()
        );
        assert!(
            MonthlyUsageLimits {
                ai_calls: Some(-1),
                ..Default::default()
            }
            .validate()
            .is_err()
        );
        assert!(
            MonthlyUsageLimits {
                ai_calls: Some(0),
                ..Default::default()
            }
            .validate()
            .is_ok()
        );
    }
}
