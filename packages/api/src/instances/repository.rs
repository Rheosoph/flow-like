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

pub(super) fn sql(query: &str, values: impl IntoIterator<Item = Value>) -> Statement {
    Statement::from_sql_and_values(DatabaseBackend::Postgres, query, values)
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

fn grant(row: QueryResult) -> Result<Grant, ApiError> {
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
        },
    })
}

fn billing(row: QueryResult) -> Result<Billing, ApiError> {
    Ok(Billing {
        info: BillingGrantResponse {
            billing_grant_id: row.try_get("", "id")?,
            grant_id: row.try_get("", "grantId")?,
            payer_id: row.try_get("", "payerId")?,
            authz_version: epoch(row.try_get("", "authzVersion")?)?,
            limit_micros: row.try_get("", "limitMicros")?,
            used_micros: row.try_get("", "usedMicros")?,
            reserved_micros: row.try_get("", "reservedMicros")?,
            expires_at: row.try_get("", "expiresAt")?,
            status: row.try_get("", "status")?,
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
    .and_then(grant)
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
    let enrollment = tx
        .query_one_raw(sql(
            r#"SELECT manifest FROM "DeviceEnrollment" WHERE id=$1"#,
            [device.receipt.enrollment_id.clone().into()],
        ))
        .await?
        .ok_or(ApiError::NOT_FOUND)?;
    let manifest: OnboardingManifest =
        serde_json::from_str(&enrollment.try_get::<String>("", "manifest")?)?;
    let row = tx.query_one_raw(sql(r#"SELECT "policyJws" FROM "DeviceManagementPolicy" WHERE "deviceId"=$1 ORDER BY version DESC LIMIT 1"#, [device.status.device_id.clone().into()])).await?.ok_or(ApiError::NOT_FOUND)?;
    let policy = verify_management_policy(
        &row.try_get::<String>("", "policyJws")?,
        &manifest.owner_invitation_key,
        now(),
    )
    .map_err(|_| ApiError::forbidden("Device deployment approval expired or changed"))?;
    let requested = ManagementScope::Placement {
        project_id: grant.project_id.clone(),
        placement_id: grant.placement_id.clone(),
    };
    let permission_deadline = policy
        .grants
        .iter()
        .filter(|permission| {
            permission.user_id == grant.delegating_user_id
                && permission.expires_at > now()
                && permission
                    .capabilities
                    .contains(&ManagementCapability::Deploy)
                && inventory_scope_contains(&permission.scope, &requested)
        })
        .map(|permission| permission.expires_at)
        .max();
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
    live(expires_at)?;
    if tx.execute_raw(sql(r#"INSERT INTO "InstanceProofReplay" ("instanceId","keyEpoch","proofId","expiresAt") VALUES ($1,$2,$3,$4) ON CONFLICT ("instanceId","keyEpoch","proofId") DO NOTHING"#,
        [id.into(),(key_epoch as i64).into(),jti.into(),expires_at.into()])).await?.rows_affected()!=1 {
        return Err(ApiError::unauthorized("Instance proof was already used"));
    }
    tx.execute_raw(sql(r#"DELETE FROM "InstanceProofReplay" WHERE ("instanceId","keyEpoch","proofId") IN (SELECT "instanceId","keyEpoch","proofId" FROM "InstanceProofReplay" WHERE "instanceId"=$1 AND "expiresAt"<$2 LIMIT 128)"#,[id.into(),(now()-60).into()])).await?;
    live(expires_at)
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

pub(super) async fn list_grants(
    db: &DatabaseConnection,
    device_id: &str,
) -> Result<Vec<ResourceGrantResponse>, ApiError> {
    db.query_all_raw(sql(r#"SELECT * FROM "PlacementResourceGrant" WHERE "deviceId"=$1 ORDER BY CASE WHEN status='active' THEN 0 ELSE 1 END,"createdAt" DESC LIMIT 1000"#,[device_id.into()])).await?.into_iter().map(|r|grant(r).map(|g|g.info)).collect()
}

pub(super) async fn list_instances(
    db: &DatabaseConnection,
    device_id: &str,
) -> Result<Vec<InstanceReceipt>, ApiError> {
    db.query_all_raw(sql(r#"SELECT * FROM "WorkloadInstance" WHERE "deviceId"=$1 AND ((status='active' AND (purpose IS NULL OR purpose='workload')) OR (status='validating' AND purpose='rollout_validation' AND "registeredAt">$3)) AND "leaseExpiresAt">$2 ORDER BY "registeredAt" DESC LIMIT 1000"#,[device_id.into(),now().into(),(now()-INSTANCE_VALIDATION_SECONDS).into()])).await?.into_iter().map(|r|instance(r).map(|i|i.receipt)).collect()
}
