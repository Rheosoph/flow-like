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
    routing::{get, post},
};
use flow_like_device_protocol::*;
use sea_orm::{ConnectionTrait, DatabaseBackend, Statement, Value};
use serde::Deserialize;
use serde_json::json;
use std::result::Result;

fn sql(query: &str, values: impl IntoIterator<Item = Value>) -> Statement {
    Statement::from_sql_and_values(DatabaseBackend::Postgres, query, values)
}
fn invalid(error: impl std::fmt::Display) -> ApiError {
    ApiError::bad_request(error.to_string())
}
fn kind(kind: &ArchiveKind) -> &'static str {
    match kind {
        ArchiveKind::Logs => "logs",
        ArchiveKind::Metrics => "metrics",
    }
}

pub(crate) fn routes() -> Router<AppState> {
    Router::new()
        .route("/{id}/archives", post(upload).get(list))
        .route("/{id}/archives/{archive}", get(read))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Upload {
    client_assertion: String,
    bundle: EncryptedArchive,
}

async fn upload(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(request): Json<Upload>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let device = super::registered_assertion(
        &context(&state),
        &id,
        &request.client_assertion,
        &format!("/devices/{id}/archives"),
    )
    .await?;
    let enrollment = super::repository(&context(&state))
        .enrollment(&device.receipt.enrollment_id)
        .await?;
    let (manifest, _) = verify_archive_content(
        &request.bundle,
        &enrollment.manifest.owner_invitation_key,
        &device.status.identity.telemetry_key,
    )
    .map_err(invalid)?;
    let now = chrono::Utc::now().timestamp();
    if manifest.device_id != id
        || manifest.created_at > now + 5
        || manifest.sequence > i64::MAX as u64
    {
        return Err(ApiError::bad_request(
            "Invalid archive publisher or sequence",
        ));
    }
    let encoded = serde_json::to_string(&request.bundle)?;
    if encoded.len() > 90 * 1024 {
        return Err(ApiError::bad_request("Archive exceeds the transfer limit"));
    }
    let digest = compact_digest(&request.bundle.manifest_jws);
    let response_digest = digest.clone();
    let response_id = manifest.archive_id.clone();
    let limits = state.platform_config.standalone.telemetry_tiers.clone();
    let epoch = device.status.auth_epoch;
    let limits = limits
        .into_iter()
        .map(|(tier, limit)| (tier, (limit.max_bytes, limit.retention_seconds)))
        .collect();
    let stored = retain_verified(
        &state.db,
        state.db_dialect,
        limits,
        id,
        epoch,
        manifest,
        encoded,
        digest,
    )
    .await?;
    Ok(Json(
        json!({"archive_id":response_id,"manifest_digest":response_digest,"stored":stored}),
    ))
}

async fn retain_verified(
    db: &sea_orm::DatabaseConnection,
    dialect: crate::db::DbDialect,
    limits: std::collections::BTreeMap<String, (u64, u64)>,
    id: String,
    epoch: u64,
    manifest: ArchiveManifest,
    encoded: String,
    digest: String,
) -> Result<bool, ApiError> {
    retry_transaction(db,dialect,None,&RetryPolicy::default(),move|tx|{
        let id=id.clone();let manifest=manifest.clone();let encoded=encoded.clone();let digest=digest.clone();let limits=limits.clone();
        Box::pin(async move{
            let device=repository::lock_active_device(tx,&id,epoch).await?;
            let owner=device.status.owner_id;
            // Every device under one owner conflicts on this row, fencing concurrent quota writes and tier changes.
            let changed=tx.execute_raw(sql(r#"UPDATE "User" SET tier=tier WHERE id=$1 AND status='ACTIVE'"#,[owner.clone().into()])).await?.rows_affected();
            if changed!=1{return Err(ApiError::FORBIDDEN);}
            let tier=tx.query_one_raw(sql(r#"SELECT tier::text AS tier FROM "User" WHERE id=$1"#,[owner.clone().into()])).await?.ok_or(ApiError::FORBIDDEN)?.try_get::<String>("","tier")?.to_uppercase();
            let (max_bytes,retention_seconds)=limits.get(&tier).copied().unwrap_or_default();
            if max_bytes==0 || retention_seconds==0{return Err(ApiError::payment_required("This account tier does not store device telemetry"));}
            let now=chrono::Utc::now().timestamp();
            // Bound deletion batches for both Postgres and DSQL. Expired rows never count toward quota or reads.
            tx.execute_raw(sql(r#"DELETE FROM "DeviceArchive" WHERE ("deviceId","archiveId") IN (SELECT "deviceId","archiveId" FROM "DeviceArchive" WHERE "ownerId"=$1 AND "expiresAt"<=$2 LIMIT 128)"#,[owner.clone().into(),now.into()])).await?;
            if let Some(old)=tx.query_one_raw(sql(r#"SELECT digest FROM "DeviceArchive" WHERE "deviceId"=$1 AND "archiveId"=$2"#,[id.clone().into(),manifest.archive_id.clone().into()])).await?{
                if old.try_get::<String>("","digest")?!=digest{return Err(ApiError::conflict("Archive identifier was already used"));}
                return Ok(true);
            }
            let scope=manifest.scope.clone();let kind=kind(&manifest.kind);
            let head=tx.query_one_raw(sql(r#"SELECT sequence,digest FROM "DeviceArchiveHead" WHERE "deviceId"=$1 AND scope=$2 AND kind=$3"#,[id.clone().into(),scope.clone().into(),kind.into()])).await?;
            let (sequence,previous)=head.map(|r|Ok::<_,ApiError>((r.try_get::<i64>("","sequence")? as u64,Some(r.try_get::<String>("","digest")?)))).transpose()?.unwrap_or((0,None));
            if manifest.sequence==sequence && previous.as_ref()==Some(&digest){return Ok(false);}
            if manifest.sequence!=sequence+1 || manifest.previous_manifest_digest!=previous{return Err(ApiError::conflict("Archive sequence does not extend the retained-history chain"));}
            let retention=retention_seconds.min(31*86400) as i64;
            let expires=manifest.created_at.saturating_add(retention);
            if expires>now {
                let used=tx.query_one_raw(sql(r#"SELECT COALESCE(SUM("sizeBytes"),0)::bigint AS used FROM "DeviceArchive" WHERE "ownerId"=$1 AND "expiresAt">$2"#,[owner.clone().into(),now.into()])).await?.ok_or_else(||ApiError::internal("Missing telemetry quota result"))?.try_get::<i64>("","used")?;
                if used<0 || (used as u64).saturating_add(encoded.len() as u64)>max_bytes{return Err(ApiError::payment_required("Device telemetry storage quota reached"));}
                tx.execute_raw(sql(r#"INSERT INTO "DeviceArchive"("deviceId","archiveId","ownerId",scope,kind,sequence,digest,bundle,"sizeBytes","createdAt","expiresAt") VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11)"#,[id.clone().into(),manifest.archive_id.clone().into(),owner.into(),scope.clone().into(),kind.into(),(manifest.sequence as i64).into(),digest.clone().into(),encoded.clone().into(),(encoded.len() as i64).into(),manifest.created_at.into(),expires.into()])).await?;
            }
            tx.execute_raw(sql(r#"INSERT INTO "DeviceArchiveHead"("deviceId",scope,kind,sequence,digest) VALUES($1,$2,$3,$4,$5) ON CONFLICT("deviceId",scope,kind) DO UPDATE SET sequence=excluded.sequence,digest=excluded.digest"#,[id.into(),scope.into(),kind.into(),(manifest.sequence as i64).into(),digest.into()])).await?;
            Ok(expires>now)
        })
    }).await
}

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct ListQuery {
    scope: String,
    kind: ArchiveKind,
    #[serde(default)]
    after: u64,
}

fn authorize_retained(
    roster: &ArchiveRoster,
    user: &str,
    owner: &str,
    policy: Option<&ManagementPolicy>,
    now: i64,
) -> Result<(), ApiError> {
    if user == owner {
        return Ok(());
    }
    let policy = policy.ok_or(ApiError::FORBIDDEN)?;
    let capability = match roster.kind {
        ArchiveKind::Logs => ManagementCapability::Logs,
        ArchiveKind::Metrics => ManagementCapability::Metrics,
    };
    let allowed = policy.device_id == roster.device_id
        && policy.issued_at <= now
        && policy.expires_at > now
        && roster
            .recipients
            .iter()
            .any(|recipient| recipient.user_id == user)
        && policy.grants.iter().any(|grant| {
            grant.user_id == user
                && grant.expires_at > now
                && grant.capabilities.contains(&capability)
                && match &grant.scope {
                    ManagementScope::Device => true,
                    ManagementScope::Project { project_id } => {
                        roster.project_id.as_ref() == Some(project_id)
                    }
                    ManagementScope::Placement {
                        project_id,
                        placement_id,
                    } => {
                        roster.project_id.as_ref() == Some(project_id)
                            && &roster.scope == placement_id
                    }
                }
        });
    if !allowed {
        return Err(ApiError::FORBIDDEN);
    }
    Ok(())
}

async fn reader_policy(
    tx: &sea_orm::DatabaseTransaction,
    device: &repository::Device,
    user: &str,
    key: &Ed25519PublicKey,
) -> Result<Option<ManagementPolicy>, ApiError> {
    if user == device.status.owner_id {
        return Ok(None);
    }
    let row = tx.query_one_raw(sql(
        r#"SELECT "policyJws" FROM "DeviceManagementPolicy" WHERE "deviceId"=$1 ORDER BY version DESC LIMIT 1"#,
        [device.status.device_id.clone().into()],
    )).await?.ok_or(ApiError::FORBIDDEN)?;
    let policy = verify_management_policy(
        &row.try_get::<String>("", "policyJws")?,
        key,
        chrono::Utc::now().timestamp(),
    )
    .map_err(|_| ApiError::FORBIDDEN)?;
    if policy.device_id != device.status.device_id {
        return Err(ApiError::FORBIDDEN);
    }
    Ok(Some(policy))
}

#[derive(Clone)]
enum ArchiveSelection {
    One(String),
    Page(ListQuery),
}

struct VerifiedArchive {
    bundle: EncryptedArchive,
    manifest: ArchiveManifest,
    roster: ArchiveRoster,
    expires_at: i64,
}

struct ArchivePage {
    archives: Vec<VerifiedArchive>,
    next: u64,
}

fn filter_reader_page(
    page: &mut ArchivePage,
    user: &str,
    owner: &str,
    policy: Option<&ManagementPolicy>,
    one: bool,
) -> Result<(), ApiError> {
    let now = chrono::Utc::now().timestamp();
    if user != owner {
        let policy = policy.ok_or(ApiError::FORBIDDEN)?;
        if policy.issued_at > now
            || policy.expires_at <= now
            || !policy
                .grants
                .iter()
                .any(|grant| grant.user_id == user && grant.expires_at > now)
        {
            return Err(ApiError::FORBIDDEN);
        }
    }
    if one {
        let archive = page.archives.first().ok_or(ApiError::NOT_FOUND)?;
        if archive.expires_at <= now {
            return Err(ApiError::NOT_FOUND);
        }
        authorize_retained(&archive.roster, user, owner, policy, now)?;
    } else {
        page.archives.retain(|archive| {
            archive.expires_at > now
                && authorize_retained(&archive.roster, user, owner, policy, now).is_ok()
        });
    }
    Ok(())
}

async fn load_reader_page(
    db: &sea_orm::DatabaseConnection,
    dialect: crate::db::DbDialect,
    id: String,
    epoch: u64,
    user: String,
    owner_key: Ed25519PublicKey,
    selection: ArchiveSelection,
) -> Result<ArchivePage, ApiError> {
    let one = matches!(selection, ArchiveSelection::One(_));
    let reader = user.clone();
    let (mut page, policy, owner) = retry_transaction(db, dialect, None, &RetryPolicy::default(), move |tx| {
        let id = id.clone(); let user = user.clone(); let owner_key = owner_key.clone(); let selection = selection.clone();
        Box::pin(async move {
            // Policy updates and registration revocation acquire this same device row.
            let device = repository::lock_active_device(tx, &id, epoch).await?;
            repository::active_account(tx, &user).await?;
            let policy = reader_policy(tx, &device, &user, &owner_key).await?;
            let now = chrono::Utc::now().timestamp();
            let (rows, mut next) = match &selection {
                ArchiveSelection::One(archive) => (tx.query_all_raw(sql(
                    r#"SELECT bundle,"expiresAt",sequence FROM "DeviceArchive" WHERE "deviceId"=$1 AND "archiveId"=$2 AND "expiresAt">$3"#,
                    [id.clone().into(), archive.clone().into(), now.into()],
                )).await?, 0),
                ArchiveSelection::Page(query) => {
                    validate_management_id(&query.scope).map_err(invalid)?;
                    let after = i64::try_from(query.after).map_err(invalid)?;
                    (tx.query_all_raw(sql(
                        r#"SELECT bundle,"expiresAt",sequence FROM "DeviceArchive" WHERE "deviceId"=$1 AND scope=$2 AND kind=$3 AND sequence>$4 AND "expiresAt">$5 ORDER BY sequence LIMIT 32"#,
                        [id.clone().into(), query.scope.clone().into(), kind(&query.kind).into(), after.into(), now.into()],
                    )).await?, query.after)
                }
            };
            repository::active_account(tx, &user).await?;
            repository::active_account(tx, &device.status.owner_id).await?;
            let mut archives = Vec::new();
            for row in rows {
                let sequence = u64::try_from(row.try_get::<i64>("", "sequence")?).map_err(invalid)?;
                next = sequence;
                let bundle: EncryptedArchive = serde_json::from_str(&row.try_get::<String>("", "bundle")?)?;
                let (manifest, roster) = verify_archive_content(&bundle, &owner_key, &device.status.identity.telemetry_key)
                    .map_err(|_| ApiError::internal("Retained archive signature or digest failed"))?;
                let bound = manifest.device_id == id && manifest.sequence == sequence && match &selection {
                    ArchiveSelection::One(archive) => &manifest.archive_id == archive,
                    ArchiveSelection::Page(query) => manifest.scope == query.scope && manifest.kind == query.kind,
                };
                if !bound { return Err(ApiError::internal("Retained archive index binding failed")); }
                archives.push(VerifiedArchive { bundle, manifest, roster, expires_at: row.try_get("", "expiresAt")? });
            }
            let mut page = ArchivePage { archives, next };
            filter_reader_page(&mut page, &user, &device.status.owner_id, policy.as_ref(), one)?;
            Ok::<_, ApiError>((page, policy, device.status.owner_id))
        })
    }).await?;
    // A slow commit must not extend a grant or a segment's retention deadline.
    filter_reader_page(&mut page, &reader, &owner, policy.as_ref(), one)?;
    Ok(page)
}

async fn list(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(id): Path<String>,
    Query(query): Query<ListQuery>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let user = human_owner(&state, &user).await?;
    let (device, _) = management::admitted_device(&state, &user, &id).await?;
    validate_management_id(&query.scope).map_err(invalid)?;
    let enrollment = super::repository(&context(&state))
        .enrollment(&device.receipt.enrollment_id)
        .await?;
    let page = load_reader_page(
        &state.db,
        state.db_dialect,
        id,
        device.status.auth_epoch,
        user,
        enrollment.manifest.owner_invitation_key,
        ArchiveSelection::Page(query),
    )
    .await?;
    let archives: Vec<_> = page.archives.into_iter().map(|archive| json!({"archive_id":archive.manifest.archive_id,"sequence":archive.manifest.sequence,"created_at":archive.manifest.created_at,"expires_at":archive.expires_at,"manifest_jws":archive.bundle.manifest_jws,"roster_jws":archive.bundle.roster_jws})).collect();
    Ok(Json(json!({"archives":archives,"next":page.next})))
}

async fn read(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((id, archive)): Path<(String, String)>,
) -> Result<Json<EncryptedArchive>, ApiError> {
    let user = human_owner(&state, &user).await?;
    let (device, _) = management::admitted_device(&state, &user, &id).await?;
    let enrollment = super::repository(&context(&state))
        .enrollment(&device.receipt.enrollment_id)
        .await?;
    let mut page = load_reader_page(
        &state.db,
        state.db_dialect,
        id,
        device.status.auth_epoch,
        user,
        enrollment.manifest.owner_invitation_key,
        ArchiveSelection::One(archive),
    )
    .await?;
    Ok(Json(page.archives.pop().ok_or(ApiError::NOT_FOUND)?.bundle))
}

/// Bounded maintenance is safe to run concurrently on API replicas.
pub(crate) async fn sweep_expired(state: &AppState) -> Result<u64, ApiError> {
    Ok(state.db.execute_raw(sql(r#"DELETE FROM "DeviceArchive" WHERE ("deviceId","archiveId") IN (SELECT "deviceId","archiveId" FROM "DeviceArchive" WHERE "expiresAt"<=$1 LIMIT 128)"#,[chrono::Utc::now().timestamp().into()])).await?.rows_affected())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::DbDialect;
    use sea_orm::{ConnectOptions, Database, DatabaseConnection, TransactionTrait};

    #[test]
    fn archive_reads_require_current_capability_and_matching_project() {
        let mut roster = ArchiveRoster {
            version: 1,
            device_id: "device".into(),
            scope: "placement".into(),
            project_id: Some("project".into()),
            kind: ArchiveKind::Logs,
            policy_version: 1,
            previous_policy_digest: None,
            management_policy_digest: None,
            recipients: vec![ArchiveRecipient {
                recipient_id: "reader-key".into(),
                user_id: "reader".into(),
                public_key: [7; 32],
            }],
            issued_at: 100,
            expires_at: 200,
        };
        let mut policy = ManagementPolicy {
            version: 1,
            device_id: "device".into(),
            policy_version: 2,
            previous_policy_digest: Some("previous".into()),
            grants: vec![ManagementGrant {
                grant_id: "grant".into(),
                user_id: "reader".into(),
                controller_key: SigningKey::generate().public_key(),
                scope: ManagementScope::Project {
                    project_id: "project".into(),
                },
                capabilities: vec![ManagementCapability::Logs],
                expires_at: 400,
                group_id: None,
                group_version: None,
            }],
            issued_at: 250,
            expires_at: 500,
        };
        // Historical roster expiry does not erase retained access, but current grants do.
        let allowed = |roster: &ArchiveRoster, policy: &ManagementPolicy| {
            authorize_retained(roster, "reader", "owner", Some(policy), 300).is_ok()
        };
        assert!(allowed(&roster, &policy));
        policy.grants[0].capabilities = vec![ManagementCapability::Status];
        assert!(!allowed(&roster, &policy));
        policy.grants[0].capabilities = vec![ManagementCapability::Logs];
        roster.project_id = Some("another-project".into());
        assert!(!allowed(&roster, &policy));
        roster.project_id = Some("project".into());
        policy.grants[0].scope = ManagementScope::Placement {
            project_id: "project".into(),
            placement_id: "another-placement".into(),
        };
        assert!(!allowed(&roster, &policy));
        policy.grants[0].scope = ManagementScope::Device;
        assert!(allowed(&roster, &policy));
        roster.kind = ArchiveKind::Metrics;
        assert!(!allowed(&roster, &policy));
        roster.kind = ArchiveKind::Logs;
        policy.grants[0].expires_at = 300;
        assert!(!allowed(&roster, &policy));
        policy.grants[0].expires_at = 400;
        policy.device_id = "another-device".into();
        assert!(!allowed(&roster, &policy));
        policy.device_id = "device".into();
        roster.recipients.clear();
        assert!(!allowed(&roster, &policy));
        assert!(authorize_retained(&roster, "owner", "owner", None, 300).is_ok());
    }

    async fn device(db: &DatabaseConnection, id: &str) -> SigningKey {
        let telemetry = SigningKey::generate();
        let identity = DeviceIdentity {
            auth_key: SigningKey::generate().public_key(),
            management_key: [42; 32],
            telemetry_key: telemetry.public_key(),
        };
        let receipt = DeviceReceipt {
            enrollment_id: "enrollment".into(),
            device_id: id.into(),
            owner_id: "owner".into(),
            name: "Archive fixture".into(),
            identity: identity.clone(),
            manifest_jws: "persistence-only".into(),
            binding_jws: "persistence-only".into(),
            registered_at: chrono::Utc::now().timestamp(),
            auth_epoch: 1,
        };
        db.execute_raw(sql(r#"INSERT INTO "ManagedDevice"(id,"ownerId",name,status,"authEpoch",identity,receipt,"registeredAt") VALUES($1,'owner','Archive fixture','active',1,$2,$3,$4)"#,[id.into(),serde_json::to_string(&identity).unwrap().into(),serde_json::to_string(&receipt).unwrap().into(),receipt.registered_at.into()])).await.unwrap();
        telemetry
    }

    fn signed_archive(owner: &SigningKey, telemetry: &SigningKey) -> EncryptedArchive {
        use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
        use sha2::{Digest, Sha256};
        let now = chrono::Utc::now().timestamp();
        let roster = ArchiveRoster {
            version: 1,
            device_id: "shared".into(),
            scope: "placement".into(),
            project_id: Some("project".into()),
            kind: ArchiveKind::Logs,
            policy_version: 1,
            previous_policy_digest: None,
            management_policy_digest: None,
            recipients: vec![ArchiveRecipient {
                recipient_id: "reader-key".into(),
                user_id: "reader".into(),
                public_key: [9; 32],
            }],
            issued_at: now - 10,
            expires_at: now + 600,
        };
        let roster_jws = sign_archive_roster(&roster, owner).unwrap();
        // The API validates signatures and opaque ciphertext integrity; it never decrypts these bytes.
        let ciphertext = [11; 32];
        let wrapped_key = [12; 48];
        let manifest = ArchiveManifest {
            version: 1,
            device_id: "shared".into(),
            scope: "placement".into(),
            kind: ArchiveKind::Logs,
            archive_id: "segment".into(),
            sequence: 1,
            previous_manifest_digest: None,
            roster_digest: compact_digest(&roster_jws),
            created_at: now,
            nonce: [13; 24],
            ciphertext_size: ciphertext.len() as u64,
            ciphertext_digest: URL_SAFE_NO_PAD.encode(Sha256::digest(ciphertext)),
            recipients: vec![ArchiveWrappedKeyManifest {
                recipient_id: "reader-key".into(),
                encapsulation: [14; 32],
                wrapped_key_digest: URL_SAFE_NO_PAD.encode(Sha256::digest(wrapped_key)),
            }],
        };
        EncryptedArchive {
            roster_jws,
            manifest_jws: sign_archive_manifest(&manifest, telemetry).unwrap(),
            ciphertext: URL_SAFE_NO_PAD.encode(ciphertext),
            recipient_keys: vec![ArchiveRecipientKey {
                recipient_id: "reader-key".into(),
                wrapped_key: URL_SAFE_NO_PAD.encode(wrapped_key),
            }],
        }
    }

    async fn insert_policy<C: ConnectionTrait>(
        db: &C,
        policy: &ManagementPolicy,
        owner: &SigningKey,
    ) -> String {
        let compact = sign_management_policy(policy, owner).unwrap();
        db.execute_raw(sql(r#"INSERT INTO "DeviceManagementPolicy"("deviceId",version,digest,"policyJws","expiresAt","createdAt") VALUES($1,$2,$3,$4,$5,$6)"#,
            [policy.device_id.clone().into(), (policy.policy_version as i64).into(), compact_digest(&compact).into(), compact.clone().into(), policy.expires_at.into(), policy.issued_at.into()])).await.unwrap();
        compact
    }

    async fn download(
        db: &DatabaseConnection,
        owner: &SigningKey,
        user: &str,
        one: bool,
    ) -> Result<ArchivePage, ApiError> {
        load_reader_page(
            db,
            DbDialect::Postgres,
            "shared".into(),
            1,
            user.into(),
            owner.public_key(),
            if one {
                ArchiveSelection::One("segment".into())
            } else {
                ArchiveSelection::Page(ListQuery {
                    scope: "placement".into(),
                    kind: ArchiveKind::Logs,
                    after: 0,
                })
            },
        )
        .await
    }

    #[tokio::test]
    #[ignore = "requires FLOW_LIKE_DEVICE_TEST_DATABASE_URL pointing to a disposable PostgreSQL server"]
    async fn archive_downloads_fence_revocation_and_expiry_during_database_waits() {
        use std::time::Duration;
        let url = std::env::var("FLOW_LIKE_DEVICE_TEST_DATABASE_URL").unwrap();
        let admin = Database::connect(&url).await.unwrap();
        let schema = format!("archive_reader_test_{}", uuid::Uuid::new_v4().simple());
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
            include_str!("../../prisma/migrations/20260922130000_device_archives/migration.sql"),
        ] {
            for statement in migration.split(';').filter(|part| !part.trim().is_empty()) {
                db.execute_unprepared(statement).await.unwrap();
            }
        }
        db.execute_unprepared(r#"CREATE TABLE "User"(id TEXT PRIMARY KEY,status TEXT NOT NULL,tier TEXT NOT NULL); INSERT INTO "User" VALUES('owner','ACTIVE','PREMIUM'),('reader','ACTIVE','FREE'),('nonrecipient','ACTIVE','FREE')"#).await.unwrap();
        let telemetry = device(&db, "shared").await;
        let owner = SigningKey::generate();
        let now = chrono::Utc::now().timestamp();
        let mut policy = ManagementPolicy {
            version: 1,
            device_id: "shared".into(),
            policy_version: 1,
            previous_policy_digest: None,
            grants: ["reader", "nonrecipient"]
                .into_iter()
                .map(|user| ManagementGrant {
                    grant_id: user.into(),
                    user_id: user.into(),
                    controller_key: SigningKey::generate().public_key(),
                    scope: ManagementScope::Project {
                        project_id: "project".into(),
                    },
                    capabilities: vec![ManagementCapability::Logs],
                    expires_at: now + 600,
                    group_id: None,
                    group_version: None,
                })
                .collect(),
            issued_at: now - 1,
            expires_at: now + 600,
        };
        let first_policy = insert_policy(&db, &policy, &owner).await;
        let bundle = signed_archive(&owner, &telemetry);
        let (manifest, _) =
            verify_archive_content(&bundle, &owner.public_key(), &telemetry.public_key()).unwrap();
        retain_verified(
            &db,
            DbDialect::Postgres,
            limits(),
            "shared".into(),
            1,
            manifest,
            serde_json::to_string(&bundle).unwrap(),
            compact_digest(&bundle.manifest_jws),
        )
        .await
        .unwrap();
        assert_eq!(
            download(&db, &owner, "reader", true)
                .await
                .unwrap()
                .archives
                .len(),
            1
        );
        let page = download(&db, &owner, "reader", false).await.unwrap();
        assert_eq!(page.archives.len(), 1);
        assert_eq!(page.next, 1);
        assert_eq!(
            download(&db, &owner, "nonrecipient", true)
                .await
                .err()
                .unwrap()
                .status(),
            axum::http::StatusCode::FORBIDDEN
        );
        assert!(
            download(&db, &owner, "nonrecipient", false)
                .await
                .unwrap()
                .archives
                .is_empty()
        );

        let change = db.begin().await.unwrap();
        repository::lock_active_device(&change, "shared", 1)
            .await
            .unwrap();
        policy.policy_version = 2;
        policy.previous_policy_digest = Some(compact_digest(&first_policy));
        policy.grants[0].capabilities = vec![ManagementCapability::Status];
        let revoked_policy = insert_policy(&change, &policy, &owner).await;
        let mut read = Box::pin(download(&db, &owner, "reader", true));
        let mut list = Box::pin(download(&db, &owner, "reader", false));
        assert!(
            tokio::time::timeout(Duration::from_millis(50), &mut read)
                .await
                .is_err()
        );
        assert!(
            tokio::time::timeout(Duration::from_millis(50), &mut list)
                .await
                .is_err()
        );
        change.commit().await.unwrap();
        assert_eq!(
            read.await.err().unwrap().status(),
            axum::http::StatusCode::FORBIDDEN
        );
        assert!(list.await.unwrap().archives.is_empty());

        // Let the policy load succeed, then block the archive SELECT until the grant expires.
        policy.policy_version = 3;
        policy.previous_policy_digest = Some(compact_digest(&revoked_policy));
        policy.grants[0].capabilities = vec![ManagementCapability::Logs];
        policy.grants[0].expires_at = chrono::Utc::now().timestamp() + 2;
        let expiring_policy = insert_policy(&db, &policy, &owner).await;
        let blocked = db.begin().await.unwrap();
        blocked
            .execute_unprepared(r#"LOCK TABLE "DeviceArchive" IN ACCESS EXCLUSIVE MODE"#)
            .await
            .unwrap();
        let mut read = Box::pin(download(&db, &owner, "reader", true));
        assert!(
            tokio::time::timeout(Duration::from_millis(50), &mut read)
                .await
                .is_err()
        );
        assert!(chrono::Utc::now().timestamp() < policy.grants[0].expires_at);
        tokio::time::sleep(Duration::from_millis(2100)).await;
        blocked.commit().await.unwrap();
        assert_eq!(
            read.await.err().unwrap().status(),
            axum::http::StatusCode::FORBIDDEN
        );

        policy.policy_version = 4;
        policy.previous_policy_digest = Some(compact_digest(&expiring_policy));
        policy.grants[0].expires_at = policy.expires_at;
        insert_policy(&db, &policy, &owner).await;
        let revoke = db.begin().await.unwrap();
        revoke
            .execute_unprepared(
                r#"UPDATE "ManagedDevice" SET status='revoked',"authEpoch"=2 WHERE id='shared'"#,
            )
            .await
            .unwrap();
        let mut read = Box::pin(download(&db, &owner, "reader", true));
        let mut list = Box::pin(download(&db, &owner, "reader", false));
        assert!(
            tokio::time::timeout(Duration::from_millis(50), &mut read)
                .await
                .is_err()
        );
        assert!(
            tokio::time::timeout(Duration::from_millis(50), &mut list)
                .await
                .is_err()
        );
        revoke.commit().await.unwrap();
        assert_eq!(
            read.await.err().unwrap().status(),
            axum::http::StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            list.await.err().unwrap().status(),
            axum::http::StatusCode::UNAUTHORIZED
        );
        db.close().await.unwrap();
        admin
            .execute_unprepared(&format!("DROP SCHEMA {schema} CASCADE"))
            .await
            .unwrap();
        admin.close().await.unwrap();
    }
    fn manifest(device: &str) -> ArchiveManifest {
        ArchiveManifest {
            version: 1,
            device_id: device.into(),
            scope: "device".into(),
            kind: ArchiveKind::Logs,
            archive_id: uuid::Uuid::new_v4().to_string(),
            sequence: 1,
            previous_manifest_digest: None,
            roster_digest: "verified-by-route".into(),
            created_at: chrono::Utc::now().timestamp(),
            nonce: [7; 24],
            ciphertext_size: 16,
            ciphertext_digest: "verified-by-route".into(),
            recipients: vec![],
        }
    }
    fn limits() -> std::collections::BTreeMap<String, (u64, u64)> {
        [("PREMIUM".into(), (90000, 3600)), ("FREE".into(), (0, 0))].into()
    }
    async fn retain(
        db: &DatabaseConnection,
        manifest: ArchiveManifest,
        digest: &str,
    ) -> Result<bool, ApiError> {
        retain_verified(
            db,
            DbDialect::Postgres,
            limits(),
            manifest.device_id.clone(),
            1,
            manifest,
            "x".repeat(60000),
            digest.into(),
        )
        .await
    }

    #[tokio::test]
    #[ignore = "requires FLOW_LIKE_DEVICE_TEST_DATABASE_URL pointing to a disposable PostgreSQL server"]
    async fn archive_quota_chain_expiry_and_revocation_are_transactional() {
        let url = std::env::var("FLOW_LIKE_DEVICE_TEST_DATABASE_URL").unwrap();
        let admin = Database::connect(&url).await.unwrap();
        let schema = format!("archive_test_{}", uuid::Uuid::new_v4().simple());
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
            include_str!("../../prisma/migrations/20260922130000_device_archives/migration.sql"),
        ] {
            for statement in migration.split(';') {
                if !statement.trim().is_empty() {
                    db.execute_unprepared(statement).await.unwrap();
                }
            }
        }
        db.execute_unprepared(r#"CREATE TABLE "User"(id TEXT PRIMARY KEY,status TEXT NOT NULL,tier TEXT NOT NULL); INSERT INTO "User" VALUES('owner','ACTIVE','PREMIUM')"#).await.unwrap();
        device(&db, "one").await;
        device(&db, "two").await;
        let one = manifest("one");
        let two = manifest("two");
        let (a, b) = tokio::join!(
            retain(&db, one.clone(), "digest-one"),
            retain(&db, two.clone(), "digest-two")
        );
        assert_eq!([a.is_ok(), b.is_ok()].into_iter().filter(|v| *v).count(), 1);
        let (winner, digest, loser) = if a.is_ok() {
            (one, "digest-one", two)
        } else {
            (two, "digest-two", one)
        };
        assert!(retain(&db, winner.clone(), digest).await.unwrap());
        assert_eq!(
            retain(&db, winner.clone(), "changed")
                .await
                .unwrap_err()
                .status(),
            axum::http::StatusCode::CONFLICT
        );
        db.execute_unprepared(r#"UPDATE "DeviceArchive" SET "expiresAt"=0"#)
            .await
            .unwrap();
        assert!(!retain(&db, winner.clone(), digest).await.unwrap());
        assert!(retain(&db, loser.clone(), "other-digest").await.unwrap());
        db.execute_unprepared(r#"UPDATE "User" SET tier='FREE'"#)
            .await
            .unwrap();
        let mut next = winner.clone();
        next.sequence = 2;
        next.previous_manifest_digest = Some(digest.into());
        next.archive_id = uuid::Uuid::new_v4().to_string();
        assert_eq!(
            retain(&db, next.clone(), "next")
                .await
                .unwrap_err()
                .status(),
            axum::http::StatusCode::PAYMENT_REQUIRED
        );
        db.execute_unprepared(
            r#"UPDATE "User" SET tier='PREMIUM'; UPDATE "DeviceArchive" SET "expiresAt"=0"#,
        )
        .await
        .unwrap();
        next.created_at -= 7200;
        assert!(!retain(&db, next.clone(), "next").await.unwrap());
        next.sequence = 3;
        next.previous_manifest_digest = Some("next".into());
        next.archive_id = uuid::Uuid::new_v4().to_string();
        next.created_at = chrono::Utc::now().timestamp();
        db.execute_raw(sql(
            r#"UPDATE "ManagedDevice" SET status='revoked',"authEpoch"=2 WHERE id=$1"#,
            [winner.device_id.into()],
        ))
        .await
        .unwrap();
        assert_eq!(
            retain(&db, next, "revoked").await.unwrap_err().status(),
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
