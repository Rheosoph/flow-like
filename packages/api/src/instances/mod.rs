//! Workload keys authorize one placement's cloud resources. They never become
//! human principals or device management credentials.

mod budget;
mod jwt;
pub(crate) mod offline;
pub(crate) mod project;
mod repository;

pub(crate) use budget::{authorize_start, reserve_budget, settle_budget};

use crate::{
    backend_jwt::{self, TokenType},
    db::{RetryPolicy, retry_transaction},
    devices::{self, DeviceContext},
    error::ApiError,
    state::AppState,
};
use axum::http::HeaderMap;
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use flow_like_device_protocol::*;
use repository::*;
use sea_orm::ConnectionTrait;
use serde::{Deserialize, Serialize};
use std::result::Result;

const LEASE_SECONDS: i64 = INSTANCE_LEASE_SECONDS;
const MAX_GRANT_SECONDS: i64 = 365 * 24 * 60 * 60;
const MAX_BUDGET_MICROS: i64 = 1_000_000_000_000;

fn now() -> i64 {
    chrono::Utc::now().timestamp()
}
fn invalid(_: impl std::fmt::Display) -> ApiError {
    ApiError::bad_request("Invalid instance protocol input")
}
fn bad_proof(_: impl std::fmt::Display) -> ApiError {
    ApiError::unauthorized("Instance proof is invalid or expired")
}
fn endpoint(state: &DeviceContext<'_>, path: &str) -> Result<String, ApiError> {
    endpoint_url(&devices::api_base_url(state)?, path).map_err(invalid)
}

/// This context is produced only by the instance token and possession verifier.
/// Persisted worker contexts are rechecked against the registry at admission and
/// immediately before provider dispatch.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct VerifiedInstanceUsage {
    pub(crate) instance_id: String,
    pub(crate) device_id: String,
    pub(crate) device_auth_epoch: u64,
    pub(crate) key_epoch: u64,
    pub(crate) grant_id: String,
    pub(crate) authz_version: u64,
    pub(crate) billing_grant_id: String,
    pub(crate) billing_authz_version: u64,
    pub(crate) delegated_user_id: String,
    pub(crate) payer_id: String,
    pub(crate) project_id: String,
    pub(crate) placement_id: String,
    pub(crate) deployment_id: String,
    pub(crate) app_id: Option<String>,
    pub(crate) model_id: String,
    pub(crate) request_method: String,
    pub(crate) request_path: String,
    pub(crate) proof_expires_at: i64,
}

pub(super) fn validate_usage(
    graph: &Graph,
    usage: &VerifiedInstanceUsage,
    require_live_proof: bool,
) -> Result<(), ApiError> {
    let grant = &graph.grant.info;
    let billing = &require_billing(&graph)?.info;
    let instance = &graph.instance.receipt;
    if usage.instance_id != instance.instance_id
        || usage.device_id != instance.device_id
        || usage.key_epoch != instance.key_epoch
        || usage.device_auth_epoch != graph.device.status.auth_epoch
        || usage.grant_id != grant.grant_id
        || usage.authz_version != grant.authz_version
        || usage.billing_grant_id != billing.billing_grant_id
        || usage.billing_authz_version != billing.authz_version
        || usage.delegated_user_id != grant.delegating_user_id
        || usage.payer_id != billing.payer_id
        || usage.project_id != grant.project_id
        || usage.placement_id != grant.placement_id
        || usage.deployment_id != grant.deployment_id
        || usage.app_id != grant.app_id
        || !grant.model_ids.contains(&usage.model_id)
        || usage.request_method != "POST"
        || !matches!(
            usage.request_path.as_str(),
            "/instances/chat/completions" | "/instances/responses" | "/instances/embeddings/embed"
        )
    {
        return Err(ApiError::forbidden(
            "Request exceeds the current instance resource grant",
        ));
    }
    if require_live_proof {
        live(usage.proof_expires_at)?;
    }
    Ok(())
}

pub(crate) async fn create_grant(
    state: &DeviceContext<'_>,
    owner: &str,
    device_id: &str,
    mut request: CreateResourceGrantRequest,
) -> Result<ResourceGrantResponse, ApiError> {
    devices::enabled(state)?;
    devices::repository::active_account(state.db, owner).await?;
    for id in [
        &request.placement_id,
        &request.deployment_id,
        &request.project_id,
    ] {
        validate_instance_identifier(id).map_err(invalid)?;
    }
    if let Some(app_id) = &request.app_id {
        validate_instance_identifier(app_id).map_err(invalid)?;
        if app_id != &request.project_id {
            return Err(ApiError::bad_request(
                "Online project identity must match its app",
            ));
        }
    }
    if request.expires_at <= now()
        || request.expires_at > now() + MAX_GRANT_SECONDS
        || !(1..=100).contains(&request.max_instances)
        || (request.model_ids.is_empty() && request.online_access.is_none())
        || request.model_ids.len() > 64
    {
        return Err(ApiError::bad_request("Invalid resource grant limits"));
    }
    for model in &request.model_ids {
        validate_instance_identifier(model).map_err(invalid)?;
    }
    request.model_ids.sort();
    if request.model_ids.windows(2).any(|pair| pair[0] == pair[1]) {
        return Err(ApiError::bad_request("Model allowlist contains duplicates"));
    }
    if request.online_access.is_some() && request.app_id.is_none() {
        return Err(ApiError::bad_request(
            "Online access requires an online project",
        ));
    }
    let info = ResourceGrantResponse {
        grant_id: uuid::Uuid::new_v4().to_string(),
        device_id: device_id.into(),
        placement_id: request.placement_id,
        deployment_id: request.deployment_id,
        project_id: request.project_id,
        app_id: request.app_id,
        delegating_user_id: owner.into(),
        authz_version: 1,
        model_ids: request.model_ids,
        max_instances: request.max_instances,
        expires_at: request.expires_at,
        status: "active".into(),
        online_access: request.online_access,
    };
    retry_transaction(state.db,state.dialect,None,&RetryPolicy::default(),move |tx| {
        let info=info.clone();
        Box::pin(async move {
            let device=lock_device(tx,&info.device_id).await?;
            device_deployment_authority(tx, &device, &info).await?;
            project_authority(tx,&info).await?; live(info.expires_at)?;
            let count=tx.query_one_raw(sql(r#"SELECT COUNT(*) AS count FROM "PlacementResourceGrant" WHERE "deviceId"=$1 AND status='active' AND "expiresAt">$2"#,[info.device_id.clone().into(),now().into()])).await?.ok_or_else(||ApiError::internal("Missing grant count"))?.try_get::<i64>("","count")?;
            if count>=100 {return Err(ApiError::too_many_requests("Device resource grant limit reached"));}
            if tx.query_one_raw(sql(r#"SELECT id FROM "PlacementResourceGrant" WHERE "deviceId"=$1 AND "placementId"=$2 AND status='active' AND "expiresAt">$3"#,[info.device_id.clone().into(),info.placement_id.clone().into(),now().into()])).await?.is_some() {return Err(ApiError::conflict("Placement already has an active resource grant"));}
            for model in &info.model_ids {
                if tx.query_one_raw(sql(r#"SELECT id FROM "Bit" WHERE id=$1"#,[model.clone().into()])).await?.is_none() {return Err(ApiError::bad_request("An allowed model does not exist"));}
            }
            tx.execute_raw(sql(r#"INSERT INTO "PlacementResourceGrant" (id,"deviceId","placementId","deploymentId","projectId","appId","delegatingUserId","approvedByUserId",status,"authzVersion","modelIds","maxInstances","expiresAt","createdAt","onlineAccess") VALUES ($1,$2,$3,$4,$5,$6,$7,$7,'active',1,$8,$9,$10,$11,$12)"#,
                vec![info.grant_id.clone().into(),info.device_id.clone().into(),info.placement_id.clone().into(),info.deployment_id.clone().into(),info.project_id.clone().into(),info.app_id.clone().into(),info.delegating_user_id.clone().into(),serde_json::to_string(&info.model_ids)?.into(),i64::from(info.max_instances).into(),info.expires_at.into(),now().into(),info.online_access.map(|access|serde_json::to_string(&access)).transpose()?.into()])).await?;
            live(info.expires_at)?; Ok(info)
        })
    }).await
}

pub(crate) async fn approve_billing(
    state: &DeviceContext<'_>,
    owner: &str,
    device_id: &str,
    grant_id: &str,
    request: ApproveBillingGrantRequest,
) -> Result<BillingGrantResponse, ApiError> {
    get_grant(state, owner, device_id, grant_id).await?;
    if request.limit_micros <= 0
        || request.limit_micros > MAX_BUDGET_MICROS
        || request.expires_at <= now()
    {
        return Err(ApiError::bad_request("Invalid billing grant limits"));
    }
    let device_id = device_id.to_owned();
    let owner = owner.to_owned();
    let grant_id = grant_id.to_owned();
    let id = uuid::Uuid::new_v4().to_string();
    retry_transaction(state.db,state.dialect,None,&RetryPolicy::default(),move |tx| {
        let device_id=device_id.clone();let owner=owner.clone();let grant_id=grant_id.clone();let id=id.clone();let request=request.clone();
        Box::pin(async move {
            let device=lock_device(tx,&device_id).await?;
            let grant=lock_grant(tx,&grant_id).await?;
            if grant.info.device_id!=device_id || (device.status.owner_id!=owner && grant.info.delegating_user_id!=owner) {return Err(ApiError::NOT_FOUND);}
            device_deployment_authority(tx, &device, &grant.info).await?;
            devices::repository::active_account(tx, &owner).await?;
            if grant.info.model_ids.is_empty() {return Err(ApiError::bad_request("This grant does not authorize hosted models"));}
            if request.expires_at>grant.info.expires_at {return Err(ApiError::bad_request("Billing grant cannot outlive resource access"));}
            live(request.expires_at)?;
            if tx.query_one_raw(sql(r#"SELECT id FROM "PlacementBillingGrant" WHERE "grantId"=$1 AND status='active' AND "expiresAt">$2"#,[grant_id.clone().into(),now().into()])).await?.is_some() {return Err(ApiError::conflict("This resource grant already has billing consent"));}
            tx.execute_raw(sql(r#"INSERT INTO "PlacementBillingGrant" (id,"grantId","payerId","approvedByUserId",status,"authzVersion","limitMicros","usedMicros","reservedMicros","expiresAt","createdAt") VALUES ($1,$2,$3,$3,'active',1,$4,0,0,$5,$6)"#,[id.clone().into(),grant_id.clone().into(),owner.clone().into(),request.limit_micros.into(),request.expires_at.into(),now().into()])).await?;
            live(request.expires_at)?;
            Ok(BillingGrantResponse {billing_grant_id:id,grant_id,payer_id:owner,authz_version:1,limit_micros:request.limit_micros,used_micros:0,reserved_micros:0,expires_at:request.expires_at,status:"active".into()})
        })
    }).await
}

/// Keep consent reachable after sharing is withdrawn so a delegator can revoke it.
pub(crate) async fn consent_devices(
    state: &DeviceContext<'_>,
    user: &str,
) -> Result<Vec<DeviceStatus>, ApiError> {
    let rows = state.db.query_all_raw(sql(r#"SELECT d.* FROM "ManagedDevice" d WHERE d."ownerId"<>$1 AND EXISTS(SELECT 1 FROM "PlacementResourceGrant" g WHERE g."deviceId"=d.id AND g."delegatingUserId"=$1) ORDER BY d."registeredAt" DESC LIMIT 1000"#, [user.into()])).await?;
    rows.into_iter()
        .map(|row| devices::repository::device(row).map(|device| device.status))
        .collect()
}

pub(crate) async fn grants(
    state: &DeviceContext<'_>,
    owner: &str,
    device_id: &str,
) -> Result<Vec<ResourceGrantResponse>, ApiError> {
    devices::enabled(state)?;
    devices::repository::active_account(state.db, owner).await?;
    let device = devices::repository::current_device(state.db, device_id).await?;
    Ok(list_grants(state.db, device_id)
        .await?
        .into_iter()
        .filter(|grant| device.status.owner_id == owner || grant.delegating_user_id == owner)
        .collect())
}
pub(crate) async fn get_grant(
    state: &DeviceContext<'_>,
    owner: &str,
    device_id: &str,
    id: &str,
) -> Result<ResourceGrantResponse, ApiError> {
    devices::enabled(state)?;
    devices::repository::active_account(state.db, owner).await?;
    let device = devices::repository::current_device(state.db, device_id).await?;
    let grant = read_grant(state.db, id).await?;
    if grant.info.device_id != device_id
        || (device.status.owner_id != owner && grant.info.delegating_user_id != owner)
    {
        return Err(ApiError::NOT_FOUND);
    }
    Ok(grant.info)
}
pub(crate) async fn get_billing(
    state: &DeviceContext<'_>,
    owner: &str,
    device_id: &str,
    grant_id: &str,
) -> Result<BillingGrantResponse, ApiError> {
    get_grant(state, owner, device_id, grant_id).await?;
    let row=state.db.query_one_raw(sql(r#"SELECT id FROM "PlacementBillingGrant" WHERE "grantId"=$1 ORDER BY CASE WHEN status='active' AND "expiresAt">$2 THEN 0 ELSE 1 END, "createdAt" DESC,id DESC LIMIT 1"#,[grant_id.into(),now().into()])).await?.ok_or(ApiError::NOT_FOUND)?;
    Ok(read_billing(state.db, &row.try_get::<String>("", "id")?)
        .await?
        .info)
}
pub(crate) async fn instances(
    state: &DeviceContext<'_>,
    owner: &str,
    device_id: &str,
) -> Result<Vec<InstanceReceipt>, ApiError> {
    let allowed = grants(state, owner, device_id).await?;
    Ok(list_instances(state.db, device_id)
        .await?
        .into_iter()
        .filter(|instance| {
            allowed
                .iter()
                .any(|grant| grant.grant_id == instance.grant_id)
        })
        .collect())
}

pub(crate) async fn revoke_grant(
    state: &DeviceContext<'_>,
    owner: &str,
    device_id: &str,
    grant_id: &str,
) -> Result<(), ApiError> {
    get_grant(state, owner, device_id, grant_id).await?;
    let owner = owner.to_owned();
    let device_id = device_id.to_owned();
    let grant_id = grant_id.to_owned();
    retry_transaction(state.db,state.dialect,None,&RetryPolicy::default(),move |tx| {
        let owner=owner.clone();let device_id=device_id.clone();let grant_id=grant_id.clone();
        Box::pin(async move {
            // A revoked device must still permit its owner to withdraw consent.
            if tx.execute_raw(sql(r#"UPDATE "ManagedDevice" SET "authEpoch"="authEpoch" WHERE id=$1"#,[device_id.clone().into()])).await?.rows_affected()!=1 {return Err(ApiError::NOT_FOUND);}
            let grant=read_grant(tx,&grant_id).await?;
            let device=devices::repository::current_device(tx,&device_id).await?;
            if grant.info.device_id!=device_id || (device.status.owner_id!=owner && grant.info.delegating_user_id!=owner) {return Err(ApiError::NOT_FOUND);}
            if tx.execute_raw(sql(r#"UPDATE "PlacementResourceGrant" SET status='revoked',"authzVersion"="authzVersion"+1 WHERE id=$1 AND status='active'"#,[grant_id.into()])).await?.rows_affected()!=1 {return Err(ApiError::NOT_FOUND);}
            Ok(())
        })
    }).await
}

pub(crate) async fn billing_grants(
    state: &DeviceContext<'_>,
    owner: &str,
    device_id: &str,
) -> Result<Vec<BillingGrantResponse>, ApiError> {
    let allowed = grants(state, owner, device_id).await?;
    let rows=state.db.query_all_raw(sql(r#"SELECT b.id FROM "PlacementBillingGrant" b JOIN "PlacementResourceGrant" g ON g.id=b."grantId" WHERE g."deviceId"=$1 ORDER BY CASE WHEN b.status='active' THEN 0 ELSE 1 END,b."createdAt" DESC LIMIT 1000"#,[device_id.into()])).await?;
    let mut result = Vec::with_capacity(rows.len());
    for row in rows {
        result.push(
            read_billing(state.db, &row.try_get::<String>("", "id")?)
                .await?
                .info,
        );
    }
    result.retain(|billing| {
        allowed
            .iter()
            .any(|grant| grant.grant_id == billing.grant_id)
    });
    Ok(result)
}

pub(crate) async fn billing_grant(
    state: &DeviceContext<'_>,
    owner: &str,
    device_id: &str,
    billing_id: &str,
) -> Result<BillingGrantResponse, ApiError> {
    let billing = read_billing(state.db, billing_id).await?;
    get_grant(state, owner, device_id, &billing.info.grant_id).await?;
    Ok(billing.info)
}

pub(crate) async fn revoke_billing(
    state: &DeviceContext<'_>,
    owner: &str,
    device_id: &str,
    billing_id: &str,
) -> Result<(), ApiError> {
    let billing = billing_grant(state, owner, device_id, billing_id).await?;
    if billing.payer_id != owner {
        return Err(ApiError::NOT_FOUND);
    }
    let device_id = device_id.to_owned();
    let owner = owner.to_owned();
    retry_transaction(state.db,state.dialect,None,&RetryPolicy::default(),move |tx| {
        let device_id=device_id.clone();let owner=owner.clone();let billing=billing.clone();
        Box::pin(async move {
            if tx.execute_raw(sql(r#"UPDATE "ManagedDevice" SET "authEpoch"="authEpoch" WHERE id=$1"#,[device_id.into()])).await?.rows_affected()!=1 {return Err(ApiError::NOT_FOUND);}
            tx.execute_raw(sql(r#"UPDATE "PlacementResourceGrant" SET "authzVersion"="authzVersion" WHERE id=$1"#,[billing.grant_id.into()])).await?;
            if tx.execute_raw(sql(r#"UPDATE "PlacementBillingGrant" SET status='revoked',"authzVersion"="authzVersion"+1 WHERE id=$1 AND "payerId"=$2 AND status='active'"#,[billing.billing_grant_id.into(),owner.into()])).await?.rows_affected()!=1 {return Err(ApiError::NOT_FOUND);}
            Ok(())
        })
    }).await
}

pub(crate) async fn register(
    state: &DeviceContext<'_>,
    device_id: &str,
    request: InstanceRegistrationRequest,
) -> Result<InstanceReceipt, ApiError> {
    devices::enabled(state)?;
    let device = devices::repository::current_device(state.db, device_id).await?;
    let url = endpoint(state, &format!("/devices/{device_id}/instances"))?;
    let binding = verify_instance_registration(
        &request.registration_jws,
        &device.status.identity.auth_key,
        device_id,
        &url,
        now(),
    )
    .map_err(bad_proof)?;
    let proof = verify_instance_possession(
        &request.possession_jws,
        &binding.workload_key,
        &binding.instance_id,
        &url,
        &request.registration_jws,
        now(),
    )
    .map_err(bad_proof)?;
    let registration_jws = request.registration_jws;
    retry_transaction(state.db,state.dialect,None,&RetryPolicy::default(),move |tx| {
        let binding=binding.clone();let proof=proof.clone();let registration_jws=registration_jws.clone();
        Box::pin(async move {
            let device=lock_device(tx,&binding.device_id).await?;
            let grant=lock_grant(tx,&binding.grant_id).await?;
            device_deployment_authority(tx, &device, &grant.info).await?;
            let billing=match &binding.billing_grant_id { Some(id) => Some(lock_billing(tx,id).await?), None => None };
            if binding.device_auth_epoch!=device.status.auth_epoch || grant.info.device_id!=binding.device_id
                || (binding.purpose==InstancePurpose::RolloutValidation && (grant.info.online_access.is_none() || binding.billing_grant_id.is_some()))
                || billing.as_ref().is_some_and(|billing| grant.info.grant_id!=billing.info.grant_id || binding.billing_authz_version!=Some(billing.info.authz_version))
                || (billing.is_none() && grant.info.online_access.is_none()) || binding.placement_id!=grant.info.placement_id
                || binding.deployment_id!=grant.info.deployment_id || binding.project_id!=grant.info.project_id
                || binding.authz_version!=grant.info.authz_version
                || binding.workload_key==device.status.identity.auth_key || binding.workload_key==device.status.identity.telemetry_key
                || binding.workload_key.to_bytes().map_err(bad_proof)?==device.status.identity.management_key {
                return Err(ApiError::unauthorized("Instance registration exceeds its placement authorization"));
            }
            let onboarding=tx.query_one_raw(sql(r#"SELECT manifest FROM "DeviceEnrollment" WHERE id=$1"#,[device.receipt.enrollment_id.clone().into()])).await?.ok_or_else(||ApiError::internal("Device onboarding trust record is missing"))?;
            let onboarding:OnboardingManifest=serde_json::from_str(&onboarding.try_get::<String>("","manifest")?)?;
            if [&onboarding.bootstrap_key,&onboarding.controller_key,&onboarding.owner_invitation_key].contains(&&binding.workload_key) {
                return Err(ApiError::forbidden("Workload key must be separate from onboarding and control keys"));
            }
            if tx.query_one_raw(sql(r#"SELECT id FROM "WorkloadInstance" WHERE "workloadKeyThumbprint"=$1"#,[binding.workload_key.thumbprint().map_err(bad_proof)?.into()])).await?.is_some() {
                return Err(ApiError::conflict("A workload key may identify only one instance incarnation"));
            }
            match binding.purpose {
                InstancePurpose::Workload => check_capacity(tx,&grant.info).await?,
                InstancePurpose::RolloutValidation => check_validation_capacity(tx,&binding.device_id).await?,
            }
            let expiry=binding.exp.min(proof.exp).min(grant.info.expires_at).min(billing.as_ref().map(|billing|billing.info.expires_at).unwrap_or(i64::MAX)); live(expiry)?;
            devices::repository::consume_proof(tx,&binding.device_id,binding.device_auth_epoch,&binding.jti,expiry).await?;
            if tx.query_one_raw(sql(r#"SELECT id FROM "WorkloadInstance" WHERE id=$1"#,[binding.instance_id.clone().into()])).await?.is_some() {return Err(ApiError::conflict("Instance is already registered; recover with its workload key"));}
            let registered_at=now();let lease_expires_at=(registered_at+LEASE_SECONDS).min(grant.info.expires_at).min(if grant.info.online_access.is_some() {i64::MAX} else {billing.as_ref().map(|billing|billing.info.expires_at).unwrap_or(0)});
            let receipt=InstanceReceipt {instance_id:binding.instance_id,purpose:binding.purpose,device_id:binding.device_id,grant_id:binding.grant_id,billing_grant_id:binding.billing_grant_id,
                workload_key:binding.workload_key,key_epoch:1,registered_at,lease_expires_at,registration_jws};
            // Older API replicas authorize only status='active'. Keep validation
            // identities outside that predicate during a rolling server upgrade.
            let status=match receipt.purpose {InstancePurpose::Workload=>"active",InstancePurpose::RolloutValidation=>"validating"};
            tx.execute_raw(sql(r#"INSERT INTO "WorkloadInstance" (id,"deviceId","grantId","billingGrantId","workloadKey","workloadKeyThumbprint","deviceAuthEpoch","grantAuthzVersion","billingAuthzVersion","keyEpoch",status,"registeredAt","leaseExpiresAt","registrationJws",purpose) VALUES ($1,$2,$3,$4,$5,$9,$10,$11,$12,1,$14,$6,$7,$8,$13)"#,
                [receipt.instance_id.clone().into(),receipt.device_id.clone().into(),receipt.grant_id.clone().into(),receipt.billing_grant_id.clone().into(),serde_json::to_string(&receipt.workload_key)?.into(),registered_at.into(),lease_expires_at.into(),receipt.registration_jws.clone().into(),receipt.workload_key.thumbprint().map_err(bad_proof)?.into(),(binding.device_auth_epoch as i64).into(),(binding.authz_version as i64).into(),binding.billing_authz_version.map(|version|version as i64).into(),receipt.purpose.as_str().into(),status.into()])).await?;
            consume_instance_proof(tx,&receipt.instance_id,1,&proof.jti,expiry).await?;live(expiry)?;Ok(receipt)
        })
    }).await
}

async fn authenticated_instance(
    state: &DeviceContext<'_>,
    id: &str,
    assertion: &str,
    path: &str,
    renew: bool,
) -> Result<Graph, ApiError> {
    devices::enabled(state)?;
    let instance = read_instance(state.db, id).await?;
    let proof = verify_workload_assertion(
        assertion,
        &instance.receipt.workload_key,
        id,
        &endpoint(state, path)?,
        now(),
    )
    .map_err(bad_proof)?;
    let id = id.to_owned();
    let key_epoch = instance.receipt.key_epoch;
    retry_transaction(
        state.db,
        state.dialect,
        None,
        &RetryPolicy::default(),
        move |tx| {
            let id = id.clone();
            let proof = proof.clone();
            Box::pin(async move {
                let mut graph = lock_graph(tx, &id, true).await?;
                if graph.instance.receipt.key_epoch != key_epoch {
                    return Err(ApiError::UNAUTHORIZED);
                }
                consume_instance_proof(tx, &id, key_epoch, &proof.jti, proof.exp).await?;
                if renew {
                    renew_lease(tx, &mut graph).await?;
                }
                live(proof.exp)?;
                live(graph.grant.info.expires_at)?;
                if graph.grant.info.online_access.is_none() {
                    require_billing(&graph)?;
                }
                Ok(graph)
            })
        },
    )
    .await
}

pub(crate) async fn token(
    state: &DeviceContext<'_>,
    id: &str,
    request: InstanceTokenRequest,
) -> Result<InstanceTokenResponse, ApiError> {
    let graph = authenticated_instance(
        state,
        id,
        &request.client_assertion,
        &format!("/instances/{id}/token"),
        true,
    )
    .await?;
    let grant = &graph.grant.info;
    let billing = &require_billing(&graph)?.info;
    devices::repository::active_account(state.db, &billing.payer_id).await?;
    let receipt = &graph.instance.receipt;
    let now = now();
    let expires_at = (now + jwt::TOKEN_SECONDS)
        .min(receipt.lease_expires_at)
        .min(grant.expires_at)
        .min(billing.expires_at);
    live(expires_at)?;
    let nonce = URL_SAFE_NO_PAD.encode(rand::random::<[u8; 32]>());
    let claims = jwt::ResourceClaims {
        sub: grant.delegating_user_id.clone(),
        act: jwt::Actor {
            sub: format!("instance:{id}"),
        },
        instance_id: id.into(),
        device_id: receipt.device_id.clone(),
        device_auth_epoch: graph.device.status.auth_epoch,
        key_epoch: receipt.key_epoch,
        grant_id: grant.grant_id.clone(),
        authz_version: grant.authz_version,
        billing_grant_id: billing.billing_grant_id.clone(),
        billing_authz_version: billing.authz_version,
        deployment_id: grant.deployment_id.clone(),
        placement_id: grant.placement_id.clone(),
        project_id: grant.project_id.clone(),
        app_id: grant.app_id.clone(),
        cnf: devices::jwt::Confirmation {
            jkt: receipt.workload_key.thumbprint().map_err(bad_proof)?,
        },
        dpop_nonce: nonce.clone(),
        scope: INSTANCE_MODELS_SCOPE.into(),
        typ: TokenType::InstanceResource,
        iss: backend_jwt::issuer().into(),
        aud: INSTANCE_MODELS_AUDIENCE.into(),
        iat: now,
        nbf: now,
        exp: expires_at,
        jti: uuid::Uuid::new_v4().to_string(),
    };
    Ok(InstanceTokenResponse {
        access_token: backend_jwt::sign_typed(&claims, jwt::JOSE_TYPE)
            .map_err(|_| ApiError::internal("Cannot sign instance token"))?,
        token_type: "DPoP".into(),
        expires_in: (expires_at - now) as u64,
        expires_at,
        dpop_nonce: nonce,
        lease_expires_at: receipt.lease_expires_at,
    })
}

pub(crate) async fn receipt(
    state: &DeviceContext<'_>,
    id: &str,
    request: ReceiptRequest,
) -> Result<InstanceReceipt, ApiError> {
    Ok(authenticated_instance(
        state,
        id,
        &request.client_assertion,
        &format!("/instances/{id}/receipt"),
        false,
    )
    .await?
    .instance
    .receipt)
}

pub(crate) async fn retire(
    state: &DeviceContext<'_>,
    device_id: &str,
    id: &str,
    request: ReceiptRequest,
) -> Result<(), ApiError> {
    devices::enabled(state)?;
    let device = devices::repository::current_device(state.db, device_id).await?;
    let proof = verify_client_assertion(
        &request.client_assertion,
        &device.status.identity.auth_key,
        device_id,
        &endpoint(state, &format!("/devices/{device_id}/instances/{id}"))?,
        now(),
    )
    .map_err(bad_proof)?;
    let device_id = device_id.to_owned();
    let id = id.to_owned();
    let epoch = device.status.auth_epoch;
    retry_transaction(state.db,state.dialect,None,&RetryPolicy::default(),move |tx| {
        let device_id=device_id.clone();let id=id.clone();let proof=proof.clone();
        Box::pin(async move {
            let device=lock_device(tx,&device_id).await?;
            if device.status.auth_epoch!=epoch {return Err(ApiError::UNAUTHORIZED);}
            let instance=read_instance(tx,&id).await?;
            if instance.receipt.device_id!=device_id {return Err(ApiError::NOT_FOUND);}
            devices::repository::consume_proof(tx,&device_id,epoch,&proof.jti,proof.exp).await?;
            tx.execute_raw(sql(r#"UPDATE "WorkloadInstance" SET status='retired',"keyEpoch"="keyEpoch"+1,"leaseExpiresAt"=$2 WHERE id=$1 AND status IN ('active','validating')"#,[id.into(),now().into()])).await?;
            live(proof.exp)
        })
    }).await
}

fn credentials(headers: &HeaderMap) -> Result<(&str, &str), ApiError> {
    if headers.get_all("dpop").iter().count() != 1
        || headers.get_all("authorization").iter().count() > 1
        || headers
            .get_all(crate::middleware::jwt::FORWARDED_AUTHORIZATION_HEADER)
            .iter()
            .count()
            > 1
    {
        return Err(ApiError::UNAUTHORIZED);
    }
    let token = crate::middleware::jwt::viewer_authorization(headers)
        .and_then(|v| v.strip_prefix("DPoP "))
        .ok_or(ApiError::UNAUTHORIZED)?;
    let proof = headers
        .get("dpop")
        .and_then(|v| v.to_str().ok())
        .ok_or(ApiError::UNAUTHORIZED)?;
    if token.len() > MAX_COMPACT_JWS_BYTES || proof.len() > MAX_COMPACT_JWS_BYTES {
        return Err(ApiError::UNAUTHORIZED);
    }
    Ok((token, proof))
}

pub(crate) async fn authenticate_model_request(
    state: &AppState,
    headers: &HeaderMap,
    method: &str,
    path: &str,
    bit_id: &str,
) -> Result<VerifiedInstanceUsage, ApiError> {
    authenticate_model_request_with_context(&devices::context(state), headers, method, path, bit_id)
        .await
}

pub(super) async fn authenticate_model_request_with_context(
    state: &DeviceContext<'_>,
    headers: &HeaderMap,
    method: &str,
    path: &str,
    bit_id: &str,
) -> Result<VerifiedInstanceUsage, ApiError> {
    devices::enabled(state)?;
    let (token, proof) = credentials(headers)?;
    let claims = jwt::verify(token).map_err(bad_proof)?;
    let instance = read_instance(state.db, &claims.instance_id).await?;
    let proof = verify_dpop(
        proof,
        &instance.receipt.workload_key,
        &DpopContext {
            method,
            url: &endpoint(state, path)?,
            access_token: Some(token),
            nonce: Some(&claims.dpop_nonce),
            key_thumbprint: &claims.cnf.jkt,
            now: now(),
        },
    )
    .map_err(bad_proof)?;
    if headers.get_all("x-flow-like-app-id").iter().count() > 1 {
        return Err(ApiError::bad_request("Duplicate project attribution"));
    }
    if let Some(app) = headers.get("x-flow-like-app-id") {
        if app.to_str().ok() != claims.app_id.as_deref() {
            return Err(ApiError::forbidden(
                "Project attribution differs from the instance grant",
            ));
        }
    }
    for name in [
        "x-flow-like-payer-id",
        "x-flow-like-user-id",
        "x-flow-like-instance-id",
    ] {
        if headers.contains_key(name) {
            return Err(ApiError::forbidden(
                "Instance attribution is selected by its grant",
            ));
        }
    }
    let method = method.to_owned();
    let path = path.to_owned();
    let bit_id = bit_id.to_owned();
    retry_transaction(
        state.db,
        state.dialect,
        None,
        &RetryPolicy::default(),
        move |tx| {
            let claims = claims.clone();
            let proof = proof.clone();
            let method = method.clone();
            let path = path.clone();
            let bit_id = bit_id.clone();
            Box::pin(async move {
                let graph = lock_graph(tx, &claims.instance_id, false).await?;
                devices::repository::active_account(tx, &require_billing(&graph)?.info.payer_id)
                    .await?;
                let usage = VerifiedInstanceUsage {
                    instance_id: claims.instance_id,
                    device_id: claims.device_id,
                    device_auth_epoch: claims.device_auth_epoch,
                    key_epoch: claims.key_epoch,
                    grant_id: claims.grant_id,
                    authz_version: claims.authz_version,
                    billing_grant_id: claims.billing_grant_id,
                    billing_authz_version: claims.billing_authz_version,
                    delegated_user_id: claims.sub,
                    payer_id: require_billing(&graph)?.info.payer_id.clone(),
                    project_id: claims.project_id,
                    placement_id: claims.placement_id,
                    deployment_id: claims.deployment_id,
                    app_id: claims.app_id,
                    model_id: bit_id,
                    request_method: method,
                    request_path: path,
                    proof_expires_at: (proof.iat + MAX_ASSERTION_TTL_SECONDS).min(claims.exp),
                };
                validate_usage(&graph, &usage, true)?;
                consume_instance_proof(
                    tx,
                    &usage.instance_id,
                    usage.key_epoch,
                    &proof.jti,
                    usage.proof_expires_at,
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
                Ok(usage)
            })
        },
    )
    .await
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod header_tests;
