use crate::{
    db::{DbDialect, RetryPolicy, retry_transaction},
    error::ApiError,
};
use flow_like_device_protocol::{
    DeviceIdentity, DeviceReceipt, DeviceRegistrationStatus, DeviceStatus, OnboardingManifest,
};
use sea_orm::{
    ConnectionTrait, DatabaseBackend, DatabaseConnection, DatabaseTransaction, QueryResult,
    Statement, Value,
};

fn sql(query: &str, values: impl IntoIterator<Item = Value>) -> Statement {
    Statement::from_sql_and_values(DatabaseBackend::Postgres, query, values)
}

fn require_live(expires_at: i64) -> Result<(), ApiError> {
    if expires_at <= chrono::Utc::now().timestamp() {
        return Err(ApiError::unauthorized(
            "Device authorization expired while waiting for the database",
        ));
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

pub(crate) fn device(row: QueryResult) -> Result<Device, ApiError> {
    let status: String = row.try_get("", "status")?;
    let epoch: i64 = row.try_get("", "authEpoch")?;
    Ok(Device {
        status: DeviceStatus {
            device_id: row.try_get("", "id")?,
            owner_id: row.try_get("", "ownerId")?,
            name: row.try_get("", "name")?,
            status: match status.as_str() {
                "active" => DeviceRegistrationStatus::Active,
                "revoked" => DeviceRegistrationStatus::Revoked,
                _ => return Err(ApiError::internal("Invalid device registry status")),
            },
            identity: serde_json::from_str::<DeviceIdentity>(
                &row.try_get::<String>("", "identity")?,
            )?,
            registered_at: row.try_get("", "registeredAt")?,
            last_seen_at: row.try_get("", "lastSeenAt")?,
            auth_epoch: u64::try_from(epoch)
                .map_err(|_| ApiError::internal("Invalid device key epoch"))?,
        },
        receipt: serde_json::from_str(&row.try_get::<String>("", "receipt")?)?,
    })
}

pub(crate) async fn active_account<C: ConnectionTrait>(db: &C, owner: &str) -> Result<(), ApiError> {
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

pub(crate) async fn current_device<C: ConnectionTrait>(db: &C, id: &str) -> Result<Device, ApiError> {
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
        return Err(ApiError::unauthorized("Device proof was already used"));
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

    pub async fn list(&self, owner: &str) -> Result<Vec<DeviceStatus>, ApiError> {
        active_account(self.db, owner).await?;
        // Keep every active device visible even when revoked history exceeds
        // the bounded inventory response. Active enrollment policy caps at 1000.
        self.db.query_all_raw(sql(r#"SELECT * FROM "ManagedDevice" WHERE "ownerId" = $1 ORDER BY CASE WHEN status = 'active' THEN 0 ELSE 1 END, "registeredAt" DESC LIMIT 1000"#, [owner.into()])).await?
            .into_iter().map(|row| device(row).map(|device| device.status)).collect()
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
                let pending = tx.query_one_raw(sql(r#"SELECT COUNT(*) AS count FROM "DeviceEnrollment" WHERE "ownerId" = $1 AND status = 'pending' AND "expiresAt" > $2"#,
                    [template.owner_id.clone().into(), now.into()])).await?.ok_or_else(|| ApiError::internal("Missing enrollment count"))?.try_get::<i64>("", "count")?;
                let devices = tx.query_one_raw(sql(r#"SELECT COUNT(*) AS count FROM "ManagedDevice" WHERE "ownerId" = $1 AND status = 'active'"#,
                    [template.owner_id.clone().into()])).await?.ok_or_else(|| ApiError::internal("Missing device count"))?.try_get::<i64>("", "count")?;
                if pending >= max_pending as i64 || devices + pending >= max_devices as i64 {
                    return Err(ApiError::too_many_requests("Device enrollment limit reached"));
                }
                tx.execute_raw(sql(r#"INSERT INTO "DeviceEnrollment" (id, "deviceId", "ownerId", "jwtId", manifest, status, "expiresAt", "createdAt") VALUES ($1,$2,$3,$4,$5,'pending',$6,$7)"#,
                    [template.enrollment_id.into(),template.device_id.into(),template.owner_id.into(),jwt_id.into(),json.into(),template.expires_at.into(),template.issued_at.into()])).await?;
                require_live(template.expires_at)?;
                Ok(())
            })
        }).await
    }

    pub async fn challenge(
        &self,
        enrollment_id: &str,
        jwt_id: &str,
        challenge_id: &str,
        nonce_hash: &str,
        expires_at: i64,
        now: i64,
    ) -> Result<(), ApiError> {
        let enrollment_id = enrollment_id.to_owned();
        let jwt_id = jwt_id.to_owned();
        let challenge_id = challenge_id.to_owned();
        let nonce_hash = nonce_hash.to_owned();
        retry_transaction(self.db,self.dialect,None,&RetryPolicy::default(),move |tx| {
            let enrollment_id=enrollment_id.clone(); let jwt_id=jwt_id.clone(); let challenge_id=challenge_id.clone(); let nonce_hash=nonce_hash.clone();
            Box::pin(async move {
                let changed=tx.execute_raw(sql(r#"UPDATE "DeviceEnrollment" SET status = status WHERE id = $1 AND "jwtId" = $2 AND status = 'pending' AND "expiresAt" > $3"#,
                    [enrollment_id.clone().into(),jwt_id.into(),now.into()])).await?.rows_affected();
                if changed != 1 { return Err(ApiError::unauthorized("Enrollment is no longer pending")); }
                let record=tx.query_one_raw(sql(r#"SELECT * FROM "DeviceEnrollment" WHERE id = $1"#,[enrollment_id.clone().into()])).await?.ok_or(ApiError::NOT_FOUND).and_then(enrollment)?;
                active_account(tx,&record.manifest.owner_id).await?;
                require_live(expires_at.min(record.manifest.expires_at))?;
                tx.execute_raw(sql(r#"INSERT INTO "DeviceChallenge" ("enrollmentId",id,"nonceHash","expiresAt") VALUES ($1,$2,$3,$4) ON CONFLICT ("enrollmentId") DO UPDATE SET id = EXCLUDED.id, "nonceHash" = EXCLUDED."nonceHash", "expiresAt" = EXCLUDED."expiresAt""#,
                    [enrollment_id.into(),challenge_id.into(),nonce_hash.into(),expires_at.into()])).await?;
                require_live(expires_at.min(record.manifest.expires_at))?;
                Ok(())
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
                    require_live(expires_at)?;
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
                    require_live(expires_at)?;
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
        let changed=self.db.execute_raw(sql(r#"UPDATE "ManagedDevice" SET status = 'revoked', "authEpoch" = "authEpoch" + 1 WHERE id = $1 AND "ownerId" = $2 AND status = 'active'"#,[id.into(),owner.into()])).await?.rows_affected();
        if changed != 1 {
            return Err(ApiError::NOT_FOUND);
        }
        Ok(())
    }
}
