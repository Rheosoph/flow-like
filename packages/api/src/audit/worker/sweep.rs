//! Expire personal values once their window has passed: raw IPs after `ip_days`,
//! details after `details_days`. Seal flags find the work without scanning records.
//! The commitments stay, so verification reports these values as redacted.

use chrono::{DateTime, TimeDelta, Utc};
use sea_orm::{
    ColumnTrait, ConnectionTrait, DbErr, EntityTrait, QueryFilter, QueryOrder, QuerySelect,
    Statement,
};

use crate::db::{RetryPolicy, retry_transaction};
use crate::entity::audit_seal;

use super::lease::Lease;
use super::{AuditWorkerContext, TickReport};

/// Seals cleared per page; the sweep keeps paging while seals are due, so it stays
/// ahead of the sealer, which writes at most 2,000 seals per tick.
const SEALS_PER_PAGE: u64 = 200;
const MAX_SEALS_PER_TICK: u64 = 5_000;
/// Quarantined records cleared per statement, below the DSQL row limit.
const QUARANTINE_BATCH: u64 = 1_000;
const BACKFILL_SEALS_PER_TICK: u64 = 1_000;

/// Older workers set the details flag only when an expiry policy was configured.
/// Repair those flags in bounded pages once per worker context.
#[derive(Default)]
pub(super) struct DetailsBackfill {
    complete: bool,
}

// Repaired rows leave this candidate set even after expiry clears the flag again.
// A cold worker can therefore continue the backfill without a process-local cursor.
const REPAIR_DETAILS_FLAGS: &str = r#"UPDATE "AuditSeal" SET "detailsPending" = true WHERE "id" IN (SELECT s."id" FROM "AuditSeal" s WHERE s."detailsPending" = false AND EXISTS (SELECT 1 FROM "AuditRecord" r WHERE r."sealId" = s."id" AND r."details" IS NOT NULL) LIMIT $1)"#;

struct PersonalValue {
    name: &'static str,
    pending: audit_seal::Column,
    clear_records: &'static str,
    clear_pending: &'static str,
    /// Quarantined records never belong to a seal, so their own timestamp decides.
    clear_quarantined: &'static str,
}

const IP: PersonalValue = PersonalValue {
    name: "IP addresses",
    pending: audit_seal::Column::IpPending,
    clear_records: r#"UPDATE "AuditRecord" SET "actorIp" = NULL, "ipSalt" = NULL WHERE "sealId" = $1 AND "actorIp" IS NOT NULL"#,
    clear_pending: r#"UPDATE "AuditSeal" SET "ipPending" = false WHERE "id" = $1"#,
    clear_quarantined: r#"UPDATE "AuditRecord" SET "actorIp" = NULL, "ipSalt" = NULL WHERE "id" IN (SELECT "id" FROM "AuditRecord" WHERE "sealId" = 'invalid' AND "actorIp" IS NOT NULL AND "timestamp" < $1 LIMIT $2)"#,
};

const DETAILS: PersonalValue = PersonalValue {
    name: "details",
    pending: audit_seal::Column::DetailsPending,
    clear_records: r#"UPDATE "AuditRecord" SET "details" = NULL, "detailsSalt" = NULL WHERE "sealId" = $1 AND "details" IS NOT NULL"#,
    clear_pending: r#"UPDATE "AuditSeal" SET "detailsPending" = false WHERE "id" = $1"#,
    clear_quarantined: r#"UPDATE "AuditRecord" SET "details" = NULL, "detailsSalt" = NULL WHERE "id" IN (SELECT "id" FROM "AuditRecord" WHERE "sealId" = 'invalid' AND "details" IS NOT NULL AND "timestamp" < $1 LIMIT $2)"#,
};

pub(super) async fn run(
    context: &AuditWorkerContext,
    lease: &Lease,
    now: DateTime<Utc>,
    report: &mut TickReport,
) -> flow_like_types::Result<()> {
    let retention = &context.config.retention;
    if !expire(context, lease, &IP, cutoff(now, retention.ip_days), report).await? {
        return Ok(());
    }
    if let Some(days) = retention.details_days {
        if !repair_details_flags(context, lease).await? {
            return Ok(());
        }
        expire(context, lease, &DETAILS, cutoff(now, days), report).await?;
    }
    Ok(())
}

async fn repair_details_flags(
    context: &AuditWorkerContext,
    lease: &Lease,
) -> flow_like_types::Result<bool> {
    let mut progress = context.details_backfill.lock().await;
    if progress.complete {
        return Ok(true);
    }
    if !lease.keepalive().await {
        return Ok(false);
    }
    let repaired = context
        .db
        .execute_raw(Statement::from_sql_and_values(
            context.db.get_database_backend(),
            REPAIR_DETAILS_FLAGS,
            [sea_orm::Value::from(BACKFILL_SEALS_PER_TICK as i64)],
        ))
        .await?
        .rows_affected();
    progress.complete = repaired < BACKFILL_SEALS_PER_TICK;
    Ok(true)
}

/// Seals whose last record is older than this have expired values. `None` when the
/// window reaches before the earliest representable time.
fn cutoff(now: DateTime<Utc>, days: u32) -> Option<DateTime<Utc>> {
    TimeDelta::try_days(i64::from(days)).and_then(|window| now.checked_sub_signed(window))
}

/// Returns false when the lease was lost.
async fn expire(
    context: &AuditWorkerContext,
    lease: &Lease,
    value: &PersonalValue,
    cutoff: Option<DateTime<Utc>>,
    report: &mut TickReport,
) -> flow_like_types::Result<bool> {
    let Some(cutoff) = cutoff else {
        return Ok(true);
    };
    let mut swept = 0;
    while swept < MAX_SEALS_PER_TICK {
        let seal_ids = audit_seal::Entity::find()
            .select_only()
            .column(audit_seal::Column::Id)
            .filter(value.pending.eq(true))
            .filter(audit_seal::Column::LastAt.lt(cutoff.fixed_offset()))
            .order_by_asc(audit_seal::Column::LastAt)
            .limit(SEALS_PER_PAGE)
            .into_tuple::<String>()
            .all(&context.db)
            .await?;
        let page = seal_ids.len() as u64;
        for seal_id in seal_ids {
            if !lease.keepalive().await {
                return Ok(false);
            }
            report.expired_values += clear(context, value, &seal_id).await.map_err(|error| {
                flow_like_types::anyhow!(
                    "expiring {} of audit seal {seal_id} failed: {error}",
                    value.name
                )
            })?;
        }
        swept += page;
        if page < SEALS_PER_PAGE {
            break;
        }
    }
    expire_quarantined(context, lease, value, cutoff, report).await
}

/// Quarantined records carry no seal, so no flag points at them; their own timestamp
/// decides, in bounded batches.
async fn expire_quarantined(
    context: &AuditWorkerContext,
    lease: &Lease,
    value: &PersonalValue,
    cutoff: DateTime<Utc>,
    report: &mut TickReport,
) -> flow_like_types::Result<bool> {
    let backend = context.db.get_database_backend();
    loop {
        if !lease.keepalive().await {
            return Ok(false);
        }
        let statement = Statement::from_sql_and_values(
            backend,
            value.clear_quarantined,
            [
                sea_orm::Value::from(cutoff.fixed_offset()),
                sea_orm::Value::from(QUARANTINE_BATCH as i64),
            ],
        );
        let cleared = context.db.execute_raw(statement).await.map_err(|error| {
            flow_like_types::anyhow!(
                "expiring {} of quarantined audit records failed: {error}",
                value.name
            )
        })?;
        report.expired_values += cleared.rows_affected();
        if cleared.rows_affected() < QUARANTINE_BATCH {
            return Ok(true);
        }
    }
}

async fn clear(
    context: &AuditWorkerContext,
    value: &PersonalValue,
    seal_id: &str,
) -> Result<u64, DbErr> {
    let backend = context.db.get_database_backend();
    let clear_records = Statement::from_sql_and_values(
        backend,
        value.clear_records,
        [sea_orm::Value::from(seal_id.to_owned())],
    );
    let clear_pending = Statement::from_sql_and_values(
        backend,
        value.clear_pending,
        [sea_orm::Value::from(seal_id.to_owned())],
    );
    retry_transaction::<_, u64, DbErr>(
        &context.db,
        context.dialect,
        None,
        &RetryPolicy::idempotent(),
        move |txn| {
            let clear_records = clear_records.clone();
            let clear_pending = clear_pending.clone();
            Box::pin(async move {
                let cleared = txn.execute_raw(clear_records).await?.rows_affected();
                txn.execute_raw(clear_pending).await?;
                Ok(cleared)
            })
        },
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cutoffs_follow_the_window_and_never_overflow() {
        let now = DateTime::from_timestamp_millis(1_758_283_200_000).unwrap();
        assert_eq!(cutoff(now, 7), Some(now - TimeDelta::days(7)));
        assert_eq!(cutoff(now, 0), Some(now));
        assert_eq!(cutoff(now, u32::MAX), None);
    }
}
