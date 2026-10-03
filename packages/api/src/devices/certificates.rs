use super::{
    DeviceContext, context, device_principal, enabled, human_owner, management, repository,
};
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
    extract::{Path, Query, State},
    http::{HeaderMap, HeaderValue, StatusCode, header::RETRY_AFTER},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use flow_like_device_protocol::{
    CertificateInventory, CertificateInventoryEntry, MAX_DEVICE_CERTIFICATES, ManagementCapability,
    ManagementGrant, ManagementScope, OnboardingManifest, validate_certificate_id,
    verify_certificate_inventory, verify_management_policy,
};
use flow_like_types::tokio;
use sea_orm::{
    ConnectionTrait, DatabaseBackend, DatabaseConnection, FromQueryResult, QueryResult, Statement,
    Value,
};
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, time::Duration};

const DAY: i64 = 86_400;
const CHECK_SECONDS: i64 = 300;
const LEASE_SECONDS: i64 = 120;
const BATCH: usize = 100;
// Scheduling and delivery have separate budgets, so a burst of inventories
// cannot use up the pass that delivers reminders.
const SCHEDULE_BUDGET: Duration = Duration::from_secs(20);
const DISPATCH_BUDGET: Duration = Duration::from_secs(25);
const DELIVERY_TIMEOUT: Duration = Duration::from_secs(30);
/// One owner's devices cannot take more than this share of a delivery pass.
const NOTICES_PER_OWNER_PER_PASS: i64 = 10;
const CHANNELS: [&str; 2] = ["push", "email"];
const NOTICE_PAGE: u32 = 50;
const MAX_NOTICE_PAGE: u32 = 200;
const FLEET_LIMIT: i64 = 1000;
/// Mutes every certificate of a device.
const ALL_CERTIFICATES: &str = "*";
const TEST_STAGE: &str = "test";
/// One test reminder per account and device in this many seconds.
const TEST_SECONDS: i64 = 600;
/// A test names no certificate; clients still read a UUID in that field.
const NO_CERTIFICATE: &str = "00000000-0000-0000-0000-000000000000";

fn sql(query: &str, values: impl IntoIterator<Item = Value>) -> Statement {
    Statement::from_sql_and_values(DatabaseBackend::Postgres, query, values)
}

/// `$first`, `$first+1`, … for `count` values.
fn placeholders(first: usize, count: usize) -> String {
    (first..first + count)
        .map(|index| format!("${index}"))
        .collect::<Vec<_>>()
        .join(",")
}

pub(crate) fn routes() -> Router<AppState> {
    Router::new()
        .route("/certificate-inventory", get(fleet_certificate_inventory))
        .route("/{id}/certificate-inventory", get(read).put(write))
        .route("/{id}/certificate-notices", get(certificate_notices))
        .route(
            "/{id}/certificate-notices/mute",
            get(certificate_notice_mutes)
                .put(mute_certificate_notices)
                .delete(unmute_certificate_notices),
        )
        .route(
            "/{id}/certificate-notices/test",
            post(test_certificate_notice),
        )
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct InventoryWrite {
    inventory_jws: String,
}

/// The public facts of one service certificate. Nothing else about it leaves the device.
#[derive(Serialize, utoipa::ToSchema)]
struct CertificateFacts {
    certificate_id: String,
    revision: u64,
    fingerprint_sha256: String,
    not_after: i64,
}

impl From<CertificateInventoryEntry> for CertificateFacts {
    fn from(entry: CertificateInventoryEntry) -> Self {
        Self {
            certificate_id: entry.certificate_id,
            revision: entry.revision,
            fingerprint_sha256: entry.fingerprint_sha256,
            not_after: entry.not_after,
        }
    }
}

#[derive(Serialize, utoipa::ToSchema)]
struct CertificateInventoryView {
    revision: u64,
    updated_at: Option<i64>,
    certificates: Vec<CertificateFacts>,
}

impl CertificateInventoryView {
    fn unreported() -> Self {
        Self {
            revision: 0,
            updated_at: None,
            certificates: vec![],
        }
    }

    /// Reads `payload` and `updatedAt`; both are NULL until the device reports.
    fn from_row(row: &QueryResult) -> Result<Self, ApiError> {
        let Some(payload) = row.try_get::<Option<String>>("", "payload")? else {
            return Ok(Self::unreported());
        };
        let inventory: CertificateInventory = serde_json::from_str(&payload)?;
        Ok(Self {
            revision: inventory.revision,
            updated_at: row.try_get("", "updatedAt")?,
            certificates: inventory.certificates.into_iter().map(Into::into).collect(),
        })
    }
}

async fn view<C: ConnectionTrait>(db: &C, id: &str) -> Result<CertificateInventoryView, ApiError> {
    let row = db
        .query_one_raw(sql(
            r#"SELECT payload,"updatedAt" FROM "DeviceCertificateInventory" WHERE "deviceId"=$1"#,
            [id.into()],
        ))
        .await?;
    row.as_ref().map_or_else(
        || Ok(CertificateInventoryView::unreported()),
        CertificateInventoryView::from_row,
    )
}

/// Whether `user` may read the certificates of device `id`. A device that does not exist
/// is refused like one the account has no access to, so asking never confirms a device.
async fn readable(state: &DeviceContext<'_>, user: &str, id: &str) -> Result<(), ApiError> {
    management::admitted(state, user, id)
        .await
        .map_err(management::no_access)?;
    let audience = audience(state.db, id, chrono::Utc::now().timestamp())
        .await?
        .ok_or(ApiError::FORBIDDEN)?;
    if !audience.admits(user) {
        return Err(ApiError::forbidden(
            "Certificates require device-wide Status or ManageCertificates access",
        ));
    }
    Ok(())
}

/// The caller's account when it may read this device's certificates.
async fn certificate_reader(
    state: &AppState,
    user: &AppUser,
    id: &str,
) -> Result<String, ApiError> {
    let context = context(state);
    enabled(&context)?;
    let user = human_owner(state, user).await?;
    readable(&context, &user, id).await?;
    Ok(user)
}

async fn read(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(id): Path<String>,
) -> Result<Json<CertificateInventoryView>, ApiError> {
    certificate_reader(&state, &user, &id).await?;
    Ok(Json(view(&state.db, &id).await?))
}

#[derive(Serialize, utoipa::ToSchema)]
struct FleetCertificateInventory {
    device_id: String,
    #[serde(flatten)]
    inventory: CertificateInventoryView,
}

impl FleetCertificateInventory {
    fn from_row(row: &QueryResult) -> Result<Self, ApiError> {
        Ok(Self {
            device_id: row.try_get("", "id")?,
            inventory: CertificateInventoryView::from_row(row)?,
        })
    }
}

/// Certificates of every device the caller may read them on: their own active
/// devices, then the `shared` ones whose current access rules admit them. A
/// device that has not reported is listed without certificates.
async fn fleet_inventory<C: ConnectionTrait>(
    db: &C,
    user: &str,
    shared: &[&str],
    now: i64,
) -> Result<Vec<FleetCertificateInventory>, ApiError> {
    const ROWS: &str = r#"SELECT d.id,i.payload,i."updatedAt" FROM "ManagedDevice" d LEFT JOIN "DeviceCertificateInventory" i ON i."deviceId"=d.id WHERE d.status='active' AND"#;
    const ORDER: &str = r#"ORDER BY d."registeredAt" DESC,d.id"#;
    let mut rows = db
        .query_all_raw(sql(
            &format!(r#"{ROWS} d."ownerId"=$1 {ORDER} LIMIT {FLEET_LIMIT}"#),
            [user.into()],
        ))
        .await?;
    let audiences = audiences(db, shared, now).await?;
    let admitted: Vec<&str> = shared
        .iter()
        .copied()
        .filter(|id| {
            audiences
                .get(*id)
                .is_some_and(|audience| audience.owner != user && audience.admits(user))
        })
        .collect();
    for chunk in admitted.chunks(BATCH) {
        rows.extend(
            db.query_all_raw(sql(
                &format!("{ROWS} d.id IN ({}) {ORDER}", placeholders(1, chunk.len())),
                chunk.iter().map(|id| Value::from(*id)),
            ))
            .await?,
        );
    }
    rows.iter()
        .map(FleetCertificateInventory::from_row)
        .collect()
}

#[utoipa::path(
    get,
    path = "/devices/certificate-inventory",
    tag = "devices",
    description = "List the service certificates of every device whose certificates you can read, in one call: your own devices and devices shared with you with whole-device View status or Manage certificates access.",
    responses(
        (status = 200, description = "One entry per device; a device that has not reported yet has revision 0 and no certificates", body = Vec<FleetCertificateInventory>),
        (status = 403, description = "Your account or token cannot manage devices"),
        (status = 503, description = "Devices are not enabled on this hub")
    ),
    security(("bearer_auth" = []), ("pat" = []))
)]
async fn fleet_certificate_inventory(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
) -> Result<Json<Vec<FleetCertificateInventory>>, ApiError> {
    enabled(&context(&state))?;
    let user = human_owner(&state, &user).await?;
    let shared = management::shared_devices(&state, &user).await?;
    let shared: Vec<&str> = shared
        .iter()
        .map(|device| device.device_id.as_str())
        .collect();
    Ok(Json(
        fleet_inventory(&state.db, &user, &shared, chrono::Utc::now().timestamp()).await?,
    ))
}

async fn write(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Json(request): Json<InventoryWrite>,
) -> Result<Json<CertificateInventoryView>, ApiError> {
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
            // Test reminders stay: deleting them would lift the limit on sending tests.
            let notices = tx.query_all_raw(sql(r#"SELECT * FROM "DeviceCertificateNotice" WHERE "deviceId"=$1 AND stage<>$2"#, [inventory.device_id.clone().into(), TEST_STAGE.into()])).await?;
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
fn certificate_grants(
    device: &str,
    manifest: Option<String>,
    policy: Option<String>,
    now: i64,
) -> Result<Vec<ManagementGrant>, ApiError> {
    let (Some(manifest), Some(policy)) = (manifest, policy) else {
        return Ok(Vec::new());
    };
    let manifest: OnboardingManifest = serde_json::from_str(&manifest)?;
    Ok(
        verify_management_policy(&policy, &manifest.owner_invitation_key, now)
            .ok()
            .filter(|policy| policy.device_id == device)
            .map(|policy| {
                policy
                    .grants
                    .into_iter()
                    .filter(|grant| covers_certificates(grant, now))
                    .collect()
            })
            .unwrap_or_default(),
    )
}

/// The audience of each active device among `ids`.
async fn audiences<C: ConnectionTrait>(
    db: &C,
    ids: &[&str],
    now: i64,
) -> Result<HashMap<String, CertificateAudience>, ApiError> {
    let mut audiences = HashMap::with_capacity(ids.len());
    for chunk in ids.chunks(BATCH) {
        let rows = db.query_all_raw(sql(&format!(r#"SELECT d.id,d."ownerId",e.manifest,p."policyJws" FROM "ManagedDevice" d
            JOIN "User" owner ON owner.id=d."ownerId" AND owner.status='ACTIVE'
            LEFT JOIN "DeviceEnrollment" e ON e."deviceId"=d.id
            LEFT JOIN "DeviceManagementPolicy" p ON p."deviceId"=d.id AND p.version=(SELECT MAX(version) FROM "DeviceManagementPolicy" WHERE "deviceId"=d.id)
            WHERE d.id IN ({}) AND d.status='active'"#, placeholders(1, chunk.len())), chunk.iter().map(|id| Value::from(*id)))).await?;
        for row in rows {
            let id: String = row.try_get("", "id")?;
            let audience = CertificateAudience {
                owner: row.try_get("", "ownerId")?,
                grants: certificate_grants(
                    &id,
                    row.try_get("", "manifest")?,
                    row.try_get("", "policyJws")?,
                    now,
                )?,
            };
            audiences.insert(id, audience);
        }
    }
    Ok(audiences)
}

async fn audience<C: ConnectionTrait>(
    db: &C,
    id: &str,
    now: i64,
) -> Result<Option<CertificateAudience>, ApiError> {
    Ok(audiences(db, &[id], now).await?.remove(id))
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

/// Active reminder mutes of a device: account → certificate → when the mute
/// ends. `ALL_CERTIFICATES` stands for every certificate of the device.
type Mutes = HashMap<String, HashMap<String, Option<i64>>>;

/// An account mutes the whole device and each certificate it reports at most once.
const MUTES_PER_ACCOUNT: usize = MAX_DEVICE_CERTIFICATES + 1;

async fn mutes<C: ConnectionTrait>(
    db: &C,
    device: &str,
    users: &[impl AsRef<str>],
    now: i64,
) -> Result<Mutes, ApiError> {
    let mut mutes = Mutes::new();
    if users.is_empty() {
        return Ok(mutes);
    }
    let scope: [Value; 2] = [device.into(), now.into()];
    let rows = db
        .query_all_raw(sql(
            &format!(
                r#"SELECT "userId","certificateId",until FROM "DeviceCertificateNoticeMute" WHERE "deviceId"=$1 AND (until IS NULL OR until>$2) AND "userId" IN ({}) LIMIT {}"#,
                placeholders(3, users.len()),
                users.len() * MUTES_PER_ACCOUNT
            ),
            scope
                .into_iter()
                .chain(users.iter().map(|user| Value::from(user.as_ref()))),
        ))
        .await?;
    for row in rows {
        mutes
            .entry(row.try_get("", "userId")?)
            .or_default()
            .insert(row.try_get("", "certificateId")?, row.try_get("", "until")?);
    }
    Ok(mutes)
}

fn muted(mutes: &Mutes, user: &str, certificate: &str) -> bool {
    mutes.get(user).is_some_and(|muted| {
        muted.contains_key(certificate) || muted.contains_key(ALL_CERTIFICATES)
    })
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
            let mutes = mutes(tx, &device, &users, now).await?;
            for cert in &inventory.certificates {
                let Some(stage) = stage(cert.not_after, now) else { continue; };
                for user in users.iter().filter(|user| !muted(&mutes, user, &cert.certificate_id)) {
                    for channel in CHANNELS {
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

/// Where a reminder takes its reader: the device's certificates, focused on one of them.
fn certificates_link(device: &str, certificate: Option<&str>) -> String {
    let mut link = format!(
        "/settings/devices?device={}&tab=certificates",
        urlencoding::encode(device)
    );
    if let Some(certificate) = certificate {
        link.push_str("&certificate=");
        link.push_str(&urlencoding::encode(certificate));
    }
    link
}

/// Whether the account still receives this device's reminders and has not muted this one.
async fn receives<C: ConnectionTrait>(db: &C, notice: &Notice, now: i64) -> Result<bool, ApiError> {
    if !recipients(db, &notice.device_id, now)
        .await?
        .contains(&notice.user_id)
    {
        return Ok(false);
    }
    let mutes = mutes(db, &notice.device_id, &[&notice.user_id], now).await?;
    Ok(!muted(&mutes, &notice.user_id, &notice.certificate_id))
}

fn expiry_delivery(notice: &Notice, device: &str, email: Option<String>) -> Delivery {
    let date = chrono::DateTime::from_timestamp(notice.not_after, 0)
        .map(|date| date.to_rfc3339())
        .unwrap_or_else(|| notice.not_after.to_string());
    let verb = if notice.stage == "expired" {
        "expired"
    } else {
        "expires soon"
    };
    Delivery {
        notification_id: notice.id.clone(),
        user_id: notice.user_id.clone(),
        email,
        title: format!("Service certificate {verb}: {device}"),
        description: format!(
            "Certificate {} on device {device} expires at {date}. Open device management to unlock the device, inspect its services and replace the certificate.",
            notice.certificate_id
        ),
        link: certificates_link(&notice.device_id, Some(&notice.certificate_id)),
    }
}

async fn preflight<C: ConnectionTrait>(
    db: &C,
    notice: &Notice,
    now: i64,
) -> Result<Option<Delivery>, ApiError> {
    if stage(notice.not_after, now) != Some(notice.stage.as_str())
        || !receives(db, notice, now).await?
    {
        return Ok(None);
    }
    let row = db
        .query_one_raw(sql(
            r#"SELECT i.payload,COALESCE(d."displayName",d.name) AS name,u.email FROM "DeviceCertificateInventory" i
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
    let device: String = row.try_get("", "name")?;
    Ok(Some(expiry_delivery(
        notice,
        &device,
        row.try_get("", "email")?,
    )))
}

/// Why a channel did not deliver. The reminder sweep retries every cause; a
/// test reminder reports it to the caller.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Undelivered {
    NotConfigured,
    NoEmail,
    Failed,
}

impl Undelivered {
    fn reason(self) -> &'static str {
        match self {
            Self::NotConfigured => "not_configured",
            Self::NoEmail => "no_email",
            Self::Failed => "failed",
        }
    }

    /// Whether a provider was asked to deliver before it failed.
    fn attempted(self) -> bool {
        self == Self::Failed
    }
}

#[async_trait::async_trait]
trait DeliverySink: Send + Sync {
    async fn deliver(&self, channel: &str, delivery: Delivery) -> Result<(), Undelivered>;
}

struct NativeDelivery<'a>(&'a AppState);

impl NativeDelivery<'_> {
    async fn push(&self, delivery: Delivery) -> Result<(), Undelivered> {
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
        .await
        .map_err(|error| {
            tracing::warn!(%error, notice = %delivery.notification_id, "Certificate notification was not stored");
            Undelivered::Failed
        })?;
        match result.push_status {
            PushDispatchStatus::Accepted | PushDispatchStatus::NoTargets => Ok(()),
            PushDispatchStatus::Disabled => Err(Undelivered::NotConfigured),
            status => {
                tracing::warn!(?status, notice = %delivery.notification_id, "Certificate push delivery failed");
                Err(Undelivered::Failed)
            }
        }
    }

    fn origin(&self) -> String {
        let config = &self.0.platform_config;
        let domain = config.domain.trim_end_matches('/');
        if domain.contains("://") {
            return domain.to_string();
        }
        let scheme = if config.secure { "https" } else { "http" };
        format!("{scheme}://{domain}")
    }

    async fn email(&self, delivery: Delivery) -> Result<(), Undelivered> {
        let mail = self
            .0
            .mail_client
            .as_ref()
            .ok_or(Undelivered::NotConfigured)?;
        let email = delivery
            .email
            .filter(|email| !email.trim().is_empty())
            .ok_or(Undelivered::NoEmail)?;
        mail.send(EmailMessage {
            to: email,
            subject: delivery.title,
            body_html: None,
            body_text: Some(format!(
                "{}\n\n{}{}\n",
                delivery.description,
                self.origin(),
                delivery.link
            )),
        })
        .await
        .map_err(|_| {
            tracing::warn!(notice = %delivery.notification_id, "Certificate email delivery failed");
            Undelivered::Failed
        })
    }
}

#[async_trait::async_trait]
impl DeliverySink for NativeDelivery<'_> {
    async fn deliver(&self, channel: &str, delivery: Delivery) -> Result<(), Undelivered> {
        match channel {
            "push" => self.push(delivery).await,
            "email" => self.email(delivery).await,
            _ => {
                tracing::error!(channel, notice = %delivery.notification_id, "Invalid certificate delivery channel");
                Err(Undelivered::Failed)
            }
        }
    }
}

async fn deliver(
    sink: &impl DeliverySink,
    channel: &str,
    delivery: Delivery,
) -> Result<(), Undelivered> {
    tokio::time::timeout(DELIVERY_TIMEOUT, sink.deliver(channel, delivery))
        .await
        .unwrap_or(Err(Undelivered::Failed))
}

/// Ends a delivery its holder leased; a lease that lapsed meanwhile belongs to the sweep.
async fn complete<C: ConnectionTrait>(
    db: &C,
    id: &str,
    lease: &str,
    status: &str,
) -> Result<(), ApiError> {
    db.execute_raw(sql(r#"UPDATE "DeviceCertificateNotice" SET status=$3,"completedAt"=$4,"leaseId"=NULL,"leaseUntil"=NULL WHERE id=$1 AND "leaseId"=$2"#, [id.into(), lease.into(), status.into(), chrono::Utc::now().timestamp().into()])).await?;
    Ok(())
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
            Ok(Some(delivery)) => deliver(sink, &notice.channel, delivery)
                .await
                .ok()
                .map(|()| "sent"),
            Ok(None) => Some("cancelled"),
            Err(_) => None,
        };
        if let Some(status) = result {
            complete(db, &notice.id, &lease, status).await?;
            if status == "sent" {
                delivered += 1;
            }
        } else {
            let now = chrono::Utc::now().timestamp();
            db.execute_raw(sql(r#"UPDATE "DeviceCertificateNotice" SET "nextAttemptAt"=$3,"leaseId"=NULL,"leaseUntil"=NULL WHERE id=$1 AND "leaseId"=$2"#, [notice.id.into(), lease.into(), (now + retry_delay(notice.attempts)).into()])).await?;
        }
    }
    Ok(delivered)
}

/// The caller's own reminders for a device, newest first. With `certificate`
/// only the reminders about that certificate, which a test never is.
async fn notices<C: ConnectionTrait>(
    db: &C,
    device: &str,
    user: &str,
    certificate: Option<&str>,
    limit: u32,
) -> Result<Vec<CertificateNotice>, ApiError> {
    Ok(CertificateNotice::find_by_statement(sql(
        r#"SELECT "certificateId" AS certificate_id,"certificateRevision" AS certificate_revision,"notAfter" AS not_after,stage,channel,status,attempts,"completedAt" AS completed_at,
        CASE WHEN status='pending' THEN "nextAttemptAt" END AS next_attempt_at FROM "DeviceCertificateNotice"
        WHERE "deviceId"=$1 AND "userId"=$2 AND ($3::text IS NULL OR ("certificateId"=$3 AND stage<>$4))
        ORDER BY COALESCE("completedAt","nextAttemptAt") DESC,id LIMIT $5"#,
        [
            device.into(),
            user.into(),
            certificate.map(str::to_owned).into(),
            TEST_STAGE.into(),
            i64::from(limit).into(),
        ],
    ))
    .all(db)
    .await?)
}

/// One reminder the hub queued or sent to the caller.
#[derive(Debug, Serialize, FromQueryResult, utoipa::ToSchema)]
struct CertificateNotice {
    /// All zeros for a test reminder, which names no certificate.
    certificate_id: String,
    #[schema(minimum = 0)]
    certificate_revision: i64,
    not_after: i64,
    /// `week`, `three_days`, `day`, `expired` or `test`.
    stage: String,
    /// `push` or `email`.
    channel: String,
    /// `pending`, `sent` or `cancelled`.
    status: String,
    #[schema(minimum = 0)]
    attempts: i64,
    completed_at: Option<i64>,
    /// When a pending reminder is tried next.
    next_attempt_at: Option<i64>,
}

#[derive(Deserialize, utoipa::IntoParams)]
#[serde(deny_unknown_fields)]
#[into_params(parameter_in = Query)]
struct NoticeQuery {
    /// Only the reminders about this certificate.
    certificate: Option<String>,
    /// How many of the newest reminders to return, up to 200. Defaults to 50.
    limit: Option<u32>,
}

fn certificate_id(id: &str) -> Result<&str, ApiError> {
    validate_certificate_id(id)
        .map(|()| id)
        .map_err(|_| ApiError::bad_request("Certificate ID must be a canonical UUID"))
}

#[utoipa::path(
    get,
    path = "/devices/{id}/certificate-notices",
    tag = "devices",
    description = "List the certificate expiry reminders the hub queued or sent to you for a device, newest first. Reminders to other people are never included.",
    params(("id" = String, Path, description = "Device ID"), NoticeQuery),
    responses(
        (status = 200, description = "Your reminders for this device", body = Vec<CertificateNotice>),
        (status = 400, description = "The certificate ID is not a canonical UUID"),
        (status = 403, description = "You cannot read this device's certificates, or there is no such device")
    ),
    security(("bearer_auth" = []), ("pat" = []))
)]
async fn certificate_notices(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(id): Path<String>,
    Query(query): Query<NoticeQuery>,
) -> Result<Json<Vec<CertificateNotice>>, ApiError> {
    let user = certificate_reader(&state, &user, &id).await?;
    let certificate = query
        .certificate
        .as_deref()
        .map(certificate_id)
        .transpose()?;
    let limit = query.limit.unwrap_or(NOTICE_PAGE).clamp(1, MAX_NOTICE_PAGE);
    Ok(Json(
        notices(&state.db, &id, &user, certificate, limit).await?,
    ))
}

/// A reminder mute of one account for one device.
#[derive(Debug, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
struct CertificateNoticeMute {
    /// The muted certificate; `null` mutes every certificate of the device.
    #[serde(default)]
    certificate_id: Option<String>,
    /// Unix time the mute ends; `null` mutes until it is lifted.
    #[serde(default)]
    until: Option<i64>,
}

/// What a mute request silences: `ALL_CERTIFICATES`, or one certificate the device reports.
fn mute_target<'a>(
    request: &'a CertificateNoticeMute,
    reported: &[CertificateFacts],
    now: i64,
) -> Result<&'a str, ApiError> {
    if request.until.is_some_and(|until| until <= now) {
        return Err(ApiError::bad_request("A mute must end in the future"));
    }
    let Some(id) = request.certificate_id.as_deref() else {
        return Ok(ALL_CERTIFICATES);
    };
    certificate_id(id)?;
    if !reported.iter().any(|entry| entry.certificate_id == id) {
        return Err(ApiError::not_found(
            "The device has not reported this certificate",
        ));
    }
    Ok(id)
}

/// Drops an account's lapsed mutes and its mutes of certificates the device no
/// longer reports, so it holds at most one row per certificate and device.
async fn prune_mutes<C: ConnectionTrait>(
    db: &C,
    device: &str,
    user: &str,
    reported: &[CertificateFacts],
    now: i64,
) -> Result<(), ApiError> {
    let kept = if reported.is_empty() {
        String::new()
    } else {
        format!(
            r#" AND "certificateId" NOT IN ({})"#,
            placeholders(5, reported.len())
        )
    };
    let scope: [Value; 4] = [
        user.into(),
        device.into(),
        now.into(),
        ALL_CERTIFICATES.into(),
    ];
    db.execute_raw(sql(
        &format!(r#"DELETE FROM "DeviceCertificateNoticeMute" WHERE "userId"=$1 AND "deviceId"=$2 AND ((until IS NOT NULL AND until<=$3) OR ("certificateId"<>$4{kept}))"#),
        scope.into_iter().chain(
            reported
                .iter()
                .map(|entry| Value::from(entry.certificate_id.as_str())),
        ),
    ))
    .await?;
    Ok(())
}

/// Mutes the caller's reminders for one reported certificate, or for the whole device.
async fn mute<C: ConnectionTrait>(
    db: &C,
    device: &str,
    user: &str,
    request: CertificateNoticeMute,
    now: i64,
) -> Result<CertificateNoticeMute, ApiError> {
    let reported = view(db, device).await?.certificates;
    let target = mute_target(&request, &reported, now)?;
    prune_mutes(db, device, user, &reported, now).await?;
    db.execute_raw(sql(
        r#"INSERT INTO "DeviceCertificateNoticeMute"("userId","deviceId","certificateId",until,"createdAt") VALUES($1,$2,$3,$4,$5)
        ON CONFLICT("userId","deviceId","certificateId") DO UPDATE SET until=excluded.until"#,
        [
            user.into(),
            device.into(),
            target.into(),
            request.until.into(),
            now.into(),
        ],
    ))
    .await?;
    Ok(request)
}

async fn unmute<C: ConnectionTrait>(
    db: &C,
    device: &str,
    user: &str,
    certificate: &str,
) -> Result<(), ApiError> {
    db.execute_raw(sql(
        r#"DELETE FROM "DeviceCertificateNoticeMute" WHERE "userId"=$1 AND "deviceId"=$2 AND "certificateId"=$3"#,
        [user.into(), device.into(), certificate.into()],
    ))
    .await?;
    Ok(())
}

/// The caller's active mutes for a device, the device-wide one first.
async fn own_mutes<C: ConnectionTrait>(
    db: &C,
    device: &str,
    user: &str,
    now: i64,
) -> Result<Vec<CertificateNoticeMute>, ApiError> {
    let mut own: Vec<_> = mutes(db, device, &[user], now)
        .await?
        .remove(user)
        .unwrap_or_default()
        .into_iter()
        .collect();
    own.sort();
    Ok(own
        .into_iter()
        .map(|(certificate, until)| CertificateNoticeMute {
            certificate_id: (certificate != ALL_CERTIFICATES).then_some(certificate),
            until,
        })
        .collect())
}

#[utoipa::path(
    get,
    path = "/devices/{id}/certificate-notices/mute",
    tag = "devices",
    description = "List your active mutes of certificate expiry reminders for a device.",
    params(("id" = String, Path, description = "Device ID")),
    responses(
        (status = 200, description = "Your active mutes; a null certificate_id mutes the whole device", body = Vec<CertificateNoticeMute>),
        (status = 403, description = "You cannot read this device's certificates, or there is no such device")
    ),
    security(("bearer_auth" = []), ("pat" = []))
)]
async fn certificate_notice_mutes(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(id): Path<String>,
) -> Result<Json<Vec<CertificateNoticeMute>>, ApiError> {
    let user = certificate_reader(&state, &user, &id).await?;
    Ok(Json(
        own_mutes(&state.db, &id, &user, chrono::Utc::now().timestamp()).await?,
    ))
}

#[utoipa::path(
    put,
    path = "/devices/{id}/certificate-notices/mute",
    tag = "devices",
    description = "Stop certificate expiry reminders to you for one certificate of a device or for the whole device, until a time you choose or until you lift the mute. Other people keep receiving their reminders.",
    params(("id" = String, Path, description = "Device ID")),
    request_body = CertificateNoticeMute,
    responses(
        (status = 200, description = "The mute as stored", body = CertificateNoticeMute),
        (status = 400, description = "The certificate ID is not a canonical UUID, or the end time is not in the future"),
        (status = 403, description = "You cannot read this device's certificates, or there is no such device"),
        (status = 404, description = "The device has not reported this certificate")
    ),
    security(("bearer_auth" = []), ("pat" = []))
)]
async fn mute_certificate_notices(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(id): Path<String>,
    Json(request): Json<CertificateNoticeMute>,
) -> Result<Json<CertificateNoticeMute>, ApiError> {
    let user = certificate_reader(&state, &user, &id).await?;
    Ok(Json(
        mute(
            &state.db,
            &id,
            &user,
            request,
            chrono::Utc::now().timestamp(),
        )
        .await?,
    ))
}

#[derive(Deserialize, utoipa::IntoParams)]
#[serde(deny_unknown_fields)]
#[into_params(parameter_in = Query)]
struct UnmuteQuery {
    /// The certificate to unmute. `*`, the default, lifts the device-wide mute.
    certificate: Option<String>,
}

#[utoipa::path(
    delete,
    path = "/devices/{id}/certificate-notices/mute",
    tag = "devices",
    description = "Lift your mute of certificate expiry reminders for one certificate of a device or for the whole device. The next reminder check sends what is currently due.",
    params(("id" = String, Path, description = "Device ID"), UnmuteQuery),
    responses(
        (status = 204, description = "The mute is lifted, or there was none"),
        (status = 400, description = "The certificate is neither * nor a canonical UUID"),
        (status = 403, description = "You cannot read this device's certificates, or there is no such device")
    ),
    security(("bearer_auth" = []), ("pat" = []))
)]
async fn unmute_certificate_notices(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(id): Path<String>,
    Query(query): Query<UnmuteQuery>,
) -> Result<StatusCode, ApiError> {
    let user = certificate_reader(&state, &user, &id).await?;
    let certificate = match query.certificate.as_deref() {
        None | Some(ALL_CERTIFICATES) => ALL_CERTIFICATES,
        Some(certificate) => certificate_id(certificate)?,
    };
    unmute(&state.db, &id, &user, certificate).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Clone, Copy, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
enum CertificateTestNoticeChannel {
    Push,
    Email,
    Both,
}

impl CertificateTestNoticeChannel {
    fn channels(self) -> &'static [&'static str] {
        match self {
            Self::Push => &CHANNELS[..1],
            Self::Email => &CHANNELS[1..],
            Self::Both => &CHANNELS,
        }
    }
}

#[derive(Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
struct CertificateTestNoticeRequest {
    channel: CertificateTestNoticeChannel,
}

#[derive(Debug, Default, PartialEq, Eq, Serialize, utoipa::ToSchema)]
struct CertificateTestNoticeResult {
    /// Channels a provider accepted the test on.
    sent: Vec<&'static str>,
    skipped: Vec<CertificateTestNoticeSkip>,
}

#[derive(Debug, PartialEq, Eq, Serialize, utoipa::ToSchema)]
struct CertificateTestNoticeSkip {
    channel: &'static str,
    /// `not_a_recipient` (reminders reach you only once you hold this device's
    /// keys), `not_configured` (the hub has no provider for the channel),
    /// `no_email` (your account has no email address), `failed` (the provider
    /// refused) or `throttled` (another test of yours is being sent).
    reason: &'static str,
}

impl CertificateTestNoticeResult {
    fn skip(&mut self, channel: &'static str, reason: &'static str) {
        self.skipped
            .push(CertificateTestNoticeSkip { channel, reason });
    }
}

#[derive(Debug, PartialEq, Eq)]
enum TestNotice {
    Sent(CertificateTestNoticeResult),
    /// Seconds until the caller may send another test for this device.
    Throttled(i64),
}

/// A test's identity is the caller, the device and a window of `TEST_SECONDS`,
/// so concurrent requests in one window claim the same row per channel.
fn test_notice_id(device: &str, user: &str, now: i64, channel: &str) -> String {
    let window = CertificateInventoryEntry {
        certificate_id: NO_CERTIFICATE.into(),
        revision: now.div_euclid(TEST_SECONDS).unsigned_abs(),
        fingerprint_sha256: String::new(),
        not_after: 0,
    };
    notice_id(device, &window, user, TEST_STAGE, channel)
}

/// One caller's test reminder for one device. It is recorded like a reminder
/// and leased to the request that sends it, so the sweep leaves it alone.
struct TestReminder<'a> {
    device: &'a str,
    user: &'a str,
    now: i64,
    device_name: String,
    email: Option<String>,
    lease: String,
}

impl<'a> TestReminder<'a> {
    /// Seconds the caller still has to wait since their last test for the device.
    async fn wait(
        db: &DatabaseConnection,
        device: &str,
        user: &str,
        now: i64,
    ) -> Result<Option<i64>, ApiError> {
        let last = db
            .query_one_raw(sql(
                r#"SELECT MAX("nextAttemptAt") AS last FROM "DeviceCertificateNotice" WHERE "deviceId"=$1 AND "userId"=$2 AND stage=$3"#,
                [device.into(), user.into(), TEST_STAGE.into()],
            ))
            .await?
            .map(|row| row.try_get::<Option<i64>>("", "last"))
            .transpose()?
            .flatten();
        Ok(last
            .map(|last| last + TEST_SECONDS - now)
            .filter(|wait| *wait > 0))
    }

    /// Also forgets the caller's tests of more than a day ago.
    async fn prepare(
        db: &DatabaseConnection,
        device: &'a str,
        user: &'a str,
        now: i64,
    ) -> Result<Self, ApiError> {
        let row = db
            .query_one_raw(sql(
                r#"SELECT COALESCE(d."displayName",d.name) AS name,u.email FROM "ManagedDevice" d JOIN "User" u ON u.id=$2 WHERE d.id=$1"#,
                [device.into(), user.into()],
            ))
            .await?
            .ok_or(ApiError::NOT_FOUND)?;
        db.execute_raw(sql(
            r#"DELETE FROM "DeviceCertificateNotice" WHERE "deviceId"=$1 AND "userId"=$2 AND stage=$3 AND "nextAttemptAt"<$4"#,
            [
                device.into(),
                user.into(),
                TEST_STAGE.into(),
                (now - DAY).into(),
            ],
        ))
        .await?;
        Ok(Self {
            device,
            user,
            now,
            device_name: row.try_get("", "name")?,
            email: row.try_get("", "email")?,
            lease: uuid::Uuid::new_v4().to_string(),
        })
    }

    /// False when another request of the caller holds this window's test on the channel.
    async fn claim(
        &self,
        db: &DatabaseConnection,
        id: &str,
        channel: &str,
    ) -> Result<bool, ApiError> {
        let claimed = db
            .execute_raw(sql(
                r#"INSERT INTO "DeviceCertificateNotice"(id,"deviceId","certificateId","certificateRevision",fingerprint,"notAfter","userId",stage,channel,status,attempts,"nextAttemptAt","leaseId","leaseUntil")
            VALUES($1,$2,$3,0,'',0,$4,$5,$6,'pending',1,$7,$8,$9) ON CONFLICT(id) DO NOTHING"#,
                [
                    id.into(),
                    self.device.into(),
                    NO_CERTIFICATE.into(),
                    self.user.into(),
                    TEST_STAGE.into(),
                    channel.into(),
                    self.now.into(),
                    self.lease.clone().into(),
                    (chrono::Utc::now().timestamp() + LEASE_SECONDS).into(),
                ],
            ))
            .await?
            .rows_affected();
        Ok(claimed == 1)
    }

    fn delivery(&self, id: &str) -> Delivery {
        let device = &self.device_name;
        Delivery {
            notification_id: id.into(),
            user_id: self.user.into(),
            email: self.email.clone(),
            title: format!("Test reminder: {device}"),
            description: format!(
                "This is a test of the certificate expiry reminders for device {device}. Real reminders are sent 7 days, 3 days and 1 day before a service certificate expires, and when it has expired."
            ),
            link: certificates_link(self.device, None),
        }
    }

    /// `None` when the channel's test for this window is already claimed. A
    /// channel refused before anything was sent leaves no record, so it does
    /// not start the wait.
    async fn send(
        &self,
        db: &DatabaseConnection,
        sink: &impl DeliverySink,
        channel: &str,
    ) -> Result<Option<Result<(), Undelivered>>, ApiError> {
        let id = test_notice_id(self.device, self.user, self.now, channel);
        if !self.claim(db, &id, channel).await? {
            return Ok(None);
        }
        let outcome = deliver(sink, channel, self.delivery(&id)).await;
        match outcome {
            Ok(()) => complete(db, &id, &self.lease, "sent").await?,
            Err(cause) if cause.attempted() => complete(db, &id, &self.lease, "cancelled").await?,
            Err(_) => {
                db.execute_raw(sql(
                    r#"DELETE FROM "DeviceCertificateNotice" WHERE id=$1 AND "leaseId"=$2"#,
                    [id.into(), self.lease.clone().into()],
                ))
                .await?;
            }
        }
        Ok(Some(outcome))
    }
}

/// Sends one test reminder to the caller only, on the reminder channels.
async fn send_test(
    db: &DatabaseConnection,
    sink: &impl DeliverySink,
    device: &str,
    user: &str,
    channels: &[&'static str],
    now: i64,
) -> Result<TestNotice, ApiError> {
    let mut result = CertificateTestNoticeResult::default();
    if !recipients(db, device, now)
        .await?
        .iter()
        .any(|recipient| recipient == user)
    {
        for &channel in channels {
            result.skip(channel, "not_a_recipient");
        }
        return Ok(TestNotice::Sent(result));
    }
    if let Some(wait) = TestReminder::wait(db, device, user, now).await? {
        return Ok(TestNotice::Throttled(wait));
    }
    let test = TestReminder::prepare(db, device, user, now).await?;
    for &channel in channels {
        match test.send(db, sink, channel).await? {
            Some(Ok(())) => result.sent.push(channel),
            Some(Err(cause)) => result.skip(channel, cause.reason()),
            None if result.sent.is_empty() && result.skipped.is_empty() => {
                return Ok(TestNotice::Throttled(TEST_SECONDS));
            }
            None => result.skip(channel, "throttled"),
        }
    }
    Ok(TestNotice::Sent(result))
}

/// The usual error object, plus how long to wait before another test.
#[derive(Serialize, utoipa::ToSchema)]
struct CertificateNoticeThrottle {
    error: CertificateNoticeThrottleError,
    /// Seconds until another test reminder can be sent.
    retry_after: u64,
}

#[derive(Serialize, utoipa::ToSchema)]
struct CertificateNoticeThrottleError {
    code: &'static str,
    message: String,
}

fn throttled(seconds: i64) -> Response {
    let retry_after = seconds.clamp(1, TEST_SECONDS).unsigned_abs();
    let mut response = (
        StatusCode::TOO_MANY_REQUESTS,
        Json(CertificateNoticeThrottle {
            error: CertificateNoticeThrottleError {
                code: "TOO_MANY_REQUESTS",
                message: format!(
                    "A test reminder for this device was sent less than ten minutes ago. Try again in {retry_after} seconds."
                ),
            },
            retry_after,
        }),
    )
        .into_response();
    response
        .headers_mut()
        .insert(RETRY_AFTER, HeaderValue::from(retry_after));
    response
}

#[utoipa::path(
    post,
    path = "/devices/{id}/certificate-notices/test",
    tag = "devices",
    description = "Send yourself a test certificate expiry reminder for a device by push, email or both, to check that reminders reach you. Nobody else is notified. One test per device every ten minutes.",
    params(("id" = String, Path, description = "Device ID")),
    request_body = CertificateTestNoticeRequest,
    responses(
        (status = 202, description = "Which channels the test was sent on, and why the others were skipped", body = CertificateTestNoticeResult),
        (status = 403, description = "You cannot read this device's certificates, or there is no such device"),
        (status = 429, description = "You sent a test for this device less than ten minutes ago", body = CertificateNoticeThrottle, headers(("Retry-After" = u64, description = "Seconds until another test reminder can be sent")))
    ),
    security(("bearer_auth" = []), ("pat" = []))
)]
async fn test_certificate_notice(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(id): Path<String>,
    Json(request): Json<CertificateTestNoticeRequest>,
) -> Result<Response, ApiError> {
    let user = certificate_reader(&state, &user, &id).await?;
    let outcome = send_test(
        &state.db,
        &NativeDelivery(&state),
        &id,
        &user,
        request.channel.channels(),
        chrono::Utc::now().timestamp(),
    )
    .await?;
    Ok(match outcome {
        TestNotice::Sent(result) => (StatusCode::ACCEPTED, Json(result)).into_response(),
        TestNotice::Throttled(seconds) => throttled(seconds),
    })
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

#[derive(utoipa::OpenApi)]
#[openapi(paths(
    fleet_certificate_inventory,
    certificate_notices,
    certificate_notice_mutes,
    mute_certificate_notices,
    unmute_certificate_notices,
    test_certificate_notice,
    super::archives::archive_usage
))]
pub(crate) struct CertificatesApi;

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

    #[test]
    fn reminder_links_open_the_certificates_tab_on_the_certificate() {
        assert_eq!(
            certificates_link("device a/b", None),
            "/settings/devices?device=device%20a%2Fb&tab=certificates"
        );
        assert_eq!(
            certificates_link("device", Some("a48aeb81-1321-402e-b09c-8b304f9c3c2f")),
            "/settings/devices?device=device&tab=certificates&certificate=a48aeb81-1321-402e-b09c-8b304f9c3c2f"
        );
    }

    #[test]
    fn a_mute_covers_its_certificate_or_the_whole_device_for_one_account() {
        let mutes: Mutes = [
            ("reader".to_owned(), [("one".to_owned(), None)].into()),
            (
                "owner".to_owned(),
                [(ALL_CERTIFICATES.to_owned(), Some(10))].into(),
            ),
        ]
        .into();
        assert!(muted(&mutes, "reader", "one"));
        assert!(!muted(&mutes, "reader", "another"));
        assert!(muted(&mutes, "owner", "one") && muted(&mutes, "owner", "another"));
        assert!(!muted(&mutes, "stranger", "one"));
    }

    #[test]
    fn a_test_reminder_has_one_identity_per_account_device_channel_and_window() {
        let window = 7 * TEST_SECONDS;
        let id = test_notice_id("device", "owner", window, "push");
        assert_eq!(
            id,
            test_notice_id("device", "owner", window + TEST_SECONDS - 1, "push")
        );
        let mut identities = HashSet::from([id]);
        for (device, user, now, channel) in [
            ("other", "owner", window, "push"),
            ("device", "reader", window, "push"),
            ("device", "owner", window + TEST_SECONDS, "push"),
            ("device", "owner", window, "email"),
        ] {
            assert!(identities.insert(test_notice_id(device, user, now, channel)));
        }
        validate_certificate_id(NO_CERTIFICATE).unwrap();
    }

    #[test]
    fn reminder_requests_reject_unknown_channels_and_fields() {
        let channels = |body: &str| {
            serde_json::from_str::<CertificateTestNoticeRequest>(body)
                .map(|request| request.channel.channels())
        };
        assert_eq!(channels(r#"{"channel":"push"}"#).unwrap(), ["push"]);
        assert_eq!(channels(r#"{"channel":"email"}"#).unwrap(), ["email"]);
        assert_eq!(channels(r#"{"channel":"both"}"#).unwrap(), CHANNELS);
        assert!(channels(r#"{"channel":"sms"}"#).is_err());
        assert!(channels(r#"{"channel":"push","user_id":"someone"}"#).is_err());

        let mute = |body: &str| serde_json::from_str::<CertificateNoticeMute>(body);
        for device_wide in [r#"{"certificate_id":null,"until":null}"#, "{}"] {
            assert_eq!(mute(device_wide).unwrap(), mute_of(None, None));
        }
        assert_eq!(
            mute(r#"{"certificate_id":"one","until":5}"#).unwrap(),
            mute_of(Some("one"), Some(5))
        );
        assert!(mute(r#"{"user_id":"someone"}"#).is_err());
        assert_eq!(
            serde_json::to_value(mute_of(None, None)).unwrap(),
            serde_json::json!({"certificate_id": null, "until": null})
        );
    }

    #[tokio::test]
    async fn a_throttled_test_tells_the_caller_how_long_to_wait() {
        let response = throttled(540);
        assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(response.headers()[RETRY_AFTER], "540");
        let body = axum::body::to_bytes(response.into_body(), 1024)
            .await
            .unwrap();
        let body: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(body["retry_after"], 540);
        assert_eq!(body["error"]["code"], "TOO_MANY_REQUESTS");
        assert!(
            body["error"]["message"]
                .as_str()
                .unwrap()
                .contains("540 seconds")
        );
        assert_eq!(throttled(0).headers()[RETRY_AFTER], "1");
        assert_eq!(throttled(i64::MAX).headers()[RETRY_AFTER], "600");
    }

    #[test]
    fn certificate_paths_are_documented() {
        use utoipa::OpenApi;
        let spec = serde_json::to_value(crate::openapi::ApiDoc::openapi()).unwrap();
        for (path, method) in [
            ("/devices/certificate-inventory", "get"),
            ("/devices/{id}/certificate-notices", "get"),
            ("/devices/{id}/certificate-notices/mute", "get"),
            ("/devices/{id}/certificate-notices/mute", "put"),
            ("/devices/{id}/certificate-notices/mute", "delete"),
            ("/devices/{id}/certificate-notices/test", "post"),
            ("/devices/archive-usage", "get"),
        ] {
            let operation = spec
                .pointer(&format!("/paths/{}/{method}", path.replace('/', "~1")))
                .unwrap_or_else(|| panic!("missing OpenAPI operation {method} {path}"));
            assert_eq!(
                operation["tags"],
                serde_json::json!(["devices"]),
                "{method} {path}"
            );
            assert_eq!(
                operation["security"],
                serde_json::json!([{"bearer_auth": []}, {"pat": []}]),
                "{method} {path}"
            );
            assert!(
                operation["description"]
                    .as_str()
                    .is_some_and(|text| !text.is_empty()),
                "{method} {path} has no description"
            );
        }
        assert!(
            spec.pointer(
                "/paths/~1devices~1{id}~1certificate-notices~1test/post/responses/429/headers/Retry-After"
            )
            .is_some(),
            "the test reminder documents no Retry-After header"
        );
        for schema in [
            "FleetCertificateInventory",
            "CertificateInventoryView",
            "CertificateFacts",
            "CertificateNotice",
            "CertificateNoticeMute",
            "CertificateTestNoticeRequest",
            "CertificateTestNoticeChannel",
            "CertificateTestNoticeResult",
            "CertificateTestNoticeSkip",
            "CertificateNoticeThrottle",
            "CertificateNoticeThrottleError",
            "ArchiveUsage",
            "ArchiveDeviceUsage",
        ] {
            assert!(
                spec.pointer(&format!("/components/schemas/{schema}"))
                    .is_some(),
                "schema {schema} is not registered"
            );
        }
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
        /// Title and link of the latest delivery.
        last: Mutex<Option<(String, String)>>,
        fail_email: AtomicBool,
        db: Option<DatabaseConnection>,
    }

    #[async_trait::async_trait]
    impl DeliverySink for FakeDelivery {
        async fn deliver(&self, channel: &str, delivery: Delivery) -> Result<(), Undelivered> {
            if let Some(db) = &self.db {
                let row = db
                    .query_one_raw(sql(
                        r#"SELECT "leaseUntil" FROM "DeviceCertificateNotice" WHERE id=$1"#,
                        [delivery.notification_id.clone().into()],
                    ))
                    .await
                    .unwrap()
                    .unwrap();
                assert!(
                    row.try_get::<i64>("", "leaseUntil").unwrap()
                        > chrono::Utc::now().timestamp() + 60,
                    "A delayed batch must claim a fresh lease before each delivery"
                );
            }
            if channel == "email" {
                if delivery.email.is_none() {
                    return Err(Undelivered::NoEmail);
                }
                if self.fail_email.load(Ordering::SeqCst) {
                    return Err(Undelivered::Failed);
                }
            }
            *self.last.lock().unwrap() = Some((delivery.title, delivery.link));
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

    async fn insert_device(db: &DatabaseConnection, id: &str, registered_at: i64) {
        let identity = DeviceIdentity {
            auth_key: SigningKey::generate().public_key(),
            management_key: [42; 32],
            telemetry_key: SigningKey::generate().public_key(),
        };
        let receipt = DeviceReceipt {
            enrollment_id: "enrollment".into(),
            device_id: id.into(),
            owner_id: "owner".into(),
            name: "Remote device".into(),
            identity: identity.clone(),
            manifest_jws: "fixture".into(),
            binding_jws: "fixture".into(),
            registered_at,
            auth_epoch: 1,
        };
        db.execute_raw(sql(r#"INSERT INTO "ManagedDevice"(id,"ownerId",name,status,"authEpoch",identity,receipt,"registeredAt") VALUES($1,'owner','Remote device','active',1,$2,$3,$4)"#, [id.into(), serde_json::to_string(&identity).unwrap().into(), serde_json::to_string(&receipt).unwrap().into(), registered_at.into()])).await.unwrap();
    }

    /// A disposable schema with one active device of `owner`, shared with a
    /// grantee of every kind: `reader` and `group-user` accepted a whole-device
    /// grant, `unaccepted` and `withdrawn` hold one without a registered key,
    /// `scoped` is limited to a project, `expired` lapsed, `disabled` is not active.
    struct Hub {
        admin: DatabaseConnection,
        db: DatabaseConnection,
        schema: String,
        now: i64,
        invitation: SigningKey,
        policy: ManagementPolicy,
        first_policy: String,
    }

    impl Hub {
        async fn stop(self) {
            self.db.close().await.unwrap();
            self.admin
                .execute_unprepared(&format!("DROP SCHEMA {} CASCADE", self.schema))
                .await
                .unwrap();
        }
    }

    async fn hub() -> Hub {
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
            include_str!("../../prisma/migrations/20261001120000_device_console/migration.sql"),
            include_str!("../../prisma/migrations/20261002120000_device_schedules/migration.sql"),
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
        insert_device(&db, "device", now).await;
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
        let policy = ManagementPolicy {
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
        Hub {
            admin,
            db,
            schema,
            now,
            invitation,
            policy,
            first_policy,
        }
    }

    #[tokio::test]
    #[ignore = "requires FLOW_LIKE_DEVICE_TEST_DATABASE_URL pointing to disposable PostgreSQL"]
    async fn expiry_outbox_retries_channels_checks_current_access_and_cancels_rotated_certificates()
    {
        let mut hub = hub().await;
        let (db, now, first_policy) = (hub.db.clone(), hub.now, hub.first_policy.clone());
        let (policy, invitation) = (&mut hub.policy, &hub.invitation);
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
        due_by(&db, now).await;
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
        hub.stop().await;
    }

    fn mute_of(certificate: Option<&str>, until: Option<i64>) -> CertificateNoticeMute {
        CertificateNoticeMute {
            certificate_id: certificate.map(Into::into),
            until,
        }
    }

    async fn queued(db: &DatabaseConnection, user: &str, channel: &str) -> Notice {
        let row = db
            .query_one_raw(sql(
                r#"SELECT * FROM "DeviceCertificateNotice" WHERE "userId"=$1 AND channel=$2 AND stage<>$3"#,
                [user.into(), channel.into(), TEST_STAGE.into()],
            ))
            .await
            .unwrap()
            .unwrap();
        Notice::from_row(row).unwrap()
    }

    async fn make_due(db: &DatabaseConnection, now: i64) {
        db.execute_raw(sql(
            r#"UPDATE "DeviceCertificateNotice" SET "nextAttemptAt"=$1 WHERE status='pending'"#,
            [now.into()],
        ))
        .await
        .unwrap();
        db.execute_unprepared(r#"UPDATE "DeviceCertificateInventory" SET "nextCheckAt"=0"#)
            .await
            .unwrap();
    }

    /// A first inventory is due at the wall clock, which may have passed the fixture's `now`.
    async fn due_by(db: &DatabaseConnection, now: i64) {
        db.execute_raw(sql(
            r#"UPDATE "DeviceCertificateInventory" SET "nextCheckAt"=LEAST("nextCheckAt",$1)"#,
            [now.into()],
        ))
        .await
        .unwrap();
    }

    #[tokio::test]
    #[ignore = "requires FLOW_LIKE_DEVICE_TEST_DATABASE_URL pointing to disposable PostgreSQL"]
    async fn a_mute_silences_only_its_account_when_scheduling_and_when_delivering() {
        let hub = hub().await;
        let (db, now) = (hub.db.clone(), hub.now);
        let inventory = fixture(now);
        let certificate = inventory.certificates[0].certificate_id.clone();
        retain(&db, DbDialect::Postgres, 1, inventory.clone())
            .await
            .unwrap();
        due_by(&db, now).await;
        for (request, status) in [
            (mute_of(Some("not-a-uuid"), None), StatusCode::BAD_REQUEST),
            (
                mute_of(Some("b48aeb81-1321-402e-b09c-8b304f9c3c2f"), None),
                StatusCode::NOT_FOUND,
            ),
            (mute_of(None, Some(now)), StatusCode::BAD_REQUEST),
        ] {
            let refused = mute(&db, "device", "reader", request, now).await;
            assert_eq!(refused.unwrap_err().status(), status);
        }

        let muted_certificate = mute_of(Some(&certificate), None);
        assert_eq!(
            mute(
                &db,
                "device",
                "reader",
                mute_of(Some(&certificate), None),
                now
            )
            .await
            .unwrap(),
            muted_certificate
        );
        schedule(&db, DbDialect::Postgres, "device".into(), now)
            .await
            .unwrap();
        let reminders = |filter: &'static str| {
            let db = db.clone();
            async move {
                count(
                    &db,
                    &format!(
                        r#"SELECT COUNT(*) AS count FROM "DeviceCertificateNotice" WHERE {filter}"#
                    ),
                )
                .await
            }
        };
        assert_eq!(reminders(r#""userId"='reader'"#).await, 0);
        assert_eq!(reminders("status='pending'").await, 4);

        // Reminders queued before a mute are dropped when their turn comes.
        let muted_device = mute_of(None, Some(now + 3600));
        mute(&db, "device", "owner", mute_of(None, Some(now + 3600)), now)
            .await
            .unwrap();
        let sink = FakeDelivery::default();
        assert_eq!(dispatch_pending(&db, &sink, now).await.unwrap(), 2);
        assert!(
            sink.sent
                .lock()
                .unwrap()
                .iter()
                .all(|(_, user, _)| user == "group-user")
        );
        assert_eq!(
            reminders(r#""userId"='owner' AND status='cancelled'"#).await,
            2
        );
        assert_eq!(
            own_mutes(&db, "device", "owner", now).await.unwrap(),
            [muted_device]
        );
        assert_eq!(
            own_mutes(&db, "device", "reader", now).await.unwrap(),
            [muted_certificate]
        );
        assert!(
            own_mutes(&db, "device", "group-user", now)
                .await
                .unwrap()
                .is_empty()
        );

        // A muted account stays silent on later checks; a lapsed or lifted mute
        // lets the next check queue what is due.
        make_due(&db, now).await;
        schedule(&db, DbDialect::Postgres, "device".into(), now)
            .await
            .unwrap();
        assert_eq!(reminders("status='pending'").await, 0);
        unmute(&db, "device", "reader", &certificate).await.unwrap();
        make_due(&db, now).await;
        schedule(&db, DbDialect::Postgres, "device".into(), now + 3600)
            .await
            .unwrap();
        assert_eq!(
            reminders(r#""userId"='owner' AND status='pending'"#).await,
            2
        );
        assert_eq!(
            reminders(r#""userId"='reader' AND status='pending'"#).await,
            2
        );
        assert!(
            own_mutes(&db, "device", "owner", now + 3600)
                .await
                .unwrap()
                .is_empty()
        );

        // The reminder names the device by its alias and opens the certificate.
        let reminder = queued(&db, "group-user", "push").await;
        let delivery = preflight(&db, &reminder, now).await.unwrap().unwrap();
        assert_eq!(
            delivery.title,
            "Service certificate expires soon: Remote device"
        );
        db.execute_unprepared(r#"UPDATE "ManagedDevice" SET "displayName"='Lab GPU (rack 2)'"#)
            .await
            .unwrap();
        let delivery = preflight(&db, &reminder, now).await.unwrap().unwrap();
        assert_eq!(
            delivery.title,
            "Service certificate expires soon: Lab GPU (rack 2)"
        );
        assert!(delivery.description.contains("on device Lab GPU (rack 2)"));
        assert_eq!(
            delivery.link,
            format!("/settings/devices?device=device&tab=certificates&certificate={certificate}")
        );
        assert!(
            preflight(&db, &queued(&db, "owner", "push").await, now)
                .await
                .unwrap()
                .is_none(),
            "the owner's mute still holds at delivery"
        );

        // Each account reads its own history, newest first.
        let history = notices(&db, "device", "group-user", Some(&certificate), 50)
            .await
            .unwrap();
        assert_eq!(history.len(), 2);
        for reminder in &history {
            assert_eq!(
                (reminder.stage.as_str(), reminder.status.as_str()),
                ("week", "sent")
            );
            assert_eq!(reminder.certificate_id, certificate);
            assert!(reminder.completed_at.is_some() && reminder.next_attempt_at.is_none());
        }
        let pending = notices(&db, "device", "reader", None, 1).await.unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].status, "pending");
        assert!(pending[0].completed_at.is_none() && pending[0].next_attempt_at.is_some());
        assert!(
            notices(&db, "device", "unaccepted", None, 50)
                .await
                .unwrap()
                .is_empty()
        );
        assert!(
            notices(&db, "device", "group-user", Some(NO_CERTIFICATE), 50)
                .await
                .unwrap()
                .is_empty()
        );

        // A mute of a certificate the device stopped reporting goes with the next mute.
        mute(
            &db,
            "device",
            "group-user",
            mute_of(Some(&certificate), None),
            now,
        )
        .await
        .unwrap();
        let mut rotated = inventory;
        rotated.revision = 2;
        rotated.certificates[0].certificate_id = "c48aeb81-1321-402e-b09c-8b304f9c3c2f".into();
        retain(&db, DbDialect::Postgres, 1, rotated).await.unwrap();
        mute(&db, "device", "group-user", mute_of(None, None), now)
            .await
            .unwrap();
        assert_eq!(
            own_mutes(&db, "device", "group-user", now).await.unwrap(),
            [mute_of(None, None)]
        );
        assert_eq!(
            count(
                &db,
                r#"SELECT COUNT(*) AS count FROM "DeviceCertificateNoticeMute""#
            )
            .await,
            2,
            "the owner's lapsing mute and the group user's device mute"
        );
        hub.stop().await;
    }

    #[tokio::test]
    #[ignore = "requires FLOW_LIKE_DEVICE_TEST_DATABASE_URL pointing to disposable PostgreSQL"]
    async fn a_test_reminder_reaches_only_its_sender_and_waits_ten_minutes_per_device() {
        let hub = hub().await;
        let (db, now) = (hub.db.clone(), hub.now);
        let sink = FakeDelivery {
            db: Some(db.clone()),
            ..Default::default()
        };
        let both = CertificateTestNoticeChannel::Both.channels();
        let outcome = |sent: &[&'static str], skipped: &[(&'static str, &'static str)]| {
            TestNotice::Sent(CertificateTestNoticeResult {
                sent: sent.to_vec(),
                skipped: skipped
                    .iter()
                    .map(|&(channel, reason)| CertificateTestNoticeSkip { channel, reason })
                    .collect(),
            })
        };
        let tests = |filter: &'static str| {
            let db = db.clone();
            async move {
                count(
                    &db,
                    &format!(
                        r#"SELECT COUNT(*) AS count FROM "DeviceCertificateNotice" WHERE stage='test' AND {filter}"#
                    ),
                )
                .await
            }
        };

        assert_eq!(
            send_test(&db, &sink, "device", "reader", both, now)
                .await
                .unwrap(),
            outcome(&["push", "email"], &[])
        );
        assert_eq!(
            *sink.sent.lock().unwrap(),
            [("push", "reader"), ("email", "reader")].map(|(channel, user)| (
                channel.to_owned(),
                user.to_owned(),
                test_notice_id("device", "reader", now, channel)
            ))
        );
        assert_eq!(
            *sink.last.lock().unwrap(),
            Some((
                "Test reminder: Remote device".to_owned(),
                "/settings/devices?device=device&tab=certificates".to_owned()
            ))
        );

        // One test per account and device in ten minutes, whatever the channel.
        for (later, wait) in [(1, TEST_SECONDS - 1), (TEST_SECONDS - 1, 1)] {
            assert_eq!(
                send_test(&db, &sink, "device", "reader", &both[..1], now + later)
                    .await
                    .unwrap(),
                TestNotice::Throttled(wait)
            );
        }
        assert_eq!(
            send_test(&db, &sink, "device", "owner", &both[..1], now)
                .await
                .unwrap(),
            outcome(&["push"], &[])
        );
        // Neither a new inventory nor the reminder sweep lifts the wait.
        retain(&db, DbDialect::Postgres, 1, fixture(now))
            .await
            .unwrap();
        assert_eq!(dispatch_pending(&db, &sink, now).await.unwrap(), 0);
        assert_eq!(tests("status='sent'").await, 3);
        assert_eq!(
            send_test(&db, &sink, "device", "reader", both, now + 1)
                .await
                .unwrap(),
            TestNotice::Throttled(TEST_SECONDS - 1)
        );

        // A provider failure is reported and still counts as a test.
        sink.fail_email.store(true, Ordering::SeqCst);
        let second = now + TEST_SECONDS;
        assert_eq!(
            send_test(&db, &sink, "device", "reader", both, second)
                .await
                .unwrap(),
            outcome(&["push"], &[("email", "failed")])
        );
        assert_eq!(
            tests(r#""userId"='reader' AND channel='email' AND status='cancelled'"#).await,
            1
        );
        assert_eq!(
            send_test(&db, &sink, "device", "reader", both, second + 1)
                .await
                .unwrap(),
            TestNotice::Throttled(TEST_SECONDS - 1)
        );

        // A channel refused before sending leaves no record and starts no wait.
        sink.fail_email.store(false, Ordering::SeqCst);
        db.execute_unprepared(r#"UPDATE "User" SET email=NULL WHERE id='reader'"#)
            .await
            .unwrap();
        let third = second + TEST_SECONDS;
        assert_eq!(
            send_test(&db, &sink, "device", "reader", &both[1..], third)
                .await
                .unwrap(),
            outcome(&[], &[("email", "no_email")])
        );
        assert_eq!(
            send_test(&db, &sink, "device", "reader", both, third + 1)
                .await
                .unwrap(),
            outcome(&["push"], &[("email", "no_email")])
        );

        // Holding a grant is not enough: reminders reach only accounts that accepted it.
        let delivered = sink.sent.lock().unwrap().len();
        assert_eq!(
            send_test(&db, &sink, "device", "unaccepted", both, now)
                .await
                .unwrap(),
            outcome(
                &[],
                &[("push", "not_a_recipient"), ("email", "not_a_recipient")]
            )
        );
        assert_eq!(sink.sent.lock().unwrap().len(), delivered);
        assert_eq!(tests(r#""userId"='unaccepted'"#).await, 0);

        // A test a crashed request left behind is closed by the sweep without being sent.
        db.execute_raw(sql(
            r#"INSERT INTO "DeviceCertificateNotice"(id,"deviceId","certificateId","certificateRevision",fingerprint,"notAfter","userId",stage,channel,status,attempts,"nextAttemptAt","leaseId","leaseUntil")
            VALUES('abandoned','device',$1,0,'',0,'group-user','test','push','pending',1,$2,'gone',$3)"#,
            [NO_CERTIFICATE.into(), now.into(), (now - 1).into()],
        ))
        .await
        .unwrap();
        assert_eq!(dispatch_pending(&db, &sink, now).await.unwrap(), 0);
        assert_eq!(sink.sent.lock().unwrap().len(), delivered);
        assert_eq!(tests("id='abandoned' AND status='cancelled'").await, 1);

        // Tests appear in the sender's history only, newest first, and name no certificate.
        db.execute_unprepared(
            r#"UPDATE "DeviceCertificateNotice" SET "completedAt"="nextAttemptAt""#,
        )
        .await
        .unwrap();
        let history = notices(&db, "device", "reader", None, 50).await.unwrap();
        assert_eq!(
            history
                .iter()
                .map(|test| test.completed_at.unwrap() - now)
                .collect::<Vec<_>>(),
            [third + 1 - now, TEST_SECONDS, TEST_SECONDS, 0, 0]
        );
        assert!(history.iter().all(|test| test.stage == TEST_STAGE
            && test.certificate_id == NO_CERTIFICATE
            && test.next_attempt_at.is_none()));
        assert!(
            notices(&db, "device", "reader", Some(NO_CERTIFICATE), 50)
                .await
                .unwrap()
                .is_empty()
        );

        // Tests older than a day are dropped by the next one.
        let next_day = now + DAY + 1;
        assert_eq!(
            send_test(&db, &sink, "device", "owner", &both[..1], next_day)
                .await
                .unwrap(),
            outcome(&["push"], &[])
        );
        assert_eq!(tests(r#""userId"='owner'"#).await, 1);
        hub.stop().await;
    }

    #[tokio::test]
    #[ignore = "requires FLOW_LIKE_DEVICE_TEST_DATABASE_URL pointing to disposable PostgreSQL"]
    async fn the_fleet_inventory_lists_own_devices_and_shared_ones_with_certificate_access() {
        let hub = hub().await;
        let (db, now) = (hub.db.clone(), hub.now);
        insert_device(&db, "unreported", now - 10).await;
        insert_device(&db, "revoked", now - 20).await;
        db.execute_unprepared(r#"UPDATE "ManagedDevice" SET status='revoked' WHERE id='revoked'"#)
            .await
            .unwrap();
        let inventory = fixture(now);
        retain(&db, DbDialect::Postgres, 1, inventory.clone())
            .await
            .unwrap();
        let devices = |fleet: &[FleetCertificateInventory]| {
            fleet
                .iter()
                .map(|row| row.device_id.clone())
                .collect::<Vec<_>>()
        };

        let own = fleet_inventory(&db, "owner", &[], now).await.unwrap();
        assert_eq!(devices(&own), ["device", "unreported"]);
        assert_eq!(
            serde_json::to_value(&own).unwrap(),
            serde_json::json!([
                {
                    "device_id": "device",
                    "revision": 1,
                    "updated_at": own[0].inventory.updated_at.unwrap(),
                    "certificates": inventory.certificates,
                },
                {"device_id": "unreported", "revision": 0, "updated_at": null, "certificates": []},
            ])
        );

        let shared = ["device", "unreported", "revoked", "missing"];
        for (user, expected) in [
            ("reader", vec!["device"]),
            ("group-user", vec!["device"]),
            ("unaccepted", vec!["device"]),
            ("scoped", vec![]),
            ("expired", vec![]),
            ("stranger", vec![]),
        ] {
            let fleet = fleet_inventory(&db, user, &shared, now).await.unwrap();
            assert_eq!(devices(&fleet), expected, "{user}");
        }
        let own = fleet_inventory(&db, "owner", &shared, now).await.unwrap();
        assert_eq!(devices(&own), ["device", "unreported"]);
        hub.stop().await;
    }

    #[tokio::test]
    #[ignore = "requires FLOW_LIKE_DEVICE_TEST_DATABASE_URL pointing to disposable PostgreSQL"]
    async fn a_missing_device_is_refused_like_one_the_account_cannot_reach() {
        let hub = hub().await;
        let (db, now) = (hub.db.clone(), hub.now);
        crate::backend_jwt::init_for_tests();
        let config = flow_like::hub::StandaloneConfig {
            enabled: true,
            ..Default::default()
        };
        let state = DeviceContext {
            db: &db,
            dialect: DbDialect::Postgres,
            config: &config,
            domain: "unused.example",
            secure: true,
        };
        for user in ["reader", "scoped"] {
            db.execute_raw(sql(
                r#"INSERT INTO "DeviceManagementRecipient"("deviceId",version,"grantId","userId","expiresAt") VALUES('device',1,$1,$1,$2)"#,
                [user.into(), (now + DAY).into()],
            ))
            .await
            .unwrap();
        }
        for user in ["owner", "reader"] {
            readable(&state, user, "device").await.unwrap();
        }
        // Shared access that does not cover certificates says what is missing.
        let scoped = readable(&state, "scoped", "device").await.unwrap_err();
        assert_eq!(scoped.status(), StatusCode::FORBIDDEN);
        assert!(scoped.public_message().is_some());

        for (user, device) in [
            ("withdrawn", "device"),
            ("withdrawn", "missing"),
            ("owner", "missing"),
        ] {
            let refused = readable(&state, user, device).await.unwrap_err();
            assert_eq!(refused.status(), StatusCode::FORBIDDEN, "{user} {device}");
            assert_eq!(refused.public_message(), None, "{user} {device}");
        }
        hub.stop().await;
    }
}
