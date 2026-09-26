//! Export legacy entries in bounded batches, deleting only the ids just uploaded.
//! Old API replicas may still write during a rolling deployment. A later tick exports
//! those rows, and a crash before deletion safely exports the remaining rows again.

use chrono::{DateTime, Utc};
use flow_like_storage::Path;
use flow_like_storage::files::store::FlowLikeStore;
use flow_like_types::create_id;
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QueryOrder, QuerySelect};

use crate::entity::{audit_archive, audit_entry};

use super::archive::{archive_row, at_millis};
use super::bucket::{ArchiveWriter, legacy_key};
use super::lease::Lease;
use super::{AuditWorkerContext, TickReport};

/// `period` of the legacy export's archive rows.
pub(super) const LEGACY_PERIOD: &str = "legacy";
const EXPORT_PAGE: u64 = 5_000;
const DELETE_BATCH: usize = 1_000;

pub(super) async fn run(
    context: &AuditWorkerContext,
    lease: &Lease,
    now: DateTime<Utc>,
    report: &mut TickReport,
) -> flow_like_types::Result<()> {
    let Some(bucket) = context.bucket.as_deref() else {
        return Ok(());
    };
    let entries = audit_entry::Entity::find()
        .order_by_asc(audit_entry::Column::Id)
        .limit(EXPORT_PAGE)
        .all(&context.db)
        .await?;
    if entries.is_empty() {
        return Ok(());
    }
    let last = audit_archive::Entity::find()
        .filter(audit_archive::Column::Period.eq(LEGACY_PERIOD))
        .order_by_desc(audit_archive::Column::Part)
        .one(&context.db)
        .await?;
    let part = match last {
        None => 0,
        Some(last) => last
            .part
            .checked_add(1)
            .ok_or_else(|| flow_like_types::anyhow!("legacy audit archive part number overflow"))?,
    };
    let key = if part == 0 {
        legacy_key()
    } else {
        Path::from(format!("legacy/{}.jsonl.zst", create_id()))
    };
    let written = upload_entries(bucket, lease, key.clone(), &entries).await?;
    let mut row = archive_row(LEGACY_PERIOD, part, &key, written, at_millis(now));
    row.record_count = sea_orm::ActiveValue::Set(entries.len() as i64);
    audit_archive::Entity::insert(row)
        .exec_without_returning(&context.db)
        .await?;
    let ids: Vec<String> = entries.into_iter().map(|entry| entry.id).collect();
    for batch in ids.chunks(DELETE_BATCH) {
        require_lease(lease).await?;
        report.legacy_exported += delete_uploaded(&context.db, batch).await?;
    }
    tracing::info!(target: "audit", rows = ids.len(), %key, "legacy audit entries exported");
    Ok(())
}

async fn upload_entries(
    bucket: &FlowLikeStore,
    lease: &Lease,
    key: Path,
    entries: &[audit_entry::Model],
) -> flow_like_types::Result<super::bucket::WrittenObject> {
    require_lease(lease).await?;
    let mut writer = ArchiveWriter::create(bucket, key).await?;
    for entry in entries {
        let result = async {
            require_lease(lease).await?;
            writer.write_line(&entry_line(entry)?).await
        }
        .await;
        if let Err(error) = result {
            writer.abort().await;
            return Err(error);
        }
    }
    writer.finish().await
}

fn entry_line(entry: &audit_entry::Model) -> flow_like_types::Result<Vec<u8>> {
    let mut line = serde_json::to_vec(entry).map_err(|error| {
        flow_like_types::anyhow!(
            "serializing legacy audit entry {} failed: {error}",
            entry.id
        )
    })?;
    line.push(b'\n');
    Ok(line)
}

async fn require_lease(lease: &Lease) -> flow_like_types::Result<()> {
    if !lease.keepalive().await {
        return Err(flow_like_types::anyhow!(
            "audit worker lost its lease while exporting legacy audit entries"
        ));
    }
    Ok(())
}

async fn delete_uploaded<C: sea_orm::ConnectionTrait>(
    db: &C,
    ids: &[String],
) -> Result<u64, sea_orm::DbErr> {
    Ok(audit_entry::Entity::delete_many()
        .filter(audit_entry::Column::Id.is_in(ids.iter().cloned()))
        .exec(db)
        .await?
        .rows_affected)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entity::sea_orm_active_enums::AuditActorType;

    #[test]
    fn entries_export_as_one_json_object_per_line() {
        let entry = audit_entry::Model {
            id: "entry-1".into(),
            sequence: 7,
            timestamp: DateTime::parse_from_rfc3339("2026-01-02T03:04:05.678Z").unwrap(),
            actor_id: "user".into(),
            actor_ip: Some("192.0.2.1".into()),
            action: "board.update".into(),
            resource_type: "Board".into(),
            resource_id: "board".into(),
            chain_id: Some("app".into()),
            summary: "Updated\nboard".into(),
            details: Some(serde_json::json!({ "count": 2 })),
            entry_hash: "hash".into(),
            prev_hash: "prev".into(),
            signature: None,
            kid: None,
            actor_type: AuditActorType::User,
        };
        let line = entry_line(&entry).unwrap();
        assert_eq!(line.last(), Some(&b'\n'));
        assert_eq!(line.iter().filter(|byte| **byte == b'\n').count(), 1);
        let parsed: serde_json::Value = serde_json::from_slice(&line).unwrap();
        assert_eq!(parsed["id"], "entry-1");
        assert_eq!(parsed["summary"], "Updated\nboard");
        assert_eq!(parsed["actor_ip"], "192.0.2.1");
    }
}
