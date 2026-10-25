use super::{archive_payload, context, enabled, human_owner, management, repository};
use crate::{
    db::{RetryPolicy, retry_transaction},
    error::ApiError,
    middleware::jwt::AppUser,
    state::AppState,
};
use archive_payload::StoredArchive;
use axum::{
    Extension, Json, Router,
    extract::{Path, Query, State},
    routing::{get, post},
};
use flow_like_device_protocol::*;
use sea_orm::{ConnectionTrait, DatabaseBackend, FromQueryResult, Statement, Value};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::result::Result;

/// However long a tier keeps history, the hub drops a record after this.
const MAX_RETENTION_SECONDS: u64 = 31 * 86_400;
const USAGE_DEVICES: i64 = 1000;
/// Expired rows one transaction removes for an owner, within DSQL's per-transaction limits.
const EXPIRED_BATCH: usize = 128;
const OFFLOAD_PER_PASS: usize = 256;

pub(super) fn sql(query: &str, values: impl IntoIterator<Item = Value>) -> Statement {
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
        .route("/archive-usage", get(archive_usage))
        .route("/{id}/archives", post(upload).get(list))
        .route("/{id}/archives/{archive}", get(read))
}

/// Bytes and seconds of history per account tier, as configured.
type TierLimits = std::collections::BTreeMap<String, (u64, u64)>;

fn tier_limits(state: &AppState) -> TierLimits {
    state
        .platform_config
        .standalone
        .telemetry_tiers
        .iter()
        .map(|(tier, limit)| (tier.clone(), (limit.max_bytes, limit.retention_seconds)))
        .collect()
}

/// The bytes a tier keeps and for how many seconds; `None` when it keeps no history.
fn tier_limit(limits: &TierLimits, tier: &str) -> Option<(u64, u64)> {
    let (max_bytes, retention_seconds) = limits.get(tier).copied().unwrap_or_default();
    (max_bytes > 0 && retention_seconds > 0)
        .then(|| (max_bytes, retention_seconds.min(MAX_RETENTION_SECONDS)))
}

async fn account_tier<C: ConnectionTrait>(db: &C, owner: &str) -> Result<String, ApiError> {
    Ok(db
        .query_one_raw(sql(
            r#"SELECT tier::text AS tier FROM "User" WHERE id=$1"#,
            [owner.into()],
        ))
        .await?
        .ok_or(ApiError::FORBIDDEN)?
        .try_get::<String>("", "tier")?
        .to_uppercase())
}

/// History of all of an owner's devices that expires after `after`: at `now` what the quota
/// counts, at `i64::MIN` every stored row.
async fn used_bytes<C: ConnectionTrait>(db: &C, owner: &str, after: i64) -> Result<u64, ApiError> {
    let used = db
        .query_one_raw(sql(
            r#"SELECT COALESCE(SUM("sizeBytes"),0)::bigint AS used FROM "DeviceArchive" WHERE "ownerId"=$1 AND "expiresAt">$2"#,
            [owner.into(), after.into()],
        ))
        .await?
        .ok_or_else(|| ApiError::internal("Missing telemetry quota result"))?
        .try_get::<i64>("", "used")?;
    u64::try_from(used)
        .map_err(|_| ApiError::internal(format!("Telemetry usage of {owner} is negative: {used}")))
}

/// Every writer of an owner's archives updates the owner's row first, so uploads from several
/// devices, sweeps and tier changes serialize: Postgres blocks on it, DSQL lets one commit win.
/// Returns the tier read under the fence; `None` when no (active) account matched.
async fn fence_owner<C: ConnectionTrait>(
    db: &C,
    owner: &str,
    active_only: bool,
) -> Result<Option<String>, ApiError> {
    let status = if active_only {
        " AND status='ACTIVE'"
    } else {
        ""
    };
    db.query_one_raw(sql(
        &format!(r#"UPDATE "User" SET tier=tier WHERE id=$1{status} RETURNING tier::text AS tier"#),
        [owner.into()],
    ))
    .await?
    .map(|row| Ok(row.try_get::<String>("", "tier")?.to_uppercase()))
    .transpose()
}

/// An owner's stored archive bytes inside a transaction that holds the owner's fence.
/// `DeviceArchiveUsage` counts every row, expired or not, so the quota stays O(1) as history grows.
struct OwnerUsage {
    owner: String,
    /// The counter as read; `None` before its first write.
    stored: Option<i64>,
    /// Bytes of every row the owner keeps once the transaction commits.
    used: u64,
    /// Expired rows removed here; a full batch may have left more behind.
    swept: usize,
}

impl OwnerUsage {
    /// Removes a batch of the owner's expired rows and reads the counter, counting the
    /// remaining rows once when it is missing or no longer covers what was removed.
    async fn settle<C: ConnectionTrait>(db: &C, owner: &str, now: i64) -> Result<Self, ApiError> {
        let swept = db.query_all_raw(sql(
            &format!(r#"DELETE FROM "DeviceArchive" WHERE ("deviceId","archiveId") IN (SELECT "deviceId","archiveId" FROM "DeviceArchive" WHERE "ownerId"=$1 AND "expiresAt"<=$2 LIMIT {EXPIRED_BATCH}) RETURNING "sizeBytes""#),
            [owner.into(), now.into()],
        )).await?;
        let freed = swept
            .iter()
            .map(|row| row.try_get::<i64>("", "sizeBytes"))
            .sum::<Result<i64, _>>()?;
        let stored = Self::stored(db, owner).await?;
        let used = match stored.map(|stored| stored.saturating_sub(freed)) {
            Some(used) if used >= 0 => used as u64,
            drifted => {
                if let Some(used) = drifted {
                    tracing::warn!(
                        owner,
                        used,
                        "Device archive usage counter fell below zero; recounting the owner's archives"
                    );
                }
                used_bytes(db, owner, i64::MIN).await?
            }
        };
        Ok(Self {
            owner: owner.into(),
            stored,
            used,
            swept: swept.len(),
        })
    }

    async fn stored<C: ConnectionTrait>(db: &C, owner: &str) -> Result<Option<i64>, ApiError> {
        db.query_one_raw(sql(
            r#"SELECT "usedBytes" FROM "DeviceArchiveUsage" WHERE "ownerId"=$1"#,
            [owner.into()],
        ))
        .await?
        .map(|row| Ok(row.try_get::<i64>("", "usedBytes")?))
        .transpose()
    }

    /// Whether `size` more bytes stay within `max_bytes` of unexpired history. Only when this
    /// transaction's batch left expired rows behind can the exact sum be below `used`.
    async fn admits<C: ConnectionTrait>(
        &self,
        db: &C,
        now: i64,
        size: u64,
        max_bytes: u64,
    ) -> Result<bool, ApiError> {
        if self.used.saturating_add(size) <= max_bytes {
            return Ok(true);
        }
        Ok(self.swept == EXPIRED_BATCH
            && used_bytes(db, &self.owner, now).await?.saturating_add(size) <= max_bytes)
    }

    async fn save<C: ConnectionTrait>(&self, db: &C) -> Result<(), ApiError> {
        let used = i64::try_from(self.used).map_err(|_| {
            ApiError::internal(format!(
                "Telemetry usage of {} overflows: {}",
                self.owner, self.used
            ))
        })?;
        if self.stored == Some(used) {
            return Ok(());
        }
        db.execute_raw(sql(
            r#"INSERT INTO "DeviceArchiveUsage"("ownerId","usedBytes") VALUES($1,$2) ON CONFLICT("ownerId") DO UPDATE SET "usedBytes"=excluded."usedBytes""#,
            [self.owner.clone().into(), used.into()],
        ))
        .await?;
        Ok(())
    }
}

#[derive(Debug, PartialEq, Eq, Serialize, utoipa::ToSchema)]
struct ArchiveUsage {
    /// The account's plan tier.
    tier: String,
    /// Bytes of history the plan keeps across all your devices; 0 when it keeps none.
    max_bytes: u64,
    /// Seconds a record is kept; 0 when the plan keeps no history.
    retention_seconds: u64,
    used_bytes: u64,
    /// Devices holding unexpired history, largest first.
    devices: Vec<ArchiveDeviceUsage>,
}

#[derive(Debug, PartialEq, Eq, Serialize, FromQueryResult, utoipa::ToSchema)]
struct ArchiveDeviceUsage {
    device_id: String,
    #[schema(minimum = 0)]
    used_bytes: i64,
    #[schema(minimum = 0)]
    segments: i64,
    oldest_created_at: i64,
    newest_expires_at: i64,
}

/// An owner's plan limits and the unexpired history each of their devices holds.
async fn usage<C: ConnectionTrait>(
    db: &C,
    limits: &TierLimits,
    owner: &str,
    now: i64,
) -> Result<ArchiveUsage, ApiError> {
    let tier = account_tier(db, owner).await?;
    let (max_bytes, retention_seconds) = tier_limit(limits, &tier).unwrap_or_default();
    let devices = ArchiveDeviceUsage::find_by_statement(sql(
        &format!(r#"SELECT "deviceId" AS device_id,SUM("sizeBytes")::bigint AS used_bytes,COUNT(*)::bigint AS segments,MIN("createdAt") AS oldest_created_at,MAX("expiresAt") AS newest_expires_at FROM "DeviceArchive" WHERE "ownerId"=$1 AND "expiresAt">$2 GROUP BY "deviceId" ORDER BY used_bytes DESC,"deviceId" LIMIT {USAGE_DEVICES}"#),
        [owner.into(), now.into()],
    ))
    .all(db)
    .await?;
    let used_bytes = if devices.len() < USAGE_DEVICES as usize {
        devices
            .iter()
            .map(|device| u64::try_from(device.used_bytes).unwrap_or_default())
            .sum()
    } else {
        used_bytes(db, owner, now).await?
    };
    Ok(ArchiveUsage {
        tier,
        max_bytes,
        retention_seconds,
        used_bytes,
        devices,
    })
}

#[utoipa::path(
    get,
    path = "/devices/archive-usage",
    tag = "devices",
    description = "Show how much retained device history (logs and metrics) your devices keep on the hub, how much your plan allows and how long records are kept.",
    responses(
        (status = 200, description = "Your plan's history limits and the unexpired history per device, largest first", body = ArchiveUsage),
        (status = 403, description = "Your account or token cannot manage devices"),
        (status = 503, description = "Devices are not enabled on this hub")
    ),
    security(("bearer_auth" = []), ("pat" = []))
)]
async fn archive_usage(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
) -> Result<Json<ArchiveUsage>, ApiError> {
    enabled(&context(&state))?;
    let owner = human_owner(&state, &user).await?;
    Ok(Json(
        usage(
            &state.db,
            &tier_limits(&state),
            &owner,
            chrono::Utc::now().timestamp(),
        )
        .await?,
    ))
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
    if encoded.len() > archive_payload::MAX_ARCHIVE_BYTES {
        return Err(ApiError::bad_request("Archive exceeds the transfer limit"));
    }
    let digest = compact_digest(&request.bundle.manifest_jws);
    let response_digest = digest.clone();
    let response_id = manifest.archive_id.clone();
    let epoch = device.status.auth_epoch;
    let limits = tier_limits(&state);
    // Reject disabled plans before writing an object. The transaction below
    // checks the tier again while holding the account's quota fence.
    let tier = account_tier(&state.db, &device.status.owner_id).await?;
    if tier_limit(&limits, &tier).is_none() {
        return Err(storage_refused(
            "ARCHIVE_TIER_WITHOUT_HISTORY",
            "This account tier does not store device telemetry",
        ));
    }
    let payload =
        archive_payload::stage(&state.content_bucket, &manifest, &request.bundle, encoded).await?;
    let stored = retain_verified(
        &state.db,
        state.db_dialect,
        limits,
        id,
        epoch,
        manifest,
        payload,
        digest,
    )
    .await?;
    Ok(Json(
        json!({"archive_id":response_id,"manifest_digest":response_digest,"stored":stored}),
    ))
}

/// The account stores no more history. Devices tell the two causes apart by code; the
/// sentences stay as they are for agents that only know those.
fn storage_refused(code: &'static str, message: &'static str) -> ApiError {
    ApiError::coded(axum::http::StatusCode::PAYMENT_REQUIRED, code, message)
}

async fn retain_verified(
    db: &sea_orm::DatabaseConnection,
    dialect: crate::db::DbDialect,
    limits: TierLimits,
    id: String,
    epoch: u64,
    manifest: ArchiveManifest,
    payload: StoredArchive,
    digest: String,
) -> Result<bool, ApiError> {
    let retention = retry_transaction(db,dialect,None,&RetryPolicy::default(),move|tx|{
        let id=id.clone();let manifest=manifest.clone();let payload=payload.clone();let digest=digest.clone();let limits=limits.clone();
        Box::pin(async move{
            let device=repository::lock_active_device(tx,&id,epoch).await?;
            let owner=device.status.owner_id;
            let Some(tier)=fence_owner(tx,&owner,true).await? else {return Err(ApiError::FORBIDDEN);};
            let Some((max_bytes,retention_seconds))=tier_limit(&limits,&tier) else {return Err(storage_refused("ARCHIVE_TIER_WITHOUT_HISTORY","This account tier does not store device telemetry"));};
            let now=chrono::Utc::now().timestamp();
            let mut usage=OwnerUsage::settle(tx,&owner,now).await?;
            let counted=&mut usage;
            let retention=async move{
                if let Some(old)=tx.query_one_raw(sql(r#"SELECT digest FROM "DeviceArchive" WHERE "deviceId"=$1 AND "archiveId"=$2"#,[id.clone().into(),manifest.archive_id.clone().into()])).await?{
                    if old.try_get::<String>("","digest")?!=digest{return Err(ApiError::conflict("Archive identifier was already used"));}
                    return Ok(Retention::Kept(true));
                }
                let scope=manifest.scope.clone();let kind=kind(&manifest.kind);
                let head=tx.query_one_raw(sql(r#"SELECT sequence,digest FROM "DeviceArchiveHead" WHERE "deviceId"=$1 AND scope=$2 AND kind=$3"#,[id.clone().into(),scope.clone().into(),kind.into()])).await?;
                let (sequence,previous)=head.map(|r|Ok::<_,ApiError>((r.try_get::<i64>("","sequence")? as u64,Some(r.try_get::<String>("","digest")?)))).transpose()?.unwrap_or((0,None));
                if manifest.sequence==sequence && previous.as_ref()==Some(&digest){return Ok(Retention::Kept(false));}
                if manifest.sequence!=sequence+1 || manifest.previous_manifest_digest!=previous{return Err(ApiError::conflict("Archive sequence does not extend the retained-history chain"));}
                let expires=manifest.created_at.saturating_add(retention_seconds as i64);
                if expires>now {
                    let size=payload.size_bytes as u64;
                    if !counted.admits(tx,now,size,max_bytes).await?{return Ok(Retention::QuotaReached);}
                    tx.execute_raw(sql(r#"INSERT INTO "DeviceArchive"("deviceId","archiveId","ownerId",scope,kind,sequence,digest,bundle,"sizeBytes","createdAt","expiresAt","payloadRef","manifestJws","rosterJws") VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14)"#,[id.clone().into(),manifest.archive_id.clone().into(),owner.into(),scope.clone().into(),kind.into(),(manifest.sequence as i64).into(),digest.clone().into(),payload.bundle.into(),payload.size_bytes.into(),manifest.created_at.into(),expires.into(),payload.payload_ref.into(),payload.manifest_jws.into(),payload.roster_jws.into()])).await?;
                    counted.used=counted.used.saturating_add(size);
                }
                tx.execute_raw(sql(r#"INSERT INTO "DeviceArchiveHead"("deviceId",scope,kind,sequence,digest) VALUES($1,$2,$3,$4,$5) ON CONFLICT("deviceId",scope,kind) DO UPDATE SET sequence=excluded.sequence,digest=excluded.digest"#,[id.into(),scope.into(),kind.into(),(manifest.sequence as i64).into(),digest.into()])).await?;
                Ok(Retention::Kept(expires>now))
            }.await?;
            usage.save(tx).await?;
            Ok(retention)
        })
    }).await?;
    match retention {
        Retention::Kept(stored) => Ok(stored),
        Retention::QuotaReached => Err(storage_refused(
            "ARCHIVE_QUOTA_REACHED",
            "Device telemetry storage quota reached",
        )),
    }
}

/// What an upload transaction committed. A refused upload still commits the expired rows
/// it removed, so a full account frees its expired history on the next attempt.
enum Retention {
    Kept(bool),
    QuotaReached,
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
    bundle: Option<EncryptedArchive>,
    payload_ref: Option<String>,
    size_bytes: i64,
    manifest_jws: String,
    roster_jws: String,
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
                    r#"SELECT bundle,"payloadRef","manifestJws","rosterJws","sizeBytes",digest,"expiresAt",sequence,"archiveId",scope,kind FROM "DeviceArchive" WHERE "deviceId"=$1 AND "archiveId"=$2 AND "expiresAt">$3"#,
                    [id.clone().into(), archive.clone().into(), now.into()],
                )).await?, 0),
                ArchiveSelection::Page(query) => {
                    validate_management_id(&query.scope).map_err(invalid)?;
                    let after = i64::try_from(query.after).map_err(invalid)?;
                    (tx.query_all_raw(sql(
                        r#"SELECT bundle,"payloadRef","manifestJws","rosterJws","sizeBytes",digest,"expiresAt",sequence,"archiveId",scope,kind FROM "DeviceArchive" WHERE "deviceId"=$1 AND scope=$2 AND kind=$3 AND sequence>$4 AND "expiresAt">$5 ORDER BY sequence LIMIT 32"#,
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
                let payload_ref = row.try_get::<Option<String>>("", "payloadRef")?;
                let (bundle, manifest_jws, roster_jws, manifest, roster) = match (
                    row.try_get::<Option<String>>("", "manifestJws")?,
                    row.try_get::<Option<String>>("", "rosterJws")?,
                ) {
                    (Some(manifest_jws), Some(roster_jws)) if payload_ref.is_some() => {
                        let (manifest, roster) = verify_archive_metadata(&manifest_jws, &roster_jws, &owner_key, &device.status.identity.telemetry_key)
                            .map_err(|_| ApiError::internal("Retained archive metadata signature failed"))?;
                        (None, manifest_jws, roster_jws, manifest, roster)
                    }
                    (None, None) if payload_ref.is_none() => {
                        let encoded = row.try_get::<Option<String>>("", "bundle")?
                            .ok_or_else(|| ApiError::internal("Retained archive content is missing"))?;
                        let bundle: EncryptedArchive = serde_json::from_str(&encoded)?;
                        let (manifest, roster) = verify_archive_content(&bundle, &owner_key, &device.status.identity.telemetry_key)
                            .map_err(|_| ApiError::internal("Retained archive signature or digest failed"))?;
                        let manifest_jws = bundle.manifest_jws.clone();
                        let roster_jws = bundle.roster_jws.clone();
                        (Some(bundle), manifest_jws, roster_jws, manifest, roster)
                    }
                    _ => return Err(ApiError::internal("Retained archive storage metadata is incomplete")),
                };
                let bound = manifest.device_id == id && manifest.sequence == sequence
                    && manifest.archive_id == row.try_get::<String>("", "archiveId")?
                    && manifest.scope == row.try_get::<String>("", "scope")?
                    && kind(&manifest.kind) == row.try_get::<String>("", "kind")?;
                if !bound || compact_digest(&manifest_jws) != row.try_get::<String>("", "digest")? { return Err(ApiError::internal("Retained archive index binding failed")); }
                archives.push(VerifiedArchive { bundle, payload_ref, size_bytes: row.try_get("", "sizeBytes")?, manifest_jws, roster_jws, manifest, roster, expires_at: row.try_get("", "expiresAt")? });
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
    let archives: Vec<_> = page.archives.into_iter().map(|archive| json!({"archive_id":archive.manifest.archive_id,"sequence":archive.manifest.sequence,"created_at":archive.manifest.created_at,"expires_at":archive.expires_at,"manifest_jws":archive.manifest_jws,"roster_jws":archive.roster_jws})).collect();
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
    Ok(Json(
        read_retained(
            &state.db,
            state.db_dialect,
            &state.content_bucket,
            &device,
            user,
            enrollment.manifest.owner_invitation_key,
            archive,
        )
        .await?,
    ))
}

async fn read_retained(
    db: &sea_orm::DatabaseConnection,
    dialect: crate::db::DbDialect,
    store: &flow_like_storage::files::store::FlowLikeStore,
    device: &repository::Device,
    user: String,
    owner_key: Ed25519PublicKey,
    archive: String,
) -> Result<EncryptedArchive, ApiError> {
    let id = device.status.device_id.clone();
    let mut page = load_reader_page(
        db,
        dialect,
        id.clone(),
        device.status.auth_epoch,
        user.clone(),
        owner_key.clone(),
        ArchiveSelection::One(archive.clone()),
    )
    .await?;
    let retained = page.archives.pop().ok_or(ApiError::NOT_FOUND)?;
    if let Some(bundle) = retained.bundle {
        return Ok(bundle);
    }
    let reference = retained
        .payload_ref
        .as_deref()
        .ok_or_else(|| ApiError::internal("Retained archive content reference is missing"))?;
    let bundle =
        archive_payload::load(store, &retained.manifest, reference, retained.size_bytes).await?;
    verify_archive_content(&bundle, &owner_key, &device.status.identity.telemetry_key)
        .map_err(|_| ApiError::internal("Retained archive signature or digest failed"))?;
    if bundle.manifest_jws != retained.manifest_jws || bundle.roster_jws != retained.roster_jws {
        return Err(ApiError::internal(
            "Retained archive content differs from its metadata",
        ));
    }
    // Object downloads happen outside the transaction. Fence revocation and
    // expiration again before serving bytes obtained during a slow download.
    let mut current = load_reader_page(
        db,
        dialect,
        id,
        device.status.auth_epoch,
        user,
        owner_key,
        ArchiveSelection::One(archive),
    )
    .await?;
    let current = current.archives.pop().ok_or(ApiError::NOT_FOUND)?;
    if current.payload_ref.as_deref() != Some(reference)
        || current.manifest_jws != bundle.manifest_jws
        || current.roster_jws != bundle.roster_jws
    {
        return Err(ApiError::internal(
            "Retained archive changed during download",
        ));
    }
    Ok(bundle)
}

/// Bounded maintenance is safe to run concurrently on API replicas.
/// Device retention: removes expired encrypted history and returns its count,
/// then removes enrollments that lapsed without becoming devices.
pub(crate) async fn sweep_expired(state: &AppState) -> Result<u64, ApiError> {
    let now = chrono::Utc::now().timestamp();
    let owners = state.db.query_all_raw(sql(
        &format!(r#"SELECT DISTINCT "ownerId" FROM (SELECT "ownerId" FROM "DeviceArchive" WHERE "expiresAt"<=$1 LIMIT {EXPIRED_BATCH}) AS expired"#),
        [now.into()],
    )).await?;
    let mut deleted = 0;
    for row in owners {
        let owner: String = row.try_get("", "ownerId")?;
        match sweep_owner(&state.db, state.db_dialect, owner.clone(), now).await {
            Ok(swept) => deleted += swept as u64,
            Err(error) => {
                tracing::warn!(%error, %owner, "Expired device history of one owner was not removed")
            }
        }
    }
    match repository::prune_abandoned_enrollments(&state.db, state.db_dialect, now).await {
        Ok(0) => {}
        Ok(pruned) => tracing::info!(pruned, "Abandoned device enrollments removed"),
        Err(error) => tracing::warn!(%error, "Abandoned device enrollment cleanup failed"),
    }
    Ok(deleted)
}

/// Removes a batch of one owner's expired history under the fence uploads take, so the
/// owner's usage counter changes with the rows. An owner without a `User` row has no uploads.
async fn sweep_owner(
    db: &sea_orm::DatabaseConnection,
    dialect: crate::db::DbDialect,
    owner: String,
    now: i64,
) -> Result<usize, ApiError> {
    retry_transaction(db, dialect, None, &RetryPolicy::default(), move |tx| {
        let owner = owner.clone();
        Box::pin(async move {
            fence_owner(tx, &owner, false).await?;
            let usage = OwnerUsage::settle(tx, &owner, now).await?;
            usage.save(tx).await?;
            Ok(usage.swept)
        })
    })
    .await
}

/// Moves a bounded batch of bundles stored inline before the content bucket existed into it
/// and returns how many moved.
pub(crate) async fn offload_inline(state: &AppState) -> Result<usize, ApiError> {
    archive_payload::offload_inline(
        &state.db,
        &state.content_bucket,
        chrono::Utc::now().timestamp(),
        OFFLOAD_PER_PASS,
    )
    .await
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

    /// A disposable schema with the device tables and active accounts.
    struct Scratch {
        admin: DatabaseConnection,
        db: DatabaseConnection,
        schema: String,
    }

    impl Scratch {
        /// `accounts` lists `(id, tier)` rows as SQL values.
        async fn with_accounts(accounts: &str) -> Self {
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
                include_str!(
                    "../../prisma/migrations/20260921120000_standalone_devices/migration.sql"
                ),
                include_str!(
                    "../../prisma/migrations/20260922120000_device_management/migration.sql"
                ),
                include_str!(
                    "../../prisma/migrations/20260922130000_device_archives/migration.sql"
                ),
                include_str!("../../prisma/migrations/20261001120000_device_console/migration.sql"),
                include_str!(
                    "../../prisma/migrations/20261002120000_device_schedules/migration.sql"
                ),
                include_str!(
                    "../../prisma/migrations/20261019120000_device_archive_payload_store/migration.sql"
                ),
                include_str!(
                    "../../prisma/migrations/20261022143341_device_archive_usage/migration.sql"
                ),
            ] {
                for statement in migration.split(';').filter(|part| !part.trim().is_empty()) {
                    db.execute_unprepared(statement).await.unwrap();
                }
            }
            db.execute_unprepared(&format!(r#"CREATE TABLE "User"(id TEXT PRIMARY KEY,tier TEXT NOT NULL,status TEXT NOT NULL DEFAULT 'ACTIVE'); INSERT INTO "User"(id,tier) VALUES{accounts}"#)).await.unwrap();
            Self { admin, db, schema }
        }

        async fn discard(self) {
            self.db.close().await.unwrap();
            self.admin
                .execute_unprepared(&format!("DROP SCHEMA {} CASCADE", self.schema))
                .await
                .unwrap();
            self.admin.close().await.unwrap();
        }
    }

    #[tokio::test]
    #[ignore = "requires FLOW_LIKE_DEVICE_TEST_DATABASE_URL pointing to a disposable PostgreSQL server"]
    async fn offloaded_and_legacy_archives_share_reads_listing_and_quota() {
        use flow_like_storage::{
            files::store::FlowLikeStore,
            object_store::{ObjectStoreExt, memory::InMemory, path::Path},
        };
        use std::sync::Arc;

        let scratch = Scratch::with_accounts("('owner','PREMIUM')").await;
        let db = scratch.db.clone();
        let telemetry = device(&db, "shared").await;
        let owner = SigningKey::generate();
        let store = FlowLikeStore::Memory(Arc::new(InMemory::new()));
        let bundle = signed_archive(&owner, &telemetry);
        let (manifest, _) =
            verify_archive_content(&bundle, &owner.public_key(), &telemetry.public_key()).unwrap();
        let encoded = serde_json::to_string(&bundle).unwrap();
        let digest = compact_digest(&bundle.manifest_jws);
        let payload = archive_payload::stage(&store, &manifest, &bundle, encoded.clone())
            .await
            .unwrap();
        let reference = payload.payload_ref.clone().unwrap();
        assert!(
            retain_verified(
                &db,
                DbDialect::Postgres,
                limits(),
                "shared".into(),
                1,
                manifest.clone(),
                payload.clone(),
                digest.clone(),
            )
            .await
            .unwrap()
        );
        let row = db
            .query_one_raw(sql(
                r#"SELECT bundle,"payloadRef","manifestJws","rosterJws","sizeBytes" FROM "DeviceArchive" WHERE "deviceId"='shared' AND "archiveId"='segment'"#,
                [],
            ))
            .await
            .unwrap()
            .unwrap();
        assert!(
            row.try_get::<Option<String>>("", "bundle")
                .unwrap()
                .is_none()
        );
        assert_eq!(row.try_get::<String>("", "payloadRef").unwrap(), reference);
        assert_eq!(
            row.try_get::<String>("", "manifestJws").unwrap(),
            bundle.manifest_jws
        );
        assert_eq!(
            row.try_get::<String>("", "rosterJws").unwrap(),
            bundle.roster_jws
        );
        assert_eq!(
            row.try_get::<i64>("", "sizeBytes").unwrap(),
            encoded.len() as i64
        );

        store
            .as_generic()
            .delete(&Path::from(reference.as_str()))
            .await
            .unwrap();
        let page = download(&db, &owner, "owner", false).await.unwrap();
        assert_eq!(page.next, 1);
        assert_eq!(page.archives.len(), 1);
        assert!(page.archives[0].bundle.is_none());
        assert_eq!(page.archives[0].manifest_jws, bundle.manifest_jws);
        let registered = repository::current_device(&db, "shared").await.unwrap();
        assert_eq!(
            read_retained(
                &db,
                DbDialect::Postgres,
                &store,
                &registered,
                "owner".into(),
                owner.public_key(),
                "segment".into(),
            )
            .await
            .unwrap_err()
            .status(),
            axum::http::StatusCode::SERVICE_UNAVAILABLE
        );
        let restored = archive_payload::stage(&store, &manifest, &bundle, encoded.clone())
            .await
            .unwrap();
        assert_eq!(restored.payload_ref.as_deref(), Some(reference.as_str()));
        let downloaded = read_retained(
            &db,
            DbDialect::Postgres,
            &store,
            &registered,
            "owner".into(),
            owner.public_key(),
            "segment".into(),
        )
        .await
        .unwrap();
        assert_eq!(serde_json::to_string(&downloaded).unwrap(), encoded);

        let mut legacy_manifest = manifest.clone();
        legacy_manifest.archive_id = "legacy".into();
        legacy_manifest.sequence = 2;
        legacy_manifest.previous_manifest_digest = Some(digest);
        let mut legacy_bundle = bundle.clone();
        legacy_bundle.manifest_jws = sign_archive_manifest(&legacy_manifest, &telemetry).unwrap();
        let legacy_encoded = serde_json::to_string(&legacy_bundle).unwrap();
        let legacy_digest = compact_digest(&legacy_bundle.manifest_jws);
        let total = (encoded.len() + legacy_encoded.len()) as u64;
        let exact_quota: TierLimits = [("PREMIUM".into(), (total, 3600))].into();
        assert!(
            retain_verified(
                &db,
                DbDialect::Postgres,
                exact_quota.clone(),
                "shared".into(),
                1,
                legacy_manifest.clone(),
                StoredArchive::inline(legacy_encoded.clone()),
                legacy_digest.clone(),
            )
            .await
            .unwrap()
        );
        let legacy_read = read_retained(
            &db,
            DbDialect::Postgres,
            &FlowLikeStore::Memory(Arc::new(InMemory::new())),
            &registered,
            "owner".into(),
            owner.public_key(),
            "legacy".into(),
        )
        .await
        .unwrap();
        assert_eq!(serde_json::to_string(&legacy_read).unwrap(), legacy_encoded);
        let page = download(&db, &owner, "owner", false).await.unwrap();
        assert_eq!(page.next, 2);
        assert_eq!(page.archives.len(), 2);
        assert!(page.archives[0].bundle.is_none());
        assert!(page.archives[1].bundle.is_some());
        let usage = usage(&db, &exact_quota, "owner", chrono::Utc::now().timestamp())
            .await
            .unwrap();
        assert_eq!(usage.used_bytes, total);
        assert_eq!(usage.devices[0].segments, 2);

        // A retry consumes no extra quota, while the next segment still needs room.
        assert!(
            retain_verified(
                &db,
                DbDialect::Postgres,
                exact_quota.clone(),
                "shared".into(),
                1,
                manifest,
                payload.clone(),
                compact_digest(&bundle.manifest_jws),
            )
            .await
            .unwrap()
        );
        let next = ArchiveManifest {
            archive_id: "next".into(),
            sequence: 3,
            previous_manifest_digest: Some(legacy_digest),
            ..legacy_manifest
        };
        let refused = retain_verified(
            &db,
            DbDialect::Postgres,
            exact_quota,
            "shared".into(),
            1,
            next,
            payload,
            "next-digest".into(),
        )
        .await
        .unwrap_err();
        assert_eq!(refused.public_code(), "ARCHIVE_QUOTA_REACHED");
        assert_eq!(
            used_bytes(&db, "owner", chrono::Utc::now().timestamp())
                .await
                .unwrap(),
            total
        );
        scratch.discard().await;
    }

    #[tokio::test]
    #[ignore = "requires FLOW_LIKE_DEVICE_TEST_DATABASE_URL pointing to a disposable PostgreSQL server"]
    async fn offloaded_download_rechecks_grants_after_object_storage_waits() {
        use flow_like_storage::{files::store::FlowLikeStore, object_store as objects};
        use futures::stream::BoxStream;
        use std::sync::Arc;
        use tokio::sync::Notify;

        #[derive(Debug)]
        struct PausedRead {
            inner: Arc<dyn objects::ObjectStore>,
            started: Arc<Notify>,
            release: Arc<Notify>,
        }
        impl std::fmt::Display for PausedRead {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("PausedRead")
            }
        }
        #[async_trait::async_trait]
        impl objects::ObjectStore for PausedRead {
            async fn put_opts(
                &self,
                path: &objects::path::Path,
                payload: objects::PutPayload,
                opts: objects::PutOptions,
            ) -> objects::Result<objects::PutResult> {
                self.inner.put_opts(path, payload, opts).await
            }
            async fn put_multipart_opts(
                &self,
                path: &objects::path::Path,
                opts: objects::PutMultipartOptions,
            ) -> objects::Result<Box<dyn objects::MultipartUpload>> {
                self.inner.put_multipart_opts(path, opts).await
            }
            async fn get_opts(
                &self,
                path: &objects::path::Path,
                opts: objects::GetOptions,
            ) -> objects::Result<objects::GetResult> {
                let result = self.inner.get_opts(path, opts).await?;
                self.started.notify_one();
                self.release.notified().await;
                Ok(result)
            }
            fn delete_stream(
                &self,
                paths: BoxStream<'static, objects::Result<objects::path::Path>>,
            ) -> BoxStream<'static, objects::Result<objects::path::Path>> {
                self.inner.delete_stream(paths)
            }
            fn list(
                &self,
                prefix: Option<&objects::path::Path>,
            ) -> BoxStream<'static, objects::Result<objects::ObjectMeta>> {
                self.inner.list(prefix)
            }
            async fn list_with_delimiter(
                &self,
                prefix: Option<&objects::path::Path>,
            ) -> objects::Result<objects::ListResult> {
                self.inner.list_with_delimiter(prefix).await
            }
            async fn copy_opts(
                &self,
                from: &objects::path::Path,
                to: &objects::path::Path,
                opts: objects::CopyOptions,
            ) -> objects::Result<()> {
                self.inner.copy_opts(from, to, opts).await
            }
        }

        let scratch = Scratch::with_accounts("('owner','PREMIUM'),('reader','FREE')").await;
        let db = scratch.db.clone();
        let telemetry = device(&db, "shared").await;
        let owner = SigningKey::generate();
        let now = chrono::Utc::now().timestamp();
        let mut policy = ManagementPolicy {
            version: 1,
            device_id: "shared".into(),
            policy_version: 1,
            previous_policy_digest: None,
            grants: vec![ManagementGrant {
                grant_id: "reader".into(),
                user_id: "reader".into(),
                controller_key: SigningKey::generate().public_key(),
                scope: ManagementScope::Device,
                capabilities: vec![ManagementCapability::Logs],
                expires_at: now + 600,
                group_id: None,
                group_version: None,
            }],
            issued_at: now - 1,
            expires_at: now + 600,
        };
        let first_policy = insert_policy(&db, &policy, &owner).await;
        let store = FlowLikeStore::Memory(Arc::new(objects::memory::InMemory::new()));
        let bundle = signed_archive(&owner, &telemetry);
        let (manifest, _) =
            verify_archive_content(&bundle, &owner.public_key(), &telemetry.public_key()).unwrap();
        let payload = archive_payload::stage(
            &store,
            &manifest,
            &bundle,
            serde_json::to_string(&bundle).unwrap(),
        )
        .await
        .unwrap();
        retain_verified(
            &db,
            DbDialect::Postgres,
            limits(),
            "shared".into(),
            1,
            manifest,
            payload,
            compact_digest(&bundle.manifest_jws),
        )
        .await
        .unwrap();
        let started = Arc::new(Notify::new());
        let release = Arc::new(Notify::new());
        let delayed = FlowLikeStore::Other(Arc::new(PausedRead {
            inner: store.as_generic(),
            started: started.clone(),
            release: release.clone(),
        }));
        let registered = repository::current_device(&db, "shared").await.unwrap();
        let mut read = Box::pin(read_retained(
            &db,
            DbDialect::Postgres,
            &delayed,
            &registered,
            "reader".into(),
            owner.public_key(),
            "segment".into(),
        ));
        tokio::select! {
            _ = started.notified() => {}
            result = &mut read => panic!("download finished before the storage wait: {result:?}"),
        }
        let change = db.begin().await.unwrap();
        repository::lock_active_device(&change, "shared", 1)
            .await
            .unwrap();
        policy.policy_version = 2;
        policy.previous_policy_digest = Some(compact_digest(&first_policy));
        policy.grants[0].capabilities = vec![ManagementCapability::Status];
        insert_policy(&change, &policy, &owner).await;
        change.commit().await.unwrap();
        release.notify_one();
        assert_eq!(
            read.await.unwrap_err().status(),
            axum::http::StatusCode::FORBIDDEN
        );
        scratch.discard().await;
    }

    #[tokio::test]
    #[ignore = "requires FLOW_LIKE_DEVICE_TEST_DATABASE_URL pointing to a disposable PostgreSQL server"]
    async fn archive_downloads_fence_revocation_and_expiry_during_database_waits() {
        use std::time::Duration;
        let scratch =
            Scratch::with_accounts("('owner','PREMIUM'),('reader','FREE'),('nonrecipient','FREE')")
                .await;
        let db = scratch.db.clone();
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
            StoredArchive::inline(serde_json::to_string(&bundle).unwrap()),
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
        scratch.discard().await;
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
            StoredArchive::inline("x".repeat(60000)),
            digest.into(),
        )
        .await
    }

    #[tokio::test]
    #[ignore = "requires FLOW_LIKE_DEVICE_TEST_DATABASE_URL pointing to a disposable PostgreSQL server"]
    async fn archive_quota_chain_expiry_and_revocation_are_transactional() {
        let scratch = Scratch::with_accounts("('owner','PREMIUM')").await;
        let db = scratch.db.clone();
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
        let refused = retain(&db, next.clone(), "next").await.unwrap_err();
        assert_eq!(
            (refused.status(), refused.public_code()),
            (
                axum::http::StatusCode::PAYMENT_REQUIRED,
                "ARCHIVE_TIER_WITHOUT_HISTORY"
            )
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
        scratch.discard().await;
    }

    #[test]
    fn a_tier_keeps_history_only_with_both_limits_and_never_past_the_hub_maximum() {
        let limits: TierLimits = [
            ("PRO".into(), (1024, 3600)),
            ("LONG".into(), (1024, 40 * 86_400)),
            ("NO_BYTES".into(), (0, 3600)),
            ("NO_TIME".into(), (1024, 0)),
        ]
        .into();
        assert_eq!(tier_limit(&limits, "PRO"), Some((1024, 3600)));
        assert_eq!(
            tier_limit(&limits, "LONG"),
            Some((1024, MAX_RETENTION_SECONDS))
        );
        for tier in ["NO_BYTES", "NO_TIME", "FREE"] {
            assert_eq!(tier_limit(&limits, tier), None, "{tier}");
        }
    }

    #[tokio::test]
    #[ignore = "requires FLOW_LIKE_DEVICE_TEST_DATABASE_URL pointing to a disposable PostgreSQL server"]
    async fn archive_usage_counts_unexpired_history_per_device_against_the_plan() {
        let scratch = Scratch::with_accounts("('owner','premium'),('other','FREE')").await;
        let db = scratch.db.clone();
        let limits: TierLimits = [("PREMIUM".into(), (200_000, 40 * 86_400))].into();
        let now = chrono::Utc::now().timestamp();
        let plan = |used_bytes, devices| ArchiveUsage {
            tier: "PREMIUM".into(),
            max_bytes: 200_000,
            retention_seconds: MAX_RETENTION_SECONDS,
            used_bytes,
            devices,
        };
        let held = |device: &str, segments, oldest, newest: i64| ArchiveDeviceUsage {
            device_id: device.into(),
            used_bytes: segments * 60_000,
            segments,
            oldest_created_at: oldest,
            newest_expires_at: newest + MAX_RETENTION_SECONDS as i64,
        };
        assert_eq!(
            usage(&db, &limits, "owner", now).await.unwrap(),
            plan(0, vec![])
        );

        device(&db, "one").await;
        device(&db, "two").await;
        let segment = |device: &str, sequence: u64, created_at| ArchiveManifest {
            sequence,
            previous_manifest_digest: (sequence > 1).then(|| format!("{device}-{}", sequence - 1)),
            created_at,
            ..manifest(device)
        };
        let store = |manifest: ArchiveManifest| {
            let digest = format!("{}-{}", manifest.device_id, manifest.sequence);
            retain_verified(
                &db,
                DbDialect::Postgres,
                limits.clone(),
                manifest.device_id.clone(),
                1,
                manifest,
                StoredArchive::inline("x".repeat(60_000)),
                digest,
            )
        };
        for manifest in [
            segment("one", 1, now - 100),
            segment("one", 2, now - 50),
            segment("two", 1, now - 10),
        ] {
            assert!(store(manifest).await.unwrap());
        }
        assert_eq!(
            usage(&db, &limits, "owner", now).await.unwrap(),
            plan(
                180_000,
                vec![
                    held("one", 2, now - 100, now - 50),
                    held("two", 1, now - 10, now - 10)
                ]
            )
        );
        // The usage is the sum the quota enforces: full now, room once a segment expires.
        let refused = store(segment("two", 2, now - 5)).await.unwrap_err();
        assert_eq!(
            (refused.status(), refused.public_code()),
            (
                axum::http::StatusCode::PAYMENT_REQUIRED,
                "ARCHIVE_QUOTA_REACHED"
            )
        );
        db.execute_raw(sql(
            r#"UPDATE "DeviceArchive" SET "expiresAt"=$1 WHERE "deviceId"='one' AND sequence=1"#,
            [now.into()],
        ))
        .await
        .unwrap();
        db.execute_raw(sql(
            r#"INSERT INTO "DeviceArchive"("deviceId","archiveId","ownerId",scope,kind,sequence,digest,bundle,"sizeBytes","createdAt","expiresAt") VALUES('foreign','segment','other','device','logs',1,'digest','x',999,$1,$2)"#,
            [(now - 1).into(), (now + 1).into()],
        ))
        .await
        .unwrap();
        assert_eq!(
            usage(&db, &limits, "owner", now).await.unwrap(),
            plan(
                120_000,
                vec![
                    held("one", 1, now - 50, now - 50),
                    held("two", 1, now - 10, now - 10)
                ]
            )
        );
        assert!(store(segment("two", 2, now - 5)).await.unwrap());
        assert_eq!(
            usage(&db, &limits, "owner", now).await.unwrap().used_bytes,
            180_000
        );

        // A tier without history still shows what it holds until that expires.
        assert_eq!(
            usage(&db, &limits, "other", now).await.unwrap(),
            ArchiveUsage {
                tier: "FREE".into(),
                max_bytes: 0,
                retention_seconds: 0,
                used_bytes: 999,
                devices: vec![ArchiveDeviceUsage {
                    device_id: "foreign".into(),
                    used_bytes: 999,
                    segments: 1,
                    oldest_created_at: now - 1,
                    newest_expires_at: now + 1,
                }],
            }
        );
        assert_eq!(
            usage(&db, &limits, "nobody", now)
                .await
                .unwrap_err()
                .status(),
            axum::http::StatusCode::FORBIDDEN
        );
        scratch.discard().await;
    }

    #[tokio::test]
    #[ignore = "requires FLOW_LIKE_DEVICE_TEST_DATABASE_URL pointing to a disposable PostgreSQL server"]
    async fn usage_counter_starts_from_stored_rows_and_moves_with_uploads_refusals_and_sweeps() {
        let scratch = Scratch::with_accounts("('owner','PREMIUM')").await;
        let db = scratch.db.clone();
        device(&db, "one").await;
        let counter = || async {
            db.query_one_raw(sql(
                r#"SELECT "usedBytes" FROM "DeviceArchiveUsage" WHERE "ownerId"='owner'"#,
                [],
            ))
            .await
            .unwrap()
            .map(|row| row.try_get::<i64>("", "usedBytes").unwrap())
        };
        let store = |sequence: u64, max_bytes: u64| {
            retain_verified(
                &db,
                DbDialect::Postgres,
                [("PREMIUM".into(), (max_bytes, 3600))].into(),
                "one".into(),
                1,
                ArchiveManifest {
                    sequence,
                    previous_manifest_digest: (sequence > 1)
                        .then(|| format!("one-{}", sequence - 1)),
                    ..manifest("one")
                },
                StoredArchive::inline("x".repeat(60_000)),
                format!("one-{sequence}"),
            )
        };
        let expire = |sequence: i64| {
            db.execute_raw(sql(
                r#"UPDATE "DeviceArchive" SET "expiresAt"=0 WHERE scope='device' AND sequence=$1"#,
                [sequence.into()],
            ))
        };
        let sweep = || {
            sweep_owner(
                &db,
                DbDialect::Postgres,
                "owner".into(),
                chrono::Utc::now().timestamp(),
            )
        };

        // An owner from before the counter: one kept segment and more expired rows than a batch.
        assert!(store(1, 1_000_000).await.unwrap());
        db.execute_unprepared(r#"DELETE FROM "DeviceArchiveUsage"; INSERT INTO "DeviceArchive"("deviceId","archiveId","ownerId",scope,kind,sequence,digest,bundle,"sizeBytes","createdAt","expiresAt") SELECT 'one','expired-'||n,'owner','old','logs',n,'digest','x',1000,0,0 FROM generate_series(1,200) AS n"#).await.unwrap();
        // The counter still holds the 72 expired rows past the batch; the exact sum admits.
        assert!(store(2, 150_000).await.unwrap());
        assert_eq!(counter().await, Some(192_000));
        assert_eq!(used_bytes(&db, "owner", i64::MIN).await.unwrap(), 192_000);
        // A refusal still commits the expired rows it removed.
        let refused = store(3, 150_000).await.unwrap_err();
        assert_eq!(refused.public_code(), "ARCHIVE_QUOTA_REACHED");
        assert_eq!(counter().await, Some(120_000));
        assert!(store(3, 180_000).await.unwrap());
        assert_eq!(counter().await, Some(180_000));

        expire(1).await.unwrap();
        assert_eq!(sweep().await.unwrap(), 1);
        assert_eq!(counter().await, Some(120_000));
        // A counter below what a delete releases is recounted from the rows.
        db.execute_unprepared(r#"UPDATE "DeviceArchiveUsage" SET "usedBytes"=0"#)
            .await
            .unwrap();
        expire(2).await.unwrap();
        assert_eq!(sweep().await.unwrap(), 1);
        assert_eq!(counter().await, Some(60_000));
        assert_eq!(sweep().await.unwrap(), 0);
        scratch.discard().await;
    }

    #[tokio::test]
    #[ignore = "requires FLOW_LIKE_DEVICE_TEST_DATABASE_URL pointing to a disposable PostgreSQL server"]
    async fn inline_bundles_move_to_content_storage_newest_first_and_stay_readable() {
        use flow_like_storage::{files::store::FlowLikeStore, object_store::memory::InMemory};
        use std::sync::Arc;

        let scratch = Scratch::with_accounts("('owner','PREMIUM')").await;
        let db = scratch.db.clone();
        let telemetry = device(&db, "shared").await;
        let owner = SigningKey::generate();
        let store = FlowLikeStore::Memory(Arc::new(InMemory::new()));
        let signed = signed_archive(&owner, &telemetry);
        let (first, _) =
            verify_archive_content(&signed, &owner.public_key(), &telemetry.public_key()).unwrap();
        let next = |sequence: u64, previous: &EncryptedArchive| {
            let bundle = EncryptedArchive {
                manifest_jws: format!("manifest-{sequence}"),
                roster_jws: "roster".into(),
                ciphertext: "ciphertext".into(),
                recipient_keys: Vec::new(),
            };
            let manifest = ArchiveManifest {
                archive_id: format!("archive-{sequence}"),
                sequence,
                previous_manifest_digest: Some(compact_digest(&previous.manifest_jws)),
                ..first.clone()
            };
            (manifest, bundle)
        };
        let retain = |manifest: ArchiveManifest, bundle: &EncryptedArchive, payload| {
            retain_verified(
                &db,
                DbDialect::Postgres,
                limits(),
                "shared".into(),
                1,
                manifest,
                payload,
                compact_digest(&bundle.manifest_jws),
            )
        };
        let mut bundles = vec![(first.clone(), signed.clone())];
        for sequence in 2..=5 {
            bundles.push(next(sequence, &bundles.last().unwrap().1));
        }
        for (manifest, bundle) in &bundles {
            let inline = StoredArchive::inline(serde_json::to_string(bundle).unwrap());
            assert!(retain(manifest.clone(), bundle, inline).await.unwrap());
        }
        let (manifest, bundle) = next(6, &bundles.last().unwrap().1);
        let staged = archive_payload::stage(
            &store,
            &manifest,
            &bundle,
            serde_json::to_string(&bundle).unwrap(),
        )
        .await
        .unwrap();
        assert!(retain(manifest, &bundle, staged).await.unwrap());

        let inline = || async {
            db.query_all_raw(sql(
                r#"SELECT sequence FROM "DeviceArchive" WHERE bundle IS NOT NULL ORDER BY sequence"#,
                [],
            ))
            .await
            .unwrap()
            .into_iter()
            .map(|row| row.try_get::<i64>("", "sequence").unwrap())
            .collect::<Vec<_>>()
        };
        let offload = |budget| {
            archive_payload::offload_inline(&db, &store, chrono::Utc::now().timestamp(), budget)
        };
        assert_eq!(inline().await, [1, 2, 3, 4, 5]);
        assert_eq!(offload(2).await.unwrap(), 2);
        assert_eq!(inline().await, [1, 2, 3]);
        // A row that cannot move stops its stream there, so the inline rows stay a prefix.
        db.execute_unprepared(r#"UPDATE "DeviceArchive" SET bundle='damaged' WHERE sequence=3"#)
            .await
            .unwrap();
        assert_eq!(offload(10).await.unwrap(), 0);
        assert_eq!(inline().await, [1, 2, 3]);
        db.execute_raw(sql(
            r#"UPDATE "DeviceArchive" SET bundle=$1 WHERE sequence=3"#,
            [serde_json::to_string(&bundles[2].1).unwrap().into()],
        ))
        .await
        .unwrap();
        assert_eq!(offload(10).await.unwrap(), 3);
        assert!(inline().await.is_empty());
        assert_eq!(offload(10).await.unwrap(), 0);

        for (manifest, bundle) in &bundles {
            let encoded = serde_json::to_string(bundle).unwrap();
            let row = db
                .query_one_raw(sql(
                    r#"SELECT "payloadRef","manifestJws","rosterJws","sizeBytes" FROM "DeviceArchive" WHERE "archiveId"=$1"#,
                    [manifest.archive_id.clone().into()],
                ))
                .await
                .unwrap()
                .unwrap();
            assert_eq!(
                row.try_get::<String>("", "manifestJws").unwrap(),
                bundle.manifest_jws
            );
            assert_eq!(
                row.try_get::<String>("", "rosterJws").unwrap(),
                bundle.roster_jws
            );
            let size = row.try_get::<i64>("", "sizeBytes").unwrap();
            assert_eq!(size, encoded.len() as i64);
            let reference = row.try_get::<String>("", "payloadRef").unwrap();
            let moved = archive_payload::load(&store, manifest, &reference, size)
                .await
                .unwrap();
            assert_eq!(serde_json::to_string(&moved).unwrap(), encoded);
        }
        let registered = repository::current_device(&db, "shared").await.unwrap();
        let read = read_retained(
            &db,
            DbDialect::Postgres,
            &store,
            &registered,
            "owner".into(),
            owner.public_key(),
            first.archive_id.clone(),
        )
        .await
        .unwrap();
        assert_eq!(
            serde_json::to_string(&read).unwrap(),
            serde_json::to_string(&signed).unwrap()
        );
        scratch.discard().await;
    }
}
