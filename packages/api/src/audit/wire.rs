//! The line format shared by monthly archives and customer exports: newline-delimited
//! JSON, one epoch, seal, record, watermark or manifest per line. Everything a verifier
//! needs to re-check hashes, Merkle proofs and signatures offline is in these lines.

use flow_like_types::Value;
use flow_like_types::base64::{Engine, engine::general_purpose::STANDARD};
use sea_orm::ActiveEnum;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::entity::{audit_epoch, audit_record, audit_seal, audit_watermark};

use super::merkle;

#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Line {
    Epoch(EpochLine),
    Seal(SealLine),
    Record(RecordLine),
    Watermark(WatermarkLine),
}

#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct EpochLine {
    pub seq: i64,
    pub prev_hash: String,
    pub seal_count: i32,
    pub seals_root: String,
    pub created_at_ms: i64,
    pub kid: String,
    /// Base64 of the raw 64-byte P-256 signature over the epoch hash.
    pub signature: String,
    pub hash: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct SealLine {
    pub id: String,
    pub chain_id: String,
    pub seq: i64,
    pub class: String,
    pub prev_hash: String,
    pub record_count: i32,
    pub records_root: String,
    pub first_at_ms: i64,
    pub last_at_ms: i64,
    pub sealed_at_ms: i64,
    pub hash: String,
    pub epoch_seq: Option<i64>,
    pub epoch_index: Option<i32>,
    /// Sibling hashes from the leaf up, hex.
    pub epoch_proof: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct RecordLine {
    pub id: String,
    pub chain_id: String,
    pub seal_id: Option<String>,
    pub timestamp_ms: i64,
    pub actor_id: String,
    pub actor_type: String,
    pub action: String,
    pub resource_type: String,
    pub resource_id: String,
    pub ip_commitment: Option<String>,
    pub details_commitment: Option<String>,
    /// Raw values travel only while they are retained; the commitments always do.
    pub actor_ip: Option<String>,
    pub ip_salt: Option<String>,
    #[schema(value_type = Option<Object>)]
    pub details: Option<Value>,
    pub details_salt: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct WatermarkLine {
    pub chain_id: String,
    pub seq: i64,
    pub hash: String,
    pub pruned_at_ms: i64,
    /// Merkle root over every watermark of the prune run; the signature covers it.
    pub batch_root: String,
    pub batch_size: i32,
    pub batch_index: i32,
    /// Sibling hashes from the leaf up, hex.
    pub batch_proof: Vec<String>,
    pub kid: String,
    pub signature: String,
}

/// Which raw personal values a line may carry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RawValues {
    pub ip: bool,
    pub details: bool,
}

impl RawValues {
    pub const ALL: Self = Self {
        ip: true,
        details: true,
    };
    pub const NONE: Self = Self {
        ip: false,
        details: false,
    };
}

impl From<&audit_epoch::Model> for EpochLine {
    fn from(epoch: &audit_epoch::Model) -> Self {
        Self {
            seq: epoch.seq,
            prev_hash: hex::encode(&epoch.prev_hash),
            seal_count: epoch.seal_count,
            seals_root: hex::encode(&epoch.seals_root),
            created_at_ms: epoch.created_at.timestamp_millis(),
            kid: epoch.kid.clone(),
            signature: STANDARD.encode(&epoch.signature),
            hash: hex::encode(&epoch.hash),
        }
    }
}

impl From<&audit_seal::Model> for SealLine {
    fn from(seal: &audit_seal::Model) -> Self {
        Self {
            id: seal.id.clone(),
            chain_id: seal.chain_id.clone(),
            seq: seal.seq,
            class: seal.class.clone(),
            prev_hash: hex::encode(&seal.prev_hash),
            record_count: seal.record_count,
            records_root: hex::encode(&seal.records_root),
            first_at_ms: seal.first_at.timestamp_millis(),
            last_at_ms: seal.last_at.timestamp_millis(),
            sealed_at_ms: seal.sealed_at.timestamp_millis(),
            hash: hex::encode(&seal.hash),
            epoch_seq: seal.epoch_seq,
            epoch_index: seal.epoch_index,
            epoch_proof: seal
                .epoch_proof
                .as_deref()
                .and_then(merkle::decode_proof)
                .unwrap_or_default()
                .iter()
                .map(hex::encode)
                .collect(),
        }
    }
}

impl RecordLine {
    pub fn from_model(record: &audit_record::Model, raw: RawValues) -> Self {
        let (actor_ip, ip_salt) = if raw.ip {
            (
                record.actor_ip.clone(),
                record.ip_salt.as_deref().map(hex::encode),
            )
        } else {
            (None, None)
        };
        let (details, details_salt) = if raw.details {
            (
                record.details.clone(),
                record.details_salt.as_deref().map(hex::encode),
            )
        } else {
            (None, None)
        };
        Self {
            id: record.id.clone(),
            chain_id: record.chain_id.clone(),
            seal_id: record.seal_id.clone(),
            timestamp_ms: record.timestamp.timestamp_millis(),
            actor_id: record.actor_id.clone(),
            actor_type: record.actor_type.to_value(),
            action: record.action.clone(),
            resource_type: record.resource_type.clone(),
            resource_id: record.resource_id.clone(),
            ip_commitment: record.ip_commitment.as_deref().map(hex::encode),
            details_commitment: record.details_commitment.as_deref().map(hex::encode),
            actor_ip,
            ip_salt,
            details,
            details_salt,
        }
    }
}

impl From<&audit_watermark::Model> for WatermarkLine {
    fn from(watermark: &audit_watermark::Model) -> Self {
        Self {
            chain_id: watermark.chain_id.clone(),
            seq: watermark.seq,
            hash: hex::encode(&watermark.hash),
            pruned_at_ms: watermark.pruned_at.timestamp_millis(),
            batch_root: hex::encode(&watermark.batch_root),
            batch_size: watermark.batch_size,
            batch_index: watermark.batch_index,
            batch_proof: merkle::decode_proof(&watermark.batch_proof)
                .unwrap_or_default()
                .iter()
                .map(hex::encode)
                .collect(),
            kid: watermark.kid.clone(),
            signature: STANDARD.encode(&watermark.signature),
        }
    }
}

/// Serialize one line, newline included.
pub fn encode(line: &Line) -> Vec<u8> {
    let mut bytes = serde_json::to_vec(line).expect("audit lines always serialize");
    bytes.push(b'\n');
    bytes
}
