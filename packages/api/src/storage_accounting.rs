//! Retention for the object-accounting rows the AWS file tracker writes.
//!
//! A deleted object keeps a zero-size row so a redelivered S3 notification
//! cannot subtract its bytes a second time. That row is only worth keeping for
//! as long as the notification queue can still hand the event back: SQS drops a
//! message after at most 14 days, so a tombstone older than the retention
//! window guards against nothing. Deployments drive [`sweep_tombstones`]
//! through `POST /maintenance/run` (`state_cleanup` job); only the AWS file
//! tracker writes this table, so there is no ticker on long-lived hosts.

use chrono::{Duration, Utc};
use sea_orm::{ColumnTrait, Condition, DatabaseConnection, DbErr};

use crate::db::{DEFAULT_WRITE_CHUNK, DbDialect, delete_in_batches};
use crate::entity::{file_accounting_object, prelude::FileAccountingObject};

pub const TOMBSTONE_RETENTION_ENV: &str = "FILE_ACCOUNTING_TOMBSTONE_RETENTION_DAYS";

/// How long a tombstone outlives its object before a sweep may remove it.
pub const DEFAULT_TOMBSTONE_RETENTION_DAYS: i64 = 30;

/// The longest a queued S3 notification can survive. Configuration cannot go
/// below it, or the sweep would drop the dedup state a redelivery still needs.
pub const MIN_TOMBSTONE_RETENTION_DAYS: i64 = 14;

/// Past this the window stops meaning anything and the cutoff arithmetic starts
/// overflowing, so a mistyped value is clamped instead of trusted.
pub const MAX_TOMBSTONE_RETENTION_DAYS: i64 = 3_650;

/// Transactions one pass may spend; a larger backlog is finished by the next pass.
const MAX_CHUNKS_PER_SWEEP: usize = 100;

/// Read [`TOMBSTONE_RETENTION_ENV`], `None` when the sweep is switched off.
///
/// `0` disables it, which is what a deployment still importing the legacy
/// DynamoDB baseline wants: that import runs once per object and is keyed on
/// the accounting row's absence, so pruning a row lets a later event for the
/// same key re-apply a baseline the totals no longer carry.
pub fn tombstone_retention_days() -> Option<i64> {
    retention_days_from(std::env::var(TOMBSTONE_RETENTION_ENV).ok().as_deref())
}

fn retention_days_from(value: Option<&str>) -> Option<i64> {
    let days = value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .and_then(|value| value.parse::<i64>().ok())
        .filter(|days| *days >= 0)
        .unwrap_or(DEFAULT_TOMBSTONE_RETENTION_DAYS);

    (days > 0).then(|| days.clamp(MIN_TOMBSTONE_RETENTION_DAYS, MAX_TOMBSTONE_RETENTION_DAYS))
}

/// Rows a sweep may remove: a deleted object contributes nothing to any total,
/// so removing its row cannot move one. Live objects keep their row, which is
/// the baseline every later size delta is measured against. A Lance dataset's
/// row (key ending in `.lance/`) is kept even when empty: its presence records
/// that the dataset's file rows and legacy baseline were already adopted.
fn expired_tombstones(cutoff: chrono::DateTime<chrono::FixedOffset>) -> Condition {
    Condition::all()
        .add(file_accounting_object::Column::Size.eq(0_i64))
        .add(file_accounting_object::Column::UpdatedAt.lt(cutoff))
        .add(file_accounting_object::Column::ObjectKey.not_like("%.lance/"))
}

/// Delete tombstones last touched more than `retention_days` ago, in
/// primary-key chunks of [`DEFAULT_WRITE_CHUNK`] rows so a bounded engine never
/// sees an oversized transaction. Returns the number of rows removed.
///
/// Each chunk repeats the predicate in its `DELETE`, so an object recreated
/// between the key page and the delete keeps the row the tracker just wrote.
pub async fn sweep_tombstones(
    db: &DatabaseConnection,
    dialect: DbDialect,
    retention_days: i64,
) -> Result<u64, DbErr> {
    let retention_days =
        retention_days.clamp(MIN_TOMBSTONE_RETENTION_DAYS, MAX_TOMBSTONE_RETENTION_DAYS);
    let cutoff = Utc::now().fixed_offset() - Duration::days(retention_days);
    let outcome = delete_in_batches::<FileAccountingObject>(
        db,
        dialect,
        expired_tombstones(cutoff),
        DEFAULT_WRITE_CHUNK,
        Some(MAX_CHUNKS_PER_SWEEP),
    )
    .await?;

    if outcome.stopped_early {
        tracing::warn!(
            deleted = outcome.rows,
            max_chunks = MAX_CHUNKS_PER_SWEEP,
            "Storage-accounting tombstone sweep hit its budget; the rest is swept next pass"
        );
    }

    Ok(outcome.rows)
}

#[cfg(test)]
mod tests {
    use super::*;
    use sea_orm::{DatabaseBackend, EntityTrait, QueryFilter, QueryTrait};

    #[test]
    fn sweep_deletes_only_zero_size_rows_past_the_cutoff_and_keeps_dataset_rows() {
        let cutoff = chrono::DateTime::parse_from_rfc3339("2026-08-12T00:00:00Z").unwrap();
        let sql = FileAccountingObject::delete_many()
            .filter(expired_tombstones(cutoff))
            .build(DatabaseBackend::Postgres)
            .to_string();

        assert_eq!(
            sql,
            r#"DELETE FROM "public"."FileAccountingObject" WHERE "FileAccountingObject"."size" = 0 AND "FileAccountingObject"."updatedAt" < '2026-08-12 00:00:00.000000 +00:00' AND "FileAccountingObject"."objectKey" NOT LIKE '%.lance/'"#
        );
    }

    #[test]
    fn retention_defaults_to_thirty_days_and_floors_at_the_queue_lifetime() {
        assert_eq!(retention_days_from(None), Some(30));
        assert_eq!(retention_days_from(Some("  ")), Some(30));
        assert_eq!(retention_days_from(Some("not a number")), Some(30));
        assert_eq!(retention_days_from(Some("-5")), Some(30));
        assert_eq!(retention_days_from(Some(" 45 ")), Some(45));
        assert_eq!(retention_days_from(Some("3")), Some(14));
        assert_eq!(retention_days_from(Some("99999999999")), Some(3_650));
    }

    #[test]
    fn zero_switches_the_sweep_off() {
        assert_eq!(retention_days_from(Some("0")), None);
    }
}
