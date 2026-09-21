//! Seal pending records. A chain is due once enough records are pending or its oldest
//! is old enough; each seal checks its records' MACs, links to the chain's previous
//! seal and claims its records in one transaction. Records that fail the check are
//! quarantined and never sealed.

use chrono::{DateTime, FixedOffset, SubsecRound, TimeDelta, Utc};
use flow_like::hub::AuditRetention;
use flow_like_types::create_id;
use sea_orm::sea_query::Expr;
use sea_orm::{
    ColumnTrait, ConnectionTrait, DatabaseConnection, DbErr, EntityTrait, QueryFilter, QueryOrder,
    QuerySelect, Statement,
};

use crate::audit::crypto::{Hash, ZERO_HASH, mac_matches, seal_mac};
use crate::audit::keys::{accepted_entry_keys, entry_key};
use crate::audit::merkle;
use crate::audit::record::chain_class;
use crate::audit::verify::{INVALID_SEAL_ID, check_record, seal_hash_of, sort_records};
use crate::db::{RetryPolicy, retry_transaction};
use crate::entity::{audit_record, audit_seal, audit_watermark};

use super::lease::Lease;
use super::{AuditWorkerContext, TickReport};

const CHAINS_PER_TICK: i64 = 500;
/// Each batch writes at most one seal.
const BATCHES_PER_TICK: u32 = 2_000;
/// Ceiling on `max_records_per_seal`, so one seal's transaction stays below the DSQL
/// row limit.
const MAX_RECORDS_PER_SEAL: u32 = 2_000;

const PENDING_CHAINS_SQL: &str = r#"SELECT "chainId", count(*) AS "pending", min("timestamp") AS "oldest" FROM "AuditRecord" WHERE "sealId" IS NULL GROUP BY "chainId" ORDER BY "oldest" ASC LIMIT $1"#;

struct PendingChain {
    chain_id: String,
    pending: u64,
    oldest: DateTime<Utc>,
}

/// A batch in hash order, split into records that verify and ids that do not.
#[derive(Default)]
struct Checked {
    intact: Vec<(audit_record::Model, Hash)>,
    invalid: Vec<String>,
}

pub(super) async fn run(
    context: &AuditWorkerContext,
    lease: &Lease,
    now: DateTime<Utc>,
    report: &mut TickReport,
) -> flow_like_types::Result<()> {
    let retention = &context.config.retention;
    let chains = pending_chains(&context.db).await?;
    let Some(oldest) = chains.iter().map(|chain| chain.oldest).min() else {
        return Ok(());
    };
    let age = now.signed_duration_since(oldest).num_seconds().max(0);
    report.oldest_pending_seconds = Some(age);
    if age > i64::from(retention.pending_alert_seconds) {
        tracing::error!(
            target: "audit",
            oldest_pending_seconds = age,
            threshold_seconds = retention.pending_alert_seconds,
            "audit records pending longer than the alert threshold"
        );
    }

    let limit = retention
        .max_records_per_seal
        .clamp(1, MAX_RECORDS_PER_SEAL);
    let sealed_at = now.trunc_subsecs(3).fixed_offset();
    let mut batches = 0;
    for chain in chains {
        if !is_due(chain.pending, chain.oldest, now, retention) {
            continue;
        }
        let mut previous = chain_head(&context.db, &chain.chain_id).await?;
        let mut remaining = chain.pending;
        loop {
            if batches >= BATCHES_PER_TICK || !lease.keepalive().await {
                return Ok(());
            }
            let records = pending_records(&context.db, &chain.chain_id, limit).await?;
            let Some(batch_oldest) = records
                .first()
                .map(|record| record.timestamp.with_timezone(&Utc))
            else {
                break;
            };
            let fetched = records.len() as u64;
            if !is_due(remaining.max(fetched), batch_oldest, now, retention) {
                break;
            }
            batches += 1;

            let checked = check_batch(records, &accepted_entry_keys());
            if !checked.invalid.is_empty() {
                report.quarantined +=
                    quarantine(&context.db, &chain.chain_id, checked.invalid).await?;
            }
            if let Some(seal) = build_seal(
                &chain.chain_id,
                &previous,
                &checked.intact,
                sealed_at,
                entry_key(),
            ) {
                let record_ids = checked
                    .intact
                    .iter()
                    .map(|(record, _)| record.id.clone())
                    .collect();
                commit_seal(context, &seal, record_ids)
                    .await
                    .map_err(|error| {
                        flow_like_types::anyhow!(
                            "committing seal {} (seq {}) of chain {} failed: {error}",
                            seal.id,
                            seal.seq,
                            seal.chain_id
                        )
                    })?;
                report.seals += 1;
                report.sealed_records += checked.intact.len() as u64;
                previous = (seal.seq, seal.hash);
            }
            remaining = remaining.saturating_sub(fetched);
            if fetched < u64::from(limit) {
                break;
            }
        }
    }
    Ok(())
}

fn is_due(
    pending: u64,
    oldest: DateTime<Utc>,
    now: DateTime<Utc>,
    retention: &AuditRetention,
) -> bool {
    pending >= u64::from(retention.seal_after_records)
        || now.signed_duration_since(oldest)
            >= TimeDelta::seconds(i64::from(retention.seal_after_seconds))
}

async fn pending_chains(db: &DatabaseConnection) -> Result<Vec<PendingChain>, DbErr> {
    let rows = db
        .query_all_raw(Statement::from_sql_and_values(
            db.get_database_backend(),
            PENDING_CHAINS_SQL,
            [sea_orm::Value::from(CHAINS_PER_TICK)],
        ))
        .await?;
    rows.iter()
        .map(|row| {
            Ok(PendingChain {
                chain_id: row.try_get("", "chainId")?,
                pending: u64::try_from(row.try_get::<i64>("", "pending")?).unwrap_or_default(),
                oldest: row.try_get("", "oldest")?,
            })
        })
        .collect()
}

/// Sequence and hash the chain's next seal links to: the higher of its newest remaining
/// seal and its prune watermark, else the start of the chain. The watermark can be the
/// higher one while the seals it covers are still being deleted.
async fn chain_head(db: &DatabaseConnection, chain_id: &str) -> Result<(i64, Vec<u8>), DbErr> {
    let latest = audit_seal::Entity::find()
        .filter(audit_seal::Column::ChainId.eq(chain_id))
        .order_by_desc(audit_seal::Column::Seq)
        .one(db)
        .await?
        .map(|seal| (seal.seq, seal.hash));
    let watermark = audit_watermark::Entity::find_by_id(chain_id)
        .one(db)
        .await?
        .map(|watermark| (watermark.seq, watermark.hash));
    Ok(match (latest, watermark) {
        (Some(seal), Some(watermark)) if watermark.0 >= seal.0 => watermark,
        (Some(seal), _) => seal,
        (None, Some(watermark)) => watermark,
        (None, None) => (0, ZERO_HASH.to_vec()),
    })
}

async fn pending_records(
    db: &DatabaseConnection,
    chain_id: &str,
    limit: u32,
) -> Result<Vec<audit_record::Model>, DbErr> {
    audit_record::Entity::find()
        .filter(audit_record::Column::SealId.is_null())
        .filter(audit_record::Column::ChainId.eq(chain_id))
        .order_by_asc(audit_record::Column::Timestamp)
        .order_by_asc(audit_record::Column::Id)
        .limit(u64::from(limit))
        .all(db)
        .await
}

/// `keys` are the entry keys a MAC may have been made with (current, then previous).
fn check_batch(mut records: Vec<audit_record::Model>, keys: &[Hash]) -> Checked {
    sort_records(&mut records);
    let mut checked = Checked::default();
    for record in records {
        let hash = check_record(&record)
            .ok()
            .map(|(hash, _)| hash)
            .filter(|hash| {
                record
                    .mac
                    .as_deref()
                    .is_some_and(|mac| keys.iter().any(|key| mac_matches(key, hash, mac)))
            });
        match hash {
            Some(hash) => checked.intact.push((record, hash)),
            None => checked.invalid.push(record.id),
        }
    }
    checked
}

async fn quarantine(
    db: &DatabaseConnection,
    chain_id: &str,
    record_ids: Vec<String>,
) -> Result<u64, DbErr> {
    let quarantined = audit_record::Entity::update_many()
        .col_expr(
            audit_record::Column::SealId,
            Expr::value(INVALID_SEAL_ID.to_owned()),
        )
        .filter(audit_record::Column::Id.is_in(record_ids.clone()))
        .filter(audit_record::Column::SealId.is_null())
        .exec(db)
        .await?
        .rows_affected;
    tracing::error!(
        target: "audit",
        chain_id = %chain_id,
        record_ids = ?record_ids,
        quarantined,
        "pending audit records failed their integrity check and were quarantined"
    );
    Ok(quarantined)
}

/// The next seal over `records`, which are in hash order. `None` when nothing verified.
/// It carries a MAC with the entry key until an epoch anchors it.
fn build_seal(
    chain_id: &str,
    previous: &(i64, Vec<u8>),
    records: &[(audit_record::Model, Hash)],
    sealed_at: DateTime<FixedOffset>,
    entry_key: &Hash,
) -> Option<audit_seal::Model> {
    let (first, _) = records.first()?;
    let (last, _) = records.last()?;
    let leaves: Vec<Hash> = records.iter().map(|(_, hash)| *hash).collect();
    let mut seal = audit_seal::Model {
        id: create_id(),
        chain_id: chain_id.to_owned(),
        seq: previous.0 + 1,
        class: chain_class(chain_id).as_str().to_owned(),
        prev_hash: previous.1.clone(),
        record_count: records.len() as i32,
        records_root: merkle::root(&leaves).to_vec(),
        first_at: first.timestamp,
        last_at: last.timestamp,
        sealed_at,
        hash: Vec::new(),
        epoch_seq: None,
        epoch_index: None,
        epoch_proof: None,
        mac: None,
        ip_pending: records.iter().any(|(record, _)| record.actor_ip.is_some()),
        details_pending: records.iter().any(|(record, _)| record.details.is_some()),
    };
    let hash = seal_hash_of(&seal);
    seal.hash = hash.to_vec();
    seal.mac = Some(seal_mac(entry_key, &hash).to_vec());
    Some(seal)
}

async fn commit_seal(
    context: &AuditWorkerContext,
    seal: &audit_seal::Model,
    record_ids: Vec<String>,
) -> Result<(), DbErr> {
    let seal = seal.clone();
    retry_transaction::<_, (), DbErr>(
        &context.db,
        context.dialect,
        None,
        &RetryPolicy::default(),
        move |txn| {
            let seal = seal.clone();
            let record_ids = record_ids.clone();
            Box::pin(async move {
                let claimed = audit_record::Entity::update_many()
                    .col_expr(audit_record::Column::SealId, Expr::value(seal.id.clone()))
                    .col_expr(
                        audit_record::Column::Mac,
                        Expr::value(Option::<Vec<u8>>::None),
                    )
                    .filter(audit_record::Column::Id.is_in(record_ids))
                    .filter(audit_record::Column::SealId.is_null())
                    .exec(txn)
                    .await?
                    .rows_affected;
                if claimed != u64::try_from(seal.record_count).unwrap_or_default() {
                    return Err(DbErr::Custom(format!(
                        "seal {} of chain {} claimed {claimed} of its {} pending records",
                        seal.id, seal.chain_id, seal.record_count
                    )));
                }
                audit_seal::Entity::insert(audit_seal::ActiveModel::from(seal))
                    .exec_without_returning(txn)
                    .await?;
                Ok(())
            })
        },
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audit::crypto::{seal_mac_matches, to_hash};
    use crate::audit::record::{AuditRecordInput, WriteMode, build_record};
    use sea_orm::TryIntoModel;

    const KEY: Hash = [7; 32];

    fn records(base: DateTime<Utc>) -> Vec<audit_record::Model> {
        (0..5i64)
            .map(|index| {
                let mut input = AuditRecordInput::system(
                    "worker",
                    "board.update",
                    "Board",
                    &format!("board-{index}"),
                )
                .on_scope("app");
                if index == 1 {
                    input.actor_ip = Some("192.0.2.1".into());
                }
                if index == 2 {
                    input = input.with_details(serde_json::json!({ "count": index }));
                }
                let mut record = build_record(
                    input,
                    WriteMode::Append,
                    &KEY,
                    base - TimeDelta::seconds(index),
                );
                // A database read supplies the column omitted by API inserts.
                record.seal_id = sea_orm::Set(None);
                record.try_into_model().unwrap()
            })
            .collect()
    }

    fn retention() -> AuditRetention {
        AuditRetention {
            seal_after_records: 500,
            seal_after_seconds: 300,
            ..Default::default()
        }
    }

    #[test]
    fn chains_are_due_by_count_or_age() {
        let now = Utc::now();
        let retention = retention();
        assert!(!is_due(499, now - TimeDelta::seconds(299), now, &retention));
        assert!(is_due(500, now, now, &retention));
        assert!(is_due(1, now - TimeDelta::seconds(300), now, &retention));
        assert!(!is_due(1, now + TimeDelta::seconds(10), now, &retention));
        let immediate = AuditRetention {
            seal_after_seconds: 0,
            ..retention
        };
        assert!(is_due(1, now, now, &immediate));
    }

    #[test]
    fn records_that_fail_their_mac_are_split_off() {
        let mut batch = records(Utc::now());
        batch[3].resource_id = "tampered".into();
        batch[4].mac = None;
        batch[0].actor_ip = None;
        batch[0].ip_salt = Some(vec![1; 32]);
        let invalid = [
            batch[0].id.clone(),
            batch[3].id.clone(),
            batch[4].id.clone(),
        ];

        let checked = check_batch(batch, &[KEY]);
        let mut quarantined = checked.invalid.clone();
        quarantined.sort();
        let mut expected = invalid.to_vec();
        expected.sort();
        assert_eq!(quarantined, expected);
        assert_eq!(checked.intact.len(), 2);

        let wrong_key = check_batch(records(Utc::now()), &[[8; 32]]);
        assert!(wrong_key.intact.is_empty());
        assert_eq!(wrong_key.invalid.len(), 5);
    }

    #[test]
    fn seals_hash_their_fields_and_root_their_records_in_order() {
        let checked = check_batch(records(Utc::now()), &[KEY]);
        assert!(checked.invalid.is_empty());
        let sealed_at = Utc::now().trunc_subsecs(3).fixed_offset();
        let previous = (4, vec![9; 32]);
        let seal = build_seal("app", &previous, &checked.intact, sealed_at, &KEY).unwrap();

        let mut ordered: Vec<audit_record::Model> = checked
            .intact
            .iter()
            .map(|(record, _)| record.clone())
            .collect();
        sort_records(&mut ordered);
        let leaves: Vec<Hash> = ordered
            .iter()
            .map(|record| check_record(record).unwrap().0)
            .collect();

        assert_eq!(seal.hash, seal_hash_of(&seal).to_vec());
        assert_eq!(seal.records_root, merkle::root(&leaves).to_vec());
        assert_eq!(seal.seq, 5);
        assert_eq!(seal.prev_hash, vec![9; 32]);
        assert_eq!(seal.class, "evidence");
        assert_eq!(seal.record_count, 5);
        assert_eq!(seal.first_at, ordered.first().unwrap().timestamp);
        assert_eq!(seal.last_at, ordered.last().unwrap().timestamp);
        assert!(seal.first_at < seal.last_at);
        assert_eq!(seal.sealed_at, sealed_at);
        assert!(seal.ip_pending && seal.details_pending);
        assert!(seal.epoch_seq.is_none() && seal.epoch_proof.is_none());
        let hash = to_hash(&seal.hash).unwrap();
        assert!(seal_mac_matches(&KEY, &hash, seal.mac.as_deref().unwrap()));
        assert!(!seal_mac_matches(
            &[8; 32],
            &hash,
            seal.mac.as_deref().unwrap()
        ));

        let activity = build_seal(
            "app#activity",
            &(0, ZERO_HASH.to_vec()),
            &checked.intact[..1],
            sealed_at,
            &KEY,
        )
        .unwrap();
        assert_eq!(activity.class, "activity");
        assert_eq!(activity.seq, 1);
        assert!(!activity.ip_pending && !activity.details_pending);
        assert!(build_seal("app", &previous, &[], sealed_at, &KEY).is_none());
    }
}
