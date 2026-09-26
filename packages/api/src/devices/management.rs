use super::{context, enabled, human_owner, repository};
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
    routing::{get, post},
};
use flow_like_device_protocol::*;
use sea_orm::{ConnectionTrait, DatabaseBackend, Statement, Value};
use serde::{Deserialize, Serialize};
use std::result::Result;

fn sql(query: &str, values: impl IntoIterator<Item = Value>) -> Statement {
    Statement::from_sql_and_values(DatabaseBackend::Postgres, query, values)
}
fn invalid(error: impl std::fmt::Display) -> ApiError {
    ApiError::bad_request(error.to_string())
}

pub(crate) async fn shared_devices(
    state: &AppState,
    user_id: &str,
) -> Result<Vec<DeviceStatus>, ApiError> {
    let now = chrono::Utc::now().timestamp();
    let rows=state.db.query_all_raw(sql(r#"SELECT d.* FROM "ManagedDevice" d WHERE d.status='active' AND d."ownerId"<>$1 AND EXISTS(SELECT 1 FROM "User" u WHERE u.id=d."ownerId" AND u.status='ACTIVE') AND EXISTS(SELECT 1 FROM "DeviceManagementRecipient" r WHERE r."deviceId"=d.id AND r."userId"=$1 AND r."expiresAt">$2 AND r.version=(SELECT MAX(p.version) FROM "DeviceManagementPolicy" p WHERE p."deviceId"=d.id)) ORDER BY d."registeredAt" DESC LIMIT 1000"#,[user_id.into(),now.into()])).await?;
    rows.into_iter()
        .map(|row| repository::device(row).map(|device| device.status))
        .collect()
}

pub(crate) fn routes() -> Router<AppState> {
    Router::new()
        .route("/{id}/signaling/device", post(signal_device))
        .route("/{id}/signaling/controller", post(signal_controller))
        .route("/{id}/management/policy", get(policy).put(put_policy))
        .route("/{id}/management/policies", post(device_policies))
        .route("/{id}/management/applied", post(device_applied))
        .route("/{id}/identity", get(identity))
        .route("/controller-vaults/{id}", get(super::recovery::get).put(super::recovery::put))
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

async fn signaling_response(
    state: &AppState,
    device: &DeviceStatus,
    subject: &str,
    role: &str,
    participant: &str,
    expires_at: i64,
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
    let expires_at = expires_at.min(now + 300);
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
    let ice = match state.realtime_ice.issue(&claims.jti).await {
        Ok(ice) => ice,
        Err(_) => {
            tracing::warn!(
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
        signaling_response(
            &state,
            &device.status,
            &id,
            "device",
            &id,
            chrono::Utc::now().timestamp() + 300,
        )
        .await?,
    ))
}

/// This grants discovery and rendezvous only. The agent separately verifies controller keys.
pub(crate) async fn admitted_device(
    state: &AppState,
    user_id: &str,
    id: &str,
) -> Result<(repository::Device, i64), ApiError> {
    enabled(&context(state))?;
    repository::active_account(&state.db, user_id).await?;
    let device = repository(&context(state)).device(id).await?;
    repository::active_account(&state.db, &device.status.owner_id).await?;
    if device.status.status != DeviceRegistrationStatus::Active {
        return Err(ApiError::FORBIDDEN);
    }
    let now = chrono::Utc::now().timestamp();
    if device.status.owner_id == user_id {
        return Ok((device, now + 300));
    }
    let row=state.db.query_one_raw(sql(r#"SELECT MAX(r."expiresAt") AS expiry FROM "DeviceManagementRecipient" r WHERE r."deviceId"=$1 AND r."userId"=$2 AND r."expiresAt">$3 AND r.version=(SELECT MAX(version) FROM "DeviceManagementPolicy" WHERE "deviceId"=$1)"#,[id.into(),user_id.into(),now.into()])).await?.ok_or(ApiError::FORBIDDEN)?;
    let expiry = row
        .try_get::<Option<i64>>("", "expiry")?
        .ok_or(ApiError::FORBIDDEN)?;
    Ok((device, expiry.min(now + 300)))
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

async fn persist_policy(
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
            if let Some(previous)=previous {
                let version=previous.try_get::<i64>("","version")? as u64; let old=previous.try_get::<String>("","digest")?;
                if policy.policy_version==version && digest==old{return Ok(());}
                if policy.policy_version!=version+1 || policy.previous_policy_digest.as_ref()!=Some(&old){return Err(ApiError::conflict("Management policy changed; reload before signing"));}
            }else if policy.policy_version!=1{return Err(ApiError::conflict("Management policy genesis required"));}
            tx.execute_raw(sql(r#"INSERT INTO "DeviceManagementPolicy"("deviceId",version,digest,"policyJws","expiresAt","createdAt") VALUES($1,$2,$3,$4,$5,$6)"#,[id.clone().into(),(policy.policy_version as i64).into(),digest.into(),compact.into(),policy.expires_at.into(),policy.issued_at.into()])).await?;
            for grant in policy.grants {
                repository::active_account(tx,&grant.user_id).await?;
                tx.execute_raw(sql(r#"INSERT INTO "DeviceManagementRecipient"("deviceId",version,"grantId","userId","expiresAt") VALUES($1,$2,$3,$4,$5)"#,[id.clone().into(),(policy.policy_version as i64).into(),grant.grant_id.into(),grant.user_id.into(),grant.expires_at.into()])).await?;
            }
            Ok(())
        })
    }).await
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
        policy.policy_version = 3;
        policy.previous_policy_digest = Some(head);
        let revoked = sign_management_policy(&policy, &owner).unwrap();
        db.execute_unprepared(r#"UPDATE "ManagedDevice" SET status='revoked',"authEpoch"=2"#)
            .await
            .unwrap();
        assert_eq!(
            persist_policy(
                &db,
                DbDialect::Postgres,
                "device".into(),
                1,
                "owner".into(),
                owner.public_key(),
                revoked
            )
            .await
            .unwrap_err()
            .status(),
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
