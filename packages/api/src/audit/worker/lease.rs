//! One audit worker at a time. The lease is a committed row claim, so it works the same
//! on PostgreSQL, CockroachDB and Aurora DSQL, and a crashed worker's lease simply
//! expires. Long steps renew it between batches.

use std::time::{Duration, Instant};

use sea_orm::{ConnectionTrait, DatabaseConnection, DbErr, Statement};

use crate::db::lease::new_owner_id;

const ENSURE_LOCK_ROW_SQL: &str =
    r#"INSERT INTO "AuditWorkerLease" ("id") VALUES ($1) ON CONFLICT ("id") DO NOTHING"#;

/// Lease length. A step renews it once half has passed.
pub const WORKER_LEASE_TTL: Duration = Duration::from_secs(300);

const CLAIM_SQL: &str = r#"UPDATE "AuditWorkerLease" SET "owner" = $2, "expiresAt" = now() + interval '300 seconds', "updatedAt" = now() WHERE "id" = $1 AND ("owner" IS NULL OR "expiresAt" IS NULL OR "expiresAt" < now() OR "owner" = $2)"#;
const RENEW_SQL: &str = r#"UPDATE "AuditWorkerLease" SET "expiresAt" = now() + interval '300 seconds', "updatedAt" = now() WHERE "id" = $1 AND "owner" = $2"#;
const RELEASE_SQL: &str = r#"UPDATE "AuditWorkerLease" SET "owner" = NULL, "expiresAt" = NULL, "updatedAt" = now() WHERE "id" = $1 AND "owner" = $2"#;

fn lock_id() -> i64 {
    flow_like_db::coordination::transaction_lock_id("audit-worker", &[])
}

fn statement(db: &DatabaseConnection, sql: &str, owner: Option<&str>) -> Statement {
    let mut values: Vec<sea_orm::Value> = vec![lock_id().into()];
    if let Some(owner) = owner {
        values.push(owner.to_owned().into());
    }
    Statement::from_sql_and_values(db.get_database_backend(), sql, values)
}

pub struct Lease {
    db: DatabaseConnection,
    owner: String,
    renewed_at: std::sync::Mutex<Instant>,
}

impl Lease {
    /// `None` when another worker holds the lease or the claim lost a commit race.
    pub async fn acquire(db: &DatabaseConnection) -> Result<Option<Self>, DbErr> {
        if let Err(error) = db
            .execute_raw(statement(db, ENSURE_LOCK_ROW_SQL, None))
            .await
        {
            // Two first-time inserts race on optimistic engines; the loser retries next run.
            if crate::db::classify_db_err(&error).is_some() {
                return Ok(None);
            }
            return Err(error);
        }
        let owner = new_owner_id();
        match db.execute_raw(statement(db, CLAIM_SQL, Some(&owner))).await {
            Ok(result) if result.rows_affected() == 1 => Ok(Some(Self {
                db: db.clone(),
                owner,
                renewed_at: std::sync::Mutex::new(Instant::now()),
            })),
            Ok(_) => Ok(None),
            Err(error) if crate::db::classify_db_err(&error).is_some() => Ok(None),
            Err(error) => Err(error),
        }
    }

    /// Renew when half the lease has passed. Returns false when the lease was lost, in
    /// which case the caller must stop writing.
    pub async fn keepalive(&self) -> bool {
        let due = {
            let renewed_at = self
                .renewed_at
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            renewed_at.elapsed() >= WORKER_LEASE_TTL / 2
        };
        if !due {
            return true;
        }
        self.renew().await
    }

    /// Renew now. Returns false when another worker holds the lease.
    pub async fn renew(&self) -> bool {
        match self
            .db
            .execute_raw(statement(&self.db, RENEW_SQL, Some(&self.owner)))
            .await
        {
            Ok(result) if result.rows_affected() == 1 => {
                *self
                    .renewed_at
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner) = Instant::now();
                true
            }
            Ok(_) => false,
            Err(error) => {
                tracing::warn!(%error, "audit worker lease renewal failed");
                false
            }
        }
    }

    pub async fn release(self) {
        if let Err(error) = self
            .db
            .execute_raw(statement(&self.db, RELEASE_SQL, Some(&self.owner)))
            .await
        {
            tracing::warn!(%error, "audit worker lease release failed; it expires on its own");
        }
    }
}
