use super::{DeviceContext, context, enabled, human_owner, repository::count};
use crate::{
    db::{DbDialect, RetryPolicy, retry_transaction},
    error::ApiError,
    middleware::jwt::AppUser,
    state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, State},
};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use flow_like_device_protocol::*;
use sea_orm::{
    ConnectionTrait, DatabaseBackend, DatabaseConnection, QueryResult, Statement, Value,
};
use serde::{Deserialize, Serialize};
use std::result::Result;
use utoipa::ToSchema;

pub(crate) const MAX_ACCOUNT_BACKUPS: u64 = 256;

/// A backup keeps its slot while its device is active or its setup package can still be
/// started; any other backup can no longer unlock anything. `v` is the backup and `$2`
/// the current time.
pub(crate) const HOLDS_SLOT: &str = r#"(EXISTS(SELECT 1 FROM "ManagedDevice" d WHERE d.id=v."keyId" AND d.status='active') OR EXISTS(SELECT 1 FROM "DeviceEnrollment" e WHERE e."deviceId"=v."keyId" AND e.status='pending' AND e."expiresAt">$2))"#;

fn sql(query: &str, values: impl IntoIterator<Item = Value>) -> Statement {
    Statement::from_sql_and_values(DatabaseBackend::Postgres, query, values)
}
fn invalid(error: impl std::fmt::Display) -> ApiError {
    ApiError::bad_request(error.to_string())
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ControllerVault {
    public_key: Ed25519PublicKey,
    ciphertext: String,
    revision: u64,
}

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct VaultWrite {
    public_key: Ed25519PublicKey,
    ciphertext: String,
    revision: u64,
    proof_jws: String,
}

fn validate_write(
    request: &VaultWrite,
    owner: &str,
    id: &str,
    api_base: &str,
) -> Result<(), ApiError> {
    validate_management_id(id).map_err(invalid)?;
    request.public_key.validate().map_err(invalid)?;
    let bytes = URL_SAFE_NO_PAD
        .decode(&request.ciphertext)
        .map_err(invalid)?;
    if !(64..=65536).contains(&bytes.len())
        || !bytes.starts_with(b"FLVAULT1")
        || URL_SAFE_NO_PAD.encode(&bytes) != request.ciphertext
    {
        return Err(ApiError::bad_request("Invalid protected controller backup"));
    }
    let proof =
        verify_controller_recovery(&request.proof_jws, &request.public_key).map_err(invalid)?;
    if proof.api_base_url != api_base
        || proof.user_id != owner
        || proof.device_id != id
        || proof.revision != request.revision
        || proof.ciphertext_sha256 != recovery_ciphertext_digest(&bytes)
    {
        return Err(ApiError::bad_request(
            "Controller backup proof does not match this account, revision or ciphertext",
        ));
    }
    Ok(())
}

pub(crate) async fn get(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(id): Path<String>,
) -> Result<Json<ControllerVault>, ApiError> {
    enabled(&context(&state))?;
    let owner = human_owner(&state, &user).await?;
    validate_management_id(&id).map_err(invalid)?;
    let row = state.db.query_one_raw(sql(r#"SELECT v."publicKey",v.ciphertext,v.revision FROM "DeviceControllerVault" v JOIN "User" u ON u.id=v."userId" AND u.status='ACTIVE' WHERE v."userId"=$1 AND v."keyId"=$2"#, [owner.into(), id.into()])).await?.ok_or(ApiError::NOT_FOUND)?;
    Ok(Json(ControllerVault {
        public_key: serde_json::from_str(&row.try_get::<String>("", "publicKey")?)?,
        ciphertext: row.try_get("", "ciphertext")?,
        revision: row.try_get::<i64>("", "revision")? as u64,
    }))
}

/// An account backup as listed. The encrypted keys are only returned when one backup
/// is read by its ID.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, ToSchema)]
pub(crate) struct AccountBackupView {
    /// The device, or the device a setup package will register, whose keys are saved.
    pub key_id: String,
    pub revision: u64,
    /// When this revision was saved (Unix seconds).
    pub updated_at: i64,
    /// Identifies the key that saved the backup.
    pub public_key_thumbprint: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, ToSchema)]
pub(crate) struct AccountBackupList {
    /// Every stored backup, newest first.
    pub vaults: Vec<AccountBackupView>,
    /// Backups that count toward `max`: those of active devices and of setup packages
    /// that can still be started. The hub removes the others when a new one is saved.
    pub used: u64,
    pub max: u64,
}

fn listed(row: &QueryResult) -> Result<AccountBackupView, ApiError> {
    let key_id: String = row.try_get("", "keyId")?;
    let public_key_thumbprint =
        serde_json::from_str::<Ed25519PublicKey>(&row.try_get::<String>("", "publicKey")?)
            .map_err(|error| error.to_string())
            .and_then(|key| key.thumbprint().map_err(|error| error.to_string()))
            .map_err(|error| {
                ApiError::internal(format!(
                    "Account backup {key_id} has an unreadable controller key: {error}"
                ))
            })?;
    Ok(AccountBackupView {
        revision: row.try_get::<i64>("", "revision")? as u64,
        updated_at: row.try_get("", "updatedAt")?,
        public_key_thumbprint,
        key_id,
    })
}

/// The caller's account backups without their ciphertext.
pub(crate) async fn backups(
    state: &DeviceContext<'_>,
    user: &str,
) -> Result<AccountBackupList, ApiError> {
    enabled(state)?;
    let now = chrono::Utc::now().timestamp();
    let rows = state.db.query_all_raw(sql(&format!(r#"SELECT v."keyId",v.revision,v."updatedAt",v."publicKey",{HOLDS_SLOT} AS "holdsSlot" FROM "DeviceControllerVault" v JOIN "User" u ON u.id=v."userId" AND u.status='ACTIVE' WHERE v."userId"=$1 ORDER BY v."updatedAt" DESC,v."keyId" LIMIT {MAX_ACCOUNT_BACKUPS}"#), [user.into(), now.into()])).await?;
    let mut used = 0;
    for row in &rows {
        used += u64::from(row.try_get::<bool>("", "holdsSlot")?);
    }
    Ok(AccountBackupList {
        vaults: rows.iter().map(listed).collect::<Result<_, _>>()?,
        used,
        max: MAX_ACCOUNT_BACKUPS,
    })
}

#[utoipa::path(
    get,
    path = "/devices/controller-vaults",
    tag = "devices",
    description = "Lists the account backups of your device keys and when each was last saved. The encrypted keys themselves are not included.",
    responses(
        (status = 200, description = "Your account backups, newest first, and how many of the allowed backups are in use", body = AccountBackupList),
        (status = 401, description = "Sign-in required"),
        (status = 403, description = "The account or access token cannot manage devices"),
        (status = 503, description = "Device enrollment is not enabled on this hub")
    ),
    security(("bearer_auth" = []), ("pat" = []))
)]
pub(crate) async fn list(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
) -> Result<Json<AccountBackupList>, ApiError> {
    let context = context(&state);
    enabled(&context)?;
    let owner = human_owner(&state, &user).await?;
    Ok(Json(backups(&context, &owner).await?))
}

pub(crate) async fn put(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(id): Path<String>,
    Json(request): Json<VaultWrite>,
) -> Result<Json<serde_json::Value>, ApiError> {
    enabled(&context(&state))?;
    let owner = human_owner(&state, &user).await?;
    validate_write(
        &request,
        &owner,
        &id,
        &super::api_base_url(&context(&state))?,
    )?;
    let revision = request.revision;
    persist(&state.db, state.db_dialect, owner, id, request).await?;
    Ok(Json(serde_json::json!({"revision": revision})))
}

async fn persist(
    db: &DatabaseConnection,
    dialect: DbDialect,
    owner: String,
    id: String,
    request: VaultWrite,
) -> Result<(), ApiError> {
    retry_transaction(db, dialect, None, &RetryPolicy::default(), move |tx| {
        let owner = owner.clone(); let id = id.clone(); let request = request.clone();
        Box::pin(async move {
            // Account locking serializes its bounded vault inventory, including
            // concurrent first saves under different device IDs.
            if tx.execute_raw(sql(r#"UPDATE "User" SET status=status WHERE id=$1 AND status='ACTIVE'"#, [owner.clone().into()])).await?.rows_affected() != 1 { return Err(ApiError::FORBIDDEN); }
            let public_key = serde_json::to_string(&request.public_key)?;
            let existing = tx.query_one_raw(sql(r#"SELECT "publicKey",ciphertext,revision FROM "DeviceControllerVault" WHERE "userId"=$1 AND "keyId"=$2"#, [owner.clone().into(), id.clone().into()])).await?;
            if let Some(row) = &existing {
                let previous = row.try_get::<i64>("", "revision")?;
                if row.try_get::<String>("", "publicKey")? != public_key { return Err(ApiError::conflict("The account backup belongs to a different controller key")); }
                if previous == request.revision as i64 && row.try_get::<String>("", "ciphertext")? == request.ciphertext { return Ok(()); }
                if previous + 1 != request.revision as i64 { return Err(ApiError::conflict("The account backup changed. Restore or reconcile it before saving")); }
            } else if request.revision != 1 { return Err(ApiError::conflict("The account backup revision is missing")); }

            let enrollment = tx.query_one_raw(sql(r#"SELECT manifest,status,"expiresAt" FROM "DeviceEnrollment" WHERE "deviceId"=$1"#, [id.clone().into()])).await?.ok_or(ApiError::FORBIDDEN)?;
            let manifest: OnboardingManifest = serde_json::from_str(&enrollment.try_get::<String>("", "manifest")?)?;
            let now = chrono::Utc::now().timestamp();
            let device = tx.query_one_raw(sql(r#"SELECT "authEpoch" FROM "ManagedDevice" WHERE id=$1"#, [id.clone().into()])).await?;
            if let Some(row) = device {
                super::repository::lock_active_device(tx, &id, row.try_get::<i64>("", "authEpoch")? as u64).await?;
            } else {
                // Enrollment cancellation/redemption and policy/revocation must
                // conflict with a new backup write, including on optimistic DBs.
                if tx.execute_raw(sql(r#"UPDATE "DeviceEnrollment" SET status=status WHERE "deviceId"=$1 AND status='pending' AND "expiresAt">$2"#, [id.clone().into(), now.into()])).await?.rows_affected() != 1 { return Err(ApiError::FORBIDDEN); }
            }
            let owner_controller = manifest.owner_id == owner && manifest.controller_key == request.public_key;
            if !owner_controller {
                let row = tx.query_one_raw(sql(r#"SELECT "policyJws" FROM "DeviceManagementPolicy" WHERE "deviceId"=$1 ORDER BY version DESC LIMIT 1"#, [id.clone().into()])).await?.ok_or(ApiError::FORBIDDEN)?;
                let policy = verify_management_policy(&row.try_get::<String>("", "policyJws")?, &manifest.owner_invitation_key, now).map_err(|_| ApiError::FORBIDDEN)?;
                if !policy.grants.iter().any(|grant| grant.user_id == owner && grant.controller_key == request.public_key && grant.expires_at > now) { return Err(ApiError::FORBIDDEN); }
            }
            if existing.is_none() {
                // Backups of revoked devices and of packages that were cancelled or
                // expired unredeemed can no longer unlock anything, so they free their slot.
                tx.execute_raw(sql(&format!(r#"DELETE FROM "DeviceControllerVault" v WHERE v."userId"=$1 AND NOT {HOLDS_SLOT}"#), [owner.clone().into(), now.into()])).await?;
                let stored = tx.query_one_raw(sql(r#"SELECT COUNT(*) AS count FROM "DeviceControllerVault" WHERE "userId"=$1"#, [owner.clone().into()])).await?.ok_or(ApiError::FORBIDDEN)?;
                if count(&stored, "count")? >= MAX_ACCOUNT_BACKUPS { return Err(ApiError::too_many_requests(format!("Controller backup storage limit of {MAX_ACCOUNT_BACKUPS} active devices and pending packages reached"))); }
            }
            tx.execute_raw(sql(r#"INSERT INTO "DeviceControllerVault"("userId","keyId","publicKey",ciphertext,revision,"updatedAt") VALUES($1,$2,$3,$4,$5,$6) ON CONFLICT("userId","keyId") DO UPDATE SET ciphertext=excluded.ciphertext,revision=excluded.revision,"updatedAt"=excluded."updatedAt""#, [owner.into(), id.into(), public_key.into(), request.ciphertext.into(), (request.revision as i64).into(), now.into()])).await?;
            Ok(())
        })
    }).await
}

#[cfg(test)]
mod tests {
    use super::*;
    fn request() -> VaultWrite {
        let key = SigningKey::generate();
        let ciphertext = [b"FLVAULT1".as_slice(), &[0; 64]].concat();
        let proof = ControllerRecoveryProof {
            version: 1,
            api_base_url: "https://api.example/api/v1".into(),
            user_id: "owner".into(),
            device_id: "device".into(),
            revision: 1,
            ciphertext_sha256: recovery_ciphertext_digest(&ciphertext),
        };
        VaultWrite {
            public_key: key.public_key(),
            ciphertext: URL_SAFE_NO_PAD.encode(ciphertext),
            revision: 1,
            proof_jws: sign_controller_recovery(&proof, &key).unwrap(),
        }
    }
    #[test]
    fn a_listed_backup_never_carries_the_encrypted_keys() {
        let list = AccountBackupList {
            vaults: vec![AccountBackupView {
                key_id: "device".into(),
                revision: 3,
                updated_at: 1_727_700_000,
                public_key_thumbprint: "thumbprint".into(),
            }],
            used: 1,
            max: MAX_ACCOUNT_BACKUPS,
        };
        assert_eq!(
            serde_json::to_value(list).unwrap(),
            serde_json::json!({
                "vaults": [{
                    "key_id": "device",
                    "revision": 3,
                    "updated_at": 1_727_700_000,
                    "public_key_thumbprint": "thumbprint",
                }],
                "used": 1,
                "max": 256,
            })
        );
    }

    #[test]
    fn account_session_alone_cannot_replace_controller_backups() {
        let mut value = request();
        assert!(validate_write(&value, "owner", "device", "https://api.example/api/v1").is_ok());
        assert!(validate_write(&value, "other", "device", "https://api.example/api/v1").is_err());
        assert!(validate_write(&value, "owner", "device", "https://other.example/api/v1").is_err());
        value.revision = 2;
        assert!(validate_write(&value, "owner", "device", "https://api.example/api/v1").is_err());
        value.revision = 1;
        value.ciphertext = URL_SAFE_NO_PAD.encode([b"FLVAULT1".as_slice(), &[1; 64]].concat());
        assert!(validate_write(&value, "owner", "device", "https://api.example/api/v1").is_err());
    }
    fn signed_write(
        key: &SigningKey,
        user: &str,
        device: &str,
        revision: u64,
        byte: u8,
    ) -> VaultWrite {
        let ciphertext = [b"FLVAULT1".as_slice(), &[byte; 64]].concat();
        let proof = ControllerRecoveryProof {
            version: 1,
            api_base_url: "https://api.example/api/v1".into(),
            user_id: user.into(),
            device_id: device.into(),
            revision,
            ciphertext_sha256: recovery_ciphertext_digest(&ciphertext),
        };
        VaultWrite {
            public_key: key.public_key(),
            ciphertext: URL_SAFE_NO_PAD.encode(ciphertext),
            revision,
            proof_jws: sign_controller_recovery(&proof, key).unwrap(),
        }
    }

    async fn write_checked(
        db: &DatabaseConnection,
        user: &str,
        device: &str,
        request: VaultWrite,
    ) -> Result<(), ApiError> {
        validate_write(&request, user, device, "https://api.example/api/v1")?;
        persist(db, DbDialect::Postgres, user.into(), device.into(), request).await
    }

    #[flow_like_types::tokio::test]
    #[ignore = "requires FLOW_LIKE_DEVICE_TEST_DATABASE_URL pointing to a disposable PostgreSQL server"]
    async fn account_recovery_cas_identity_scope_and_revocation() {
        use axum::http::StatusCode;
        use sea_orm::{ConnectOptions, Database, TransactionTrait};
        let url = std::env::var("FLOW_LIKE_DEVICE_TEST_DATABASE_URL").unwrap();
        let admin = Database::connect(&url).await.unwrap();
        let schema = format!("account_recovery_test_{}", uuid::Uuid::new_v4().simple());
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
            include_str!("../../prisma/migrations/20261001120000_device_console/migration.sql"),
        ] {
            for sql in migration.split(';').filter(|s| !s.trim().is_empty()) {
                db.execute_unprepared(sql).await.unwrap();
            }
        }
        db.execute_unprepared(r#"CREATE TABLE "User"(id TEXT PRIMARY KEY,status TEXT NOT NULL); INSERT INTO "User" VALUES('owner','ACTIVE'),('reader','ACTIVE'),('outsider','ACTIVE')"#).await.unwrap();
        let now = chrono::Utc::now().timestamp();
        let owner = SigningKey::generate();
        let invitation = SigningKey::generate();
        let reader = SigningKey::generate();
        let outsider = SigningKey::generate();
        crate::backend_jwt::init_for_tests();
        let hub = flow_like::hub::StandaloneConfig {
            enabled: true,
            ..Default::default()
        };
        let context = DeviceContext {
            db: &db,
            dialect: DbDialect::Postgres,
            config: &hub,
            domain: "unused.example",
            secure: true,
        };
        // An account's listed backups as (device, revision, saving key) and its slots in use.
        let listing = |user: &'static str| {
            let context = &context;
            async move {
                let list = backups(context, user).await.unwrap();
                let vaults = list
                    .vaults
                    .into_iter()
                    .map(|vault| (vault.key_id, vault.revision, vault.public_key_thumbprint))
                    .collect::<Vec<_>>();
                (vaults, list.used)
            }
        };
        let listed = |device: &str, revision: u64, key: &SigningKey| {
            (
                device.to_owned(),
                revision,
                key.public_key().thumbprint().unwrap(),
            )
        };
        let manifest = OnboardingManifest {
            version: 1,
            enrollment_id: "recovery-enrollment".into(),
            device_id: "recovery-device".into(),
            owner_id: "owner".into(),
            name: "Recovery test".into(),
            api_base_url: "https://api.example/api/v1".into(),
            bootstrap_key: SigningKey::generate().public_key(),
            controller_key: owner.public_key(),
            owner_invitation_key: invitation.public_key(),
            issued_at: now,
            expires_at: now + 600,
        };
        db.execute_raw(sql(r#"INSERT INTO "DeviceEnrollment"(id,"deviceId","ownerId","jwtId",manifest,status,"expiresAt","createdAt") VALUES($1,$2,'owner','jwt',$3,'pending',$4,$5)"#, [manifest.enrollment_id.clone().into(),manifest.device_id.clone().into(),serde_json::to_string(&manifest).unwrap().into(),manifest.expires_at.into(),now.into()])).await.unwrap();
        let first = signed_write(&owner, "owner", &manifest.device_id, 1, 1);
        write_checked(&db, "owner", &manifest.device_id, first.clone())
            .await
            .unwrap();
        write_checked(&db, "owner", &manifest.device_id, first.clone())
            .await
            .unwrap();
        assert_eq!(
            listing("owner").await,
            (vec![listed("recovery-device", 1, &owner)], 1)
        );
        assert_eq!(listing("reader").await, (Vec::new(), 0));
        assert_eq!(
            write_checked(&db, "reader", &manifest.device_id, first.clone())
                .await
                .err()
                .unwrap()
                .status(),
            StatusCode::BAD_REQUEST
        );
        assert_eq!(
            write_checked(&db, "owner", "other-device", first.clone())
                .await
                .err()
                .unwrap()
                .status(),
            StatusCode::BAD_REQUEST
        );
        assert_eq!(
            write_checked(
                &db,
                "owner",
                &manifest.device_id,
                signed_write(&outsider, "owner", &manifest.device_id, 2, 2)
            )
            .await
            .err()
            .unwrap()
            .status(),
            StatusCode::CONFLICT
        );
        db.execute_raw(sql(
            r#"UPDATE "DeviceEnrollment" SET "expiresAt"=$1 WHERE id=$2"#,
            [(now - 1).into(), manifest.enrollment_id.clone().into()],
        ))
        .await
        .unwrap();
        assert_eq!(
            write_checked(
                &db,
                "owner",
                &manifest.device_id,
                signed_write(&owner, "owner", &manifest.device_id, 2, 2)
            )
            .await
            .err()
            .unwrap()
            .status(),
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            listing("owner").await,
            (vec![listed("recovery-device", 1, &owner)], 0),
            "the backup of a lapsed package stays stored and holds no slot"
        );
        let identity = DeviceIdentity {
            auth_key: SigningKey::generate().public_key(),
            management_key: [42; 32],
            telemetry_key: SigningKey::generate().public_key(),
        };
        let receipt = DeviceReceipt {
            enrollment_id: manifest.enrollment_id.clone(),
            device_id: manifest.device_id.clone(),
            owner_id: "owner".into(),
            name: "Recovery test".into(),
            identity: identity.clone(),
            manifest_jws: "fixture".into(),
            binding_jws: "fixture".into(),
            registered_at: now,
            auth_epoch: 1,
        };
        db.execute_raw(sql(r#"INSERT INTO "ManagedDevice"(id,"ownerId",name,status,"authEpoch",identity,receipt,"registeredAt") VALUES($1,'owner','Recovery test','active',1,$2,$3,$4)"#, [manifest.device_id.clone().into(),serde_json::to_string(&identity).unwrap().into(),serde_json::to_string(&receipt).unwrap().into(),now.into()])).await.unwrap();
        db.execute_raw(sql(
            r#"UPDATE "DeviceEnrollment" SET status='consumed' WHERE id=$1"#,
            [manifest.enrollment_id.clone().into()],
        ))
        .await
        .unwrap();
        let second = signed_write(&owner, "owner", &manifest.device_id, 2, 2);
        write_checked(&db, "owner", &manifest.device_id, second.clone())
            .await
            .unwrap();
        let a = signed_write(&owner, "owner", &manifest.device_id, 3, 3);
        let b = signed_write(&owner, "owner", &manifest.device_id, 3, 4);
        let (left, right) = flow_like_types::tokio::join!(
            write_checked(&db, "owner", &manifest.device_id, a.clone()),
            write_checked(&db, "owner", &manifest.device_id, b.clone())
        );
        assert_ne!(left.is_ok(), right.is_ok());
        let committed = if left.is_ok() { a } else { b };
        write_checked(&db, "owner", &manifest.device_id, committed.clone())
            .await
            .unwrap();
        assert_eq!(
            write_checked(&db, "owner", &manifest.device_id, second)
                .await
                .err()
                .unwrap()
                .status(),
            StatusCode::CONFLICT
        );
        let mut policy = ManagementPolicy {
            version: 1,
            device_id: manifest.device_id.clone(),
            policy_version: 1,
            previous_policy_digest: None,
            issued_at: now,
            expires_at: now + 600,
            grants: vec![ManagementGrant {
                grant_id: "shared-reader".into(),
                user_id: "reader".into(),
                controller_key: reader.public_key(),
                scope: ManagementScope::Device,
                capabilities: vec![ManagementCapability::Status],
                expires_at: now + 300,
                group_id: None,
                group_version: None,
            }],
        };
        let signed = sign_management_policy(&policy, &invitation).unwrap();
        db.execute_raw(sql(r#"INSERT INTO "DeviceManagementPolicy"("deviceId",version,digest,"policyJws","expiresAt","createdAt") VALUES($1,1,$2,$3,$4,$5)"#, [manifest.device_id.clone().into(),compact_digest(&signed).into(),signed.clone().into(),policy.expires_at.into(),now.into()])).await.unwrap();
        let reader_backup = signed_write(&reader, "reader", &manifest.device_id, 1, 5);
        write_checked(&db, "reader", &manifest.device_id, reader_backup.clone())
            .await
            .unwrap();
        assert_eq!(
            listing("reader").await,
            (vec![listed("recovery-device", 1, &reader)], 1)
        );
        assert_eq!(
            listing("owner").await,
            (vec![listed("recovery-device", 3, &owner)], 1)
        );
        assert_eq!(
            write_checked(
                &db,
                "outsider",
                &manifest.device_id,
                signed_write(&outsider, "outsider", &manifest.device_id, 1, 6)
            )
            .await
            .err()
            .unwrap()
            .status(),
            StatusCode::FORBIDDEN
        );
        policy.policy_version = 2;
        policy.previous_policy_digest = Some(compact_digest(&signed));
        policy.grants.clear();
        let signed = sign_management_policy(&policy, &invitation).unwrap();
        let revocation = db.begin().await.unwrap();
        revocation
            .execute_raw(sql(
                r#"UPDATE "ManagedDevice" SET "authEpoch"="authEpoch" WHERE id=$1"#,
                [manifest.device_id.clone().into()],
            ))
            .await
            .unwrap();
        revocation.execute_raw(sql(r#"INSERT INTO "DeviceManagementPolicy"("deviceId",version,digest,"policyJws","expiresAt","createdAt") VALUES($1,2,$2,$3,$4,$5)"#, [manifest.device_id.clone().into(),compact_digest(&signed).into(),signed.into(),policy.expires_at.into(),now.into()])).await.unwrap();
        let (raced, _) = flow_like_types::tokio::join!(
            write_checked(
                &db,
                "reader",
                &manifest.device_id,
                signed_write(&reader, "reader", &manifest.device_id, 2, 6)
            ),
            async {
                flow_like_types::tokio::time::sleep(std::time::Duration::from_millis(25)).await;
                revocation.commit().await.unwrap();
            }
        );
        assert_eq!(raced.err().unwrap().status(), StatusCode::FORBIDDEN);
        assert_eq!(
            write_checked(
                &db,
                "reader",
                &manifest.device_id,
                signed_write(&reader, "reader", &manifest.device_id, 2, 6)
            )
            .await
            .err()
            .unwrap()
            .status(),
            StatusCode::FORBIDDEN
        );
        // Exact retries acknowledge already-stored encrypted bytes without extending authority.
        write_checked(&db, "reader", &manifest.device_id, reader_backup)
            .await
            .unwrap();
        db.execute_raw(sql(
            r#"UPDATE "ManagedDevice" SET status='revoked',"authEpoch"=2 WHERE id=$1"#,
            [manifest.device_id.clone().into()],
        ))
        .await
        .unwrap();
        assert_eq!(
            write_checked(
                &db,
                "owner",
                &manifest.device_id,
                signed_write(&owner, "owner", &manifest.device_id, 4, 7)
            )
            .await
            .err()
            .unwrap()
            .status(),
            StatusCode::UNAUTHORIZED
        );
        write_checked(&db, "owner", &manifest.device_id, committed)
            .await
            .unwrap();
        let count = db
            .query_one_raw(sql(
                r#"SELECT COUNT(*) AS count FROM "DeviceControllerVault" WHERE "keyId"=$1"#,
                [manifest.device_id.clone().into()],
            ))
            .await
            .unwrap()
            .unwrap()
            .try_get::<i64>("", "count")
            .unwrap();
        assert_eq!(count, 2);
        assert_eq!(
            listing("owner").await,
            (vec![listed("recovery-device", 3, &owner)], 0),
            "the backup of a revoked device stays stored and holds no slot"
        );
        // Backups of the revoked device and of abandoned packages stop counting
        // against the account limit when the next device's first backup is saved.
        db.execute_raw(sql(
            r#"INSERT INTO "DeviceControllerVault"("userId","keyId","publicKey",ciphertext,revision,"updatedAt") SELECT 'owner','abandoned-' || n,'{}','fixture',1,$1 FROM generate_series(1,300) AS n"#,
            [now.into()],
        ))
        .await
        .unwrap();
        let mut next = manifest.clone();
        next.enrollment_id = "next-enrollment".into();
        next.device_id = "next-device".into();
        next.expires_at = now + 600;
        db.execute_raw(sql(r#"INSERT INTO "DeviceEnrollment"(id,"deviceId","ownerId","jwtId",manifest,status,"expiresAt","createdAt") VALUES($1,$2,'owner','next-jwt',$3,'pending',$4,$5)"#, [next.enrollment_id.clone().into(),next.device_id.clone().into(),serde_json::to_string(&next).unwrap().into(),next.expires_at.into(),now.into()])).await.unwrap();
        write_checked(
            &db,
            "owner",
            &next.device_id,
            signed_write(&owner, "owner", &next.device_id, 1, 8),
        )
        .await
        .unwrap();
        let remaining = db
            .query_one_raw(sql(
                r#"SELECT COUNT(*) AS count FROM "DeviceControllerVault" WHERE "userId"='owner'"#,
                [],
            ))
            .await
            .unwrap()
            .unwrap()
            .try_get::<i64>("", "count")
            .unwrap();
        assert_eq!(remaining, 1);
        assert_eq!(
            listing("owner").await,
            (vec![listed("next-device", 1, &owner)], 1)
        );
        db.close().await.unwrap();
        admin
            .execute_unprepared(&format!("DROP SCHEMA {schema} CASCADE"))
            .await
            .unwrap();
    }
}
