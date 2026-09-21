//! Anchor unanchored seals in the next epoch: a Merkle root over their hashes, linked
//! to the previous epoch and signed with the audit key. Each seal keeps its inclusion
//! proof, so an app owner verifies its chain without reading other chains' seals.

use chrono::{DateTime, FixedOffset, SubsecRound, TimeDelta, Utc};
use sea_orm::{
    ActiveValue::Set, ConnectionTrait, DatabaseConnection, DbBackend, DbErr, EntityTrait,
    QueryOrder, Statement, sea_query::OnConflict,
};

use std::collections::HashSet;

use crate::audit::crypto::{Hash, ZERO_HASH, seal_mac_matches, to_hash};
use crate::audit::keys::accepted_entry_keys;
use crate::audit::merkle;
use crate::audit::verify::{EPOCH_WATERMARK, epoch_hash_of, seal_hash_of};
use crate::db::{RetryPolicy, retry_transaction};
use crate::entity::{audit_epoch, audit_held_chain, audit_seal, audit_watermark};

use super::lease::Lease;
use super::{AuditWorkerContext, TickReport};

/// Seals one signature covers; this many waiting seals also make an epoch due early.
const MAX_SEALS_PER_EPOCH: u64 = 20_000;
/// Backlog epochs one tick may sign after the worker was down.
const MAX_EPOCHS_PER_TICK: u32 = 10;
/// Signatures of the hourly budget kept for watermark batches and archive manifests.
const RESERVED_SIGNATURES: usize = 4;
const ANCHOR_CHUNK: usize = 500;
/// Seal rows one anchoring transaction updates, below the DSQL row limit.
const ANCHORS_PER_TRANSACTION: usize = 2_000;

/// Seals waiting for an epoch, without held chains.
const WAITING_SQL: &str = r#"SELECT count(*) AS "waiting", min(s."sealedAt") AS "oldest" FROM "AuditSeal" s WHERE s."epochSeq" IS NULL AND NOT EXISTS (SELECT 1 FROM "AuditHeldChain" h WHERE h."chainId" = s."chainId")"#;
/// The oldest waiting seals, without held chains. Sealing time first keeps every
/// chain's seals in sequence across epochs.
const WINDOW_SQL: &str = r#"SELECT s.* FROM "AuditSeal" s WHERE s."epochSeq" IS NULL AND NOT EXISTS (SELECT 1 FROM "AuditHeldChain" h WHERE h."chainId" = s."chainId") ORDER BY s."sealedAt" ASC, s."chainId" ASC, s."seq" ASC LIMIT $1"#;

/// A seal that failed its hash or MAC before any epoch signed it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct HeldSeal {
    pub chain_id: String,
    pub seq: i64,
    pub seal_id: String,
    pub reason: &'static str,
}

/// A seal's position in its epoch.
struct Anchor {
    seal_id: String,
    index: i32,
    proof: Vec<u8>,
}

/// Each epoch is one request to the key service, so an epoch is signed only when the
/// oldest unanchored seal has waited `epoch_interval_seconds` or a full epoch waits.
/// Until then seals are protected by their entry-key MAC.
pub(super) async fn run(
    context: &AuditWorkerContext,
    lease: &Lease,
    now: DateTime<Utc>,
    report: &mut TickReport,
) -> flow_like_types::Result<()> {
    let Some(signer) = context.signer() else {
        return Ok(());
    };
    let (mut waiting, oldest) = waiting_seals(&context.db).await?;
    let interval = TimeDelta::seconds(i64::from(context.config.retention.epoch_interval_seconds));
    if !is_due(waiting, oldest, now, interval) {
        return Ok(());
    }
    for _ in 0..MAX_EPOCHS_PER_TICK {
        if signer
            .remaining_budget()
            .is_some_and(|remaining| remaining <= RESERVED_SIGNATURES)
        {
            tracing::warn!(
                target: "audit",
                waiting,
                "audit signing budget is nearly spent; epochs wait for the next window"
            );
            break;
        }
        let anchored = write_epoch(context, lease, now, interval, report).await?;
        waiting = waiting.saturating_sub(anchored);
        if anchored == 0 || waiting < MAX_SEALS_PER_EPOCH {
            break;
        }
    }
    Ok(())
}

fn is_due(
    waiting: u64,
    oldest: Option<DateTime<Utc>>,
    now: DateTime<Utc>,
    interval: TimeDelta,
) -> bool {
    waiting >= MAX_SEALS_PER_EPOCH
        || oldest.is_some_and(|oldest| now.signed_duration_since(oldest) >= interval)
}

async fn waiting_seals(db: &DatabaseConnection) -> Result<(u64, Option<DateTime<Utc>>), DbErr> {
    let row = db
        .query_one_raw(Statement::from_string(
            db.get_database_backend(),
            WAITING_SQL,
        ))
        .await?;
    let Some(row) = row else {
        return Ok((0, None));
    };
    let waiting = u64::try_from(row.try_get::<i64>("", "waiting")?).unwrap_or_default();
    let oldest: Option<DateTime<Utc>> = row.try_get("", "oldest")?;
    Ok((waiting, oldest))
}

/// Anchor the oldest unanchored seals in one signed epoch. Returns how many. The due
/// check repeats on the seals that can be signed, so a held-back seal never makes every
/// tick sign.
async fn write_epoch(
    context: &AuditWorkerContext,
    lease: &Lease,
    now: DateTime<Utc>,
    interval: TimeDelta,
    report: &mut TickReport,
) -> flow_like_types::Result<u64> {
    let Some(signer) = context.signer() else {
        return Ok(0);
    };
    let db = &context.db;
    let window = audit_seal::Entity::find()
        .from_raw_sql(Statement::from_sql_and_values(
            db.get_database_backend(),
            WINDOW_SQL,
            [sea_orm::Value::from(MAX_SEALS_PER_EPOCH as i64)],
        ))
        .all(db)
        .await?;
    let (seals, held) = intact_seals(window, &accepted_entry_keys());
    if !held.is_empty() {
        hold(db, &held, now).await?;
    }
    let oldest = seals.first().map(|seal| seal.sealed_at.with_timezone(&Utc));
    if !is_due(seals.len() as u64, oldest, now, interval) {
        return Ok(0);
    }
    let (seals_root, anchors) = anchors(&seals)?;
    let previous = previous_epoch(&context.db).await?;
    let (mut epoch, hash) = build_epoch(
        previous,
        &seals_root,
        seals.len(),
        now.trunc_subsecs(3).fixed_offset(),
        signer.kid(),
    );
    epoch.signature = signer
        .sign(&hash)
        .await
        .map_err(|error| {
            flow_like_types::anyhow!(
                "signing audit epoch {} with key {} failed: {error}",
                epoch.seq,
                epoch.kid
            )
        })?
        .to_vec();
    if !lease.keepalive().await {
        return Ok(0);
    }

    let backend = db.get_database_backend();
    let seq = epoch.seq;
    let groups: Vec<(Vec<Statement>, u64)> = anchors
        .chunks(ANCHORS_PER_TRANSACTION)
        .map(|group| {
            let statements = group
                .chunks(ANCHOR_CHUNK)
                .map(|chunk| anchor_statement(backend, seq, chunk))
                .collect();
            (statements, group.len() as u64)
        })
        .collect();
    let mut groups = groups.into_iter();
    let first = groups.next().unwrap_or_default();
    commit_anchors(context, Some(epoch), first.0, first.1)
        .await
        .map_err(|error| {
            flow_like_types::anyhow!(
                "committing audit epoch {seq} over {} seals failed: {error}",
                seals.len()
            )
        })?;
    report.epochs += 1;
    let mut anchored = first.1;
    // Seals of a group that fails stay unanchored; a later epoch covers them again.
    for (statements, count) in groups {
        if !lease.keepalive().await {
            break;
        }
        commit_anchors(context, None, statements, count)
            .await
            .map_err(|error| {
                flow_like_types::anyhow!(
                    "anchoring {count} more seals in audit epoch {seq} failed: {error}"
                )
            })?;
        anchored += count;
    }
    report.anchored_seals += anchored;
    Ok(anchored)
}

/// Only seals whose fields still hash to their stored hash and whose entry-key MAC
/// verifies are signed, so a seal rewritten in the database before its epoch is never
/// anchored. A chain with a failing seal is held from that seal on, which keeps its
/// sequence gap-free; other chains are anchored as usual. `seals` must be ordered so
/// each chain's seals appear in sequence; `keys` are the accepted entry keys.
pub(super) fn intact_seals(
    seals: Vec<audit_seal::Model>,
    keys: &[Hash],
) -> (Vec<audit_seal::Model>, Vec<HeldSeal>) {
    let mut held_chains: HashSet<String> = HashSet::new();
    let mut held = Vec::new();
    let intact = seals
        .into_iter()
        .filter(|seal| {
            if held_chains.contains(&seal.chain_id) {
                return false;
            }
            let hash = seal_hash_of(seal);
            let reason = if hash.as_slice() != seal.hash.as_slice() {
                Some("seal hash does not match its fields")
            } else if !seal
                .mac
                .as_deref()
                .is_some_and(|mac| keys.iter().any(|key| seal_mac_matches(key, &hash, mac)))
            {
                Some("seal MAC does not verify with any accepted entry key")
            } else {
                None
            };
            let Some(reason) = reason else {
                return true;
            };
            held_chains.insert(seal.chain_id.clone());
            held.push(HeldSeal {
                chain_id: seal.chain_id.clone(),
                seq: seal.seq,
                seal_id: seal.id.clone(),
                reason,
            });
            false
        })
        .collect();
    (intact, held)
}

/// Put chains on hold. Their seals are never signed, archived or pruned until an
/// operator resolves the incident and removes the row.
async fn hold(db: &DatabaseConnection, held: &[HeldSeal], now: DateTime<Utc>) -> Result<(), DbErr> {
    for seal in held {
        tracing::error!(
            target: "audit",
            chain_id = %seal.chain_id,
            seal_id = %seal.seal_id,
            seq = seal.seq,
            reason = seal.reason,
            "unanchored audit seal fails its hash or MAC; its chain is held and not signed"
        );
    }
    audit_held_chain::Entity::insert_many(held.iter().map(|seal| audit_held_chain::ActiveModel {
        chain_id: Set(seal.chain_id.clone()),
        seq: Set(seal.seq),
        seal_id: Set(seal.seal_id.clone()),
        held_at: Set(now.trunc_subsecs(3).fixed_offset()),
        reason: Set(seal.reason.to_owned()),
    }))
    .on_conflict(
        OnConflict::column(audit_held_chain::Column::ChainId)
            .do_nothing()
            .to_owned(),
    )
    .exec_without_returning(db)
    .await?;
    Ok(())
}

fn anchors(seals: &[audit_seal::Model]) -> flow_like_types::Result<(Hash, Vec<Anchor>)> {
    let leaves = seals
        .iter()
        .map(|seal| {
            to_hash(&seal.hash).ok_or_else(|| {
                flow_like_types::anyhow!(
                    "seal {} (seq {}) of chain {} has a {}-byte hash, expected 32",
                    seal.id,
                    seal.seq,
                    seal.chain_id,
                    seal.hash.len()
                )
            })
        })
        .collect::<flow_like_types::Result<Vec<Hash>>>()?;
    let (root, proofs) = merkle::root_with_proofs(&leaves);
    let anchors = seals
        .iter()
        .zip(proofs)
        .enumerate()
        .map(|(index, (seal, proof))| Anchor {
            seal_id: seal.id.clone(),
            index: index as i32,
            proof: merkle::encode_proof(&proof),
        })
        .collect();
    Ok((root, anchors))
}

/// The epoch after `previous` (`(seq, hash)`), unsigned, and the hash to sign.
fn build_epoch(
    previous: (i64, Vec<u8>),
    seals_root: &Hash,
    seal_count: usize,
    created_at: DateTime<FixedOffset>,
    kid: &str,
) -> (audit_epoch::Model, Hash) {
    let (previous_seq, previous_hash) = previous;
    let mut epoch = audit_epoch::Model {
        seq: previous_seq + 1,
        prev_hash: previous_hash,
        seal_count: seal_count as i32,
        seals_root: seals_root.to_vec(),
        created_at,
        hash: Vec::new(),
        kid: kid.to_owned(),
        signature: Vec::new(),
    };
    let hash = epoch_hash_of(&epoch);
    epoch.hash = hash.to_vec();
    (epoch, hash)
}

/// Newest epoch, else the timeline's prune watermark, else the start.
async fn previous_epoch(db: &DatabaseConnection) -> Result<(i64, Vec<u8>), DbErr> {
    let latest = audit_epoch::Entity::find()
        .order_by_desc(audit_epoch::Column::Seq)
        .one(db)
        .await?;
    if let Some(epoch) = latest {
        return Ok((epoch.seq, epoch.hash));
    }
    let watermark = audit_watermark::Entity::find_by_id(EPOCH_WATERMARK)
        .one(db)
        .await?;
    Ok(watermark.map_or_else(
        || (0, ZERO_HASH.to_vec()),
        |watermark| (watermark.seq, watermark.hash),
    ))
}

/// One multi-row update for a chunk of seals. Types are cast in `VALUES` so
/// PostgreSQL, CockroachDB and Aurora DSQL infer the same ones. The epoch signature
/// replaces the seal's MAC.
fn anchor_statement(backend: DbBackend, epoch_seq: i64, anchors: &[Anchor]) -> Statement {
    let rows: Vec<String> = (0..anchors.len())
        .map(|row| {
            let first = 2 + row * 3;
            format!(
                "(${first}::text, ${}::integer, ${}::bytea)",
                first + 1,
                first + 2
            )
        })
        .collect();
    let sql = format!(
        r#"UPDATE "AuditSeal" AS s SET "epochSeq" = $1::bigint, "epochIndex" = v.idx, "epochProof" = v.proof, "mac" = NULL FROM (VALUES {}) AS v(id, idx, proof) WHERE s."id" = v.id AND s."epochSeq" IS NULL"#,
        rows.join(", ")
    );
    let mut values = Vec::with_capacity(1 + anchors.len() * 3);
    values.push(sea_orm::Value::from(epoch_seq));
    for anchor in anchors {
        values.push(anchor.seal_id.clone().into());
        values.push(anchor.index.into());
        values.push(anchor.proof.clone().into());
    }
    Statement::from_sql_and_values(backend, sql, values)
}

/// One transaction: the epoch row when given, and one group of seal anchors.
async fn commit_anchors(
    context: &AuditWorkerContext,
    epoch: Option<audit_epoch::Model>,
    statements: Vec<Statement>,
    seal_count: u64,
) -> Result<(), DbErr> {
    retry_transaction::<_, (), DbErr>(
        &context.db,
        context.dialect,
        None,
        &RetryPolicy::default(),
        move |txn| {
            let epoch = epoch.clone();
            let statements = statements.clone();
            Box::pin(async move {
                if let Some(epoch) = epoch {
                    audit_epoch::Entity::insert(audit_epoch::ActiveModel::from(epoch))
                        .exec_without_returning(txn)
                        .await?;
                }
                let mut anchored = 0;
                for statement in statements {
                    anchored += txn.execute_raw(statement).await?.rows_affected();
                }
                if anchored != seal_count {
                    return Err(DbErr::Custom(format!(
                        "anchored {anchored} of {seal_count} seals"
                    )));
                }
                Ok(())
            })
        },
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audit::signer::{AuditSigner, LocalSigner, register_verifying_key};
    use crate::audit::verify::check_epoch;
    use p256::ecdsa::SigningKey;

    fn seal(index: u8) -> audit_seal::Model {
        let at = DateTime::from_timestamp_millis(1_700_000_000_000 + i64::from(index))
            .unwrap()
            .fixed_offset();
        audit_seal::Model {
            id: format!("seal-{index}"),
            chain_id: "app".into(),
            seq: i64::from(index),
            class: "evidence".into(),
            prev_hash: ZERO_HASH.to_vec(),
            record_count: 1,
            records_root: vec![index; 32],
            first_at: at,
            last_at: at,
            sealed_at: at,
            hash: blake3::hash(&[index]).as_bytes().to_vec(),
            epoch_seq: None,
            epoch_index: None,
            epoch_proof: None,
            mac: None,
            ip_pending: false,
            details_pending: false,
        }
    }

    fn sealed(chain_id: &str, seq: i64, key: &Hash) -> audit_seal::Model {
        let mut seal = seal(seq as u8);
        seal.chain_id = chain_id.into();
        seal.id = format!("{chain_id}-{seq}");
        let hash = seal_hash_of(&seal);
        seal.hash = hash.to_vec();
        seal.mac = Some(crate::audit::crypto::seal_mac(key, &hash).to_vec());
        seal
    }

    #[test]
    fn rewritten_or_unkeyed_seals_hold_back_their_chain_only() {
        let key = [5; 32];
        let mut rewritten = sealed("a", 2, &key);
        rewritten.records_root = vec![9; 32];
        rewritten.hash = seal_hash_of(&rewritten).to_vec();
        let mut unkeyed = sealed("c", 1, &key);
        unkeyed.mac = None;
        let (kept, held) = intact_seals(
            vec![
                sealed("a", 1, &key),
                sealed("b", 1, &key),
                rewritten,
                sealed("b", 2, &key),
                sealed("a", 3, &key),
                unkeyed,
            ],
            &[key],
        );
        let kept: Vec<(String, i64)> = kept
            .into_iter()
            .map(|seal| (seal.chain_id, seal.seq))
            .collect();
        assert_eq!(
            kept,
            vec![("a".into(), 1), ("b".into(), 1), ("b".into(), 2)]
        );
        let held: Vec<(&str, i64)> = held
            .iter()
            .map(|seal| (seal.chain_id.as_str(), seal.seq))
            .collect();
        assert_eq!(held, vec![("a", 2), ("c", 1)]);
        assert!(
            intact_seals(vec![sealed("a", 1, &key)], &[[6; 32]])
                .0
                .is_empty()
        );
    }

    #[test]
    fn a_previous_entry_key_keeps_seals_signable_during_rotation() {
        let (old, new) = ([5; 32], [6; 32]);
        let (kept, held) = intact_seals(vec![sealed("a", 1, &old)], &[new, old]);
        assert_eq!((kept.len(), held.len()), (1, 0));
    }

    #[test]
    fn epochs_wait_for_the_interval_or_a_full_batch() {
        let now = Utc::now();
        let interval = TimeDelta::seconds(300);
        assert!(!is_due(0, None, now, interval));
        assert!(!is_due(
            5,
            Some(now - TimeDelta::seconds(299)),
            now,
            interval
        ));
        assert!(is_due(
            5,
            Some(now - TimeDelta::seconds(300)),
            now,
            interval
        ));
        assert!(is_due(MAX_SEALS_PER_EPOCH, Some(now), now, interval));
        assert!(is_due(1, Some(now), now, TimeDelta::zero()));
    }

    #[flow_like_types::tokio::test]
    async fn epochs_sign_their_hash_and_prove_every_seal() {
        let seals: Vec<_> = (1..=7).map(seal).collect();
        let (seals_root, anchors) = anchors(&seals).unwrap();
        let signer = LocalSigner::new(
            SigningKey::from_slice(&[21; 32]).unwrap(),
            Some("epoch-step-test".into()),
        );
        register_verifying_key(signer.kid(), signer.verifying_key()).unwrap();
        let created_at = DateTime::from_timestamp_millis(1_700_000_000_123)
            .unwrap()
            .fixed_offset();
        let (mut epoch, hash) = build_epoch(
            (3, vec![1; 32]),
            &seals_root,
            seals.len(),
            created_at,
            signer.kid(),
        );
        epoch.signature = signer.sign(&hash).await.unwrap().to_vec();

        assert_eq!(epoch.seq, 4);
        assert_eq!(epoch.prev_hash, vec![1; 32]);
        assert_eq!(epoch.seal_count, 7);
        assert_eq!(epoch.hash, hash.to_vec());
        assert_eq!(check_epoch(&epoch), Ok(true));

        let root = to_hash(&epoch.seals_root).unwrap();
        for (index, (seal, anchor)) in seals.iter().zip(&anchors).enumerate() {
            assert_eq!(anchor.seal_id, seal.id);
            assert_eq!(anchor.index, index as i32);
            let proof = merkle::decode_proof(&anchor.proof).unwrap();
            let leaf = to_hash(&seal.hash).unwrap();
            assert!(merkle::verify_inclusion(
                &leaf,
                anchor.index as u64,
                epoch.seal_count as u64,
                &proof,
                &root
            ));
        }

        epoch.seal_count += 1;
        assert!(check_epoch(&epoch).is_err());
    }

    #[test]
    fn malformed_seal_hashes_are_refused() {
        let mut broken = seal(1);
        broken.hash.pop();
        assert!(anchors(&[seal(2), broken]).is_err());
    }

    #[test]
    fn anchor_statements_bind_three_values_per_seal() {
        let anchors: Vec<Anchor> = (0..2)
            .map(|index| Anchor {
                seal_id: format!("seal-{index}"),
                index,
                proof: vec![index as u8; 32],
            })
            .collect();
        let statement = anchor_statement(DbBackend::Postgres, 9, &anchors);
        assert!(statement.sql.contains(
            "VALUES ($2::text, $3::integer, $4::bytea), ($5::text, $6::integer, $7::bytea)"
        ));
        assert!(statement.sql.ends_with(r#"AND s."epochSeq" IS NULL"#));
        let values = statement.values.unwrap().0;
        assert_eq!(values.len(), 7);
        assert_eq!(values[0], sea_orm::Value::from(9i64));
        assert_eq!(values[4], sea_orm::Value::from("seal-1".to_owned()));
    }
}
