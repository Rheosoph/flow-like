use super::{DeviceContext, context, enabled, human_owner, repository};
use crate::{
    backend_jwt::{self, TokenType},
    db::{RetryPolicy, retry_transaction},
    error::ApiError,
    middleware::jwt::AppUser,
    state::AppState,
};
use axum::{
    Extension, Json, Router,
    extract::{Path, State},
    http::StatusCode,
    routing::{get, post},
};
use flow_like_device_protocol::*;
use sea_orm::{ConnectionTrait, DatabaseBackend, Statement, Value};
use serde::{Deserialize, Serialize};
use std::result::Result;
use utoipa::ToSchema;

fn sql(query: &str, values: impl IntoIterator<Item = Value>) -> Statement {
    Statement::from_sql_and_values(DatabaseBackend::Postgres, query, values)
}
fn invalid(error: impl std::fmt::Display) -> ApiError {
    ApiError::bad_request(error.to_string())
}

const SIGNALING_TOKEN_SECONDS: i64 = 300;
/// Relay credentials cannot be revoked, so they lapse with the subject's
/// authorization and never outlive this even when the provider allows longer.
const MANAGEMENT_TURN_SECONDS: i64 = 60 * 60;

pub(crate) async fn shared_devices(
    state: &AppState,
    user_id: &str,
) -> Result<Vec<DeviceStatus>, ApiError> {
    let now = chrono::Utc::now().timestamp();
    let rows = state
        .db
        .query_all_raw(sql(
            &format!(
                r#"SELECT d.* FROM "ManagedDevice" d WHERE {} ORDER BY d."registeredAt" DESC LIMIT 1000"#,
                super::view::SHARED
            ),
            [user_id.into(), now.into()],
        ))
        .await?;
    rows.iter().map(repository::status).collect()
}

pub(crate) fn routes() -> Router<AppState> {
    Router::new()
        .route("/{id}/signaling/device", post(signal_device))
        .route("/{id}/signaling/controller", post(signal_controller))
        .route("/{id}/management/policy", get(policy).put(put_policy))
        .route("/{id}/management/policies", post(device_policies))
        .route("/{id}/management/applied", post(device_applied))
        .route("/{id}/management/my-access", get(my_access))
        .route("/{id}/identity", get(identity))
        .route("/controller-vaults", get(super::recovery::list))
        .route(
            "/controller-vaults/{id}",
            get(super::recovery::get).put(super::recovery::put),
        )
}

#[derive(Serialize)]
struct SignalingClaims {
    typ: TokenType,
    aud: String,
    iss: String,
    scope: String,
    sub: String,
    device_id: String,
    device_auth_epoch: u64,
    role: String,
    participant_id: String,
    iat: i64,
    nbf: i64,
    exp: i64,
    jti: String,
}

/// `authorized_until` is when the subject's access ends; `None` means only the
/// device registration itself bounds it.
async fn signaling_response(
    state: &AppState,
    device: &DeviceStatus,
    subject: &str,
    role: &str,
    participant: &str,
    authorized_until: Option<i64>,
) -> Result<DeviceSignalingResponse, ApiError> {
    if participant.is_empty()
        || participant.len() > 128
        || !participant
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"_.-".contains(&c))
        || [".", ".."].contains(&participant)
    {
        return Err(ApiError::bad_request("Invalid signaling participant"));
    }
    let now = chrono::Utc::now().timestamp();
    let authorized_until = authorized_until.unwrap_or(i64::MAX);
    let expires_at = authorized_until.min(now + SIGNALING_TOKEN_SECONDS);
    if expires_at <= now {
        return Err(ApiError::UNAUTHORIZED);
    }
    let mut urls = Vec::new();
    for configured in state
        .platform_config
        .signaling
        .as_deref()
        .unwrap_or_default()
        .iter()
        .take(4)
    {
        let mut url = reqwest::Url::parse(configured)
            .map_err(|_| ApiError::service_unavailable("Invalid signaling configuration"))?;
        if url.scheme() != "wss"
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
        {
            return Err(ApiError::service_unavailable(
                "Device signaling requires a WSS endpoint",
            ));
        }
        url.set_path(&format!("{}/ws/devices", url.path().trim_end_matches('/')));
        urls.push(url.to_string());
    }
    if urls.is_empty() {
        return Err(ApiError::service_unavailable(
            "Device signaling is not configured",
        ));
    }
    let claims = SignalingClaims {
        typ: TokenType::DeviceSignaling,
        aud: TokenType::DeviceSignaling.audience().into(),
        iss: backend_jwt::issuer().into(),
        scope: "device:signal".into(),
        sub: subject.into(),
        device_id: device.device_id.clone(),
        device_auth_epoch: device.auth_epoch,
        role: role.into(),
        participant_id: participant.into(),
        iat: now,
        nbf: now,
        exp: expires_at,
        jti: uuid::Uuid::new_v4().to_string(),
    };
    let turn_seconds = authorized_until
        .saturating_sub(now)
        .clamp(0, MANAGEMENT_TURN_SECONDS);
    let ice = match state
        .realtime_ice
        .issue_bounded(
            &format!(
                "device-management:{}:{}:{role}:{subject}",
                device.device_id, device.auth_epoch
            ),
            &claims.jti,
            u32::try_from(turn_seconds).unwrap_or(0),
            now,
        )
        .await
    {
        Ok(ice) => ice,
        Err(error) => {
            tracing::warn!(
                error = %error,
                "Device TURN credentials unavailable; direct ICE and encrypted WebSocket remain available"
            );
            None
        }
    };
    let (ice_servers, ice_expires_at) = ice
        .map(|ice| {
            (
                ice.ice_servers
                    .into_iter()
                    .map(|entry| DeviceIceServer {
                        urls: entry.urls,
                        username: entry.username,
                        credential: entry.credential,
                    })
                    .collect(),
                Some(ice.expires_at),
            )
        })
        .unwrap_or_default();
    let policy = policy_view(state, &device.device_id).await?;
    Ok(DeviceSignalingResponse {
        policy_version: policy.version,
        policy_digest: policy.digest,
        ice_servers,
        ice_expires_at,
        token: backend_jwt::sign_typed(&claims, "flow-like-device-signaling+jwt")
            .map_err(|e| ApiError::internal(e.to_string()))?,
        expires_at,
        device_auth_epoch: device.auth_epoch,
        signaling_urls: urls,
    })
}

async fn signal_device(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(request): Json<DeviceSignalingRequest>,
) -> Result<Json<DeviceSignalingResponse>, ApiError> {
    let device = super::registered_assertion(
        &context(&state),
        &id,
        &request.client_assertion,
        &format!("/devices/{id}/signaling/device"),
    )
    .await?;
    Ok(Json(
        signaling_response(&state, &device.status, &id, "device", &id, None).await?,
    ))
}

/// This grants discovery and rendezvous only. The agent separately verifies controller keys.
/// The second value is when a grantee's access ends; it is `None` for the owner.
pub(crate) async fn admitted_device(
    state: &AppState,
    user_id: &str,
    id: &str,
) -> Result<(repository::Device, Option<i64>), ApiError> {
    admitted(&context(state), user_id, id).await
}

pub(super) async fn admitted(
    state: &DeviceContext<'_>,
    user_id: &str,
    id: &str,
) -> Result<(repository::Device, Option<i64>), ApiError> {
    enabled(state)?;
    repository::active_account(state.db, user_id).await?;
    let device = repository(state).device(id).await?;
    repository::active_account(state.db, &device.status.owner_id).await?;
    if device.status.status != DeviceRegistrationStatus::Active {
        return Err(ApiError::FORBIDDEN);
    }
    let now = chrono::Utc::now().timestamp();
    if device.status.owner_id == user_id {
        return Ok((device, None));
    }
    let row=state.db.query_one_raw(sql(r#"SELECT MAX(r."expiresAt") AS expiry FROM "DeviceManagementRecipient" r WHERE r."deviceId"=$1 AND r."userId"=$2 AND r."expiresAt">$3 AND r.version=(SELECT MAX(version) FROM "DeviceManagementPolicy" WHERE "deviceId"=$1)"#,[id.into(),user_id.into(),now.into()])).await?.ok_or(ApiError::FORBIDDEN)?;
    let expiry = row
        .try_get::<Option<i64>>("", "expiry")?
        .ok_or(ApiError::FORBIDDEN)?;
    Ok((device, Some(expiry)))
}

async fn signal_controller(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(id): Path<String>,
    Json(request): Json<ControllerSignalingRequest>,
) -> Result<Json<DeviceSignalingResponse>, ApiError> {
    let user_id = human_owner(&state, &user).await?;
    let (device, expiry) = admitted_device(&state, &user_id, &id).await?;
    Ok(Json(
        signaling_response(
            &state,
            &device.status,
            &user_id,
            "controller",
            &request.participant_id,
            expiry,
        )
        .await?,
    ))
}

async fn identity(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(id): Path<String>,
) -> Result<Json<DeviceReceipt>, ApiError> {
    let user = human_owner(&state, &user).await?;
    Ok(Json(admitted_device(&state, &user, &id).await?.0.receipt))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PolicyWrite {
    policy_jws: String,
}
#[derive(Serialize)]
struct PolicyView {
    policy_jws: Option<String>,
    version: u64,
    digest: Option<String>,
    applied_version: u64,
    applied_digest: Option<String>,
}

async fn policy_view(state: &AppState, id: &str) -> Result<PolicyView, ApiError> {
    let head=state.db.query_one_raw(sql(r#"SELECT version,digest,"policyJws" FROM "DeviceManagementPolicy" WHERE "deviceId"=$1 ORDER BY version DESC LIMIT 1"#,[id.into()])).await?;
    let applied = state
        .db
        .query_one_raw(sql(
            r#"SELECT version,digest FROM "DeviceManagementApplied" WHERE "deviceId"=$1"#,
            [id.into()],
        ))
        .await?;
    Ok(PolicyView {
        version: head
            .as_ref()
            .map(|r| r.try_get::<i64>("", "version"))
            .transpose()?
            .unwrap_or(0) as u64,
        digest: head.as_ref().map(|r| r.try_get("", "digest")).transpose()?,
        policy_jws: head
            .as_ref()
            .map(|r| r.try_get("", "policyJws"))
            .transpose()?,
        applied_version: applied
            .as_ref()
            .map(|r| r.try_get::<i64>("", "version"))
            .transpose()?
            .unwrap_or(0) as u64,
        applied_digest: applied
            .as_ref()
            .map(|r| r.try_get("", "digest"))
            .transpose()?,
    })
}

async fn require_owner(
    state: &AppState,
    user: &AppUser,
    id: &str,
) -> Result<repository::Device, ApiError> {
    let owner = human_owner(state, user).await?;
    let (device, _) = admitted_device(state, &owner, id).await?;
    if device.status.owner_id != owner {
        return Err(ApiError::FORBIDDEN);
    }
    Ok(device)
}

async fn policy(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(id): Path<String>,
) -> Result<Json<PolicyView>, ApiError> {
    require_owner(&state, &user, &id).await?;
    Ok(Json(policy_view(&state, &id).await?))
}

async fn put_policy(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(id): Path<String>,
    Json(request): Json<PolicyWrite>,
) -> Result<Json<PolicyView>, ApiError> {
    let device = require_owner(&state, &user, &id).await?;
    let enrollment = repository(&context(&state))
        .enrollment(&device.receipt.enrollment_id)
        .await?;
    let owner_key = enrollment.manifest.owner_invitation_key;
    let policy = verify_management_policy(
        &request.policy_jws,
        &owner_key,
        chrono::Utc::now().timestamp(),
    )
    .map_err(invalid)?;
    if policy.device_id != id || policy.policy_version > 10_000 {
        return Err(ApiError::bad_request(
            "Invalid management policy identity or exhausted history",
        ));
    }
    persist_policy(
        &state.db,
        state.db_dialect,
        id.clone(),
        device.status.auth_epoch,
        device.status.owner_id,
        owner_key,
        request.policy_jws,
    )
    .await?;
    Ok(Json(policy_view(&state, &id).await?))
}

pub(super) async fn persist_policy(
    db: &sea_orm::DatabaseConnection,
    dialect: crate::db::DbDialect,
    device_id: String,
    epoch: u64,
    owner_id: String,
    owner_key: Ed25519PublicKey,
    compact: String,
) -> Result<(), ApiError> {
    retry_transaction(db,dialect,None,&RetryPolicy::default(),move |tx|{
        let id=device_id.clone(); let compact=compact.clone(); let owner_key=owner_key.clone(); let owner_id=owner_id.clone();
        Box::pin(async move{
            let device=repository::lock_active_device(tx,&id,epoch).await?;
            if device.status.owner_id!=owner_id{return Err(ApiError::FORBIDDEN);}
            let policy=verify_management_policy(&compact,&owner_key,chrono::Utc::now().timestamp()).map_err(invalid)?;
            let digest=compact_digest(&compact);
            let previous=tx.query_one_raw(sql(r#"SELECT version,digest FROM "DeviceManagementPolicy" WHERE "deviceId"=$1 ORDER BY version DESC LIMIT 1"#,[id.clone().into()])).await?;
            let mut retained=std::collections::HashSet::new();
            if let Some(previous)=previous {
                let version=previous.try_get::<i64>("","version")? as u64; let old=previous.try_get::<String>("","digest")?;
                if policy.policy_version==version && digest==old{return Ok(());}
                if policy.policy_version!=version+1 || policy.previous_policy_digest.as_ref()!=Some(&old){return Err(ApiError::conflict("Management policy changed; reload before signing"));}
                for row in tx.query_all_raw(sql(r#"SELECT "grantId","userId" FROM "DeviceManagementRecipient" WHERE "deviceId"=$1 AND version=$2"#,[id.clone().into(),(version as i64).into()])).await? {
                    retained.insert((row.try_get::<String>("","grantId")?,row.try_get::<String>("","userId")?));
                }
            }else if policy.policy_version!=1{return Err(ApiError::conflict("Management policy genesis required"));}
            tx.execute_raw(sql(r#"INSERT INTO "DeviceManagementPolicy"("deviceId",version,digest,"policyJws","expiresAt","createdAt") VALUES($1,$2,$3,$4,$5,$6)"#,[id.clone().into(),(policy.policy_version as i64).into(),digest.into(),compact.into(),policy.expires_at.into(),policy.issued_at.into()])).await?;
            for grant in policy.grants {
                // Carried-over grants never block a revocation when their account is
                // suspended; admission rechecks the account on every use.
                if !retained.contains(&(grant.grant_id.clone(),grant.user_id.clone())) {
                    require_grantee_account(tx,&grant).await?;
                }
                tx.execute_raw(sql(r#"INSERT INTO "DeviceManagementRecipient"("deviceId",version,"grantId","userId","expiresAt") VALUES($1,$2,$3,$4,$5)"#,[id.clone().into(),(policy.policy_version as i64).into(),grant.grant_id.into(),grant.user_id.into(),grant.expires_at.into()])).await?;
            }
            Ok(())
        })
    }).await
}

async fn require_grantee_account<C: ConnectionTrait>(
    db: &C,
    grant: &ManagementGrant,
) -> Result<(), ApiError> {
    repository::active_account(db, &grant.user_id)
        .await
        .map_err(|error| {
            if error.status() == axum::http::StatusCode::FORBIDDEN {
                ApiError::forbidden(format!(
                    "Grant {} names account {}, which is not an active account",
                    grant.grant_id, grant.user_id
                ))
            } else {
                error
            }
        })
}

/// How the caller relates to a device's access rules.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
#[schema(as = DeviceAccessRole)]
pub(crate) enum AccessRole {
    /// The caller registered the device and needs no grant.
    Owner,
    /// The owner's current access rules name the caller.
    Grantee,
}

/// One permission the owner currently gives the caller.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, ToSchema)]
#[schema(as = DeviceAccessGrant)]
pub(crate) struct AccessGrant {
    pub grant_id: String,
    /// What the grant covers: the whole device (`{"kind":"device"}`), one project
    /// (`kind`, `project_id`) or one service (`kind`, `project_id`, `placement_id`).
    #[schema(value_type = Object)]
    pub scope: ManagementScope,
    /// What the caller may do there, for example `status`, `logs` or `deploy`.
    #[schema(value_type = Vec<String>)]
    pub capabilities: Vec<ManagementCapability>,
    /// When the grant ends (Unix seconds).
    pub expires_at: i64,
    /// Identifies the key on the caller's computer that the grant was issued to.
    pub controller_key_thumbprint: String,
    /// The group the grant came from, when the owner shared with a group.
    pub group_id: Option<String>,
}

/// The caller's own access to a device. Other people's grants and the signed access
/// rules themselves are never part of it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, ToSchema)]
#[schema(as = DeviceMyAccess)]
pub(crate) struct MyAccess {
    pub device_id: String,
    pub role: AccessRole,
    pub owner_id: String,
    /// Version of the owner's current access rules; 0 while none were saved.
    pub policy_version: u64,
    /// When those rules expire (Unix seconds).
    pub policy_expires_at: Option<i64>,
    /// The newest version the device confirmed.
    pub applied_version: u64,
    /// Whether the device runs the current rules.
    pub applied: bool,
    /// The caller's current grants. The owner has none and needs none.
    pub grants: Vec<AccessGrant>,
}

/// The latest access rules of device `$1`, the version the device confirmed, and the
/// setup manifest of its enrollment `$2`, which names the key that signs the rules.
const LATEST_RULES: &str = r#"SELECT p.version,p."policyJws",p."expiresAt",(SELECT a.version FROM "DeviceManagementApplied" a WHERE a."deviceId"=$1) AS "appliedVersion",(SELECT e.manifest FROM "DeviceEnrollment" e WHERE e.id=$2) AS manifest FROM "DeviceManagementPolicy" p WHERE p."deviceId"=$1 ORDER BY p.version DESC LIMIT 1"#;

#[derive(Deserialize)]
struct RulesSigner {
    owner_invitation_key: Ed25519PublicKey,
}

/// What the owner's signed rules give `user_id` right now. Rules that lapsed while the
/// request ran give nothing, and rules the hub cannot verify are never passed on.
/// `manifest` is the device's setup manifest, which names the key that signs its rules.
fn current_grants(
    policy_jws: &str,
    manifest: &str,
    device_id: &str,
    user_id: &str,
    now: i64,
) -> Result<Vec<AccessGrant>, ApiError> {
    let unverified = |reason: String| {
        ApiError::internal(format!(
            "Access rules of device {device_id} could not be verified: {reason}"
        ))
    };
    let signer: RulesSigner =
        serde_json::from_str(manifest).map_err(|error| unverified(error.to_string()))?;
    let policy = match verify_management_policy(policy_jws, &signer.owner_invitation_key, now) {
        Ok(policy) => policy,
        Err(ProtocolError::InvalidTime) => return Ok(Vec::new()),
        Err(error) => return Err(unverified(error.to_string())),
    };
    policy
        .grants
        .into_iter()
        .filter(|grant| grant.user_id == user_id && grant.expires_at > now)
        .map(|grant| {
            Ok(AccessGrant {
                controller_key_thumbprint: grant
                    .controller_key
                    .thumbprint()
                    .map_err(|error| unverified(error.to_string()))?,
                grant_id: grant.grant_id,
                scope: grant.scope,
                capabilities: grant.capabilities,
                expires_at: grant.expires_at,
                group_id: grant.group_id,
            })
        })
        .collect()
}

/// A missing device, an unrelated caller and ended access get one answer, so asking
/// never confirms that a device exists.
pub(super) fn no_access(error: ApiError) -> ApiError {
    match error.status() {
        StatusCode::NOT_FOUND | StatusCode::FORBIDDEN => ApiError::FORBIDDEN,
        _ => error,
    }
}

pub(crate) async fn own_access(
    state: &DeviceContext<'_>,
    user_id: &str,
    id: &str,
) -> Result<MyAccess, ApiError> {
    let (device, _) = admitted(state, user_id, id).await.map_err(no_access)?;
    let now = chrono::Utc::now().timestamp();
    let owns = device.status.owner_id == user_id;
    let rules = state
        .db
        .query_one_raw(sql(
            LATEST_RULES,
            [id.into(), device.receipt.enrollment_id.into()],
        ))
        .await?;
    let grants = match &rules {
        Some(rules) if !owns => current_grants(
            &rules.try_get::<String>("", "policyJws")?,
            &rules
                .try_get::<Option<String>>("", "manifest")?
                .unwrap_or_default(),
            id,
            user_id,
            now,
        )?,
        _ => Vec::new(),
    };
    if !owns && grants.is_empty() {
        return Err(ApiError::FORBIDDEN);
    }
    let version = |column: &str| -> Result<u64, ApiError> {
        let Some(rules) = &rules else { return Ok(0) };
        Ok(rules.try_get::<Option<i64>>("", column)?.unwrap_or(0) as u64)
    };
    let (policy_version, applied_version) = (version("version")?, version("appliedVersion")?);
    Ok(MyAccess {
        device_id: device.status.device_id,
        role: if owns {
            AccessRole::Owner
        } else {
            AccessRole::Grantee
        },
        owner_id: device.status.owner_id,
        policy_version,
        policy_expires_at: rules
            .as_ref()
            .map(|rules| rules.try_get("", "expiresAt"))
            .transpose()?,
        applied_version,
        applied: applied_version == policy_version,
        grants,
    })
}

#[utoipa::path(
    get,
    path = "/devices/{id}/management/my-access",
    tag = "devices",
    description = "Shows what you may do on a device: whether you own it, or which permissions its owner currently gives you.",
    params(("id" = String, Path, description = "Device ID")),
    responses(
        (status = 200, description = "Your role on the device and your current permissions", body = MyAccess),
        (status = 401, description = "Sign-in required"),
        (status = 403, description = "You have no current access to this device, or the account or access token cannot manage devices"),
        (status = 503, description = "Device enrollment is not enabled on this hub")
    ),
    security(("bearer_auth" = []), ("pat" = []))
)]
async fn my_access(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(id): Path<String>,
) -> Result<Json<MyAccess>, ApiError> {
    let context = context(&state);
    enabled(&context)?;
    let user = human_owner(&state, &user).await?;
    Ok(Json(own_access(&context, &user, &id).await?))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PoliciesRequest {
    client_assertion: String,
    after_version: u64,
}

async fn device_policies(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(request): Json<PoliciesRequest>,
) -> Result<Json<Vec<String>>, ApiError> {
    super::registered_assertion(
        &context(&state),
        &id,
        &request.client_assertion,
        &format!("/devices/{id}/management/policies"),
    )
    .await?;
    let after = i64::try_from(request.after_version).map_err(invalid)?;
    let rows=state.db.query_all_raw(sql(r#"SELECT "policyJws" FROM "DeviceManagementPolicy" WHERE "deviceId"=$1 AND version>$2 ORDER BY version LIMIT 32"#,[id.into(),after.into()])).await?;
    Ok(Json(
        rows.into_iter()
            .map(|r| r.try_get("", "policyJws"))
            .collect::<Result<_, _>>()?,
    ))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AppliedRequest {
    client_assertion: String,
    version: u64,
    digest: String,
}
async fn device_applied(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(request): Json<AppliedRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let device = super::registered_assertion(
        &context(&state),
        &id,
        &request.client_assertion,
        &format!("/devices/{id}/management/applied"),
    )
    .await?;
    let version = i64::try_from(request.version).map_err(invalid)?;
    let receipt = state
        .db
        .query_one_raw(sql(
            r#"SELECT digest FROM "DeviceManagementPolicy" WHERE "deviceId"=$1 AND version=$2"#,
            [id.clone().into(), version.into()],
        ))
        .await?
        .ok_or(ApiError::NOT_FOUND)?;
    if receipt.try_get::<String>("", "digest")? != request.digest {
        return Err(ApiError::bad_request("Policy digest mismatch"));
    }
    state.db.execute_raw(sql(r#"INSERT INTO "DeviceManagementApplied"("deviceId",version,digest,"appliedAt") VALUES($1,$2,$3,$4) ON CONFLICT("deviceId") DO UPDATE SET version=excluded.version,digest=excluded.digest,"appliedAt"=excluded."appliedAt" WHERE "DeviceManagementApplied".version<excluded.version"#,[device.status.device_id.into(),version.into(),request.digest.into(),chrono::Utc::now().timestamp().into()])).await?;
    Ok(Json(serde_json::json!({"applied":true})))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::DbDialect;
    use sea_orm::{ConnectOptions, Database};

    fn grant(
        id: &str,
        user: &str,
        key: &SigningKey,
        scope: ManagementScope,
        capabilities: &[ManagementCapability],
        expires_at: i64,
    ) -> ManagementGrant {
        ManagementGrant {
            grant_id: id.into(),
            user_id: user.into(),
            controller_key: key.public_key(),
            scope,
            capabilities: capabilities.to_vec(),
            expires_at,
            group_id: None,
            group_version: None,
        }
    }

    fn setup_manifest(owner: &SigningKey, now: i64) -> String {
        serde_json::to_string(&OnboardingManifest {
            version: PROTOCOL_VERSION,
            enrollment_id: "enrollment".into(),
            device_id: "device".into(),
            owner_id: "owner".into(),
            name: "Access fixture".into(),
            api_base_url: "https://api.example/api/v1".into(),
            bootstrap_key: SigningKey::generate().public_key(),
            controller_key: SigningKey::generate().public_key(),
            owner_invitation_key: owner.public_key(),
            issued_at: now,
            expires_at: now + 600,
        })
        .unwrap()
    }

    #[test]
    fn a_recipient_reads_only_their_own_current_grants() {
        let now = chrono::Utc::now().timestamp();
        let (owner, reader, other) = (
            SigningKey::generate(),
            SigningKey::generate(),
            SigningKey::generate(),
        );
        let mut from_group = grant(
            "reader-project",
            "reader",
            &reader,
            ManagementScope::Project {
                project_id: "invoice-ai".into(),
            },
            &[ManagementCapability::Deploy, ManagementCapability::Start],
            now + 200,
        );
        from_group.group_id = Some("operators".into());
        from_group.group_version = Some(3);
        let rules = sign_management_policy(
            &ManagementPolicy {
                version: 1,
                device_id: "device".into(),
                policy_version: 1,
                previous_policy_digest: None,
                grants: vec![
                    grant(
                        "reader-device",
                        "reader",
                        &reader,
                        ManagementScope::Device,
                        &[
                            ManagementCapability::Status,
                            ManagementCapability::UpdateAgent,
                        ],
                        now + 300,
                    ),
                    grant(
                        "reader-lapsed",
                        "reader",
                        &reader,
                        ManagementScope::Device,
                        &[ManagementCapability::Reboot],
                        now - 10,
                    ),
                    from_group,
                    grant(
                        "other-device",
                        "other",
                        &other,
                        ManagementScope::Device,
                        &[ManagementCapability::Logs],
                        now + 300,
                    ),
                ],
                issued_at: now - 100,
                expires_at: now + 600,
            },
            &owner,
        )
        .unwrap();
        let manifest = setup_manifest(&owner, now);
        let read = |user: &str, at: i64| current_grants(&rules, &manifest, "device", user, at);

        let thumbprint = reader.public_key().thumbprint().unwrap();
        assert_eq!(
            serde_json::to_value(read("reader", now).unwrap()).unwrap(),
            serde_json::json!([
                {
                    "grant_id": "reader-device",
                    "scope": {"kind": "device"},
                    "capabilities": ["status", "update_agent"],
                    "expires_at": now + 300,
                    "controller_key_thumbprint": thumbprint,
                    "group_id": null,
                },
                {
                    "grant_id": "reader-project",
                    "scope": {"kind": "project", "project_id": "invoice-ai"},
                    "capabilities": ["deploy", "start"],
                    "expires_at": now + 200,
                    "controller_key_thumbprint": thumbprint,
                    "group_id": "operators",
                },
            ]),
            "a grant names neither its account nor its key, and never another person's"
        );
        assert_eq!(read("other", now).unwrap().len(), 1);
        assert!(read("outsider", now).unwrap().is_empty());
        // The project grant has ended by then, and the rules themselves 350 seconds later.
        assert_eq!(read("reader", now + 250).unwrap().len(), 1);
        assert!(read("reader", now + 600).unwrap().is_empty());

        // Rules the owner's key did not sign are an error, never an answer.
        for unusable in [setup_manifest(&SigningKey::generate(), now), String::new()] {
            assert_eq!(
                current_grants(&rules, &unusable, "device", "reader", now)
                    .unwrap_err()
                    .status(),
                StatusCode::INTERNAL_SERVER_ERROR
            );
        }
    }

    #[test]
    fn a_missing_device_and_ended_access_are_refused_alike() {
        for refusal in [
            ApiError::NOT_FOUND,
            ApiError::FORBIDDEN,
            ApiError::forbidden("An active account is required"),
        ] {
            let answer = no_access(refusal);
            assert_eq!(answer.status(), StatusCode::FORBIDDEN);
            assert_eq!(answer.public_message(), None);
        }
        assert_eq!(
            no_access(ApiError::service_unavailable("disabled")).status(),
            StatusCode::SERVICE_UNAVAILABLE
        );
    }

    #[tokio::test]
    #[ignore = "requires FLOW_LIKE_DEVICE_TEST_DATABASE_URL pointing to a disposable PostgreSQL server"]
    async fn management_roster_revisions_are_atomic_and_revocation_fenced() {
        let url = std::env::var("FLOW_LIKE_DEVICE_TEST_DATABASE_URL").unwrap();
        let admin = Database::connect(&url).await.unwrap();
        let schema = format!("management_test_{}", uuid::Uuid::new_v4().simple());
        admin
            .execute_unprepared(&format!("CREATE SCHEMA {schema}"))
            .await
            .unwrap();
        let mut url = reqwest::Url::parse(&url).unwrap();
        url.query_pairs_mut()
            .append_pair("options", &format!("-c search_path={schema}"));
        let mut options = ConnectOptions::new(url.to_string());
        options.max_connections(8).min_connections(1);
        let db = Database::connect(options).await.unwrap();
        for migration in [
            include_str!("../../prisma/migrations/20260921120000_standalone_devices/migration.sql"),
            include_str!("../../prisma/migrations/20260922120000_device_management/migration.sql"),
            include_str!("../../prisma/migrations/20261001120000_device_console/migration.sql"),
        ] {
            for statement in migration.split(';') {
                if !statement.trim().is_empty() {
                    db.execute_unprepared(statement).await.unwrap();
                }
            }
        }
        db.execute_unprepared(r#"CREATE TABLE "User"(id TEXT PRIMARY KEY,status TEXT NOT NULL); INSERT INTO "User" VALUES('owner','ACTIVE'),('reader','ACTIVE')"#).await.unwrap();
        let now = chrono::Utc::now().timestamp();
        let owner = SigningKey::generate();
        let identity = DeviceIdentity {
            auth_key: SigningKey::generate().public_key(),
            management_key: [42; 32],
            telemetry_key: SigningKey::generate().public_key(),
        };
        let receipt = DeviceReceipt {
            enrollment_id: "enrollment".into(),
            device_id: "device".into(),
            owner_id: "owner".into(),
            name: "Policy fixture".into(),
            identity: identity.clone(),
            manifest_jws: "persistence-only".into(),
            binding_jws: "persistence-only".into(),
            registered_at: now,
            auth_epoch: 1,
        };
        db.execute_raw(sql(r#"INSERT INTO "ManagedDevice"(id,"ownerId",name,status,"authEpoch",identity,receipt,"registeredAt") VALUES('device','owner','Policy fixture','active',1,$1,$2,$3)"#,[serde_json::to_string(&identity).unwrap().into(),serde_json::to_string(&receipt).unwrap().into(),now.into()])).await.unwrap();
        let mut policy = ManagementPolicy {
            version: 1,
            device_id: "device".into(),
            policy_version: 1,
            previous_policy_digest: None,
            grants: vec![ManagementGrant {
                grant_id: "reader".into(),
                user_id: "reader".into(),
                controller_key: SigningKey::generate().public_key(),
                scope: ManagementScope::Device,
                capabilities: vec![ManagementCapability::Status],
                expires_at: now + 300,
                group_id: None,
                group_version: None,
            }],
            issued_at: now,
            expires_at: now + 600,
        };
        let first = sign_management_policy(&policy, &owner).unwrap();
        persist_policy(
            &db,
            DbDialect::Postgres,
            "device".into(),
            1,
            "owner".into(),
            owner.public_key(),
            first.clone(),
        )
        .await
        .unwrap();
        persist_policy(
            &db,
            DbDialect::Postgres,
            "device".into(),
            1,
            "owner".into(),
            owner.public_key(),
            first.clone(),
        )
        .await
        .unwrap();
        policy.policy_version = 2;
        policy.previous_policy_digest = Some(compact_digest(&first));
        let a = sign_management_policy(&policy, &owner).unwrap();
        policy.grants.clear();
        let b = sign_management_policy(&policy, &owner).unwrap();
        let (left, right) = tokio::join!(
            persist_policy(
                &db,
                DbDialect::Postgres,
                "device".into(),
                1,
                "owner".into(),
                owner.public_key(),
                a
            ),
            persist_policy(
                &db,
                DbDialect::Postgres,
                "device".into(),
                1,
                "owner".into(),
                owner.public_key(),
                b
            )
        );
        assert_eq!(
            [left.is_ok(), right.is_ok()]
                .into_iter()
                .filter(|v| *v)
                .count(),
            1
        );
        assert_eq!(
            persist_policy(
                &db,
                DbDialect::Postgres,
                "device".into(),
                1,
                "owner".into(),
                owner.public_key(),
                first
            )
            .await
            .unwrap_err()
            .status(),
            axum::http::StatusCode::CONFLICT
        );
        let head = db
            .query_one_raw(sql(
                r#"SELECT digest FROM "DeviceManagementPolicy" WHERE version=2"#,
                [],
            ))
            .await
            .unwrap()
            .unwrap()
            .try_get::<String>("", "digest")
            .unwrap();
        let connection = &db;
        let owner_key = owner.public_key();
        let persist = move |compact: String| {
            persist_policy(
                connection,
                DbDialect::Postgres,
                "device".into(),
                1,
                "owner".into(),
                owner_key.clone(),
                compact,
            )
        };
        let reader = ManagementGrant {
            grant_id: "reader".into(),
            user_id: "reader".into(),
            controller_key: SigningKey::generate().public_key(),
            scope: ManagementScope::Device,
            capabilities: vec![ManagementCapability::Status],
            expires_at: now + 300,
            group_id: None,
            group_version: None,
        };
        policy.policy_version = 3;
        policy.previous_policy_digest = Some(head);
        policy.grants = vec![reader.clone()];
        let shared = sign_management_policy(&policy, &owner).unwrap();
        persist(shared.clone()).await.unwrap();
        // A suspended grantee carried over from the head never blocks the owner's
        // next revision, but it cannot be named in a new grant.
        db.execute_unprepared(r#"UPDATE "User" SET status='SUSPENDED' WHERE id='reader'"#)
            .await
            .unwrap();
        policy.policy_version = 4;
        policy.previous_policy_digest = Some(compact_digest(&shared));
        policy.grants = vec![
            reader.clone(),
            ManagementGrant {
                grant_id: "fresh".into(),
                ..reader.clone()
            },
        ];
        let added = persist(sign_management_policy(&policy, &owner).unwrap())
            .await
            .unwrap_err();
        assert_eq!(added.status(), axum::http::StatusCode::FORBIDDEN);
        assert!(
            added
                .public_message()
                .is_some_and(|message| message.contains("fresh"))
        );
        policy.grants = vec![reader];
        let retained = sign_management_policy(&policy, &owner).unwrap();
        persist(retained.clone()).await.unwrap();
        policy.policy_version = 5;
        policy.previous_policy_digest = Some(compact_digest(&retained));
        policy.grants.clear();
        let revoked = sign_management_policy(&policy, &owner).unwrap();
        db.execute_unprepared(r#"UPDATE "ManagedDevice" SET status='revoked',"authEpoch"=2"#)
            .await
            .unwrap();
        assert_eq!(
            persist(revoked).await.unwrap_err().status(),
            axum::http::StatusCode::UNAUTHORIZED
        );
        db.close().await.unwrap();
        admin
            .execute_unprepared(&format!("DROP SCHEMA {schema} CASCADE"))
            .await
            .unwrap();
        admin.close().await.unwrap();
    }
}
