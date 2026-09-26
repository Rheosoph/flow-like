//! The daily head: once a UTC day has its first epoch, the newest epoch is written to
//! `heads/YYYY/MM/DD.json` in the audit bucket, which the API cannot write. A later
//! rewrite of the timeline no longer matches the heads.

use chrono::{DateTime, Utc};
use flow_like_storage::files::store::FlowLikeStore;
use flow_like_storage::object_store::{Error as StoreError, ObjectStoreExt};
use sea_orm::{EntityTrait, QueryOrder};
use serde::Serialize;

use crate::audit::wire::EpochLine;
use crate::entity::audit_epoch;

use super::bucket::{head_key, put_bytes};
use super::{AuditWorkerContext, TickReport};

#[derive(Serialize)]
struct Head {
    epoch: EpochLine,
    written_at_ms: i64,
}

pub(super) async fn run(
    context: &AuditWorkerContext,
    now: DateTime<Utc>,
    report: &mut TickReport,
) -> flow_like_types::Result<()> {
    let Some(bucket) = context.bucket.as_deref() else {
        return Ok(());
    };
    let Some(epoch) = audit_epoch::Entity::find()
        .order_by_desc(audit_epoch::Column::Seq)
        .one(&context.db)
        .await?
    else {
        return Ok(());
    };
    if epoch.created_at.with_timezone(&Utc).date_naive() != now.date_naive() {
        return Ok(());
    }
    if write_head(bucket, &epoch, now).await? {
        report.head_written = true;
    }
    Ok(())
}

/// Write today's head unless it exists. Returns whether it was written.
async fn write_head(
    bucket: &FlowLikeStore,
    epoch: &audit_epoch::Model,
    now: DateTime<Utc>,
) -> flow_like_types::Result<bool> {
    let key = head_key(now.date_naive());
    match bucket.as_generic().head(&key).await {
        Ok(_) => return Ok(false),
        Err(StoreError::NotFound { .. }) => {}
        Err(error) => {
            return Err(flow_like_types::anyhow!(
                "checking audit head {key} failed: {error}"
            ));
        }
    }
    let body = serde_json::to_vec_pretty(&Head {
        epoch: EpochLine::from(epoch),
        written_at_ms: now.timestamp_millis(),
    })?;
    put_bytes(bucket, &key, body).await?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use flow_like_storage::object_store::memory::InMemory;
    use std::sync::Arc;

    fn epoch(created_at_ms: i64) -> audit_epoch::Model {
        audit_epoch::Model {
            seq: 12,
            prev_hash: vec![1; 32],
            seal_count: 3,
            seals_root: vec![2; 32],
            created_at: DateTime::from_timestamp_millis(created_at_ms)
                .unwrap()
                .fixed_offset(),
            hash: vec![3; 32],
            kid: "head-test".into(),
            signature: vec![4; 64],
        }
    }

    #[flow_like_types::tokio::test]
    async fn the_head_is_written_once_per_day() {
        let bucket = FlowLikeStore::Memory(Arc::new(InMemory::new()));
        let now = DateTime::from_timestamp_millis(1_758_283_200_000).unwrap();
        let newest = epoch(now.timestamp_millis() - 1_000);
        assert!(write_head(&bucket, &newest, now).await.unwrap());
        assert!(
            !write_head(&bucket, &epoch(now.timestamp_millis()), now)
                .await
                .unwrap()
        );

        let key = head_key(now.date_naive());
        let stored = bucket
            .as_generic()
            .get(&key)
            .await
            .unwrap()
            .bytes()
            .await
            .unwrap();
        let head: serde_json::Value = serde_json::from_slice(&stored).unwrap();
        assert_eq!(head["epoch"]["seq"], 12);
        assert_eq!(head["epoch"]["hash"], hex::encode([3u8; 32]));
        assert_eq!(
            head["epoch"]["created_at_ms"],
            now.timestamp_millis() - 1_000
        );
        assert_eq!(head["written_at_ms"], now.timestamp_millis());
        assert!(stored.contains(&b'\n'));
    }
}
