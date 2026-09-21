//! Watermarks of one prune run share one signature: the worker signs the Merkle root
//! over their hashes and each row keeps its inclusion proof. Pruning a thousand chains
//! costs one request to the key service, not a thousand.

use chrono::{DateTime, FixedOffset, Utc};
use sea_orm::sea_query::{Alias, Expr, ExprTrait, OnConflict};
use sea_orm::{ActiveValue::Set, ConnectionTrait, DbErr, EntityTrait};

use crate::audit::crypto::{watermark_batch_hash, watermark_hash};
use crate::audit::merkle;
use crate::audit::signer::SharedSigner;
use crate::entity::audit_watermark;

/// Largest batch one signature covers; more targets wait for the next run.
pub const MAX_BATCH: usize = 10_000;
const UPSERT_CHUNK: usize = 500;

/// The newest seal (or epoch, for [`crate::audit::verify::EPOCH_WATERMARK`]) a chain
/// is about to lose.
#[derive(Clone, Debug)]
pub struct WatermarkTarget {
    pub chain_id: String,
    pub seq: i64,
    pub hash: Vec<u8>,
}

/// Sign `targets` with one signature. Call before any transaction: this awaits the key
/// service. At most [`MAX_BATCH`] targets.
pub async fn sign_batch(
    signer: &SharedSigner,
    targets: Vec<WatermarkTarget>,
    now: DateTime<Utc>,
) -> flow_like_types::Result<Vec<audit_watermark::ActiveModel>> {
    if targets.is_empty() {
        return Ok(Vec::new());
    }
    if targets.len() > MAX_BATCH {
        return Err(flow_like_types::anyhow!(
            "watermark batch of {} exceeds the limit of {MAX_BATCH}",
            targets.len()
        ));
    }
    let pruned_at: DateTime<FixedOffset> = DateTime::from_timestamp_millis(now.timestamp_millis())
        .expect("current time fits in milliseconds")
        .fixed_offset();
    let pruned_at_ms = pruned_at.timestamp_millis();
    let leaves: Vec<_> = targets
        .iter()
        .map(|target| watermark_hash(&target.chain_id, target.seq, &target.hash, pruned_at_ms))
        .collect();
    let (root, proofs) = merkle::root_with_proofs(&leaves);
    let size = targets.len() as i32;
    let kid = signer.kid().to_owned();
    let signature = signer
        .sign(&watermark_batch_hash(
            &root,
            i64::from(size),
            pruned_at_ms,
            &kid,
        ))
        .await?;
    Ok(targets
        .into_iter()
        .zip(proofs)
        .enumerate()
        .map(|(index, (target, proof))| audit_watermark::ActiveModel {
            chain_id: Set(target.chain_id),
            seq: Set(target.seq),
            hash: Set(target.hash),
            pruned_at: Set(pruned_at),
            batch_root: Set(root.to_vec()),
            batch_size: Set(size),
            batch_index: Set(index as i32),
            batch_proof: Set(merkle::encode_proof(&proof)),
            kid: Set(kid.clone()),
            signature: Set(signature.to_vec()),
            pending_delete: Set(true),
        })
        .collect())
}

/// Write signed watermarks, replacing each chain's previous one. A watermark only moves
/// forward: a stale worker's older target never overwrites a newer one.
pub async fn upsert<C: ConnectionTrait>(
    db: &C,
    watermarks: Vec<audit_watermark::ActiveModel>,
) -> Result<(), DbErr> {
    for chunk in watermarks.chunks(UPSERT_CHUNK) {
        audit_watermark::Entity::insert_many(chunk.to_vec())
            .on_conflict(
                OnConflict::column(audit_watermark::Column::ChainId)
                    .update_columns([
                        audit_watermark::Column::Seq,
                        audit_watermark::Column::Hash,
                        audit_watermark::Column::PrunedAt,
                        audit_watermark::Column::BatchRoot,
                        audit_watermark::Column::BatchSize,
                        audit_watermark::Column::BatchIndex,
                        audit_watermark::Column::BatchProof,
                        audit_watermark::Column::Kid,
                        audit_watermark::Column::Signature,
                        audit_watermark::Column::PendingDelete,
                    ])
                    .action_and_where(
                        Expr::col((audit_watermark::Entity, audit_watermark::Column::Seq)).lt(
                            Expr::col((Alias::new("excluded"), audit_watermark::Column::Seq)),
                        ),
                    )
                    .to_owned(),
            )
            .exec_without_returning(db)
            .await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audit::signer::{AuditSigner, LocalSigner, register_verifying_key};
    use crate::audit::verify::check_watermark;
    use p256::ecdsa::SigningKey;
    use sea_orm::TryIntoModel;
    use std::sync::Arc;

    fn signer() -> SharedSigner {
        let local = LocalSigner::new(
            SigningKey::from_slice(&[31; 32]).unwrap(),
            Some("watermark-test".into()),
        );
        register_verifying_key(local.kid(), local.verifying_key()).unwrap();
        Arc::new(local)
    }

    fn targets(count: usize) -> Vec<WatermarkTarget> {
        (0..count)
            .map(|index| WatermarkTarget {
                chain_id: format!("chain-{index}"),
                seq: index as i64 + 1,
                hash: vec![index as u8; 32],
            })
            .collect()
    }

    #[flow_like_types::tokio::test]
    async fn every_watermark_of_a_batch_verifies_with_one_signature() {
        let models = sign_batch(&signer(), targets(7), Utc::now()).await.unwrap();
        assert_eq!(models.len(), 7);
        let models: Vec<audit_watermark::Model> = models
            .into_iter()
            .map(|model| model.try_into_model().unwrap())
            .collect();
        assert!(
            models
                .windows(2)
                .all(|pair| pair[0].signature == pair[1].signature)
        );
        for model in &models {
            assert_eq!(check_watermark(model), Ok(true));
        }

        let mut moved = models[3].clone();
        moved.seq += 1;
        assert!(check_watermark(&moved).is_err());
        let mut swapped = models[3].clone();
        swapped.batch_index = 4;
        assert!(check_watermark(&swapped).is_err());
        let mut resized = models[3].clone();
        resized.batch_size = 8;
        assert!(check_watermark(&resized).is_err());
    }

    #[flow_like_types::tokio::test]
    async fn empty_and_oversized_batches() {
        assert!(
            sign_batch(&signer(), Vec::new(), Utc::now())
                .await
                .unwrap()
                .is_empty()
        );
        assert!(
            sign_batch(&signer(), targets(MAX_BATCH + 1), Utc::now())
                .await
                .is_err()
        );
    }
}
