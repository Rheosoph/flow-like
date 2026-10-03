use crate::{
    db::{DbDialect, RetryPolicy, retry_transaction},
    error::ApiError,
};
use flow_like_device_protocol::{
    DeviceIdentity, DeviceReceipt, DeviceRegistrationStatus, DeviceStatus, Ed25519PublicKey,
    OnboardingManifest, compact_digest,
};
use sea_orm::{
    ConnectionTrait, DatabaseBackend, DatabaseConnection, DatabaseTransaction, QueryResult,
    Statement, Value,
};

fn sql(query: &str, values: impl IntoIterator<Item = Value>) -> Statement {
    Statement::from_sql_and_values(DatabaseBackend::Postgres, query, values)
}

pub(super) fn expired_authorization() -> ApiError {
    super::device_proof_invalid("Device authorization expired while waiting for the database")
}

fn lapsed(expires_at: i64) -> bool {
    expires_at <= chrono::Utc::now().timestamp()
}

/// Owner sessions and enrollment tokens are not device proofs, so a lapse
/// during a slow database wait is reported as retryable, never as clock skew.
fn require_live(expires_at: i64) -> Result<(), ApiError> {
    if lapsed(expires_at) {
        return Err(ApiError::service_unavailable(format!(
            "Authorization valid until {expires_at} expired while waiting for the database; retry the request"
        )));
    }
    Ok(())
}

fn require_live_proof(expires_at: i64) -> Result<(), ApiError> {
    if lapsed(expires_at) {
        return Err(expired_authorization());
    }
    Ok(())
}

#[cfg(test)]
mod tests;

pub struct Repository<'a> {
    pub db: &'a DatabaseConnection,
    pub dialect: DbDialect,
}

#[derive(Clone)]
pub struct Enrollment {
    pub manifest: OnboardingManifest,
    pub jwt_id: String,
    pub status: String,
}

#[derive(Clone)]
pub struct Device {
    pub status: DeviceStatus,
    pub receipt: DeviceReceipt,
}

fn enrollment(row: QueryResult) -> Result<Enrollment, ApiError> {
    Ok(Enrollment {
        manifest: serde_json::from_str(&row.try_get::<String>("", "manifest")?)?,
        jwt_id: row.try_get("", "jwtId")?,
        status: row.try_get("", "status")?,
    })
}

/// Why the hub last refused a device, stored in `"DeviceAuthRejection".code`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
#[schema(as = DeviceAuthRejectionCode)]
pub(crate) enum AuthRejectionCode {
    /// A proof signed by the registered key was outside its validity window.
    ClockSkew,
    /// A session issued by this hub no longer matches the registration.
    RevokedCredential,
}

impl AuthRejectionCode {
    pub(crate) fn as_str(&self) -> &str {
        match self {
            Self::ClockSkew => "clock_skew",
            Self::RevokedCredential => "revoked_credential",
        }
    }

    pub(crate) fn parse(code: &str) -> Option<Self> {
        [Self::ClockSkew, Self::RevokedCredential]
            .into_iter()
            .find(|known| known.as_str() == code)
    }
}

/// A pending or cancelled setup package. The manifest is read only for its name and
/// controller key; the bootstrap key and the token id never leave the registry.
pub(crate) struct EnrollmentRecord {
    pub enrollment_id: String,
    pub device_id: String,
    pub name: String,
    pub controller_key: Ed25519PublicKey,
    pub cancelled: bool,
    pub created_at: i64,
    pub expires_at: i64,
}

#[derive(serde::Deserialize)]
struct ManifestSummary {
    name: String,
    controller_key: Ed25519PublicKey,
}

pub(crate) const MAX_LISTED_ENROLLMENTS: usize = 200;

fn enrollment_record(row: QueryResult) -> Result<EnrollmentRecord, ApiError> {
    let manifest: ManifestSummary = serde_json::from_str(&row.try_get::<String>("", "manifest")?)?;
    Ok(EnrollmentRecord {
        enrollment_id: row.try_get("", "id")?,
        device_id: row.try_get("", "deviceId")?,
        name: manifest.name,
        controller_key: manifest.controller_key,
        cancelled: row.try_get::<String>("", "status")? == "cancelled",
        created_at: row.try_get("", "createdAt")?,
        expires_at: row.try_get("", "expiresAt")?,
    })
}

pub(crate) fn status(row: &QueryResult) -> Result<DeviceStatus, ApiError> {
    let status: String = row.try_get("", "status")?;
    let epoch: i64 = row.try_get("", "authEpoch")?;
    Ok(DeviceStatus {
        device_id: row.try_get("", "id")?,
        owner_id: row.try_get("", "ownerId")?,
        name: row.try_get("", "name")?,
        status: match status.as_str() {
            "active" => DeviceRegistrationStatus::Active,
            "revoked" => DeviceRegistrationStatus::Revoked,
            _ => return Err(ApiError::internal("Invalid device registry status")),
        },
        identity: serde_json::from_str::<DeviceIdentity>(&row.try_get::<String>("", "identity")?)?,
        registered_at: row.try_get("", "registeredAt")?,
        last_seen_at: row.try_get("", "lastSeenAt")?,
        auth_epoch: u64::try_from(epoch)
            .map_err(|_| ApiError::internal("Invalid device key epoch"))?,
    })
}

pub(crate) fn device(row: QueryResult) -> Result<Device, ApiError> {
    Ok(Device {
        status: status(&row)?,
        receipt: serde_json::from_str(&row.try_get::<String>("", "receipt")?)?,
    })
}

pub(crate) async fn active_account<C: ConnectionTrait>(
    db: &C,
    owner: &str,
) -> Result<(), ApiError> {
    let row = db
        .query_one_raw(sql(
            r#"SELECT id FROM "User" WHERE id = $1 AND status = 'ACTIVE'"#,
            [owner.into()],
        ))
        .await?;
    if row.is_none() {
        return Err(ApiError::forbidden("An active account is required"));
    }
    Ok(())
}

pub(crate) async fn current_device<C: ConnectionTrait>(
    db: &C,
    id: &str,
) -> Result<Device, ApiError> {
    db.query_one_raw(sql(
        r#"SELECT * FROM "ManagedDevice" WHERE id = $1"#,
        [id.into()],
    ))
    .await?
    .ok_or(ApiError::NOT_FOUND)
    .and_then(device)
}

/// Rewriting the active row creates a conflict with a concurrent revocation on
/// optimistic engines too. A process-local mutex cannot provide that guarantee.
pub(crate) async fn lock_active_device(
    tx: &DatabaseTransaction,
    id: &str,
    epoch: u64,
) -> Result<Device, ApiError> {
    let epoch = i64::try_from(epoch).map_err(|_| ApiError::UNAUTHORIZED)?;
    let changed = tx.execute_raw(sql(r#"UPDATE "ManagedDevice" SET "authEpoch" = "authEpoch" WHERE id = $1 AND status = 'active' AND "authEpoch" = $2"#,
        [id.into(), epoch.into()])).await?.rows_affected();
    if changed != 1 {
        return Err(ApiError::unauthorized(
            "Device registration is no longer active",
        ));
    }
    let current = current_device(tx, id).await?;
    active_account(tx, &current.status.owner_id).await?;
    Ok(current)
}

/// Unredeemed packages stay inspectable for a week after they lapse.
const ABANDONED_ENROLLMENT_GRACE_SECONDS: i64 = 7 * 86_400;
const ABANDONED_ENROLLMENT_BATCH: u64 = 128;
const ENROLLMENT_CREATION_WINDOW_SECONDS: i64 = 86_400;
pub(crate) const AUTH_REJECTION_THROTTLE_SECONDS: i64 = 10;

pub(crate) fn count(row: &QueryResult, column: &str) -> Result<u64, ApiError> {
    u64::try_from(row.try_get::<i64>("", column)?)
        .map_err(|_| ApiError::internal(format!("Invalid device registry count {column}")))
}

/// What an owner uses of the enrollment limits. Creating a package enforces these
/// numbers and the usage view reports them, so both read them here.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct EnrollmentCounts {
    /// Setup packages that can still be started.
    pub pending: u64,
    pub active_devices: u64,
    /// Packages created in the last 24 hours, whatever became of them.
    pub last_day: u64,
}

pub(crate) async fn enrollment_counts<C: ConnectionTrait>(
    db: &C,
    owner: &str,
    now: i64,
) -> Result<EnrollmentCounts, ApiError> {
    let row = db.query_one_raw(sql(r#"SELECT (SELECT COUNT(*) FROM "DeviceEnrollment" WHERE "ownerId" = $1 AND status = 'pending' AND "expiresAt" > $2) AS pending, (SELECT COUNT(*) FROM "ManagedDevice" WHERE "ownerId" = $1 AND status = 'active') AS devices, (SELECT COUNT(*) FROM "DeviceEnrollment" WHERE "ownerId" = $1 AND "createdAt" > $3) AS recent"#,
        [owner.into(), now.into(), (now - ENROLLMENT_CREATION_WINDOW_SECONDS).into()])).await?
        .ok_or_else(|| ApiError::internal(format!("Missing enrollment counts for owner {owner}")))?;
    Ok(EnrollmentCounts {
        pending: count(&row, "pending")?,
        active_devices: count(&row, "devices")?,
        last_day: count(&row, "recent")?,
    })
}

/// Cancelled and lapsed packages are retained until pruning, so creation is bounded
/// too: at most two full fleets' worth of packages per day.
pub(crate) fn daily_enrollment_limit(max_devices: u32, max_pending: u32) -> u64 {
    2 * (u64::from(max_devices) + u64::from(max_pending))
}

impl EnrollmentCounts {
    /// Whether the owner may create one more setup package.
    fn admit(&self, max_devices: u32, max_pending: u32) -> Result<(), ApiError> {
        if self.pending >= u64::from(max_pending)
            || self.active_devices + self.pending >= u64::from(max_devices)
        {
            return Err(ApiError::too_many_requests(
                "Device enrollment limit reached",
            ));
        }
        let (daily_limit, recent) = (
            daily_enrollment_limit(max_devices, max_pending),
            self.last_day,
        );
        if recent >= daily_limit {
            return Err(ApiError::too_many_requests(format!(
                "Device enrollment packages are limited to {daily_limit} per day; {recent} were created in the last 24 hours"
            )));
        }
        Ok(())
    }
}

/// Cancelled and expired enrollments never become devices, so they and their
/// challenges are removed after a grace period. Consumed enrollments remain as
/// the manifest trust record of their device.
pub(crate) async fn prune_abandoned_enrollments(
    db: &DatabaseConnection,
    dialect: DbDialect,
    now: i64,
) -> Result<u64, ApiError> {
    let cutoff = now - ABANDONED_ENROLLMENT_GRACE_SECONDS;
    retry_transaction(db, dialect, None, &RetryPolicy::default(), move |tx| {
        Box::pin(async move {
            let ids = tx.query_all_raw(sql(&format!(r#"SELECT id FROM "DeviceEnrollment" WHERE status IN ('pending','cancelled') AND "expiresAt" < $1 ORDER BY "expiresAt", id LIMIT {ABANDONED_ENROLLMENT_BATCH}"#), [cutoff.into()])).await?
                .into_iter()
                .map(|row| row.try_get::<String>("", "id"))
                .collect::<Result<Vec<_>, _>>()?;
            if ids.is_empty() {
                return Ok(0);
            }
            let placeholders = (1..=ids.len()).map(|index| format!("${index}")).collect::<Vec<_>>().join(",");
            let values = ids.iter().map(|id| Value::from(id.clone())).collect::<Vec<_>>();
            tx.execute_raw(sql(&format!(r#"DELETE FROM "DeviceChallenge" WHERE "enrollmentId" IN ({placeholders})"#), values.clone())).await?;
            let cutoff_index = ids.len() + 1;
            Ok(tx.execute_raw(sql(&format!(r#"DELETE FROM "DeviceEnrollment" WHERE id IN ({placeholders}) AND status IN ('pending','cancelled') AND "expiresAt" < ${cutoff_index}"#), values.into_iter().chain([cutoff.into()]))).await?.rows_affected())
        })
    })
    .await
}

pub(crate) async fn consume_proof(
    tx: &DatabaseTransaction,
    id: &str,
    epoch: u64,
    proof_id: &str,
    expires_at: i64,
) -> Result<(), ApiError> {
    let inserted = tx.execute_raw(sql(r#"INSERT INTO "DeviceProofReplay" ("deviceId", "authEpoch", "proofId", "expiresAt") VALUES ($1, $2, $3, $4) ON CONFLICT ("deviceId", "authEpoch", "proofId") DO NOTHING"#,
        [id.into(), (epoch as i64).into(), proof_id.into(), expires_at.into()])).await?.rows_affected();
    if inserted != 1 {
        return Err(super::device_proof_invalid(format!(
            "Device proof {proof_id} was already used"
        )));
    }
    // Proofs last at most 60 seconds. Keep an additional minute before removing
    // a bounded batch, including when different API replicas have clock skew.
    let cutoff = chrono::Utc::now().timestamp() - 60;
    tx.execute_raw(sql(r#"DELETE FROM "DeviceProofReplay" WHERE ("deviceId", "authEpoch", "proofId") IN (SELECT "deviceId", "authEpoch", "proofId" FROM "DeviceProofReplay" WHERE "expiresAt" < $1 AND "deviceId" = $2 LIMIT 128)"#, [cutoff.into(), id.into()])).await?;
    Ok(())
}

impl Repository<'_> {
    pub async fn active_account(&self, owner: &str) -> Result<(), ApiError> {
        active_account(self.db, owner).await
    }

    pub async fn enrollment(&self, id: &str) -> Result<Enrollment, ApiError> {
        self.db
            .query_one_raw(sql(
                r#"SELECT * FROM "DeviceEnrollment" WHERE id = $1"#,
                [id.into()],
            ))
            .await?
            .ok_or(ApiError::NOT_FOUND)
            .and_then(enrollment)
    }

    pub async fn device(&self, id: &str) -> Result<Device, ApiError> {
        current_device(self.db, id).await
    }

    /// Setup packages that never became a device, newest first. Lapsed pending ones stay
    /// listed until `prune_abandoned_enrollments` removes them.
    pub async fn enrollments(
        &self,
        owner: &str,
        with_cancelled: bool,
    ) -> Result<Vec<EnrollmentRecord>, ApiError> {
        let statuses = if with_cancelled {
            "'pending','cancelled'"
        } else {
            "'pending'"
        };
        self.db.query_all_raw(sql(&format!(r#"SELECT id,"deviceId",manifest,status,"createdAt","expiresAt" FROM "DeviceEnrollment" WHERE "ownerId" = $1 AND status IN ({statuses}) ORDER BY "createdAt" DESC, id LIMIT {MAX_LISTED_ENROLLMENTS}"#), [owner.into()])).await?
            .into_iter().map(enrollment_record).collect()
    }

    /// `None` clears the owner's label. The signed setup name is never touched.
    pub async fn rename(
        &self,
        owner: &str,
        id: &str,
        display_name: Option<String>,
    ) -> Result<(), ApiError> {
        active_account(self.db, owner).await?;
        let changed=self.db.execute_raw(sql(r#"UPDATE "ManagedDevice" SET "displayName" = $1 WHERE id = $2 AND "ownerId" = $3 AND status = 'active'"#,[display_name.into(),id.into(),owner.into()])).await?.rows_affected();
        if changed != 1 {
            return Err(ApiError::NOT_FOUND);
        }
        Ok(())
    }

    /// One row per device, overwritten and throttled, so a retrying device cannot
    /// amplify writes. `count` and `firstAt` restart when the reason changes or the
    /// device has checked in since the previous rejection.
    pub async fn record_auth_rejection(
        &self,
        id: &str,
        code: AuthRejectionCode,
        skew_seconds: Option<i64>,
        last_seen_at: Option<i64>,
        now: i64,
    ) -> Result<(), ApiError> {
        const CONTINUES: &str =
            r#""DeviceAuthRejection".code = EXCLUDED.code AND "DeviceAuthRejection"."lastAt" > $5"#;
        self.db.execute_raw(sql(&format!(r#"INSERT INTO "DeviceAuthRejection" ("deviceId",code,"skewSeconds",count,"firstAt","lastAt") VALUES ($1,$2,$3,1,$4,$4)
            ON CONFLICT ("deviceId") DO UPDATE SET code = EXCLUDED.code, "skewSeconds" = EXCLUDED."skewSeconds", "lastAt" = EXCLUDED."lastAt",
                count = CASE WHEN {CONTINUES} THEN "DeviceAuthRejection".count + 1 ELSE 1 END,
                "firstAt" = CASE WHEN {CONTINUES} THEN "DeviceAuthRejection"."firstAt" ELSE EXCLUDED."firstAt" END
            WHERE "DeviceAuthRejection"."lastAt" < EXCLUDED."lastAt" - {AUTH_REJECTION_THROTTLE_SECONDS}"#),
            [id.into(),code.as_str().into(),skew_seconds.into(),now.into(),last_seen_at.unwrap_or_default().into()])).await?;
        Ok(())
    }

    pub async fn create_enrollment(
        &self,
        template: &OnboardingManifest,
        jwt_id: &str,
        max_devices: u32,
        max_pending: u32,
    ) -> Result<(), ApiError> {
        let template = template.clone();
        let json = serde_json::to_string(&template)?;
        let jwt_id = jwt_id.to_owned();
        retry_transaction(self.db, self.dialect, None, &RetryPolicy::default(), move |tx| {
            let template = template.clone(); let json = json.clone(); let jwt_id = jwt_id.clone();
            Box::pin(async move {
                // Every enrollment issuer and redeemer for this owner writes this
                // existing row, preventing quota write skew across API replicas.
                let updated = tx.execute_raw(sql(r#"UPDATE "User" SET "updatedAt" = now() WHERE id = $1 AND status = 'ACTIVE'"#, [template.owner_id.clone().into()])).await?.rows_affected();
                if updated != 1 { return Err(ApiError::forbidden("An active account is required")); }
                require_live(template.expires_at)?;
                let now = chrono::Utc::now().timestamp();
                enrollment_counts(tx, &template.owner_id, now).await?.admit(max_devices, max_pending)?;
                tx.execute_raw(sql(r#"INSERT INTO "DeviceEnrollment" (id, "deviceId", "ownerId", "jwtId", manifest, status, "expiresAt", "createdAt") VALUES ($1,$2,$3,$4,$5,'pending',$6,$7)"#,
                    [template.enrollment_id.into(),template.device_id.into(),template.owner_id.into(),jwt_id.into(),json.into(),template.expires_at.into(),template.issued_at.into()])).await?;
                require_live(template.expires_at)?;
                Ok(())
            })
        }).await
    }

    /// Returns the outstanding challenge while it is unexpired, otherwise stores
    /// `challenge_id` and returns it. The locked enrollment row serializes both.
    pub async fn challenge(
        &self,
        enrollment_id: &str,
        jwt_id: &str,
        challenge_id: &str,
        expires_at: i64,
        now: i64,
    ) -> Result<(String, i64), ApiError> {
        let enrollment_id = enrollment_id.to_owned();
        let jwt_id = jwt_id.to_owned();
        let challenge_id = challenge_id.to_owned();
        retry_transaction(self.db,self.dialect,None,&RetryPolicy::default(),move |tx| {
            let enrollment_id=enrollment_id.clone(); let jwt_id=jwt_id.clone(); let challenge_id=challenge_id.clone();
            Box::pin(async move {
                let changed=tx.execute_raw(sql(r#"UPDATE "DeviceEnrollment" SET status = status WHERE id = $1 AND "jwtId" = $2 AND status = 'pending' AND "expiresAt" > $3"#,
                    [enrollment_id.clone().into(),jwt_id.into(),now.into()])).await?.rows_affected();
                if changed != 1 { return Err(ApiError::unauthorized("Enrollment is no longer pending")); }
                let record=tx.query_one_raw(sql(r#"SELECT * FROM "DeviceEnrollment" WHERE id = $1"#,[enrollment_id.clone().into()])).await?.ok_or(ApiError::NOT_FOUND).and_then(enrollment)?;
                active_account(tx,&record.manifest.owner_id).await?;
                require_live(expires_at.min(record.manifest.expires_at))?;
                if let Some(outstanding)=tx.query_one_raw(sql(r#"SELECT id,"nonceHash","expiresAt" FROM "DeviceChallenge" WHERE "enrollmentId" = $1 AND "expiresAt" > $2"#,
                    [enrollment_id.clone().into(),now.into()])).await? {
                    let id=outstanding.try_get::<String>("","id")?;
                    // Challenges stored before nonces were derived cannot be reissued.
                    if outstanding.try_get::<String>("","nonceHash")? == compact_digest(&super::challenge_nonce(&id)) {
                        return Ok((id,outstanding.try_get("","expiresAt")?));
                    }
                }
                let nonce_hash=compact_digest(&super::challenge_nonce(&challenge_id));
                tx.execute_raw(sql(r#"INSERT INTO "DeviceChallenge" ("enrollmentId",id,"nonceHash","expiresAt") VALUES ($1,$2,$3,$4) ON CONFLICT ("enrollmentId") DO UPDATE SET id = EXCLUDED.id, "nonceHash" = EXCLUDED."nonceHash", "expiresAt" = EXCLUDED."expiresAt""#,
                    [enrollment_id.into(),challenge_id.clone().into(),nonce_hash.into(),expires_at.into()])).await?;
                require_live(expires_at.min(record.manifest.expires_at))?;
                Ok((challenge_id,expires_at))
            })
        }).await
    }

    pub async fn redeem(
        &self,
        receipt: &DeviceReceipt,
        jwt_id: &str,
        challenge_id: &str,
        nonce_hash: &str,
        proof_expires_at: i64,
        max_devices: u32,
    ) -> Result<DeviceReceipt, ApiError> {
        let receipt = receipt.clone();
        let jwt_id = jwt_id.to_owned();
        let challenge_id = challenge_id.to_owned();
        let nonce_hash = nonce_hash.to_owned();
        retry_transaction(self.db,self.dialect,None,&RetryPolicy::default(),move |tx| {
            let mut receipt=receipt.clone(); let jwt_id=jwt_id.clone(); let challenge_id=challenge_id.clone(); let nonce_hash=nonce_hash.clone();
            Box::pin(async move {
                let account=tx.execute_raw(sql(r#"UPDATE "User" SET "updatedAt" = now() WHERE id = $1 AND status = 'ACTIVE'"#,[receipt.owner_id.clone().into()])).await?.rows_affected();
                if account!=1 { return Err(ApiError::forbidden("An active account is required")); }
                require_live(proof_expires_at)?;
                let active=tx.query_one_raw(sql(r#"SELECT COUNT(*) AS count FROM "ManagedDevice" WHERE "ownerId" = $1 AND status = 'active'"#, [receipt.owner_id.clone().into()])).await?.ok_or_else(|| ApiError::internal("Missing device count"))?.try_get::<i64>("", "count")?;
                if active >= i64::from(max_devices) { return Err(ApiError::too_many_requests("Device limit reached")); }
                let now=chrono::Utc::now().timestamp();
                let consumed=tx.execute_raw(sql(r#"UPDATE "DeviceEnrollment" SET status = 'consumed', "consumedAt" = $1 WHERE id = $2 AND "jwtId" = $3 AND "deviceId" = $4 AND "ownerId" = $5 AND status = 'pending' AND "expiresAt" > $1"#,
                    [now.into(),receipt.enrollment_id.clone().into(),jwt_id.into(),receipt.device_id.clone().into(),receipt.owner_id.clone().into()])).await?.rows_affected();
                if consumed!=1 { return Err(ApiError::conflict("Enrollment was consumed, cancelled or expired; recover a receipt with the registered key")); }
                require_live(proof_expires_at)?;
                let now=chrono::Utc::now().timestamp();
                let challenge=tx.execute_raw(sql(r#"DELETE FROM "DeviceChallenge" WHERE "enrollmentId" = $1 AND id = $2 AND "nonceHash" = $3 AND "expiresAt" > $4"#,
                    [receipt.enrollment_id.clone().into(),challenge_id.into(),nonce_hash.into(),now.into()])).await?.rows_affected();
                if challenge!=1 { return Err(ApiError::unauthorized("Enrollment challenge expired or was replaced")); }
                require_live(proof_expires_at)?;
                receipt.registered_at=chrono::Utc::now().timestamp();
                tx.execute_raw(sql(r#"INSERT INTO "ManagedDevice" (id,"ownerId",name,status,"authEpoch",identity,receipt,"registeredAt") VALUES ($1,$2,$3,'active',1,$4,$5,$6)"#,
                    [receipt.device_id.clone().into(),receipt.owner_id.clone().into(),receipt.name.clone().into(),serde_json::to_string(&receipt.identity)?.into(),serde_json::to_string(&receipt)?.into(),receipt.registered_at.into()])).await?;
                require_live(proof_expires_at)?;
                Ok(receipt)
            })
        }).await
    }

    /// Consume a proof and resolve current device status in the same transaction.
    /// This is used by sessions, receipt recovery and each DPoP API request.
    pub async fn authorize_proof(
        &self,
        id: &str,
        epoch: u64,
        proof_id: &str,
        expires_at: i64,
        heartbeat: bool,
    ) -> Result<Device, ApiError> {
        let id = id.to_owned();
        let proof_id = proof_id.to_owned();
        retry_transaction(
            self.db,
            self.dialect,
            None,
            &RetryPolicy::default(),
            move |tx| {
                let id = id.clone();
                let proof_id = proof_id.clone();
                Box::pin(async move {
                    let mut device = lock_active_device(tx, &id, epoch).await?;
                    require_live_proof(expires_at)?;
                    consume_proof(tx, &id, epoch, &proof_id, expires_at).await?;
                    if heartbeat {
                        let now = chrono::Utc::now()
                            .timestamp()
                            .max(device.status.last_seen_at.unwrap_or_default());
                        tx.execute_raw(sql(
                            r#"UPDATE "ManagedDevice" SET "lastSeenAt" = $1 WHERE id = $2"#,
                            [now.into(), id.into()],
                        ))
                        .await?;
                        device.status.last_seen_at = Some(now);
                    }
                    require_live_proof(expires_at)?;
                    Ok(device)
                })
            },
        )
        .await
    }

    pub async fn cancel_enrollment(&self, owner: &str, id: &str) -> Result<(), ApiError> {
        active_account(self.db, owner).await?;
        let changed=self.db.execute_raw(sql(r#"UPDATE "DeviceEnrollment" SET status = 'cancelled' WHERE id = $1 AND "ownerId" = $2 AND status = 'pending'"#,[id.into(),owner.into()])).await?.rows_affected();
        if changed != 1 {
            return Err(ApiError::NOT_FOUND);
        }
        Ok(())
    }

    pub async fn revoke(&self, owner: &str, id: &str) -> Result<(), ApiError> {
        active_account(self.db, owner).await?;
        let changed=self.db.execute_raw(sql(r#"UPDATE "ManagedDevice" SET status = 'revoked', "authEpoch" = "authEpoch" + 1, "revokedAt" = $3 WHERE id = $1 AND "ownerId" = $2 AND status = 'active'"#,[id.into(),owner.into(),chrono::Utc::now().timestamp().into()])).await?.rows_affected();
        if changed != 1 {
            return Err(ApiError::NOT_FOUND);
        }
        Ok(())
    }
}
