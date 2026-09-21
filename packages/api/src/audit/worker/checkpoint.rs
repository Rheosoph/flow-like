//! Signed daily epoch and per-chain tips, kept outside the database. Operators can
//! retain these objects independently and check a restored database before trusting it.

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use flow_like_storage::{
    Path,
    object_store::{Error as StoreError, ObjectStoreExt},
};
use flow_like_types::{
    Result, anyhow,
    base64::{Engine, engine::general_purpose::STANDARD},
};
use futures::TryStreamExt;
use sea_orm::{ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, QueryOrder, Statement};
use serde::{Deserialize, Serialize};

use super::{AuditWorkerContext, bucket::put_bytes, lease::Lease};
use crate::{
    audit::{
        crypto, merkle,
        signer::{self, SignatureCheck},
        verify::{self, HeadCheck, RetainedSeal},
    },
    entity::{audit_epoch, audit_seal, audit_watermark},
};

const DOMAIN: &str = "flow-like.audit-checkpoint/v1";

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Tip {
    pub seq: i64,
    pub hash: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Checkpoint {
    pub version: u32,
    pub written_at_ms: i64,
    pub epoch: Tip,
    pub chains: BTreeMap<String, Tip>,
    pub kid: String,
    pub signature: String,
}

#[derive(Default)]
pub(super) struct CheckpointState {
    loaded: bool,
    latest: Option<Checkpoint>,
}

impl CheckpointState {
    pub(super) fn invalidate(&mut self) {
        self.loaded = false;
        self.latest = None;
    }
}

fn digest(checkpoint: &Checkpoint) -> crypto::Hash {
    let value = serde_json::json!({
        "domain": DOMAIN, "version": checkpoint.version,
        "written_at_ms": checkpoint.written_at_ms, "epoch": checkpoint.epoch,
        "chains": checkpoint.chains, "kid": checkpoint.kid,
    });
    *blake3::hash(crypto::canonical_json(&value).as_bytes()).as_bytes()
}

fn hash(tip: &Tip) -> Result<crypto::Hash> {
    let bytes = hex::decode(&tip.hash)?;
    if tip.seq < 1 {
        return Err(anyhow!("checkpoint sequence must be positive"));
    }
    crypto::to_hash(&bytes).ok_or_else(|| anyhow!("invalid checkpoint hash"))
}

pub fn authenticate(checkpoint: &Checkpoint) -> Result<()> {
    if checkpoint.version != 1 {
        return Err(anyhow!("unsupported audit checkpoint version"));
    }
    if DateTime::from_timestamp_millis(checkpoint.written_at_ms).is_none() {
        return Err(anyhow!("invalid checkpoint timestamp"));
    }
    hash(&checkpoint.epoch)?;
    for tip in checkpoint.chains.values() {
        hash(tip)?;
    }
    let signature = STANDARD.decode(&checkpoint.signature)?;
    if signer::verify(&checkpoint.kid, &digest(checkpoint), &signature) != SignatureCheck::Valid {
        return Err(anyhow!(
            "checkpoint signature is invalid or its public key is unavailable"
        ));
    }
    Ok(())
}

/// Checks a retained, signed checkpoint with public keys and read-only DB access.
/// Archived tips are accepted only through the existing signed prune-watermark check.
pub async fn verify_retained(db: &DatabaseConnection, checkpoint: &Checkpoint) -> Result<()> {
    verify_retained_inner(db, checkpoint, None).await
}

async fn keep_lease(lease: Option<&Lease>) -> Result<()> {
    if let Some(lease) = lease
        && !lease.keepalive().await
    {
        return Err(anyhow!("audit checkpoint worker lost its lease"));
    }
    Ok(())
}

fn verify_tip_seal(seal: &audit_seal::Model, epoch: &audit_epoch::Model) -> Result<()> {
    let seal_hash = verify::seal_hash_of(seal);
    let root = crypto::to_hash(&epoch.seals_root).ok_or_else(|| anyhow!("invalid epoch root"))?;
    let proof = seal
        .epoch_proof
        .as_deref()
        .and_then(merkle::decode_proof)
        .ok_or_else(|| anyhow!("invalid checkpoint inclusion proof"))?;
    if seal.hash != seal_hash
        || !verify::check_epoch(epoch).map_err(|error| anyhow!(error))?
        || !seal.epoch_index.is_some_and(|index| {
            index >= 0
                && merkle::verify_inclusion(
                    &seal_hash,
                    index as u64,
                    epoch.seal_count.max(0) as u64,
                    &proof,
                    &root,
                )
        })
    {
        return Err(anyhow!(
            "checkpoint seal {} fails its hash or inclusion proof",
            seal.id
        ));
    }
    Ok(())
}

async fn verify_retained_inner(
    db: &DatabaseConnection,
    checkpoint: &Checkpoint,
    lease: Option<&Lease>,
) -> Result<()> {
    authenticate(checkpoint)?;
    keep_lease(lease).await?;
    let epoch_hash = hash(&checkpoint.epoch)?;
    if verify::check_head(db, checkpoint.epoch.seq, &epoch_hash, None).await? == HeadCheck::Differs
    {
        return Err(anyhow!(
            "audit timeline differs from retained epoch {}",
            checkpoint.epoch.seq
        ));
    }
    if let Some(epoch) = audit_epoch::Entity::find_by_id(checkpoint.epoch.seq)
        .one(db)
        .await?
        && !verify::check_epoch(&epoch).map_err(|error| anyhow!(error))?
    {
        return Err(anyhow!("retained checkpoint epoch is unverifiable"));
    }
    for (chain, tip) in &checkpoint.chains {
        keep_lease(lease).await?;
        let tip_hash = hash(tip)?;
        let retained = RetainedSeal {
            chain_id: chain,
            seq: tip.seq,
            hash: &tip_hash,
        };
        if verify::check_head(db, checkpoint.epoch.seq, &epoch_hash, Some(retained)).await?
            == HeadCheck::Differs
        {
            return Err(anyhow!(
                "audit chain {chain} differs from retained seal {}",
                tip.seq
            ));
        }
        if let Some(seal) = audit_seal::Entity::find()
            .filter(audit_seal::Column::ChainId.eq(chain))
            .filter(audit_seal::Column::Seq.eq(tip.seq))
            .one(db)
            .await?
        {
            let epoch_seq = seal
                .epoch_seq
                .ok_or_else(|| anyhow!("retained checkpoint seal is unanchored"))?;
            if epoch_seq > checkpoint.epoch.seq {
                return Err(anyhow!("retained checkpoint seal is ahead of its epoch"));
            }
            let epoch = audit_epoch::Entity::find_by_id(epoch_seq)
                .one(db)
                .await?
                .ok_or_else(|| anyhow!("retained checkpoint seal has no epoch"))?;
            verify_tip_seal(&seal, &epoch)?;
        }
    }
    Ok(())
}

pub(super) async fn check_previous(context: &AuditWorkerContext, lease: &Lease) -> Result<()> {
    let Some(bucket) = context.bucket.as_deref() else {
        return Ok(());
    };
    let mut state = context.checkpoint.lock().await;
    if !state.loaded {
        let prefix = Path::from("checkpoints/");
        let mut objects = bucket.as_generic().list(Some(&prefix));
        let mut latest = None;
        while let Some(object) = objects.try_next().await? {
            keep_lease(Some(lease)).await?;
            if object.location.as_ref().ends_with(".json")
                && latest.as_ref().is_none_or(|key| object.location > *key)
            {
                latest = Some(object.location);
            }
        }
        if let Some(key) = latest {
            let bytes = bucket.as_generic().get(&key).await?.bytes().await?;
            state.latest = Some(serde_json::from_slice(&bytes)?);
        }
        state.loaded = true;
    }
    if let Some(checkpoint) = &state.latest {
        verify_retained_inner(&context.db, checkpoint, Some(lease)).await?;
    }
    Ok(())
}

pub(super) async fn write_daily(
    context: &AuditWorkerContext,
    lease: &Lease,
    now: DateTime<Utc>,
) -> Result<()> {
    let (Some(bucket), Some(signer)) = (context.bucket.as_deref(), context.signer()) else {
        return Ok(());
    };
    keep_lease(Some(lease)).await?;
    let mut state = context.checkpoint.lock().await;
    if state.latest.as_ref().is_some_and(|checkpoint| {
        DateTime::from_timestamp_millis(checkpoint.written_at_ms)
            .is_some_and(|at| at.date_naive() >= now.date_naive())
    }) {
        return Ok(());
    }
    let key = Path::from(format!("checkpoints/{}.json", now.format("%Y/%m/%d")));
    match bucket.as_generic().get(&key).await {
        Ok(object) => {
            let checkpoint = serde_json::from_slice(&object.bytes().await?)?;
            verify_retained_inner(&context.db, &checkpoint, Some(lease)).await?;
            state.latest = Some(checkpoint);
            return Ok(());
        }
        Err(StoreError::NotFound { .. }) => {}
        Err(error) => return Err(error.into()),
    }
    let Some(epoch) = audit_epoch::Entity::find()
        .order_by_desc(audit_epoch::Column::Seq)
        .one(&context.db)
        .await?
    else {
        return Ok(());
    };
    if verify::check_epoch(&epoch).map_err(|error| anyhow!(error))? != true {
        return Err(anyhow!("cannot checkpoint an unverifiable epoch"));
    }
    // One row per chain. Include signed watermarks for chains already fully pruned.
    let seals = audit_seal::Entity::find().from_raw_sql(Statement::from_string(context.db.get_database_backend(),
        r#"SELECT s.* FROM "AuditSeal" s JOIN (SELECT "chainId", max("seq") AS seq FROM "AuditSeal" WHERE "epochSeq" IS NOT NULL GROUP BY "chainId") t ON s."chainId" = t."chainId" AND s."seq" = t.seq"#,
    )).all(&context.db).await?;
    let mut chains = BTreeMap::new();
    // Fetch epochs in bounded chunks; never sign a chain tip just because its row
    // claims it belongs to a signed epoch.
    for chunk in seals.chunks(500) {
        keep_lease(Some(lease)).await?;
        let epochs: BTreeMap<_, _> = audit_epoch::Entity::find()
            .filter(audit_epoch::Column::Seq.is_in(chunk.iter().filter_map(|seal| seal.epoch_seq)))
            .all(&context.db)
            .await?
            .into_iter()
            .map(|epoch| (epoch.seq, epoch))
            .collect();
        for seal in chunk {
            let epoch = seal
                .epoch_seq
                .and_then(|seq| epochs.get(&seq))
                .ok_or_else(|| anyhow!("checkpoint seal has no epoch"))?;
            verify_tip_seal(seal, epoch)?;
            chains.insert(
                seal.chain_id.clone(),
                Tip {
                    seq: seal.seq,
                    hash: hex::encode(&seal.hash),
                },
            );
        }
    }
    for watermark in audit_watermark::Entity::find().all(&context.db).await? {
        keep_lease(Some(lease)).await?;
        if watermark.chain_id == verify::EPOCH_WATERMARK {
            continue;
        }
        if !verify::check_watermark(&watermark).map_err(|error| anyhow!(error))? {
            return Err(anyhow!("cannot checkpoint an unverifiable watermark"));
        }
        let tip = Tip {
            seq: watermark.seq,
            hash: hex::encode(&watermark.hash),
        };
        if chains
            .get(&watermark.chain_id)
            .is_none_or(|old| old.seq < tip.seq)
        {
            chains.insert(watermark.chain_id, tip);
        }
    }
    let mut checkpoint = Checkpoint {
        version: 1,
        written_at_ms: now.timestamp_millis(),
        epoch: Tip {
            seq: epoch.seq,
            hash: hex::encode(&epoch.hash),
        },
        chains,
        kid: signer.kid().to_owned(),
        signature: String::new(),
    };
    keep_lease(Some(lease)).await?;
    checkpoint.signature = STANDARD.encode(signer.sign(&digest(&checkpoint)).await?);
    keep_lease(Some(lease)).await?;
    put_bytes(bucket, &key, serde_json::to_vec_pretty(&checkpoint)?).await?;
    state.latest = Some(checkpoint);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audit::signer::{AuditSigner, LocalSigner};
    use p256::ecdsa::SigningKey;

    #[flow_like_types::tokio::test]
    async fn checkpoint_signature_covers_chain_tips_and_timestamp() {
        let signer = LocalSigner::new(SigningKey::from_bytes((&[61u8; 32]).into()).unwrap(), None);
        signer::register_verifying_key(signer.kid(), signer.verifying_key()).unwrap();
        let mut checkpoint = Checkpoint {
            version: 1,
            written_at_ms: 1,
            epoch: Tip {
                seq: 1,
                hash: hex::encode([1u8; 32]),
            },
            chains: BTreeMap::from([(
                "app".into(),
                Tip {
                    seq: 2,
                    hash: hex::encode([2u8; 32]),
                },
            )]),
            kid: signer.kid().into(),
            signature: String::new(),
        };
        checkpoint.signature = STANDARD.encode(signer.sign(&digest(&checkpoint)).await.unwrap());
        authenticate(&checkpoint).unwrap();
        let encoded = serde_json::to_vec(&checkpoint).unwrap();
        authenticate(&serde_json::from_slice(&encoded).unwrap()).unwrap();
        checkpoint.chains.clear();
        assert!(authenticate(&checkpoint).is_err());
        let mut checkpoint: Checkpoint = serde_json::from_slice(&encoded).unwrap();
        checkpoint.written_at_ms += 1;
        assert!(authenticate(&checkpoint).is_err());
    }
}
