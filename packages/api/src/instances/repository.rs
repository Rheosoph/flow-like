use super::*;
use crate::{
    devices::repository::{active_account, current_device, lock_active_device},
    middleware::jwt::fresh_user_role,
    permission::role_permission::{RolePermissions, has_role_permission},
};
use sea_orm::{
    ConnectionTrait, DatabaseBackend, DatabaseConnection, DatabaseTransaction, QueryResult,
    Statement, Value,
};
use std::collections::HashMap;

pub(super) fn sql(query: &str, values: impl IntoIterator<Item = Value>) -> Statement {
    Statement::from_sql_and_values(DatabaseBackend::Postgres, query, values)
}

/// `$from, …` for `count` bound values.
pub(super) fn placeholders(from: usize, count: usize) -> String {
    (from..from + count)
        .map(|position| format!("${position}"))
        .collect::<Vec<_>>()
        .join(",")
}

pub(super) fn live(expires_at: i64) -> Result<(), ApiError> {
    if expires_at <= now() {
        return Err(ApiError::unauthorized("Instance authorization expired"));
    }
    Ok(())
}

pub(super) fn epoch(value: i64) -> Result<u64, ApiError> {
    u64::try_from(value)
        .ok()
        .filter(|v| *v > 0)
        .ok_or_else(|| ApiError::internal("Invalid instance authorization version"))
}

#[derive(Clone)]
pub(super) struct Grant {
    pub info: ResourceGrantResponse,
}

#[derive(Clone)]
pub(super) struct Billing {
    pub info: BillingGrantResponse,
}

#[derive(Clone)]
pub(super) struct Instance {
    pub receipt: InstanceReceipt,
    pub status: String,
    pub device_auth_epoch: u64,
    pub grant_authz_version: u64,
    pub billing_authz_version: Option<u64>,
}

pub(super) struct Graph {
    pub device: crate::devices::repository::Device,
    pub grant: Grant,
    pub billing: Option<Billing>,
    pub instance: Instance,
}

fn grant(row: &QueryResult) -> Result<Grant, ApiError> {
    Ok(Grant {
        info: ResourceGrantResponse {
            grant_id: row.try_get("", "id")?,
            device_id: row.try_get("", "deviceId")?,
            placement_id: row.try_get("", "placementId")?,
            deployment_id: row.try_get("", "deploymentId")?,
            project_id: row.try_get("", "projectId")?,
            app_id: row.try_get("", "appId")?,
            delegating_user_id: row.try_get("", "delegatingUserId")?,
            authz_version: epoch(row.try_get("", "authzVersion")?)?,
            model_ids: serde_json::from_str(&row.try_get::<String>("", "modelIds")?)?,
            max_instances: u32::try_from(row.try_get::<i64>("", "maxInstances")?)
                .map_err(|_| ApiError::internal("Invalid instance limit"))?,
            expires_at: row.try_get("", "expiresAt")?,
            status: row.try_get("", "status")?,
            online_access: row
                .try_get::<Option<String>>("", "onlineAccess")?
                .map(|value| serde_json::from_str(&value))
                .transpose()?,
            effective_expires_at: None,
            effective_limit: None,
            online_write_blocked: None,
            approved_by_user_id: Some(row.try_get("", "approvedByUserId")?),
            created_at: Some(row.try_get("", "createdAt")?),
        },
    })
}

const BILLING_COLUMNS: [&str; 11] = [
    "id",
    "grantId",
    "payerId",
    "authzVersion",
    "limitMicros",
    "usedMicros",
    "reservedMicros",
    "expiresAt",
    "status",
    "approvedByUserId",
    "createdAt",
];

fn billing(row: QueryResult) -> Result<Billing, ApiError> {
    billing_at(&row, "")
}

/// Reads a spending limit whose columns were selected under `prefix`.
fn billing_at(row: &QueryResult, prefix: &str) -> Result<Billing, ApiError> {
    Ok(Billing {
        info: BillingGrantResponse {
            billing_grant_id: row.try_get(prefix, "id")?,
            grant_id: row.try_get(prefix, "grantId")?,
            payer_id: row.try_get(prefix, "payerId")?,
            authz_version: epoch(row.try_get(prefix, "authzVersion")?)?,
            limit_micros: row.try_get(prefix, "limitMicros")?,
            used_micros: row.try_get(prefix, "usedMicros")?,
            reserved_micros: row.try_get(prefix, "reservedMicros")?,
            expires_at: row.try_get(prefix, "expiresAt")?,
            status: row.try_get(prefix, "status")?,
            approved_by_user_id: Some(row.try_get(prefix, "approvedByUserId")?),
            created_at: Some(row.try_get(prefix, "createdAt")?),
        },
    })
}

fn instance(row: QueryResult) -> Result<Instance, ApiError> {
    Ok(Instance {
        status: row.try_get("", "status")?,
        device_auth_epoch: epoch(row.try_get("", "deviceAuthEpoch")?)?,
        grant_authz_version: epoch(row.try_get("", "grantAuthzVersion")?)?,
        billing_authz_version: row
            .try_get::<Option<i64>>("", "billingAuthzVersion")?
            .map(epoch)
            .transpose()?,
        receipt: InstanceReceipt {
            instance_id: row.try_get("", "id")?,
            purpose: match row.try_get::<Option<String>>("", "purpose")?.as_deref() {
                None | Some("workload") => InstancePurpose::Workload,
                Some("rollout_validation") => InstancePurpose::RolloutValidation,
                _ => return Err(ApiError::internal("Invalid instance purpose")),
            },
            device_id: row.try_get("", "deviceId")?,
            grant_id: row.try_get("", "grantId")?,
            billing_grant_id: row.try_get("", "billingGrantId")?,
            workload_key: serde_json::from_str(&row.try_get::<String>("", "workloadKey")?)?,
            key_epoch: epoch(row.try_get("", "keyEpoch")?)?,
            registered_at: row.try_get("", "registeredAt")?,
            lease_expires_at: row.try_get("", "leaseExpiresAt")?,
            registration_jws: row.try_get("", "registrationJws")?,
        },
    })
}

pub(super) async fn read_grant<C: ConnectionTrait>(db: &C, id: &str) -> Result<Grant, ApiError> {
    db.query_one_raw(sql(
        r#"SELECT * FROM "PlacementResourceGrant" WHERE id=$1"#,
        [id.into()],
    ))
    .await?
    .ok_or(ApiError::NOT_FOUND)
    .and_then(|row| grant(&row))
}
pub(super) async fn read_billing<C: ConnectionTrait>(
    db: &C,
    id: &str,
) -> Result<Billing, ApiError> {
    db.query_one_raw(sql(
        r#"SELECT * FROM "PlacementBillingGrant" WHERE id=$1"#,
        [id.into()],
    ))
    .await?
    .ok_or(ApiError::NOT_FOUND)
    .and_then(billing)
}
pub(super) async fn read_instance<C: ConnectionTrait>(
    db: &C,
    id: &str,
) -> Result<Instance, ApiError> {
    db.query_one_raw(sql(
        r#"SELECT * FROM "WorkloadInstance" WHERE id=$1"#,
        [id.into()],
    ))
    .await?
    .ok_or(ApiError::NOT_FOUND)
    .and_then(instance)
}

pub(super) async fn project_authority(
    tx: &DatabaseTransaction,
    grant: &ResourceGrantResponse,
) -> Result<(), ApiError> {
    let mut accounts = vec![grant.delegating_user_id.clone()];
    if let Some(app_id) = &grant.app_id {
        if tx
            .execute_raw(sql(
                r#"UPDATE "App" SET status=status WHERE id=$1 AND status='ACTIVE'"#,
                [app_id.clone().into()],
            ))
            .await?
            .rows_affected()
            != 1
        {
            return Err(ApiError::forbidden("Project is no longer active"));
        }
        if tx
            .execute_raw(sql(
                r#"UPDATE "Membership" SET "roleId"="roleId" WHERE "userId"=$1 AND "appId"=$2"#,
                [
                    grant.delegating_user_id.clone().into(),
                    app_id.clone().into(),
                ],
            ))
            .await?
            .rows_affected()
            != 1
        {
            return Err(ApiError::forbidden(
                "Current project membership is required",
            ));
        }
        let membership = tx
            .query_one_raw(sql(
                r#"SELECT "roleId" FROM "Membership" WHERE "userId"=$1 AND "appId"=$2"#,
                [
                    grant.delegating_user_id.clone().into(),
                    app_id.clone().into(),
                ],
            ))
            .await?
            .ok_or(ApiError::FORBIDDEN)?;
        let role_id: String = membership.try_get("", "roleId")?;
        if tx
            .execute_raw(sql(
                r#"UPDATE "Role" SET permissions=permissions WHERE id=$1 AND "appId"=$2"#,
                [role_id.into(), app_id.clone().into()],
            ))
            .await?
            .rows_affected()
            != 1
        {
            return Err(ApiError::FORBIDDEN);
        }
        let permissions = fresh_user_role(tx, &grant.delegating_user_id, app_id).await?;
        if !permissions.intersects(RolePermissions::Admin | RolePermissions::Owner)
            || !has_role_permission(&permissions, RolePermissions::ExecuteBoards)
        {
            return Err(ApiError::forbidden(
                "Current project deployment authority is required",
            ));
        }
        let project_owner = crate::capacity::payer_for_app(tx, app_id)
            .await?
            .ok_or_else(|| ApiError::forbidden("Project has no active owner"))?;
        if grant.online_access.is_some() && project_owner != grant.delegating_user_id {
            return Err(ApiError::forbidden(
                "Online storage requires the current project owner's approval",
            ));
        }
        accounts.push(project_owner);
    }
    accounts.sort();
    accounts.dedup();
    for account in accounts {
        if tx
            .execute_raw(sql(
                r#"UPDATE "User" SET "updatedAt"="updatedAt" WHERE id=$1 AND status='ACTIVE'"#,
                [account.into()],
            ))
            .await?
            .rows_affected()
            != 1
        {
            return Err(ApiError::forbidden("Current account authority is required"));
        }
    }
    Ok(())
}

pub(super) async fn lock_device(
    tx: &DatabaseTransaction,
    device_id: &str,
) -> Result<crate::devices::repository::Device, ApiError> {
    let device = current_device(tx, device_id).await?;
    lock_active_device(tx, device_id, device.status.auth_epoch).await
}

/// Device hosting consent and project resource consent are independent. Callers
/// hold the device row lock, so a policy update cannot race this decision.
pub(super) async fn device_deployment_authority(
    tx: &DatabaseTransaction,
    device: &crate::devices::repository::Device,
    grant: &ResourceGrantResponse,
) -> Result<(), ApiError> {
    device_deployment_deadline(tx, device, grant)
        .await
        .map(|_| ())
}

pub(super) async fn device_deployment_deadline(
    tx: &DatabaseTransaction,
    device: &crate::devices::repository::Device,
    grant: &ResourceGrantResponse,
) -> Result<i64, ApiError> {
    if device.status.owner_id == grant.delegating_user_id {
        return Ok(grant.expires_at);
    }
    let (policy_jws, owner_key) = latest_policy(tx, device)
        .await?
        .ok_or(ApiError::NOT_FOUND)?;
    let policy = verify_management_policy(&policy_jws, &owner_key, now())
        .map_err(|_| ApiError::forbidden("Device deployment approval expired or changed"))?;
    let permission_deadline =
        deploy_permission_deadline(&policy, grant).filter(|expires_at| *expires_at > now());
    if policy.device_id != device.status.device_id || permission_deadline.is_none() {
        return Err(ApiError::forbidden(
            "Current device deployment approval is required",
        ));
    }
    Ok(grant
        .expires_at
        .min(policy.expires_at)
        .min(permission_deadline.ok_or(ApiError::FORBIDDEN)?))
}

/// The owner's newest signed access rules and the key they must be signed with.
async fn latest_policy<C: ConnectionTrait>(
    db: &C,
    device: &crate::devices::repository::Device,
) -> Result<Option<(String, Ed25519PublicKey)>, ApiError> {
    let Some(enrollment) = db
        .query_one_raw(sql(
            r#"SELECT manifest FROM "DeviceEnrollment" WHERE id=$1"#,
            [device.receipt.enrollment_id.clone().into()],
        ))
        .await?
    else {
        return Ok(None);
    };
    let manifest: OnboardingManifest =
        serde_json::from_str(&enrollment.try_get::<String>("", "manifest")?)?;
    let Some(row) = db.query_one_raw(sql(r#"SELECT "policyJws" FROM "DeviceManagementPolicy" WHERE "deviceId"=$1 ORDER BY version DESC LIMIT 1"#, [device.status.device_id.clone().into()])).await? else {
        return Ok(None);
    };
    Ok(Some((
        row.try_get("", "policyJws")?,
        manifest.owner_invitation_key,
    )))
}

/// The latest expiry among the Deploy permissions the approver holds on the placement,
/// ended ones included.
fn deploy_permission_deadline(
    policy: &ManagementPolicy,
    grant: &ResourceGrantResponse,
) -> Option<i64> {
    let requested = ManagementScope::Placement {
        project_id: grant.project_id.clone(),
        placement_id: grant.placement_id.clone(),
    };
    policy
        .grants
        .iter()
        .filter(|permission| {
            permission.user_id == grant.delegating_user_id
                && permission
                    .capabilities
                    .contains(&ManagementCapability::Deploy)
                && inventory_scope_contains(&permission.scope, &requested)
        })
        .map(|permission| permission.expires_at)
        .max()
}

#[cfg(test)]
thread_local! {
    /// How often this thread verified a device's access rules for an approval read.
    pub(super) static ACCESS_RULE_READS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// A device's access rules as a reader sees them, from a row of its id, enrollment
/// manifest and newest rules. Ended rules keep their end time; rules that fail
/// verification for another reason count as absent. Never an authorization decision:
/// `device_deployment_deadline` makes those.
fn readable_rules(
    row: &QueryResult,
    now: i64,
) -> Result<Option<(String, ManagementPolicy)>, ApiError> {
    #[cfg(test)]
    ACCESS_RULE_READS.with(|reads| reads.set(reads.get() + 1));
    let device_id: String = row.try_get("", "deviceId")?;
    let manifest: OnboardingManifest =
        serde_json::from_str(&row.try_get::<String>("", "manifest")?)?;
    let policy_jws: String = row.try_get("", "policyJws")?;
    let owner_key = manifest.owner_invitation_key;
    let policy = match verify_management_policy(&policy_jws, &owner_key, now) {
        Ok(policy) => Some(policy),
        Err(_) => verify_historical_management_policy(&policy_jws, &owner_key)
            .ok()
            .filter(|policy| policy.expires_at <= now),
    };
    Ok(policy
        .filter(|policy| policy.device_id == device_id)
        .map(|policy| (device_id, policy)))
}

const RULES_BATCH: usize = 100;

/// The readable access rules of every device among `device_ids` that has any: one read per
/// batch, one verification per device.
async fn access_rules<C: ConnectionTrait>(
    db: &C,
    device_ids: &[String],
    now: i64,
) -> Result<HashMap<String, ManagementPolicy>, ApiError> {
    let mut rules = HashMap::new();
    for batch in device_ids.chunks(RULES_BATCH) {
        let rows = db
            .query_all_raw(sql(
                &format!(
                    r#"SELECT e."deviceId",e.manifest,p."policyJws" FROM "DeviceEnrollment" e JOIN "DeviceManagementPolicy" p ON p."deviceId"=e."deviceId" AND p.version=(SELECT MAX(q.version) FROM "DeviceManagementPolicy" q WHERE q."deviceId"=e."deviceId") WHERE e."deviceId" IN ({})"#,
                    placeholders(1, batch.len())
                ),
                batch.iter().map(|id| id.clone().into()),
            ))
            .await?;
        for row in &rows {
            rules.extend(readable_rules(row, now)?);
        }
    }
    Ok(rules)
}

/// When an approval by someone other than the device owner stops working and what ends
/// it first. The approval itself wins a tie. A time at or before `now` means it has ended.
pub(super) fn delegated_deadline(
    rules: Option<&ManagementPolicy>,
    grant: &ResourceGrantResponse,
    now: i64,
) -> (i64, EffectiveLimit) {
    let approval = (grant.expires_at, EffectiveLimit::Approval);
    let Some(policy) = rules else {
        return [approval, (now, EffectiveLimit::AccessRules)]
            .into_iter()
            .min_by_key(|(at, _)| *at)
            .unwrap_or(approval);
    };
    // Rules that no longer name the approver took the permission away when they were issued.
    let sharing =
        deploy_permission_deadline(policy, grant).unwrap_or_else(|| policy.issued_at.min(now));
    [
        approval,
        (policy.expires_at, EffectiveLimit::AccessRules),
        (sharing, EffectiveLimit::SharingGrant),
    ]
    .into_iter()
    .min_by_key(|(at, _)| *at)
    .unwrap_or(approval)
}

/// Fills the effective end of every active approval on one device. Its access rules are
/// read and verified at most once, however many approvals depend on them.
pub(super) async fn effective_expiry<C: ConnectionTrait>(
    db: &C,
    device: &crate::devices::repository::Device,
    grants: &mut [ResourceGrantResponse],
) -> Result<(), ApiError> {
    let owners = HashMap::from([(
        device.status.device_id.clone(),
        device.status.owner_id.clone(),
    )]);
    effective_ends(db, &owners, grants).await
}

/// Fills the effective end of every active approval, wherever it is. `owners` names the
/// owner of each device. Access rules are read only for devices with an approval by
/// someone else, and verified once per device.
pub(super) async fn effective_ends<C: ConnectionTrait>(
    db: &C,
    owners: &HashMap<String, String>,
    grants: &mut [ResourceGrantResponse],
) -> Result<(), ApiError> {
    let delegated = |grant: &ResourceGrantResponse| {
        grant.status == "active" && owners.get(&grant.device_id) != Some(&grant.delegating_user_id)
    };
    let now = now();
    let mut ruled: Vec<String> = grants
        .iter()
        .filter(|grant| delegated(grant))
        .map(|grant| grant.device_id.clone())
        .collect();
    ruled.sort_unstable();
    ruled.dedup();
    let rules = access_rules(db, &ruled, now).await?;
    for grant in grants.iter_mut().filter(|grant| grant.status == "active") {
        let (at, limit) = if delegated(grant) {
            delegated_deadline(rules.get(&grant.device_id), grant, now)
        } else {
            (grant.expires_at, EffectiveLimit::Approval)
        };
        grant.effective_expires_at = Some(at);
        grant.effective_limit = Some(limit);
    }
    Ok(())
}

pub(super) async fn lock_grant(tx: &DatabaseTransaction, id: &str) -> Result<Grant, ApiError> {
    if tx.execute_raw(sql(r#"UPDATE "PlacementResourceGrant" SET "authzVersion"="authzVersion" WHERE id=$1 AND status='active'"#,[id.into()])).await?.rows_affected()!=1 {
        return Err(ApiError::unauthorized("Resource grant is no longer active"));
    }
    let grant = read_grant(tx, id).await?;
    live(grant.info.expires_at)?;
    project_authority(tx, &grant.info).await?;
    Ok(grant)
}

pub(super) async fn lock_billing(tx: &DatabaseTransaction, id: &str) -> Result<Billing, ApiError> {
    if tx.execute_raw(sql(r#"UPDATE "PlacementBillingGrant" SET "authzVersion"="authzVersion" WHERE id=$1 AND status='active'"#,[id.into()])).await?.rows_affected()!=1 {
        return Err(ApiError::unauthorized("Billing grant is no longer active"));
    }
    let billing = read_billing(tx, id).await?;
    live(billing.info.expires_at)?;
    active_account(tx, &billing.info.payer_id).await?;
    Ok(billing)
}

/// Metered callers acquire account-quota first. No registry mutation acquires
/// that account lock after these device, resource, billing and instance writes.
pub(super) async fn lock_graph(
    tx: &DatabaseTransaction,
    id: &str,
    allow_expired_lease: bool,
) -> Result<Graph, ApiError> {
    let initial = read_instance(tx, id).await?;
    let device = lock_device(tx, &initial.receipt.device_id).await?;
    let grant = lock_grant(tx, &initial.receipt.grant_id).await?;
    device_deployment_authority(tx, &device, &grant.info).await?;
    let billing = match &initial.receipt.billing_grant_id {
        Some(id) => {
            tx.execute_raw(sql(
                r#"UPDATE "PlacementBillingGrant" SET "authzVersion"="authzVersion" WHERE id=$1"#,
                [id.clone().into()],
            ))
            .await?;
            Some(read_billing(tx, id).await?)
        }
        None => None,
    };
    if tx.execute_raw(sql(r#"UPDATE "WorkloadInstance" SET "keyEpoch"="keyEpoch" WHERE id=$1 AND ((status='active' AND (purpose IS NULL OR purpose='workload')) OR (status='validating' AND purpose='rollout_validation'))"#,[id.into()])).await?.rows_affected()!=1 {
        return Err(ApiError::unauthorized("Instance is no longer active"));
    }
    let instance = read_instance(tx, id).await?;
    if grant.info.device_id != device.status.device_id
        || instance.receipt.device_id != grant.info.device_id
        || billing
            .as_ref()
            .is_some_and(|billing| billing.info.grant_id != grant.info.grant_id)
        || instance.device_auth_epoch != device.status.auth_epoch
        || instance.grant_authz_version != grant.info.authz_version
    {
        return Err(ApiError::unauthorized("Instance authority binding changed"));
    }
    if !allow_expired_lease {
        live(instance.receipt.lease_expires_at)?;
    }
    let graph = Graph {
        device,
        grant,
        billing,
        instance,
    };
    if graph.instance.receipt.purpose == InstancePurpose::RolloutValidation {
        live(graph.instance.receipt.registered_at + INSTANCE_VALIDATION_SECONDS)?;
        if graph.grant.info.online_access.is_none()
            || graph.instance.receipt.billing_grant_id.is_some()
        {
            return Err(ApiError::FORBIDDEN);
        }
    }
    if graph.grant.info.online_access.is_none() {
        require_billing(&graph)?;
    }
    Ok(graph)
}

pub(super) fn require_billing(graph: &Graph) -> Result<&Billing, ApiError> {
    if graph.instance.receipt.purpose != InstancePurpose::Workload {
        return Err(ApiError::forbidden(
            "Validation instances cannot invoke models",
        ));
    }
    let billing = graph
        .billing
        .as_ref()
        .ok_or_else(|| ApiError::forbidden("This instance has no hosted model billing approval"))?;
    if billing.info.status != "active"
        || graph.instance.billing_authz_version != Some(billing.info.authz_version)
    {
        return Err(ApiError::unauthorized("Instance billing approval changed"));
    }
    live(billing.info.expires_at)?;
    Ok(billing)
}

pub(super) fn resource_deadline(graph: &Graph) -> Result<i64, ApiError> {
    let mut expiry = graph
        .grant
        .info
        .expires_at
        .min(graph.instance.receipt.lease_expires_at);
    if graph.instance.receipt.purpose == InstancePurpose::RolloutValidation {
        expiry = expiry.min(graph.instance.receipt.registered_at + INSTANCE_VALIDATION_SECONDS);
    }
    if graph.grant.info.online_access.is_none() {
        expiry = expiry.min(require_billing(graph)?.info.expires_at);
    }
    Ok(expiry)
}

pub(super) async fn consume_instance_proof(
    tx: &DatabaseTransaction,
    id: &str,
    key_epoch: u64,
    jti: &str,
    expires_at: i64,
) -> Result<(), ApiError> {
    proof_live(expires_at)?;
    if tx.execute_raw(sql(r#"INSERT INTO "InstanceProofReplay" ("instanceId","keyEpoch","proofId","expiresAt") VALUES ($1,$2,$3,$4) ON CONFLICT ("instanceId","keyEpoch","proofId") DO NOTHING"#,
        [id.into(),(key_epoch as i64).into(),jti.into(),expires_at.into()])).await?.rows_affected()!=1 {
        return Err(bad_proof("Instance proof was already used"));
    }
    tx.execute_raw(sql(r#"DELETE FROM "InstanceProofReplay" WHERE ("instanceId","keyEpoch","proofId") IN (SELECT "instanceId","keyEpoch","proofId" FROM "InstanceProofReplay" WHERE "instanceId"=$1 AND "expiresAt"<$2 LIMIT 128)"#,[id.into(),(now()-60).into()])).await?;
    proof_live(expires_at)
}

pub(super) async fn renew_lease(
    tx: &DatabaseTransaction,
    graph: &mut Graph,
) -> Result<(), ApiError> {
    if graph.instance.receipt.purpose == InstancePurpose::RolloutValidation {
        // Validation has one fixed window. A retained workload key cannot extend it.
        live(resource_deadline(graph)?)?;
        return Ok(());
    }
    if graph.instance.receipt.lease_expires_at <= now() {
        check_capacity(tx, &graph.grant.info).await?;
    }
    let mut expires_at = (now() + LEASE_SECONDS).min(graph.grant.info.expires_at);
    if graph.grant.info.online_access.is_none() {
        expires_at = expires_at.min(require_billing(graph)?.info.expires_at);
    }
    live(expires_at)?;
    tx.execute_raw(sql(
        r#"UPDATE "WorkloadInstance" SET "leaseExpiresAt"=$2 WHERE id=$1"#,
        [
            graph.instance.receipt.instance_id.clone().into(),
            expires_at.into(),
        ],
    ))
    .await?;
    graph.instance.receipt.lease_expires_at = expires_at;
    Ok(())
}

pub(super) async fn check_capacity(
    tx: &DatabaseTransaction,
    grant: &ResourceGrantResponse,
) -> Result<(), ApiError> {
    let count=tx.query_one_raw(sql(r#"SELECT COUNT(*) AS count FROM "WorkloadInstance" i LEFT JOIN "PlacementBillingGrant" b ON b.id=i."billingGrantId" WHERE i."grantId"=$1 AND (i.purpose IS NULL OR i.purpose='workload') AND i.status='active' AND i."leaseExpiresAt">$2 AND i."grantAuthzVersion"=$3 AND ($4 OR (b.status='active' AND b."expiresAt">$2 AND i."billingAuthzVersion"=b."authzVersion"))"#,[grant.grant_id.clone().into(),now().into(),(grant.authz_version as i64).into(),grant.online_access.is_some().into()])).await?.ok_or_else(||ApiError::internal("Missing instance count"))?.try_get::<i64>("","count")?;
    if count >= i64::from(grant.max_instances) {
        return Err(ApiError::too_many_requests(
            "Placement instance limit reached",
        ));
    }
    Ok(())
}

pub(super) async fn check_validation_capacity(
    tx: &DatabaseTransaction,
    device_id: &str,
) -> Result<(), ApiError> {
    // Registration already holds the device write lock, including across grants.
    let count = tx.query_one_raw(sql(
        r#"SELECT COUNT(*) AS count FROM "WorkloadInstance" WHERE "deviceId"=$1 AND purpose='rollout_validation' AND status='validating' AND "leaseExpiresAt">$2 AND "registeredAt">$3"#,
        [device_id.into(), now().into(), (now() - INSTANCE_VALIDATION_SECONDS).into()],
    )).await?.ok_or_else(|| ApiError::internal("Missing validation instance count"))?
        .try_get::<i64>("", "count")?;
    if count >= 2 {
        return Err(ApiError::too_many_requests(
            "Device validation instance limit reached",
        ));
    }
    Ok(())
}

/// The approvals of device `$1` that viewer `$3` sees: every one when `$2` says the viewer
/// owns the device, otherwise only their own.
const VISIBLE_GRANTS: &str = r#"g."deviceId"=$1 AND ($2 OR g."delegatingUserId"=$3)"#;

fn viewer_scope(device: &crate::devices::repository::Device, viewer: &str) -> [Value; 3] {
    [
        device.status.device_id.clone().into(),
        (device.status.owner_id == viewer).into(),
        viewer.into(),
    ]
}

pub(super) async fn list_grants(
    db: &DatabaseConnection,
    device: &crate::devices::repository::Device,
    viewer: &str,
) -> Result<Vec<ResourceGrantResponse>, ApiError> {
    db.query_all_raw(sql(&format!(r#"SELECT g.* FROM "PlacementResourceGrant" g WHERE {VISIBLE_GRANTS} ORDER BY CASE WHEN g.status='active' THEN 0 ELSE 1 END,g."createdAt" DESC LIMIT 1000"#),viewer_scope(device,viewer))).await?.iter().map(|r|grant(r).map(|g|g.info)).collect()
}

pub(super) async fn list_billing(
    db: &DatabaseConnection,
    device: &crate::devices::repository::Device,
    viewer: &str,
) -> Result<Vec<BillingGrantResponse>, ApiError> {
    db.query_all_raw(sql(&format!(r#"SELECT b.* FROM "PlacementBillingGrant" b JOIN "PlacementResourceGrant" g ON g.id=b."grantId" WHERE {VISIBLE_GRANTS} ORDER BY CASE WHEN b.status='active' THEN 0 ELSE 1 END,b."createdAt" DESC LIMIT 1000"#),viewer_scope(device,viewer))).await?.into_iter().map(|r|billing(r).map(|b|b.info)).collect()
}

pub(super) async fn list_instances(
    db: &DatabaseConnection,
    device: &crate::devices::repository::Device,
    viewer: &str,
) -> Result<Vec<InstanceReceipt>, ApiError> {
    let values = viewer_scope(device, viewer)
        .into_iter()
        .chain([now().into(), (now() - INSTANCE_VALIDATION_SECONDS).into()]);
    db.query_all_raw(sql(&format!(r#"SELECT i.* FROM "WorkloadInstance" i JOIN "PlacementResourceGrant" g ON g.id=i."grantId" WHERE i."deviceId"=$1 AND {VISIBLE_GRANTS} AND {listed} ORDER BY i."registeredAt" DESC LIMIT 1000"#, listed = listed_leases("$4", "$5")),values)).await?.into_iter().map(|r|instance(r).map(|i|i.receipt)).collect()
}

/// The leases of instance `i` that lists show: running services, and rollout checks that
/// began after `validation_start`, while the lease lasts beyond `now`.
fn listed_leases(now: &str, validation_start: &str) -> String {
    format!(
        r#"((i.status='active' AND (i.purpose IS NULL OR i.purpose='workload')) OR (i.status='validating' AND i.purpose='rollout_validation' AND i."registeredAt">{validation_start})) AND i."leaseExpiresAt">{now}"#
    )
}

/// How many approvals one summary of the whole fleet lists.
const SUMMARY_LIMIT: usize = 1000;
/// How many approvals of one app are read for its placements.
const APP_PLACEMENT_LIMIT: usize = 500;
/// Column prefix of the spending limit joined to an approval.
const JOINED_BILLING: &str = "billing_";

/// Approvals `g` that still run first, then the newest. `$2` is the current time.
const LIVE_FIRST: &str =
    r#"CASE WHEN g.status='active' AND g."expiresAt">$2 THEN 0 ELSE 1 END,g."createdAt" DESC,g.id"#;

/// Orders the spending limits `c` of one approval: the unexpired one, else the latest,
/// and on a tie one that was not withdrawn. `$2` is the current time.
pub(super) const CURRENT_LIMIT_FIRST: &str = r#"CASE WHEN c.status='active' AND c."expiresAt">$2 THEN 0 ELSE 1 END,c."createdAt" DESC,CASE WHEN c.status='active' THEN 0 ELSE 1 END,c.id DESC"#;

/// Ids of the approvals viewer `$1` sees anywhere: every one on a device they own, the
/// ones they gave, and the ones carrying a spending limit of theirs that was not
/// withdrawn. `scope` narrows each source.
fn visible_anywhere(scope: &str) -> String {
    format!(
        r#"SELECT v.id FROM "ManagedDevice" o JOIN "PlacementResourceGrant" v ON v."deviceId"=o.id WHERE o."ownerId"=$1{scope} UNION SELECT v.id FROM "PlacementResourceGrant" v WHERE v."delegatingUserId"=$1{scope} UNION SELECT v.id FROM "PlacementBillingGrant" s JOIN "PlacementResourceGrant" v ON v.id=s."grantId" WHERE s."payerId"=$1 AND s.status='active'{scope}"#
    )
}

/// The leases held under one approval.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct Leases {
    pub active: i64,
    pub newest_expires_at: Option<i64>,
}

/// Approvals from across the fleet and the facts lists show beside them.
#[derive(Debug, Default)]
pub(super) struct FleetApprovals {
    /// Approvals that still run first, then the newest.
    pub grants: Vec<ResourceGrantResponse>,
    /// The owner of each device those approvals are on.
    pub owners: HashMap<String, String>,
    /// The spending limit in force, by approval: its current one, unless that was withdrawn.
    pub billing: HashMap<String, BillingGrantResponse>,
    /// The leases held, by approval. Only read for one app.
    pub leases: HashMap<String, Leases>,
}

/// One bounded read of the approvals `viewer` sees: all of them, or those of `app` with
/// their leases.
pub(super) fn fleet_statement(viewer: &str, app: Option<&str>, now: i64) -> Statement {
    let mut values: Vec<Value> = vec![viewer.into(), now.into()];
    let (scope, leases, limit) = match app {
        None => ("", String::new(), SUMMARY_LIMIT),
        Some(app) => {
            values.extend([app.into(), (now - INSTANCE_VALIDATION_SECONDS).into()]);
            let held = format!(
                r#"FROM "WorkloadInstance" i WHERE i."grantId"=g.id AND {}"#,
                listed_leases("$2", "$4")
            );
            (
                r#" AND v."appId"=$3"#,
                format!(
                    r#",(SELECT COUNT(*) {held}) AS "leases",(SELECT MAX(i."leaseExpiresAt") {held}) AS "newestLease""#
                ),
                APP_PLACEMENT_LIMIT,
            )
        }
    };
    let billing = BILLING_COLUMNS
        .iter()
        .map(|column| format!(r#"b."{column}" AS "{JOINED_BILLING}{column}""#))
        .collect::<Vec<_>>()
        .join(",");
    sql(
        &format!(
            r#"SELECT g.*,{billing}{leases} FROM (SELECT g.*,d."ownerId" AS "deviceOwnerId",(SELECT c.id FROM "PlacementBillingGrant" c WHERE c."grantId"=g.id ORDER BY {CURRENT_LIMIT_FIRST} LIMIT 1) AS "currentLimitId" FROM "PlacementResourceGrant" g JOIN "ManagedDevice" d ON d.id=g."deviceId" WHERE g.id IN ({visible}) ORDER BY {LIVE_FIRST} LIMIT {limit}) g LEFT JOIN "PlacementBillingGrant" b ON b.id=g."currentLimitId" AND b.status='active' ORDER BY {LIVE_FIRST}"#,
            visible = visible_anywhere(scope)
        ),
        values,
    )
}

pub(super) async fn fleet_approvals<C: ConnectionTrait>(
    db: &C,
    viewer: &str,
    app: Option<&str>,
    now: i64,
) -> Result<FleetApprovals, ApiError> {
    let mut found = FleetApprovals::default();
    for row in &db.query_all_raw(fleet_statement(viewer, app, now)).await? {
        let approval = grant(row)?.info;
        found.owners.insert(
            approval.device_id.clone(),
            row.try_get("", "deviceOwnerId")?,
        );
        let id = &approval.grant_id;
        found
            .billing
            .extend(joined_billing(row)?.map(|billing| (id.clone(), billing)));
        found
            .leases
            .extend(held_leases(row, app.is_some())?.map(|leases| (id.clone(), leases)));
        found.grants.push(approval);
    }
    Ok(found)
}

/// The spending limit joined to an approval's row, when one is in force.
fn joined_billing(row: &QueryResult) -> Result<Option<BillingGrantResponse>, ApiError> {
    if row
        .try_get::<Option<String>>(JOINED_BILLING, "id")?
        .is_none()
    {
        return Ok(None);
    }
    Ok(Some(billing_at(row, JOINED_BILLING)?.info))
}

/// The leases counted beside an approval's row. Only the read of one app counts them.
fn held_leases(row: &QueryResult, counted: bool) -> Result<Option<Leases>, ApiError> {
    if !counted {
        return Ok(None);
    }
    Ok(Some(Leases {
        active: row.try_get("", "leases")?,
        newest_expires_at: row.try_get("", "newestLease")?,
    }))
}

fn usage_totals(row: QueryResult) -> Result<BillingGrantUsageTotals, ApiError> {
    Ok(BillingGrantUsageTotals {
        used_micros: row.try_get("", "used")?,
        reserved_micros: row.try_get("", "reserved")?,
        operations: row.try_get("", "operations")?,
    })
}

fn instance_usage(row: QueryResult) -> Result<BillingGrantInstanceUsage, ApiError> {
    Ok(BillingGrantInstanceUsage {
        instance_id: row.try_get("", "instanceId")?,
        used_micros: row.try_get("", "used")?,
        reserved_micros: row.try_get("", "reserved")?,
        operations: row.try_get("", "operations")?,
        first_at: row.try_get("", "first_at")?,
        last_at: row.try_get("", "last_at")?,
    })
}

/// Totals cover every admission; the instance rows are capped.
pub(super) async fn read_billing_usage(
    db: &DatabaseConnection,
    billing_grant_id: String,
) -> Result<BillingGrantUsage, ApiError> {
    let totals = db
        .query_one_raw(sql(
            r#"SELECT COALESCE(SUM("usedMicros"),0)::bigint AS used,COALESCE(SUM("reservedMicros"),0)::bigint AS reserved,COUNT(*) AS operations FROM "InstanceUsageAdmission" WHERE "billingGrantId"=$1"#,
            [billing_grant_id.clone().into()],
        ))
        .await?
        .ok_or_else(|| ApiError::internal("Missing spending totals"))
        .and_then(usage_totals)?;
    let instances = db
        .query_all_raw(sql(
            r#"SELECT "instanceId",SUM("usedMicros")::bigint AS used,SUM("reservedMicros")::bigint AS reserved,COUNT(*) AS operations,MIN("createdAt") AS first_at,MAX("createdAt") AS last_at FROM "InstanceUsageAdmission" WHERE "billingGrantId"=$1 GROUP BY "instanceId" ORDER BY MAX("createdAt") DESC,"instanceId" LIMIT 100"#,
            [billing_grant_id.clone().into()],
        ))
        .await?
        .into_iter()
        .map(instance_usage)
        .collect::<Result<_, ApiError>>()?;
    Ok(BillingGrantUsage {
        billing_grant_id,
        totals,
        instances,
    })
}
