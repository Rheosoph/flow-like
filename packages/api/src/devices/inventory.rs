use super::{context, human_owner, management, repository};
use crate::{
    db::{RetryPolicy, retry_transaction},
    error::ApiError,
    middleware::jwt::AppUser,
    state::AppState,
};
use axum::{
    Extension, Json, Router,
    extract::{Path, Query, State},
    routing::get,
};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use flow_like_device_protocol::*;
use sea_orm::{ConnectionTrait, DatabaseBackend, DatabaseConnection, Statement, Value};
use std::result::Result;

fn sql(query: &str, values: impl IntoIterator<Item = Value>) -> Statement {
    Statement::from_sql_and_values(DatabaseBackend::Postgres, query, values)
}

pub(crate) fn routes() -> Router<AppState> {
    Router::new().route("/{id}/inventory/{key}", get(read).put(write))
}

fn status_scopes(
    owner: &str,
    owner_controller: &Ed25519PublicKey,
    user: &str,
    key: &Ed25519PublicKey,
    policy: Option<&ManagementPolicy>,
    now: i64,
    grant_id: Option<&str>,
) -> Vec<ManagementScope> {
    if owner == user && owner_controller == key && grant_id.is_none_or(|id| id == "owner") {
        return vec![ManagementScope::Device];
    }
    let Some(policy) = policy.filter(|p| p.issued_at <= now && p.expires_at > now) else {
        return vec![];
    };
    let mut scopes = Vec::new();
    for grant in &policy.grants {
        if grant_id.is_none_or(|id| id == grant.grant_id)
            && grant.user_id == user
            && &grant.controller_key == key
            && grant.expires_at > now
            && grant.capabilities.contains(&ManagementCapability::Status)
            && !scopes.contains(&grant.scope)
        {
            scopes.push(grant.scope.clone());
        }
    }
    scopes
}

#[derive(serde::Deserialize)]
struct InventoryQuery {
    grant_id: Option<String>,
}

async fn read(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((id, key)): Path<(String, String)>,
    Query(query): Query<InventoryQuery>,
) -> Result<Json<InventoryView>, ApiError> {
    let user = human_owner(&state, &user).await?;
    let (device, _) = management::admitted_device(&state, &user, &id).await?;
    let manifest = super::repository(&context(&state))
        .enrollment(&device.receipt.enrollment_id)
        .await?
        .manifest;
    Ok(Json(
        retain(
            &state.db,
            state.db_dialect,
            device.status,
            manifest,
            user,
            key,
            query.grant_id,
            None,
        )
        .await?,
    ))
}

async fn write(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((id, key)): Path<(String, String)>,
    Json(request): Json<EncryptedInventory>,
) -> Result<Json<InventoryView>, ApiError> {
    let user = human_owner(&state, &user).await?;
    let (device, _) = management::admitted_device(&state, &user, &id).await?;
    let manifest = super::repository(&context(&state))
        .enrollment(&device.receipt.enrollment_id)
        .await?
        .manifest;
    Ok(Json(
        retain(
            &state.db,
            state.db_dialect,
            device.status,
            manifest,
            user,
            key,
            None,
            Some(request),
        )
        .await?,
    ))
}

fn validate_payload(
    request: &EncryptedInventory,
    user: &str,
    device: &str,
    key: &str,
) -> Result<(), ApiError> {
    request
        .binding
        .validate()
        .map_err(|e| ApiError::bad_request(e.to_string()))?;
    if request.binding.account_id != user
        || request.binding.device_id != device
        || URL_SAFE_NO_PAD.encode(
            request
                .binding
                .controller_key
                .to_bytes()
                .map_err(|_| ApiError::bad_request("Invalid inventory controller"))?,
        ) != key
    {
        return Err(ApiError::FORBIDDEN);
    }
    if request.ciphertext.len() > (MAX_INVENTORY_PLAINTEXT + 40).div_ceil(3) * 4 {
        return Err(ApiError::bad_request(
            "Inventory exceeds its retention limit",
        ));
    }
    let bytes = URL_SAFE_NO_PAD
        .decode(&request.ciphertext)
        .map_err(|_| ApiError::bad_request("Invalid inventory ciphertext"))?;
    if !(40..=MAX_INVENTORY_PLAINTEXT + 40).contains(&bytes.len()) {
        return Err(ApiError::bad_request("Invalid inventory ciphertext"));
    }
    Ok(())
}

/// The device row also fences management-policy changes and device revocation.
#[allow(clippy::too_many_arguments)]
async fn retain(
    db: &DatabaseConnection,
    dialect: crate::db::DbDialect,
    device: DeviceStatus,
    manifest: OnboardingManifest,
    user: String,
    key_id: String,
    grant_id: Option<String>,
    request: Option<EncryptedInventory>,
) -> Result<InventoryView, ApiError> {
    let key_bytes: [u8; 32] = URL_SAFE_NO_PAD
        .decode(&key_id)
        .ok()
        .and_then(|bytes| bytes.try_into().ok())
        .ok_or_else(|| ApiError::bad_request("Invalid inventory controller"))?;
    let key = Ed25519PublicKey::from_bytes(key_bytes)
        .map_err(|_| ApiError::bad_request("Invalid inventory controller"))?;
    if URL_SAFE_NO_PAD.encode(
        key.to_bytes()
            .map_err(|_| ApiError::bad_request("Invalid inventory controller"))?,
    ) != key_id
    {
        return Err(ApiError::bad_request("Invalid inventory controller"));
    }
    if let Some(request) = &request {
        validate_payload(request, &user, &device.device_id, &key_id)?;
    }
    retry_transaction(db, dialect, None, &RetryPolicy::default(), move |tx| {
        let device = device.clone(); let manifest = manifest.clone(); let user = user.clone(); let key = key.clone(); let request = request.clone(); let key_id = key_id.clone(); let grant_id = grant_id.clone();
        Box::pin(async move {
            let current = repository::lock_active_device(tx, &device.device_id, device.auth_epoch).await?;
            repository::active_account(tx, &user).await?;
            let now = chrono::Utc::now().timestamp();
            let compact = tx.query_one_raw(sql(r#"SELECT "policyJws" FROM "DeviceManagementPolicy" WHERE "deviceId"=$1 ORDER BY version DESC LIMIT 1"#, [device.device_id.clone().into()])).await?;
            // An expired policy grants no access; it must not hide the owner's own inventory.
            let policy = compact.map(|row| row.try_get::<String>("", "policyJws")).transpose()?.and_then(|compact| verify_management_policy(&compact, &manifest.owner_invitation_key, now).ok());
            let scopes = status_scopes(&current.status.owner_id, &manifest.controller_key, &user, &key, policy.as_ref(), now, grant_id.as_deref());
            if scopes.is_empty() { return Err(ApiError::FORBIDDEN); }
            if let Some(request) = request {
                if !scopes.iter().any(|scope| inventory_scope_contains(scope, &request.binding.scope)) { return Err(ApiError::FORBIDDEN); }
                let scope_key = inventory_scope_key(&request.binding.scope).map_err(|e| ApiError::bad_request(e.to_string()))?;
                let old = tx.query_one_raw(sql(r#"SELECT revision,payload FROM "DeviceInventoryObservation" WHERE "userId"=$1 AND "deviceId"=$2 AND "keyId"=$3 AND "scopeKey"=$4"#, [user.clone().into(), device.device_id.clone().into(), key_id.clone().into(), scope_key.clone().into()])).await?;
                let encoded = serde_json::to_string(&request)?;
                if let Some(old) = old {
                    let revision = old.try_get::<i64>("", "revision")? as u64;
                    if request.binding.revision == revision && encoded == old.try_get::<String>("", "payload")? {
                        // An acknowledgement can be lost after the exact ciphertext was stored.
                    } else if request.binding.revision != revision + 1 {
                        return Err(ApiError::conflict("Inventory changed; reload before retaining this observation"));
                    } else {
                        tx.execute_raw(sql(r#"UPDATE "DeviceInventoryObservation" SET revision=$5,payload=$6,"updatedAt"=$7 WHERE "userId"=$1 AND "deviceId"=$2 AND "keyId"=$3 AND "scopeKey"=$4"#, [user.clone().into(), device.device_id.clone().into(), key_id.clone().into(), scope_key.into(), (request.binding.revision as i64).into(), encoded.into(), now.into()])).await?;
                    }
                } else {
                    if request.binding.revision != 1 { return Err(ApiError::conflict("Inventory genesis revision required")); }
                    let count = tx.query_one_raw(sql(r#"SELECT COUNT(*) AS count FROM "DeviceInventoryObservation" WHERE "userId"=$1 AND "deviceId"=$2"#, [user.clone().into(), device.device_id.clone().into()])).await?.ok_or_else(|| ApiError::internal("Inventory count unavailable"))?.try_get::<i64>("", "count")?;
                    if count >= 128 { return Err(ApiError::bad_request("This device has reached its retained inventory scope limit")); }
                    tx.execute_raw(sql(r#"INSERT INTO "DeviceInventoryObservation"("userId","deviceId","keyId","scopeKey",revision,payload,"updatedAt") VALUES($1,$2,$3,$4,$5,$6,$7)"#, [user.clone().into(), device.device_id.clone().into(), key_id.clone().into(), scope_key.into(), 1i64.into(), encoded.into(), now.into()])).await?;
                }
            }
            let rows = tx.query_all_raw(sql(r#"SELECT payload FROM "DeviceInventoryObservation" WHERE "userId"=$1 AND "deviceId"=$2 AND "keyId"=$3 ORDER BY "scopeKey" LIMIT 128"#, [user.into(), device.device_id.into(), key_id.into()])).await?;
            let mut observations = Vec::new();
            for row in rows {
                let encrypted: EncryptedInventory = serde_json::from_str(&row.try_get::<String>("", "payload")?)?;
                if scopes.iter().any(|scope| inventory_scope_contains(scope, &encrypted.binding.scope)) { observations.push(encrypted); }
            }
            Ok(InventoryView { scopes, observations })
        })
    }).await
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn status_scope_is_bound_to_current_user_key_expiry_and_capability() {
        let owner_key = SigningKey::from_bytes(&[1; 32]).public_key();
        let key = SigningKey::from_bytes(&[2; 32]).public_key();
        let mut policy = ManagementPolicy {
            version: 1,
            device_id: "device".into(),
            policy_version: 1,
            previous_policy_digest: None,
            issued_at: 1,
            expires_at: 100,
            grants: vec![ManagementGrant {
                grant_id: "grant".into(),
                user_id: "reader".into(),
                controller_key: key.clone(),
                scope: ManagementScope::Project {
                    project_id: "project".into(),
                },
                capabilities: vec![ManagementCapability::Status],
                expires_at: 90,
                group_id: None,
                group_version: None,
            }],
        };
        assert_eq!(
            status_scopes("owner", &owner_key, "reader", &key, Some(&policy), 10, None).len(),
            1
        );
        assert!(
            status_scopes("owner", &owner_key, "other", &key, Some(&policy), 10, None).is_empty()
        );
        assert!(
            status_scopes(
                "owner",
                &owner_key,
                "reader",
                &owner_key,
                Some(&policy),
                10,
                None
            )
            .is_empty()
        );
        assert!(
            status_scopes("owner", &owner_key, "reader", &key, Some(&policy), 90, None).is_empty()
        );
        assert!(
            status_scopes(
                "owner",
                &owner_key,
                "reader",
                &key,
                Some(&policy),
                10,
                Some("unrelated-grant")
            )
            .is_empty()
        );
        policy.grants[0].capabilities = vec![ManagementCapability::Deploy];
        assert!(
            status_scopes("owner", &owner_key, "reader", &key, Some(&policy), 10, None).is_empty()
        );
        policy.grants.clear();
        assert!(
            status_scopes("owner", &owner_key, "reader", &key, Some(&policy), 10, None).is_empty()
        );
        assert_eq!(
            status_scopes("owner", &owner_key, "owner", &owner_key, None, 10, None),
            vec![ManagementScope::Device]
        );
    }
    #[flow_like_types::tokio::test]
    #[ignore = "requires FLOW_LIKE_DEVICE_TEST_DATABASE_URL pointing to a disposable PostgreSQL server"]
    async fn retained_inventory_cas_reload_scope_revocation_and_device_revocation() {
        use sea_orm::{ConnectOptions, Database};
        let url = std::env::var("FLOW_LIKE_DEVICE_TEST_DATABASE_URL").unwrap();
        let admin = Database::connect(&url).await.unwrap();
        let schema = format!("inventory_test_{}", uuid::Uuid::new_v4().simple());
        admin
            .execute_unprepared(&format!("CREATE SCHEMA {schema}"))
            .await
            .unwrap();
        let mut scoped = reqwest::Url::parse(&url).unwrap();
        scoped
            .query_pairs_mut()
            .append_pair("options", &format!("-c search_path={schema}"));
        let mut options = ConnectOptions::new(scoped.to_string());
        options.max_connections(6).min_connections(1);
        let db = Database::connect(options).await.unwrap();
        for migration in [
            include_str!("../../prisma/migrations/20260921120000_standalone_devices/migration.sql"),
            include_str!("../../prisma/migrations/20260922120000_device_management/migration.sql"),
            include_str!("../../prisma/migrations/20260924120000_device_inventory/migration.sql"),
        ] {
            for sql in migration.split(';').filter(|s| !s.trim().is_empty()) {
                db.execute_unprepared(sql).await.unwrap();
            }
        }
        db.execute_unprepared(r#"CREATE TABLE "User"(id TEXT PRIMARY KEY,status TEXT NOT NULL); INSERT INTO "User" VALUES('owner','ACTIVE'),('reader','ACTIVE')"#).await.unwrap();
        let now = chrono::Utc::now().timestamp();
        let owner = SigningKey::generate();
        let reader = SigningKey::generate();
        let identity = DeviceIdentity {
            auth_key: SigningKey::generate().public_key(),
            management_key: [42; 32],
            telemetry_key: SigningKey::generate().public_key(),
        };
        let receipt = DeviceReceipt {
            enrollment_id: "enrollment".into(),
            device_id: "device".into(),
            owner_id: "owner".into(),
            name: "Inventory".into(),
            identity: identity.clone(),
            manifest_jws: "fixture".into(),
            binding_jws: "fixture".into(),
            registered_at: now,
            auth_epoch: 1,
        };
        db.execute_raw(sql(r#"INSERT INTO "ManagedDevice"(id,"ownerId",name,status,"authEpoch",identity,receipt,"registeredAt") VALUES('device','owner','Inventory','active',1,$1,$2,$3)"#, [serde_json::to_string(&identity).unwrap().into(),serde_json::to_string(&receipt).unwrap().into(),now.into()])).await.unwrap();
        let device = repository::current_device(&db, "device")
            .await
            .unwrap()
            .status;
        let manifest = OnboardingManifest {
            version: 1,
            enrollment_id: "enrollment".into(),
            device_id: "device".into(),
            owner_id: "owner".into(),
            name: "Inventory".into(),
            api_base_url: "https://hub.example/api/v1".into(),
            bootstrap_key: SigningKey::generate().public_key(),
            controller_key: owner.public_key(),
            owner_invitation_key: owner.public_key(),
            issued_at: now,
            expires_at: now + 600,
        };
        let mut policy = ManagementPolicy {
            version: 1,
            device_id: "device".into(),
            policy_version: 1,
            previous_policy_digest: None,
            issued_at: now,
            expires_at: now + 600,
            grants: vec![ManagementGrant {
                grant_id: "reader".into(),
                user_id: "reader".into(),
                controller_key: reader.public_key(),
                scope: ManagementScope::Project {
                    project_id: "project".into(),
                },
                capabilities: vec![ManagementCapability::Status],
                expires_at: now + 300,
                group_id: None,
                group_version: None,
            }],
        };
        let compact = sign_management_policy(&policy, &owner).unwrap();
        db.execute_raw(sql(r#"INSERT INTO "DeviceManagementPolicy"("deviceId",version,digest,"policyJws","expiresAt","createdAt") VALUES('device',1,$1,$2,$3,$4)"#, [compact_digest(&compact).into(),compact.clone().into(),policy.expires_at.into(),now.into()])).await.unwrap();
        let key = URL_SAFE_NO_PAD.encode(reader.public_key().to_bytes().unwrap());
        let first = EncryptedInventory {
            binding: InventoryBinding {
                issuer: "issuer".into(),
                api_origin: "https://hub.example".into(),
                account_id: "reader".into(),
                device_id: "device".into(),
                controller_key: reader.public_key(),
                scope: ManagementScope::Project {
                    project_id: "project".into(),
                },
                revision: 1,
            },
            ciphertext: URL_SAFE_NO_PAD.encode([3u8; 80]),
        };
        let call = |request| {
            retain(
                &db,
                crate::db::DbDialect::Postgres,
                device.clone(),
                manifest.clone(),
                "reader".into(),
                key.clone(),
                None,
                request,
            )
        };
        assert_eq!(
            call(Some(first.clone())).await.unwrap().observations,
            vec![first.clone()]
        );
        assert_eq!(call(None).await.unwrap().observations, vec![first.clone()]);
        assert_eq!(
            call(Some(first.clone())).await.unwrap().observations,
            vec![first.clone()]
        );
        let mut a = first.clone();
        a.binding.revision = 2;
        a.ciphertext = URL_SAFE_NO_PAD.encode([4u8; 80]);
        let mut b = a.clone();
        b.ciphertext = URL_SAFE_NO_PAD.encode([5u8; 80]);
        let (a, b) = flow_like_types::tokio::join!(call(Some(a)), call(Some(b)));
        assert_ne!(a.is_ok(), b.is_ok());
        assert_eq!(
            call(Some(first.clone())).await.err().unwrap().status(),
            axum::http::StatusCode::CONFLICT
        );
        let mut outside = first.clone();
        outside.binding.scope = ManagementScope::Device;
        assert_eq!(
            call(Some(outside)).await.err().unwrap().status(),
            axum::http::StatusCode::FORBIDDEN
        );
        policy.policy_version = 2;
        policy.previous_policy_digest = Some(compact_digest(&compact));
        policy.grants[0].scope = ManagementScope::Placement {
            project_id: "project".into(),
            placement_id: "placement".into(),
        };
        let compact = sign_management_policy(&policy, &owner).unwrap();
        db.execute_raw(sql(r#"INSERT INTO "DeviceManagementPolicy"("deviceId",version,digest,"policyJws","expiresAt","createdAt") VALUES('device',2,$1,$2,$3,$4)"#, [compact_digest(&compact).into(),compact.clone().into(),policy.expires_at.into(),now.into()])).await.unwrap();
        assert!(call(None).await.unwrap().observations.is_empty());
        policy.policy_version = 3;
        policy.previous_policy_digest = Some(compact_digest(&compact));
        policy.grants.clear();
        let compact = sign_management_policy(&policy, &owner).unwrap();
        db.execute_raw(sql(r#"INSERT INTO "DeviceManagementPolicy"("deviceId",version,digest,"policyJws","expiresAt","createdAt") VALUES('device',3,$1,$2,$3,$4)"#, [compact_digest(&compact).into(),compact.into(),policy.expires_at.into(),now.into()])).await.unwrap();
        assert_eq!(
            call(None).await.err().unwrap().status(),
            axum::http::StatusCode::FORBIDDEN
        );
        db.execute_unprepared(
            r#"UPDATE "ManagedDevice" SET status='revoked',"authEpoch"=2 WHERE id='device'"#,
        )
        .await
        .unwrap();
        assert_eq!(
            call(None).await.err().unwrap().status(),
            axum::http::StatusCode::UNAUTHORIZED
        );
        db.close().await.unwrap();
        admin
            .execute_unprepared(&format!("DROP SCHEMA {schema} CASCADE"))
            .await
            .unwrap();
    }
}
