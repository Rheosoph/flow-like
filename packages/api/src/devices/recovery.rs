use super::{context, enabled, human_owner};
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
use sea_orm::{ConnectionTrait, DatabaseBackend, DatabaseConnection, Statement, Value};
use serde::{Deserialize, Serialize};
use std::result::Result;

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
                let count = tx.query_one_raw(sql(r#"SELECT COUNT(*) AS count FROM "DeviceControllerVault" WHERE "userId"=$1"#, [owner.clone().into()])).await?.ok_or(ApiError::FORBIDDEN)?;
                if count.try_get::<i64>("", "count")? >= 256 { return Err(ApiError::too_many_requests("Controller backup storage limit reached")); }
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
                [manifest.device_id.into()],
            ))
            .await
            .unwrap()
            .unwrap()
            .try_get::<i64>("", "count")
            .unwrap();
        assert_eq!(count, 2);
        db.close().await.unwrap();
        admin
            .execute_unprepared(&format!("DROP SCHEMA {schema} CASCADE"))
            .await
            .unwrap();
    }
}
