use super::{context, device_principal, human_owner, management, repository};
use crate::{
    db::{DbDialect, RetryPolicy, retry_transaction},
    entity::sea_orm_active_enums::NotificationType,
    error::ApiError,
    mail::EmailMessage,
    middleware::jwt::AppUser,
    push_notifications::{
        DispatchNotificationInput, PushDispatchStatus, dispatch_notification_idempotent,
    },
    state::AppState,
};
use axum::{
    Extension, Json, Router,
    extract::{Path, State},
    http::HeaderMap,
    routing::get,
};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use flow_like_device_protocol::{
    CertificateInventory, CertificateInventoryEntry, ManagementCapability, ManagementGrant,
    ManagementScope, OnboardingManifest, verify_certificate_inventory, verify_management_policy,
};
use flow_like_types::tokio;
use sea_orm::{
    ConnectionTrait, DatabaseBackend, DatabaseConnection, QueryResult, Statement, Value,
};
use serde::{Deserialize, Serialize};
use std::time::Duration;

const DAY: i64 = 86_400;
const CHECK_SECONDS: i64 = 300;
const LEASE_SECONDS: i64 = 120;
const BATCH: usize = 100;
// Scheduling and delivery have separate budgets, so a burst of inventories
// cannot use up the pass that delivers reminders.
const SCHEDULE_BUDGET: Duration = Duration::from_secs(20);
const DISPATCH_BUDGET: Duration = Duration::from_secs(25);
/// One owner's devices cannot take more than this share of a delivery pass.
const NOTICES_PER_OWNER_PER_PASS: i64 = 10;

fn sql(query: &str, values: impl IntoIterator<Item = Value>) -> Statement {
    Statement::from_sql_and_values(DatabaseBackend::Postgres, query, values)
}

pub(crate) fn routes() -> Router<AppState> {
    Router::new().route("/{id}/certificate-inventory", get(read).put(write))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct InventoryWrite {
    inventory_jws: String,
}

#[derive(Serialize)]
struct InventoryView {
    revision: u64,
    updated_at: Option<i64>,
    certificates: Vec<CertificateInventoryEntry>,
}

async fn view<C: ConnectionTrait>(db: &C, id: &str) -> Result<InventoryView, ApiError> {
    let row = db
        .query_one_raw(sql(
            r#"SELECT payload,"updatedAt" FROM "DeviceCertificateInventory" WHERE "deviceId"=$1"#,
            [id.into()],
        ))
        .await?;
    let Some(row) = row else {
        return Ok(InventoryView {
            revision: 0,
            updated_at: None,
            certificates: vec![],
        });
    };
    let inventory: CertificateInventory =
        serde_json::from_str(&row.try_get::<String>("", "payload")?)?;
    Ok(InventoryView {
        revision: inventory.revision,
        updated_at: Some(row.try_get("", "updatedAt")?),
        certificates: inventory.certificates,
    })
}

async fn read(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(id): Path<String>,
) -> Result<Json<InventoryView>, ApiError> {
    let user = human_owner(&state, &user).await?;
    management::admitted_device(&state, &user, &id).await?;
    let audience = audience(&state.db, &id, chrono::Utc::now().timestamp())
        .await?
        .ok_or(ApiError::FORBIDDEN)?;
    if !audience.admits(&user) {
        return Err(ApiError::forbidden(
            "Certificate inventory requires device-wide Status or ManageCertificates access",
        ));
    }
    Ok(Json(view(&state.db, &id).await?))
}

async fn write(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Json(request): Json<InventoryWrite>,
) -> Result<Json<InventoryView>, ApiError> {
    let principal = device_principal(
        &context(&state),
        &id,
        &headers,
        "PUT",
        &format!("/devices/{id}/certificate-inventory"),
        false,
    )
    .await?;
    let inventory = verify_certificate_inventory(
        &request.inventory_jws,
        &principal.status.identity.auth_key,
        &id,
        chrono::Utc::now().timestamp(),
    )
    .map_err(|_| ApiError::bad_request("Invalid signed certificate inventory"))?;
    retain(
        &state.db,
        state.db_dialect,
        principal.status.auth_epoch,
        inventory,
    )
    .await?;
    Ok(Json(view(&state.db, &id).await?))
}

fn same_certificate(a: &CertificateInventoryEntry, b: &CertificateInventoryEntry) -> bool {
    a.certificate_id == b.certificate_id
        && a.revision == b.revision
        && a.fingerprint_sha256 == b.fingerprint_sha256
        && a.not_after == b.not_after
}

/// Timestamps refresh the upload proof; only material changes advance the revision.
fn inventory_transition(
    previous: &CertificateInventory,
    next: &CertificateInventory,
) -> Result<bool, ApiError> {
    let same = previous.version == next.version
        && previous.device_id == next.device_id
        && previous.certificates.len() == next.certificates.len()
        && previous.certificates.iter().all(|old| {
            next.certificates
                .iter()
                .any(|entry| same_certificate(old, entry))
        });
    if next.revision == previous.revision && same {
        return Ok(false);
    }
    if next.revision <= previous.revision {
        return Err(ApiError::conflict(
            "Certificate inventory revision is stale",
        ));
    }
    for entry in &next.certificates {
        if let Some(old) = previous
            .certificates
            .iter()
            .find(|old| old.certificate_id == entry.certificate_id)
            && (entry.revision < old.revision
                || (entry.revision == old.revision && !same_certificate(old, entry)))
        {
            return Err(ApiError::conflict("Certificate revision is stale"));
        }
    }
    Ok(true)
}

async fn retain(
    db: &DatabaseConnection,
    dialect: DbDialect,
    epoch: u64,
    inventory: CertificateInventory,
) -> Result<(), ApiError> {
    retry_transaction(db, dialect, None, &RetryPolicy::default(), move |tx| {
        let inventory = inventory.clone();
        Box::pin(async move {
            repository::lock_active_device(tx, &inventory.device_id, epoch).await?;
            let now = chrono::Utc::now().timestamp();
            let previous = tx.query_one_raw(sql(r#"SELECT payload FROM "DeviceCertificateInventory" WHERE "deviceId"=$1"#, [inventory.device_id.clone().into()])).await?;
            if let Some(previous) = previous {
                let previous: CertificateInventory = serde_json::from_str(&previous.try_get::<String>("", "payload")?)?;
                if !inventory_transition(&previous, &inventory)? {
                    // A fresh proof confirms receipt without postponing a reminder or
                    // changing the material revision that deduplicates its deliveries.
                    tx.execute_raw(sql(r#"UPDATE "DeviceCertificateInventory" SET "updatedAt"=$2 WHERE "deviceId"=$1"#, [inventory.device_id.clone().into(), now.into()])).await?;
                    return Ok(());
                }
            }
            // A material change keeps the device's reminder cadence, so rewriting the
            // inventory cannot schedule notices more often than once per check.
            tx.execute_raw(sql(r#"INSERT INTO "DeviceCertificateInventory"("deviceId",revision,payload,"updatedAt","nextCheckAt") VALUES($1,$2,$3,$4,$4)
                ON CONFLICT("deviceId") DO UPDATE SET revision=$2,payload=$3,"updatedAt"=$4"#,
                [inventory.device_id.clone().into(), (inventory.revision as i64).into(), serde_json::to_string(&inventory)?.into(), now.into()])).await?;
            // Deleting stale deliveries also removes their retry leases. A sender rechecks
            // both the inventory and access immediately before dispatching each channel.
            let notices = tx.query_all_raw(sql(r#"SELECT * FROM "DeviceCertificateNotice" WHERE "deviceId"=$1"#, [inventory.device_id.clone().into()])).await?;
            for row in notices {
                let notice = Notice::from_row(row)?;
                if !inventory.certificates.iter().any(|entry| notice.matches(entry)) {
                    tx.execute_raw(sql(r#"DELETE FROM "DeviceCertificateNotice" WHERE id=$1"#, [notice.id.into()])).await?;
                }
            }
            Ok(())
        })
    }).await
}

fn stage(not_after: i64, now: i64) -> Option<&'static str> {
    match not_after.saturating_sub(now) {
        left if left <= 0 => Some("expired"),
        left if left <= DAY => Some("day"),
        left if left <= 3 * DAY => Some("three_days"),
        left if left <= 7 * DAY => Some("week"),
        _ => None,
    }
}

/// Who may see a device's certificates: its owner, and grantees whose current
/// grant covers the whole device with Status or ManageCertificates, the same
/// access the device itself requires for its Certificates command.
struct CertificateAudience {
    owner: String,
    grants: Vec<ManagementGrant>,
}

impl CertificateAudience {
    fn admits(&self, user: &str) -> bool {
        self.owner == user || self.grants.iter().any(|grant| grant.user_id == user)
    }
}

fn covers_certificates(grant: &ManagementGrant, now: i64) -> bool {
    grant.expires_at > now
        && grant.scope == ManagementScope::Device
        && grant.capabilities.iter().any(|capability| {
            matches!(
                capability,
                ManagementCapability::Status | ManagementCapability::ManageCertificates
            )
        })
}

/// Only the latest owner-signed policy counts; an unsigned or expired one grants nothing.
async fn audience<C: ConnectionTrait>(
    db: &C,
    id: &str,
    now: i64,
) -> Result<Option<CertificateAudience>, ApiError> {
    let Some(row) = db.query_one_raw(sql(r#"SELECT d."ownerId",e.manifest,p."policyJws" FROM "ManagedDevice" d
        JOIN "User" owner ON owner.id=d."ownerId" AND owner.status='ACTIVE'
        LEFT JOIN "DeviceEnrollment" e ON e."deviceId"=d.id
        LEFT JOIN "DeviceManagementPolicy" p ON p."deviceId"=d.id AND p.version=(SELECT MAX(version) FROM "DeviceManagementPolicy" WHERE "deviceId"=d.id)
        WHERE d.id=$1 AND d.status='active'"#, [id.into()])).await? else {
        return Ok(None);
    };
    let manifest = row
        .try_get::<Option<String>>("", "manifest")?
        .map(|manifest| serde_json::from_str::<OnboardingManifest>(&manifest))
        .transpose()?;
    let grants = match (manifest, row.try_get::<Option<String>>("", "policyJws")?) {
        (Some(manifest), Some(policy)) => {
            verify_management_policy(&policy, &manifest.owner_invitation_key, now)
                .ok()
                .filter(|policy| policy.device_id == id)
                .map(|policy| {
                    policy
                        .grants
                        .into_iter()
                        .filter(|grant| covers_certificates(grant, now))
                        .collect()
                })
                .unwrap_or_default()
        }
        _ => Vec::new(),
    };
    Ok(Some(CertificateAudience {
        owner: row.try_get("", "ownerId")?,
        grants,
    }))
}

/// Notices go only to grantees who accepted the share: under their own session
/// they registered the grant's controller key for this device, either as a
/// fleet reader or as an account backup. A policy naming an account grants it
/// nothing it can be notified about until then.
async fn recipients<C: ConnectionTrait>(
    db: &C,
    id: &str,
    now: i64,
) -> Result<Vec<String>, ApiError> {
    let Some(audience) = audience(db, id, now).await? else {
        return Ok(Vec::new());
    };
    let mut users = vec![audience.owner.clone()];
    if !audience.grants.is_empty() {
        let mut reader_pairs = Vec::with_capacity(audience.grants.len());
        let mut backup_pairs = Vec::with_capacity(audience.grants.len());
        for grant in &audience.grants {
            let key_id =
                URL_SAFE_NO_PAD.encode(grant.controller_key.to_bytes().map_err(|error| {
                    ApiError::internal(format!(
                        "Invalid controller key in grant {}: {error}",
                        grant.grant_id
                    ))
                })?);
            reader_pairs.push((grant.user_id.clone(), key_id));
            backup_pairs.push((
                grant.user_id.clone(),
                serde_json::to_string(&grant.controller_key)?,
            ));
        }
        // A deleted reader withdrew its acceptance.
        let readers = existing_pairs(
            db,
            id,
            &reader_pairs,
            |pairs| format!(r#"SELECT r."userId",r."keyId" FROM "DeviceFleetReader" r JOIN "User" u ON u.id=r."userId" AND u.status='ACTIVE' WHERE r."deviceId"=$1 AND r.deleted=false AND (r."userId",r."keyId") IN ({pairs})"#),
        )
        .await?;
        let backups = existing_pairs(
            db,
            id,
            &backup_pairs,
            |pairs| format!(r#"SELECT v."userId",v."publicKey" FROM "DeviceControllerVault" v JOIN "User" u ON u.id=v."userId" AND u.status='ACTIVE' WHERE v."keyId"=$1 AND (v."userId",v."publicKey") IN ({pairs})"#),
        )
        .await?;
        for (reader, backup) in reader_pairs.into_iter().zip(backup_pairs) {
            if readers.contains(&reader) || backups.contains(&backup) {
                users.push(reader.0);
            }
        }
    }
    users.sort();
    users.dedup();
    Ok(users)
}

/// The rows among `pairs` that exist, with `id` bound as $1. Only the named
/// pairs are loaded, so no row limit can drop a real match.
async fn existing_pairs<C: ConnectionTrait>(
    db: &C,
    id: &str,
    pairs: &[(String, String)],
    query: impl FnOnce(&str) -> String,
) -> Result<std::collections::HashSet<(String, String)>, ApiError> {
    let placeholders = (0..pairs.len())
        .map(|index| format!("(${},${})", 2 * index + 2, 2 * index + 3))
        .collect::<Vec<_>>()
        .join(",");
    let values = std::iter::once(Value::from(id)).chain(
        pairs
            .iter()
            .flat_map(|(user, key)| [Value::from(user.as_str()), Value::from(key.as_str())]),
    );
    db.query_all_raw(sql(&query(&placeholders), values))
        .await?
        .into_iter()
        .map(|row| Ok((row.try_get_by_index(0)?, row.try_get_by_index(1)?)))
        .collect()
}

#[derive(Clone, Debug)]
struct Notice {
    id: String,
    device_id: String,
    certificate_id: String,
    certificate_revision: i64,
    fingerprint: String,
    not_after: i64,
    user_id: String,
    stage: String,
    channel: String,
    attempts: i64,
}

impl Notice {
    fn from_row(row: QueryResult) -> Result<Self, ApiError> {
        Ok(Self {
            id: row.try_get("", "id")?,
            device_id: row.try_get("", "deviceId")?,
            certificate_id: row.try_get("", "certificateId")?,
            certificate_revision: row.try_get("", "certificateRevision")?,
            fingerprint: row.try_get("", "fingerprint")?,
            not_after: row.try_get("", "notAfter")?,
            user_id: row.try_get("", "userId")?,
            stage: row.try_get("", "stage")?,
            channel: row.try_get("", "channel")?,
            attempts: row.try_get("", "attempts")?,
        })
    }
    fn matches(&self, entry: &CertificateInventoryEntry) -> bool {
        self.certificate_id == entry.certificate_id
            && self.certificate_revision as u64 == entry.revision
            && self.fingerprint == entry.fingerprint_sha256
            && self.not_after == entry.not_after
    }
}

fn notice_id(
    device: &str,
    cert: &CertificateInventoryEntry,
    user: &str,
    stage: &str,
    channel: &str,
) -> String {
    let encoded = serde_json::to_vec(&(
        device,
        &cert.certificate_id,
        cert.revision,
        &cert.fingerprint_sha256,
        cert.not_after,
        user,
        stage,
        channel,
    ))
    .expect("serializable certificate identity");
    format!("device-certificate:{}", blake3::hash(&encoded).to_hex())
}

async fn schedule(
    db: &DatabaseConnection,
    dialect: DbDialect,
    device: String,
    now: i64,
) -> Result<(), ApiError> {
    retry_transaction(db, dialect, None, &RetryPolicy::default(), move |tx| {
        let device = device.clone();
        Box::pin(async move {
            let current = repository::current_device(tx, &device).await?;
            if current.status.status != flow_like_device_protocol::DeviceRegistrationStatus::Active { return Ok(()); }
            repository::lock_active_device(tx, &device, current.status.auth_epoch).await?;
            let Some(row) = tx.query_one_raw(sql(r#"SELECT payload FROM "DeviceCertificateInventory" WHERE "deviceId"=$1 AND "nextCheckAt"<=$2"#, [device.clone().into(), now.into()])).await? else { return Ok(()); };
            let inventory: CertificateInventory = serde_json::from_str(&row.try_get::<String>("", "payload")?)?;
            let users = recipients(tx, &device, now).await?;
            for cert in &inventory.certificates {
                let Some(stage) = stage(cert.not_after, now) else { continue; };
                for user in &users {
                    for channel in ["push", "email"] {
                        tx.execute_raw(sql(r#"INSERT INTO "DeviceCertificateNotice"(id,"deviceId","certificateId","certificateRevision",fingerprint,"notAfter","userId",stage,channel,status,attempts,"nextAttemptAt")
                            VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,'pending',0,$10) ON CONFLICT(id) DO UPDATE SET status='pending',"nextAttemptAt"=$10,"completedAt"=NULL WHERE "DeviceCertificateNotice".status='cancelled'"#,
                            [notice_id(&device, cert, user, stage, channel).into(), device.clone().into(), cert.certificate_id.clone().into(), (cert.revision as i64).into(), cert.fingerprint_sha256.clone().into(), cert.not_after.into(), user.clone().into(), stage.into(), channel.into(), now.into()])).await?;
                    }
                }
            }
            tx.execute_raw(sql(r#"UPDATE "DeviceCertificateInventory" SET "nextCheckAt"=$2 WHERE "deviceId"=$1"#, [device.into(), (now + CHECK_SECONDS).into()])).await?;
            Ok(())
        })
    }).await
}

struct Delivery {
    notification_id: String,
    user_id: String,
    email: Option<String>,
    title: String,
    description: String,
    link: String,
}

async fn preflight<C: ConnectionTrait>(
    db: &C,
    notice: &Notice,
    now: i64,
) -> Result<Option<Delivery>, ApiError> {
    if stage(notice.not_after, now) != Some(notice.stage.as_str()) {
        return Ok(None);
    }
    let users = recipients(db, &notice.device_id, now).await?;
    if !users.contains(&notice.user_id) {
        return Ok(None);
    }
    let row = db
        .query_one_raw(sql(
            r#"SELECT i.payload,d.name,u.email FROM "DeviceCertificateInventory" i
        JOIN "ManagedDevice" d ON d.id=i."deviceId" JOIN "User" u ON u.id=$2
        WHERE i."deviceId"=$1"#,
            [
                notice.device_id.clone().into(),
                notice.user_id.clone().into(),
            ],
        ))
        .await?;
    let Some(row) = row else {
        return Ok(None);
    };
    let inventory: CertificateInventory =
        serde_json::from_str(&row.try_get::<String>("", "payload")?)?;
    if !inventory
        .certificates
        .iter()
        .any(|entry| notice.matches(entry))
    {
        return Ok(None);
    }
    let name: String = row.try_get("", "name")?;
    let date = chrono::DateTime::from_timestamp(notice.not_after, 0)
        .map(|date| date.to_rfc3339())
        .unwrap_or_else(|| notice.not_after.to_string());
    let verb = if notice.stage == "expired" {
        "expired"
    } else {
        "expires soon"
    };
    Ok(Some(Delivery {
        notification_id: notice.id.clone(),
        user_id: notice.user_id.clone(),
        email: row.try_get("", "email")?,
        title: format!("Service certificate {verb}: {name}"),
        description: format!(
            "Certificate {} on device {name} expires at {date}. Open device management to unlock the device, inspect its services and replace the certificate.",
            notice.certificate_id
        ),
        link: format!(
            "/settings/devices?device={}",
            urlencoding::encode(&notice.device_id)
        ),
    }))
}

#[async_trait::async_trait]
trait DeliverySink: Send + Sync {
    async fn deliver(&self, channel: &str, delivery: Delivery) -> Result<(), ApiError>;
}

struct NativeDelivery<'a>(&'a AppState);

#[async_trait::async_trait]
impl DeliverySink for NativeDelivery<'_> {
    async fn deliver(&self, channel: &str, delivery: Delivery) -> Result<(), ApiError> {
        match channel {
            "push" => {
                let result = dispatch_notification_idempotent(
                    self.0,
                    &delivery.notification_id,
                    DispatchNotificationInput {
                        user_id: delivery.user_id,
                        app_id: None,
                        title: delivery.title,
                        description: Some(delivery.description),
                        icon: Some("shield-check".into()),
                        image: None,
                        link: Some(delivery.link),
                        notification_type: NotificationType::System,
                        source_run_id: None,
                        source_node_id: None,
                    },
                )
                .await?;
                if !matches!(
                    result.push_status,
                    PushDispatchStatus::Accepted | PushDispatchStatus::NoTargets
                ) {
                    return Err(ApiError::service_unavailable(
                        "Certificate push delivery will be retried",
                    ));
                }
            }
            "email" => {
                let mail = self.0.mail_client.as_ref().ok_or_else(|| {
                    ApiError::service_unavailable("Certificate email delivery is not configured")
                })?;
                let email = delivery
                    .email
                    .filter(|email| !email.trim().is_empty())
                    .ok_or_else(|| {
                        ApiError::service_unavailable("Certificate recipient has no email address")
                    })?;
                let domain = self.0.platform_config.domain.trim_end_matches('/');
                let origin = if domain.contains("://") {
                    domain.to_string()
                } else {
                    format!(
                        "{}://{domain}",
                        if self.0.platform_config.secure {
                            "https"
                        } else {
                            "http"
                        }
                    )
                };
                mail.send(EmailMessage {
                    to: email,
                    subject: delivery.title,
                    body_html: None,
                    body_text: Some(format!(
                        "{}\n\n{}{}\n",
                        delivery.description, origin, delivery.link
                    )),
                })
                .await
                .map_err(|_| {
                    ApiError::service_unavailable("Certificate email delivery will be retried")
                })?;
            }
            _ => return Err(ApiError::internal("Invalid certificate delivery channel")),
        }
        Ok(())
    }
}

fn retry_delay(attempts: i64) -> i64 {
    (30_i64.saturating_mul(1_i64 << attempts.clamp(0, 7))).min(3600)
}

async fn dispatch_pending(
    db: &DatabaseConnection,
    sink: &impl DeliverySink,
    now: i64,
) -> Result<u64, ApiError> {
    let rows = db.query_all_raw(sql(r#"SELECT n.* FROM (SELECT c.*,ROW_NUMBER() OVER (PARTITION BY d."ownerId" ORDER BY c."nextAttemptAt",c.id) AS "ownerRank"
        FROM "DeviceCertificateNotice" c JOIN "ManagedDevice" d ON d.id=c."deviceId"
        WHERE c.status='pending' AND c."nextAttemptAt"<=$1 AND (c."leaseUntil" IS NULL OR c."leaseUntil"<=$1)) n
        WHERE n."ownerRank"<=$2 ORDER BY n."nextAttemptAt",n.id LIMIT 100"#, [now.into(), NOTICES_PER_OWNER_PER_PASS.into()])).await?;
    let mut delivered = 0;
    for row in rows {
        let notice = Notice::from_row(row)?;
        let lease = uuid::Uuid::new_v4().to_string();
        // Earlier deliveries may have waited on providers. Each claim needs its
        // full lease from this attempt, rather than the batch selection time.
        let claim_now = chrono::Utc::now().timestamp();
        let claimed = db.execute_raw(sql(r#"UPDATE "DeviceCertificateNotice" SET "leaseId"=$2,"leaseUntil"=$3,attempts=attempts+1 WHERE id=$1 AND status='pending' AND "nextAttemptAt"<=$4 AND attempts=$5 AND ("leaseUntil" IS NULL OR "leaseUntil"<=$4)"#,
            [notice.id.clone().into(), lease.clone().into(), (claim_now + LEASE_SECONDS).into(), claim_now.into(), notice.attempts.into()])).await?.rows_affected();
        if claimed != 1 {
            continue;
        }
        let result = match preflight(db, &notice, chrono::Utc::now().timestamp()).await {
            Ok(Some(delivery)) => match tokio::time::timeout(
                Duration::from_secs(30),
                sink.deliver(&notice.channel, delivery),
            )
            .await
            {
                Ok(Ok(())) => Some("sent"),
                _ => None,
            },
            Ok(None) => Some("cancelled"),
            Err(_) => None,
        };
        let now = chrono::Utc::now().timestamp();
        if let Some(status) = result {
            db.execute_raw(sql(r#"UPDATE "DeviceCertificateNotice" SET status=$3,"completedAt"=$4,"leaseId"=NULL,"leaseUntil"=NULL WHERE id=$1 AND "leaseId"=$2"#, [notice.id.into(), lease.into(), status.into(), now.into()])).await?;
            if status == "sent" {
                delivered += 1;
            }
        } else {
            db.execute_raw(sql(r#"UPDATE "DeviceCertificateNotice" SET "nextAttemptAt"=$3,"leaseId"=NULL,"leaseUntil"=NULL WHERE id=$1 AND "leaseId"=$2"#, [notice.id.into(), lease.into(), (now + retry_delay(notice.attempts)).into()])).await?;
        }
    }
    Ok(delivered)
}

async fn bounded_sweep(
    budget: Duration,
    work: impl std::future::Future<Output = Result<u64, ApiError>>,
) -> Result<u64, ApiError> {
    tokio::time::timeout(budget, work)
        .await
        .map_err(|_| ApiError::service_unavailable("Certificate reminder pass incomplete; queued deliveries and leases will resume on the next pass"))?
}

pub(crate) async fn sweep(state: &AppState) -> Result<u64, ApiError> {
    if !state.platform_config.standalone.enabled {
        return Ok(0);
    }
    let now = chrono::Utc::now().timestamp();
    let scheduled = bounded_sweep(SCHEDULE_BUDGET, schedule_due(state, now)).await;
    let delivered = bounded_sweep(
        DISPATCH_BUDGET,
        dispatch_pending(&state.db, &NativeDelivery(state), now),
    )
    .await;
    scheduled?;
    delivered
}

async fn schedule_due(state: &AppState, now: i64) -> Result<u64, ApiError> {
    let rows = state.db.query_all_raw(sql(r#"SELECT i."deviceId" FROM "DeviceCertificateInventory" i JOIN "ManagedDevice" d ON d.id=i."deviceId" JOIN "User" u ON u.id=d."ownerId" WHERE i."nextCheckAt"<=$1 AND d.status='active' AND u.status='ACTIVE' ORDER BY i."nextCheckAt",i."deviceId" LIMIT 100"#, [now.into()])).await?;
    let mut scheduled = 0;
    for row in rows.into_iter().take(BATCH) {
        schedule(
            &state.db,
            state.db_dialect,
            row.try_get("", "deviceId")?,
            now,
        )
        .await?;
        scheduled += 1;
    }
    Ok(scheduled)
}

pub fn spawn_sweeper(state: AppState) -> Option<tokio::task::JoinHandle<()>> {
    if !state.platform_config.standalone.enabled {
        return None;
    }
    Some(tokio::spawn(async move {
        let mut ticker = tokio::time::interval(Duration::from_secs(60));
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            ticker.tick().await;
            if let Err(error) = sweep(&state).await {
                tracing::warn!(error = %error, "Device certificate reminder sweep failed");
            }
        }
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use flow_like_device_protocol::{
        DeviceIdentity, DeviceReceipt, ManagementPolicy, SigningKey, compact_digest,
        sign_management_policy,
    };
    use std::{
        collections::HashSet,
        sync::{
            Mutex,
            atomic::{AtomicBool, Ordering},
        },
    };

    fn fixture(now: i64) -> CertificateInventory {
        CertificateInventory {
            version: 1,
            device_id: "device".into(),
            revision: 1,
            issued_at: now,
            certificates: vec![CertificateInventoryEntry {
                certificate_id: "a48aeb81-1321-402e-b09c-8b304f9c3c2f".into(),
                revision: 1,
                fingerprint_sha256: "ab".repeat(32),
                not_after: now + 6 * DAY,
            }],
        }
    }

    #[test]
    fn certificate_access_requires_a_live_device_wide_status_or_certificate_grant() {
        let grant = |scope, capability, expires_at| ManagementGrant {
            grant_id: "grant".into(),
            user_id: "reader".into(),
            controller_key: SigningKey::generate().public_key(),
            scope,
            capabilities: vec![capability],
            expires_at,
            group_id: None,
            group_version: None,
        };
        let project = || ManagementScope::Project {
            project_id: "project".into(),
        };
        assert!(covers_certificates(
            &grant(ManagementScope::Device, ManagementCapability::Status, 20),
            10
        ));
        assert!(covers_certificates(
            &grant(
                ManagementScope::Device,
                ManagementCapability::ManageCertificates,
                20
            ),
            10
        ));
        assert!(!covers_certificates(
            &grant(ManagementScope::Device, ManagementCapability::Logs, 20),
            10
        ));
        assert!(!covers_certificates(
            &grant(project(), ManagementCapability::Status, 20),
            10
        ));
        assert!(!covers_certificates(
            &grant(ManagementScope::Device, ManagementCapability::Status, 10),
            10
        ));
    }

    #[test]
    fn reminders_start_at_seven_days_and_only_emit_the_current_threshold() {
        let expiry = 20 * DAY;
        for (now, expected) in [
            (expiry - 7 * DAY - 1, None),
            (expiry - 7 * DAY, Some("week")),
            (expiry - 3 * DAY - 1, Some("week")),
            (expiry - 3 * DAY, Some("three_days")),
            (expiry - DAY, Some("day")),
            (expiry, Some("expired")),
            (expiry + DAY, Some("expired")),
        ] {
            assert_eq!(stage(expiry, now), expected);
        }
    }

    #[test]
    fn inventory_retries_allow_fresh_proof_timestamps_but_reject_rollback_and_equivocation() {
        let previous = fixture(10);
        let mut retry = previous.clone();
        retry.issued_at += 600;
        assert!(!inventory_transition(&previous, &retry).unwrap());
        retry.certificates[0].not_after += DAY;
        assert!(inventory_transition(&previous, &retry).is_err());
        retry.revision += 1;
        assert!(inventory_transition(&previous, &retry).is_err());
        retry.certificates[0].revision += 1;
        assert!(inventory_transition(&previous, &retry).unwrap());
        assert!(inventory_transition(&retry, &previous).is_err());
        retry.certificates.clear();
        assert!(inventory_transition(&previous, &retry).unwrap());
        retry.revision += 5;
        assert!(inventory_transition(&previous, &retry).unwrap());
    }

    #[test]
    fn delivery_identity_deduplicates_retries_and_separates_rotation_users_and_channels() {
        let inventory = fixture(10);
        let cert = &inventory.certificates[0];
        let id = notice_id("device", cert, "owner", "week", "push");
        assert_eq!(id, notice_id("device", cert, "owner", "week", "push"));
        let mut identities = HashSet::from([id]);
        for (device, user, stage, channel) in [
            ("other", "owner", "week", "push"),
            ("device", "reader", "week", "push"),
            ("device", "owner", "day", "push"),
            ("device", "owner", "week", "email"),
        ] {
            assert!(identities.insert(notice_id(device, cert, user, stage, channel)));
        }
        let mut rotated = cert.clone();
        rotated.revision += 1;
        assert!(identities.insert(notice_id("device", &rotated, "owner", "week", "push")));
        rotated.fingerprint_sha256 = "cd".repeat(32);
        assert!(identities.insert(notice_id("device", &rotated, "owner", "week", "push")));
        assert_eq!(retry_delay(0), 30);
        assert_eq!(retry_delay(1), 60);
        assert_eq!(retry_delay(i64::MAX), 3600);
    }

    #[tokio::test(start_paused = true)]
    async fn slow_reminder_pass_is_cancelled_at_its_budget_and_reports_incomplete() {
        struct Cancelled(std::sync::Arc<AtomicBool>);
        impl Drop for Cancelled {
            fn drop(&mut self) {
                self.0.store(true, Ordering::SeqCst);
            }
        }
        let cancelled = std::sync::Arc::new(AtomicBool::new(false));
        let started = tokio::time::Instant::now();
        let result = bounded_sweep(SCHEDULE_BUDGET, async {
            let _in_flight = Cancelled(cancelled.clone());
            std::future::pending::<Result<u64, ApiError>>().await
        })
        .await;
        assert_eq!(
            result.unwrap_err().status(),
            axum::http::StatusCode::SERVICE_UNAVAILABLE
        );
        assert_eq!(started.elapsed(), SCHEDULE_BUDGET);
        assert!(cancelled.load(Ordering::SeqCst));
        assert_eq!(
            bounded_sweep(DISPATCH_BUDGET, async { Ok(2) })
                .await
                .unwrap(),
            2
        );
        assert!(SCHEDULE_BUDGET + DISPATCH_BUDGET < Duration::from_secs(60));
    }

    #[derive(Default)]
    struct FakeDelivery {
        sent: Mutex<Vec<(String, String, String)>>,
        fail_email: AtomicBool,
        db: Option<DatabaseConnection>,
    }

    #[async_trait::async_trait]
    impl DeliverySink for FakeDelivery {
        async fn deliver(&self, channel: &str, delivery: Delivery) -> Result<(), ApiError> {
            if let Some(db) = &self.db {
                let row = db
                    .query_one_raw(sql(
                        r#"SELECT "leaseUntil" FROM "DeviceCertificateNotice" WHERE id=$1"#,
                        [delivery.notification_id.clone().into()],
                    ))
                    .await?
                    .unwrap();
                assert!(
                    row.try_get::<i64>("", "leaseUntil")? > chrono::Utc::now().timestamp() + 60,
                    "A delayed batch must claim a fresh lease before each delivery"
                );
            }
            if channel == "email" && self.fail_email.load(Ordering::SeqCst) {
                return Err(ApiError::service_unavailable("Fake email outage"));
            }
            self.sent.lock().unwrap().push((
                channel.into(),
                delivery.user_id,
                delivery.notification_id,
            ));
            Ok(())
        }
    }

    async fn count(db: &DatabaseConnection, query: &str) -> i64 {
        db.query_one_raw(sql(query, []))
            .await
            .unwrap()
            .unwrap()
            .try_get("", "count")
            .unwrap()
    }

    #[tokio::test]
    #[ignore = "requires FLOW_LIKE_DEVICE_TEST_DATABASE_URL pointing to disposable PostgreSQL"]
    async fn expiry_outbox_retries_channels_checks_current_access_and_cancels_rotated_certificates()
    {
        use sea_orm::{ConnectOptions, Database};
        let url = std::env::var("FLOW_LIKE_DEVICE_TEST_DATABASE_URL").unwrap();
        let admin = Database::connect(&url).await.unwrap();
        let schema = format!("certificate_test_{}", uuid::Uuid::new_v4().simple());
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
            include_str!("../../prisma/migrations/20260924150000_device_fleet/migration.sql"),
            include_str!(
                "../../prisma/migrations/20260925120000_device_certificates/migration.sql"
            ),
        ] {
            for statement in migration
                .split(';')
                .filter(|statement| !statement.trim().is_empty())
            {
                db.execute_unprepared(statement).await.unwrap();
            }
        }
        db.execute_unprepared(r#"CREATE TABLE "User"(id TEXT PRIMARY KEY,status TEXT NOT NULL,email TEXT);
            INSERT INTO "User" VALUES('owner','ACTIVE','owner@example.invalid'),('reader','ACTIVE','reader@example.invalid'),('group-user','ACTIVE','group@example.invalid'),('expired','ACTIVE',NULL),('disabled','DISABLED',NULL),('scoped','ACTIVE',NULL),('unaccepted','ACTIVE',NULL),('withdrawn','ACTIVE',NULL)"#).await.unwrap();
        let now = chrono::Utc::now().timestamp();
        let identity = DeviceIdentity {
            auth_key: SigningKey::generate().public_key(),
            management_key: [42; 32],
            telemetry_key: SigningKey::generate().public_key(),
        };
        let receipt = DeviceReceipt {
            enrollment_id: "enrollment".into(),
            device_id: "device".into(),
            owner_id: "owner".into(),
            name: "Remote device".into(),
            identity: identity.clone(),
            manifest_jws: "fixture".into(),
            binding_jws: "fixture".into(),
            registered_at: now,
            auth_epoch: 1,
        };
        db.execute_raw(sql(r#"INSERT INTO "ManagedDevice"(id,"ownerId",name,status,"authEpoch",identity,receipt,"registeredAt") VALUES('device','owner','Remote device','active',1,$1,$2,$3)"#, [serde_json::to_string(&identity).unwrap().into(), serde_json::to_string(&receipt).unwrap().into(), now.into()])).await.unwrap();
        let invitation = SigningKey::generate();
        let manifest = OnboardingManifest {
            version: 1,
            enrollment_id: "enrollment".into(),
            device_id: "device".into(),
            owner_id: "owner".into(),
            name: "Remote device".into(),
            api_base_url: "https://hub.example/api/v1".into(),
            bootstrap_key: SigningKey::generate().public_key(),
            controller_key: SigningKey::generate().public_key(),
            owner_invitation_key: invitation.public_key(),
            issued_at: now,
            expires_at: now + 600,
        };
        db.execute_raw(sql(r#"INSERT INTO "DeviceEnrollment"(id,"deviceId","ownerId","jwtId",manifest,status,"expiresAt","createdAt") VALUES('enrollment','device','owner','jwt',$1,'consumed',$2,$3)"#, [serde_json::to_string(&manifest).unwrap().into(), manifest.expires_at.into(), now.into()])).await.unwrap();
        let keys: std::collections::HashMap<&str, SigningKey> = [
            "reader",
            "group-user",
            "expired",
            "disabled",
            "scoped",
            "unaccepted",
            "withdrawn",
        ]
        .into_iter()
        .map(|user| (user, SigningKey::generate()))
        .collect();
        let grant = |user: &str, scope: ManagementScope, capability, expires_at| ManagementGrant {
            grant_id: user.into(),
            user_id: user.into(),
            controller_key: keys[user].public_key(),
            scope,
            capabilities: vec![capability],
            expires_at,
            group_id: None,
            group_version: None,
        };
        let mut policy = ManagementPolicy {
            version: 1,
            device_id: "device".into(),
            policy_version: 1,
            previous_policy_digest: None,
            grants: vec![
                grant(
                    "reader",
                    ManagementScope::Device,
                    ManagementCapability::Status,
                    now + DAY,
                ),
                ManagementGrant {
                    group_id: Some("operators".into()),
                    group_version: Some(1),
                    ..grant(
                        "group-user",
                        ManagementScope::Device,
                        ManagementCapability::ManageCertificates,
                        now + DAY,
                    )
                },
                grant(
                    "expired",
                    ManagementScope::Device,
                    ManagementCapability::Status,
                    now - 1,
                ),
                grant(
                    "disabled",
                    ManagementScope::Device,
                    ManagementCapability::Status,
                    now + DAY,
                ),
                grant(
                    "scoped",
                    ManagementScope::Project {
                        project_id: "project".into(),
                    },
                    ManagementCapability::Status,
                    now + DAY,
                ),
                grant(
                    "unaccepted",
                    ManagementScope::Device,
                    ManagementCapability::Status,
                    now + DAY,
                ),
                grant(
                    "withdrawn",
                    ManagementScope::Device,
                    ManagementCapability::Status,
                    now + DAY,
                ),
            ],
            issued_at: now - 100,
            expires_at: now + 30 * DAY,
        };
        let first_policy = sign_management_policy(&policy, &invitation).unwrap();
        db.execute_raw(sql(r#"INSERT INTO "DeviceManagementPolicy"("deviceId",version,digest,"policyJws","expiresAt","createdAt") VALUES('device',1,$1,$2,$3,$4)"#, [compact_digest(&first_policy).into(), first_policy.clone().into(), policy.expires_at.into(), now.into()])).await.unwrap();
        // Historical reader rows written first must not crowd out real acceptances.
        db.execute_unprepared(r#"INSERT INTO "DeviceFleetReader"("deviceId","userId","keyId",revision,"readerJws",deleted) SELECT 'device','reader','historical-'||n,1,'fixture',false FROM generate_series(1,1100) n"#).await.unwrap();
        // Grantees accept a share by registering the grant's key for this device
        // under their own session, as a fleet reader or as an account backup.
        // Deleting the reader withdraws that acceptance.
        for (user, deleted) in [
            ("reader", false),
            ("expired", false),
            ("disabled", false),
            ("scoped", false),
            ("withdrawn", true),
        ] {
            let key_id = URL_SAFE_NO_PAD.encode(keys[user].public_key().to_bytes().unwrap());
            db.execute_raw(sql(r#"INSERT INTO "DeviceFleetReader"("deviceId","userId","keyId",revision,"readerJws",deleted) VALUES('device',$1,$2,1,'fixture',$3)"#, [user.into(), key_id.into(), deleted.into()])).await.unwrap();
        }
        db.execute_raw(sql(r#"INSERT INTO "DeviceControllerVault"("userId","keyId","publicKey",ciphertext,revision,"updatedAt") VALUES('group-user','device',$1,'fixture',1,$2)"#, [serde_json::to_string(&keys["group-user"].public_key()).unwrap().into(), now.into()])).await.unwrap();
        assert_eq!(
            recipients(&db, "device", now).await.unwrap(),
            vec!["group-user", "owner", "reader"]
        );
        let visible = audience(&db, "device", now).await.unwrap().unwrap();
        assert!(
            visible.admits("owner") && visible.admits("reader") && visible.admits("unaccepted")
        );
        assert!(!visible.admits("scoped") && !visible.admits("expired"));
        db.execute_unprepared(r#"UPDATE "DeviceManagementPolicy" SET "policyJws"='tampered'"#)
            .await
            .unwrap();
        assert_eq!(recipients(&db, "device", now).await.unwrap(), vec!["owner"]);
        db.execute_raw(sql(
            r#"UPDATE "DeviceManagementPolicy" SET "policyJws"=$1"#,
            [first_policy.clone().into()],
        ))
        .await
        .unwrap();
        db.execute_unprepared(r#"UPDATE "User" SET status='PAUSED' WHERE id='owner'"#)
            .await
            .unwrap();
        assert!(recipients(&db, "device", now).await.unwrap().is_empty());
        db.execute_unprepared(r#"UPDATE "User" SET status='ACTIVE' WHERE id='owner'"#)
            .await
            .unwrap();
        let inventory = fixture(now);
        retain(&db, DbDialect::Postgres, 1, inventory.clone())
            .await
            .unwrap();
        schedule(&db, DbDialect::Postgres, "device".into(), now)
            .await
            .unwrap();
        db.execute_raw(sql(
            r#"UPDATE "DeviceCertificateInventory" SET "updatedAt"=$1 WHERE "deviceId"='device'"#,
            [(now - 3600).into()],
        ))
        .await
        .unwrap();
        let mut fresh_proof = inventory.clone();
        fresh_proof.issued_at += 1;
        retain(&db, DbDialect::Postgres, 1, fresh_proof)
            .await
            .unwrap();
        let refreshed = db.query_one_raw(sql(r#"SELECT revision,payload,"updatedAt","nextCheckAt" FROM "DeviceCertificateInventory" WHERE "deviceId"='device'"#, [])).await.unwrap().unwrap();
        assert!(refreshed.try_get::<i64>("", "updatedAt").unwrap() >= now);
        assert_eq!(
            refreshed.try_get::<i64>("", "nextCheckAt").unwrap(),
            now + CHECK_SECONDS
        );
        assert_eq!(refreshed.try_get::<i64>("", "revision").unwrap(), 1);
        assert_eq!(
            refreshed.try_get::<String>("", "payload").unwrap(),
            serde_json::to_string(&inventory).unwrap()
        );
        schedule(&db, DbDialect::Postgres, "device".into(), now)
            .await
            .unwrap();
        assert_eq!(
            count(
                &db,
                r#"SELECT COUNT(*) AS count FROM "DeviceCertificateNotice""#
            )
            .await,
            6
        );

        // A group grant is an explicit resolved recipient. Revoking it after
        // scheduling must cancel both channels before either is dispatched.
        policy.policy_version = 2;
        policy.previous_policy_digest = Some(compact_digest(&first_policy));
        policy.grants.retain(|grant| grant.user_id == "reader");
        let second_policy = sign_management_policy(&policy, &invitation).unwrap();
        db.execute_raw(sql(r#"INSERT INTO "DeviceManagementPolicy"("deviceId",version,digest,"policyJws","expiresAt","createdAt") VALUES('device',2,$1,$2,$3,$4)"#, [compact_digest(&second_policy).into(), second_policy.into(), policy.expires_at.into(), now.into()])).await.unwrap();
        let sink = FakeDelivery {
            db: Some(db.clone()),
            ..Default::default()
        };
        sink.fail_email.store(true, Ordering::SeqCst);
        let old_batch_time = now - 3600;
        db.execute_raw(sql(
            r#"UPDATE "DeviceCertificateNotice" SET "nextAttemptAt"=$1"#,
            [old_batch_time.into()],
        ))
        .await
        .unwrap();
        assert_eq!(
            dispatch_pending(&db, &sink, old_batch_time).await.unwrap(),
            2
        );
        assert_eq!(count(&db, r#"SELECT COUNT(*) AS count FROM "DeviceCertificateNotice" WHERE status='cancelled'"#).await, 2);
        assert_eq!(
            count(
                &db,
                r#"SELECT COUNT(*) AS count FROM "DeviceCertificateNotice" WHERE status='pending'"#
            )
            .await,
            2
        );
        assert!(
            sink.sent
                .lock()
                .unwrap()
                .iter()
                .all(|(channel, user, _)| channel == "push" && user != "group-user")
        );
        assert_eq!(dispatch_pending(&db, &sink, now).await.unwrap(), 0);
        sink.fail_email.store(false, Ordering::SeqCst);
        db.execute_raw(sql(
            r#"UPDATE "DeviceCertificateNotice" SET "nextAttemptAt"=$1 WHERE status='pending'"#,
            [now.into()],
        ))
        .await
        .unwrap();
        assert_eq!(dispatch_pending(&db, &sink, now).await.unwrap(), 2);
        assert_eq!(sink.sent.lock().unwrap().len(), 4);

        db.execute_unprepared(r#"UPDATE "DeviceCertificateInventory" SET "nextCheckAt"=0"#)
            .await
            .unwrap();
        schedule(&db, DbDialect::Postgres, "device".into(), now)
            .await
            .unwrap();
        assert_eq!(dispatch_pending(&db, &sink, now).await.unwrap(), 0);
        let mut rotated = inventory.clone();
        rotated.revision = 2;
        rotated.certificates[0].revision = 2;
        rotated.certificates[0].fingerprint_sha256 = "cd".repeat(32);
        retain(&db, DbDialect::Postgres, 1, rotated.clone())
            .await
            .unwrap();
        assert_eq!(
            count(
                &db,
                r#"SELECT COUNT(*) AS count FROM "DeviceCertificateNotice""#
            )
            .await,
            0
        );
        assert!(
            retain(&db, DbDialect::Postgres, 1, inventory)
                .await
                .is_err()
        );
        // A material change waits for the device's next check instead of
        // scheduling immediately, so rewriting inventories cannot flood recipients.
        schedule(&db, DbDialect::Postgres, "device".into(), now)
            .await
            .unwrap();
        assert_eq!(
            count(
                &db,
                r#"SELECT COUNT(*) AS count FROM "DeviceCertificateNotice""#
            )
            .await,
            0
        );
        let next_check = now + CHECK_SECONDS;
        schedule(&db, DbDialect::Postgres, "device".into(), next_check)
            .await
            .unwrap();
        assert_eq!(
            count(
                &db,
                r#"SELECT COUNT(*) AS count FROM "DeviceCertificateNotice""#
            )
            .await,
            4
        );
        // Device revocation is checked at delivery, including while it is offline.
        db.execute_unprepared(r#"UPDATE "ManagedDevice" SET status='revoked'"#)
            .await
            .unwrap();
        assert_eq!(dispatch_pending(&db, &sink, next_check).await.unwrap(), 0);
        assert_eq!(sink.sent.lock().unwrap().len(), 4);
        db.execute_unprepared(r#"UPDATE "ManagedDevice" SET status='active'"#)
            .await
            .unwrap();
        rotated.revision = 3;
        rotated.certificates.clear();
        retain(&db, DbDialect::Postgres, 1, rotated).await.unwrap();
        assert_eq!(
            count(
                &db,
                r#"SELECT COUNT(*) AS count FROM "DeviceCertificateNotice""#
            )
            .await,
            0
        );
        assert!(view(&db, "device").await.unwrap().certificates.is_empty());
        db.close().await.unwrap();
        admin
            .execute_unprepared(&format!("DROP SCHEMA {schema} CASCADE"))
            .await
            .unwrap();
    }
}
