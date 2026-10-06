use super::*;
use crate::entity::sea_orm_active_enums::BitType;
use flow_like::hub::{UserTier, UserTiers};
use sea_orm::{ActiveEnum, DatabaseTransaction, QueryResult};
use std::{collections::HashMap, sync::Arc};

/// Whether the caller's plan covers the hosted models of an approval they would pay for.
#[derive(Debug, Serialize, PartialEq, Eq, utoipa::ToSchema)]
pub(crate) struct BillingEligibility {
    pub payer_id: String,
    pub plan: String,
    /// The plan covers every model the approval allows.
    pub eligible: bool,
    pub models: Vec<ModelEligibility>,
}

#[derive(Debug, Serialize, PartialEq, Eq, utoipa::ToSchema)]
pub(crate) struct ModelEligibility {
    pub model_id: String,
    /// The plan tier the model requires; `null` when it requires none or is no longer offered.
    pub tier: Option<String>,
    pub allowed: bool,
}

struct Admission {
    authority: VerifiedInstanceUsage,
    required_model_tier: String,
    ceiling: i64,
    used: i64,
    reserved: i64,
    status: String,
}
fn admission(row: QueryResult) -> Result<Admission, ApiError> {
    Ok(Admission {
        authority: serde_json::from_str(&row.try_get::<String>("", "authority")?)?,
        required_model_tier: row.try_get("", "requiredModelTier")?,
        ceiling: row.try_get("", "ceilingMicros")?,
        used: row.try_get("", "usedMicros")?,
        reserved: row.try_get("", "reservedMicros")?,
        status: row.try_get("", "status")?,
    })
}
async fn read_admission(tx: &DatabaseTransaction, id: &str) -> Result<Option<Admission>, ApiError> {
    tx.query_one_raw(sql(
        r#"SELECT * FROM "InstanceUsageAdmission" WHERE "operationId"=$1"#,
        [id.into()],
    ))
    .await?
    .map(admission)
    .transpose()
}

async fn model_and_tier(
    tx: &DatabaseTransaction,
    usage: &VerifiedInstanceUsage,
    expected_tier: &str,
    tiers: &UserTiers,
) -> Result<(), ApiError> {
    // Bit edits and deletion conflict with the final authorization decision.
    if tx
        .execute_raw(sql(
            r#"UPDATE "Bit" SET "updatedAt"="updatedAt" WHERE id=$1"#,
            [usage.model_id.clone().into()],
        ))
        .await?
        .rows_affected()
        != 1
    {
        return Err(ApiError::forbidden("Approved model is no longer available"));
    }
    let tier =
        crate::routes::chat::current_instance_model_tier(tx, &usage.model_id, &usage.request_path)
            .await?;
    if tier != expected_tier {
        return Err(ApiError::conflict("Model policy changed before dispatch"));
    }
    let (plan, policy) = payer_plan(tx, &usage.payer_id, tiers).await?;
    if !plan_covers(policy, &tier) {
        return Err(ApiError::hosted_model_unavailable(
            &usage.payer_id,
            &plan,
            &tier,
        ));
    }
    Ok(())
}

/// The plan an active account is on and what it includes.
async fn payer_plan<'a>(
    tx: &DatabaseTransaction,
    payer_id: &str,
    tiers: &'a UserTiers,
) -> Result<(String, &'a UserTier), ApiError> {
    let account = tx
        .query_one_raw(sql(
            r#"SELECT tier::text AS tier FROM "User" WHERE id=$1 AND status='ACTIVE'"#,
            [payer_id.into()],
        ))
        .await?
        .ok_or(ApiError::FORBIDDEN)?;
    let plan = account.try_get::<String>("", "tier")?.to_uppercase();
    let policy = tiers
        .get(&plan)
        .ok_or_else(|| ApiError::internal("Missing payer plan policy"))?;
    Ok((plan, policy))
}

/// Embeddings require no tier and are open to every plan.
fn plan_covers(policy: &UserTier, tier: &str) -> bool {
    tier.is_empty() || crate::model_tier::tier_allows(policy, tier)
}

/// The tier the hosted endpoint serving this model requires, as its requests are judged
/// at use; `None` when no endpoint serves it any more.
async fn hosted_model_tier(
    tx: &DatabaseTransaction,
    model_id: &str,
    bit_type: &str,
) -> Result<Option<String>, ApiError> {
    let paths: &[&str] = if bit_type == BitType::Embedding.to_value() {
        &[EMBEDDINGS_PATH]
    } else if bit_type == BitType::SystemOne.to_value() {
        &[super::SYSTEMONE_PATH]
    } else {
        &[CHAT_PATH, RESPONSES_PATH]
    };
    for path in paths {
        match crate::routes::chat::current_instance_model_tier(tx, model_id, path).await {
            Ok(tier) => return Ok(Some(tier)),
            Err(refusal) if refusal.status().is_client_error() => {}
            Err(error) => return Err(error),
        }
    }
    Ok(None)
}

async fn model_types(
    tx: &DatabaseTransaction,
    model_ids: &[String],
) -> Result<HashMap<String, String>, ApiError> {
    if model_ids.is_empty() {
        return Ok(HashMap::new());
    }
    tx.query_all_raw(sql(
        &format!(
            r#"SELECT id,type::text AS type FROM "Bit" WHERE id IN ({})"#,
            placeholders(1, model_ids.len())
        ),
        model_ids.iter().map(|id| id.clone().into()),
    ))
    .await?
    .into_iter()
    .map(|row| Ok((row.try_get("", "id")?, row.try_get("", "type")?)))
    .collect()
}

/// What the hosted endpoints would decide for each model of an approval if the caller
/// paid for it. Reads only: nothing is reserved and no approval changes.
pub(crate) async fn billing_eligibility(
    state: &DeviceContext<'_>,
    tiers: &UserTiers,
    payer: &str,
    device_id: &str,
    grant_id: &str,
) -> Result<BillingEligibility, ApiError> {
    let (_, grant) = visible_grant(state, payer, device_id, grant_id).await?;
    let payer = payer.to_owned();
    let model_ids = Arc::new(grant.model_ids);
    let tiers = Arc::new(tiers.clone());
    retry_transaction(
        state.db,
        state.dialect,
        None,
        &RetryPolicy::default(),
        move |tx| {
            let payer = payer.clone();
            let model_ids = model_ids.clone();
            let tiers = tiers.clone();
            Box::pin(async move {
                let (plan, policy) = payer_plan(tx, &payer, &tiers).await?;
                let types = model_types(tx, &model_ids).await?;
                let mut models = Vec::with_capacity(model_ids.len());
                for model_id in model_ids.iter() {
                    let tier = match types.get(model_id) {
                        Some(bit_type) => hosted_model_tier(tx, model_id, bit_type).await?,
                        None => None,
                    };
                    models.push(ModelEligibility {
                        model_id: model_id.clone(),
                        allowed: tier
                            .as_deref()
                            .is_some_and(|tier| plan_covers(policy, tier)),
                        tier: tier.filter(|tier| !tier.is_empty()),
                    });
                }
                Ok(BillingEligibility {
                    payer_id: payer,
                    plan,
                    eligible: !models.is_empty() && models.iter().all(|model| model.allowed),
                    models,
                })
            })
        },
    )
    .await
}

/// The caller holds account-quota and invokes this within the quota reservation
/// transaction. Rollback therefore releases both the account and placement budget.
pub(crate) async fn reserve_budget(
    tx: &DatabaseTransaction,
    usage: &VerifiedInstanceUsage,
    operation_id: &str,
    reserve_micros: i64,
    required_model_tier: &str,
    tiers: &UserTiers,
) -> Result<(), ApiError> {
    if reserve_micros < 0 || operation_id.is_empty() {
        return Err(ApiError::bad_request("Invalid instance budget reservation"));
    }
    let graph = lock_graph(tx, &usage.instance_id, false).await?;
    validate_usage(&graph, usage, true)?;
    model_and_tier(tx, usage, required_model_tier, tiers).await?;
    live(
        graph
            .grant
            .info
            .expires_at
            .min(require_billing(&graph)?.info.expires_at)
            .min(graph.instance.receipt.lease_expires_at),
    )?;
    if let Some(existing) = read_admission(tx, operation_id).await? {
        if existing.authority != *usage
            || existing.ceiling != reserve_micros
            || existing.required_model_tier != required_model_tier
        {
            return Err(ApiError::conflict(
                "Operation already has different instance authority",
            ));
        }
        live(
            graph
                .grant
                .info
                .expires_at
                .min(require_billing(&graph)?.info.expires_at)
                .min(graph.instance.receipt.lease_expires_at),
        )?;
        proof_live(usage.proof_expires_at)?;
        return Ok(());
    }
    let billing = &require_billing(&graph)?.info;
    let total = billing
        .used_micros
        .checked_add(billing.reserved_micros)
        .and_then(|v| v.checked_add(reserve_micros))
        .ok_or_else(|| ApiError::too_many_requests("Placement budget exhausted"))?;
    if total > billing.limit_micros {
        return Err(ApiError::too_many_requests("Placement budget exhausted"));
    }
    tx.execute_raw(sql(
        r#"UPDATE "PlacementBillingGrant" SET "reservedMicros"="reservedMicros"+$2 WHERE id=$1"#,
        [
            billing.billing_grant_id.clone().into(),
            reserve_micros.into(),
        ],
    ))
    .await?;
    tx.execute_raw(sql(r#"INSERT INTO "InstanceUsageAdmission" ("operationId","billingGrantId","instanceId","payerId",authority,"requiredModelTier","ceilingMicros","usedMicros","reservedMicros",status,"createdAt") VALUES ($1,$2,$3,$4,$5,$6,$7,0,$7,'reserved',$8)"#,
        [operation_id.into(),billing.billing_grant_id.clone().into(),usage.instance_id.clone().into(),billing.payer_id.clone().into(),serde_json::to_string(usage)?.into(),required_model_tier.into(),reserve_micros.into(),now().into()])).await?;
    live(
        graph
            .grant
            .info
            .expires_at
            .min(require_billing(&graph)?.info.expires_at)
            .min(graph.instance.receipt.lease_expires_at),
    )?;
    proof_live(usage.proof_expires_at)?;
    Ok(())
}

/// Called in the same account-locked transaction that claims provider dispatch.
/// Work admitted before a revocation but still queued must not start afterward.
pub(crate) async fn authorize_start(
    tx: &DatabaseTransaction,
    operation_id: &str,
    tiers: &UserTiers,
) -> Result<(), ApiError> {
    let Some(admission) = read_admission(tx, operation_id).await? else {
        return Err(ApiError::internal(
            "Instance operation authority is missing",
        ));
    };
    if admission.status != "reserved" {
        return Err(ApiError::conflict(
            "Instance operation has already started or settled",
        ));
    }
    let graph = lock_graph(tx, &admission.authority.instance_id, false).await?;
    validate_usage(&graph, &admission.authority, false)?;
    model_and_tier(
        tx,
        &admission.authority,
        &admission.required_model_tier,
        tiers,
    )
    .await?;
    live(
        graph
            .grant
            .info
            .expires_at
            .min(require_billing(&graph)?.info.expires_at)
            .min(graph.instance.receipt.lease_expires_at),
    )?;
    tx.execute_raw(sql(
        r#"UPDATE "InstanceUsageAdmission" SET status='running' WHERE "operationId"=$1"#,
        [operation_id.into()],
    ))
    .await?;
    live(
        graph
            .grant
            .info
            .expires_at
            .min(require_billing(&graph)?.info.expires_at)
            .min(graph.instance.receipt.lease_expires_at),
    )?;
    Ok(())
}

/// Settlement follows the quota ledger's cumulative totals, including its
/// explicitly authorized corrections. Revoked consent still settles prior work.
pub(crate) async fn settle_budget(
    tx: &DatabaseTransaction,
    operation_id: &str,
    actual_micros: i64,
    finalized: bool,
) -> Result<(), ApiError> {
    let Some(initial) = read_admission(tx, operation_id).await? else {
        return Err(ApiError::internal(
            "Instance operation authority is missing",
        ));
    };
    if tx
        .execute_raw(sql(
            r#"UPDATE "PlacementBillingGrant" SET "authzVersion"="authzVersion" WHERE id=$1"#,
            [initial.authority.billing_grant_id.clone().into()],
        ))
        .await?
        .rows_affected()
        != 1
    {
        return Err(ApiError::internal("Instance billing record is missing"));
    }
    let admission = read_admission(tx, operation_id)
        .await?
        .ok_or_else(|| ApiError::internal("Instance operation authority is missing"))?;
    if actual_micros < 0 || actual_micros > admission.ceiling {
        return Err(ApiError::internal(
            "Instance settlement exceeds admitted budget",
        ));
    }
    if admission.status == "settled" && !finalized {
        return Ok(());
    }
    let is_final = finalized || admission.status == "settled";
    let reserved = if is_final {
        0
    } else {
        admission.ceiling - actual_micros
    };
    let billing = read_billing(tx, &admission.authority.billing_grant_id).await?;
    let used = billing
        .info
        .used_micros
        .checked_add(actual_micros - admission.used)
        .filter(|v| *v >= 0)
        .ok_or_else(|| ApiError::internal("Invalid instance budget settlement"))?;
    let remaining = billing
        .info
        .reserved_micros
        .checked_add(reserved - admission.reserved)
        .filter(|v| *v >= 0)
        .ok_or_else(|| ApiError::internal("Invalid instance budget reservation"))?;
    tx.execute_raw(sql(
        r#"UPDATE "PlacementBillingGrant" SET "usedMicros"=$2,"reservedMicros"=$3 WHERE id=$1"#,
        [
            billing.info.billing_grant_id.into(),
            used.into(),
            remaining.into(),
        ],
    ))
    .await?;
    tx.execute_raw(sql(r#"UPDATE "InstanceUsageAdmission" SET "usedMicros"=$2,"reservedMicros"=$3,status=$4 WHERE "operationId"=$1"#,[operation_id.into(),actual_micros.into(),reserved.into(),if is_final {"settled"}else{"unknown"}.into()])).await?;
    Ok(())
}
