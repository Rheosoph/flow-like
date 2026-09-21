//! Hashes, commitments and MACs of the audit trail.
//!
//! Every structure is hashed with BLAKE3 over canonical JSON that starts with a domain
//! string, so field boundaries are unambiguous and a hash of one kind can never be
//! replayed as another. Personal values never enter a record hash directly: the hash
//! covers a salted commitment, so the raw value can expire without breaking the chain.

use flow_like_types::Value;
use std::collections::BTreeMap;

pub type Hash = [u8; 32];

/// `prevHash` of the first seal of a chain and of the first epoch.
pub const ZERO_HASH: Hash = [0; 32];

const RECORD_DOMAIN: &str = "flow-like.audit-record/v1";
const SEAL_DOMAIN: &str = "flow-like.audit-seal/v1";
const EPOCH_DOMAIN: &str = "flow-like.audit-epoch/v1";
const WATERMARK_DOMAIN: &str = "flow-like.audit-watermark/v1";
const WATERMARK_BATCH_DOMAIN: &str = "flow-like.audit-watermark-batch/v1";
const MANIFEST_DOMAIN: &str = "flow-like.audit-manifest/v1";
const IP_CONTEXT: &[u8] = b"flow-like.audit-ip/v1\0";
const DETAILS_CONTEXT: &[u8] = b"flow-like.audit-details/v1\0";
const MAC_CONTEXT: &[u8] = b"flow-like.audit-mac/v1\0";
const SEAL_MAC_CONTEXT: &[u8] = b"flow-like.audit-seal-mac/v1\0";
const ONCE_CONTEXT: &[u8] = b"flow-like.audit-once/v1\0";

/// The fields a record hash covers. Everything stored on the row except the raw
/// personal values, their salts, the MAC and the seal id.
#[derive(Clone, Debug)]
pub struct RecordFields<'a> {
    pub id: &'a str,
    pub chain_id: &'a str,
    pub timestamp_ms: i64,
    pub actor_id: &'a str,
    pub actor_type: &'a str,
    pub action: &'a str,
    pub resource_type: &'a str,
    pub resource_id: &'a str,
    pub ip_commitment: Option<&'a [u8]>,
    pub details_commitment: Option<&'a [u8]>,
}

pub fn record_hash(fields: &RecordFields<'_>) -> Hash {
    digest(&serde_json::json!({
        "domain": RECORD_DOMAIN,
        "id": fields.id,
        "chain_id": fields.chain_id,
        "timestamp_ms": fields.timestamp_ms,
        "actor_id": fields.actor_id,
        "actor_type": fields.actor_type,
        "action": fields.action,
        "resource_type": fields.resource_type,
        "resource_id": fields.resource_id,
        "ip_commitment": fields.ip_commitment.map(hex::encode),
        "details_commitment": fields.details_commitment.map(hex::encode),
    }))
}

#[derive(Clone, Debug)]
pub struct SealFields<'a> {
    pub id: &'a str,
    pub chain_id: &'a str,
    pub seq: i64,
    pub class: &'a str,
    pub prev_hash: &'a [u8],
    pub record_count: i64,
    pub records_root: &'a [u8],
    pub first_at_ms: i64,
    pub last_at_ms: i64,
    pub sealed_at_ms: i64,
}

pub fn seal_hash(fields: &SealFields<'_>) -> Hash {
    digest(&serde_json::json!({
        "domain": SEAL_DOMAIN,
        "id": fields.id,
        "chain_id": fields.chain_id,
        "seq": fields.seq,
        "class": fields.class,
        "prev_hash": hex::encode(fields.prev_hash),
        "record_count": fields.record_count,
        "records_root": hex::encode(fields.records_root),
        "first_at_ms": fields.first_at_ms,
        "last_at_ms": fields.last_at_ms,
        "sealed_at_ms": fields.sealed_at_ms,
    }))
}

#[derive(Clone, Debug)]
pub struct EpochFields<'a> {
    pub seq: i64,
    pub prev_hash: &'a [u8],
    pub seal_count: i64,
    pub seals_root: &'a [u8],
    pub created_at_ms: i64,
    pub kid: &'a str,
}

/// The value the audit key signs.
pub fn epoch_hash(fields: &EpochFields<'_>) -> Hash {
    digest(&serde_json::json!({
        "domain": EPOCH_DOMAIN,
        "seq": fields.seq,
        "prev_hash": hex::encode(fields.prev_hash),
        "seal_count": fields.seal_count,
        "seals_root": hex::encode(fields.seals_root),
        "created_at_ms": fields.created_at_ms,
        "kid": fields.kid,
    }))
}

/// Last pruned seal or epoch of a chain: a leaf of its prune run's batch.
pub fn watermark_hash(chain_id: &str, seq: i64, hash: &[u8], pruned_at_ms: i64) -> Hash {
    digest(&serde_json::json!({
        "domain": WATERMARK_DOMAIN,
        "chain_id": chain_id,
        "seq": seq,
        "hash": hex::encode(hash),
        "pruned_at_ms": pruned_at_ms,
    }))
}

/// The value the audit key signs once per prune run, covering every watermark it wrote.
pub fn watermark_batch_hash(root: &[u8], size: i64, pruned_at_ms: i64, kid: &str) -> Hash {
    digest(&serde_json::json!({
        "domain": WATERMARK_BATCH_DOMAIN,
        "root": hex::encode(root),
        "size": size,
        "pruned_at_ms": pruned_at_ms,
        "kid": kid,
    }))
}

/// Hash of an archive manifest. `manifest` must not contain a signature.
pub fn manifest_hash(manifest: &Value) -> Hash {
    digest(&serde_json::json!({
        "domain": MANIFEST_DOMAIN,
        "manifest": manifest,
    }))
}

pub fn ip_commitment(salt: &Hash, ip: &str) -> Hash {
    keyed(salt, IP_CONTEXT, ip.as_bytes())
}

pub fn details_commitment(salt: &Hash, details: &Value) -> Hash {
    keyed(salt, DETAILS_CONTEXT, canonical_json(details).as_bytes())
}

/// MAC a pending record carries until it is sealed. It binds the record hash, which
/// in turn binds the commitments and therefore the raw personal values.
pub fn record_mac(entry_key: &Hash, record_hash: &Hash) -> Hash {
    keyed(entry_key, MAC_CONTEXT, record_hash)
}

/// MAC a seal carries until an epoch anchors it, so sealed records stay protected by
/// the entry key between sealing and the next signature.
pub fn seal_mac(entry_key: &Hash, seal_hash: &Hash) -> Hash {
    keyed(entry_key, SEAL_MAC_CONTEXT, seal_hash)
}

/// Constant-time check of a stored record MAC.
pub fn mac_matches(entry_key: &Hash, record_hash: &Hash, stored: &[u8]) -> bool {
    same(&record_mac(entry_key, record_hash), stored)
}

/// Constant-time check of a stored seal MAC.
pub fn seal_mac_matches(entry_key: &Hash, seal_hash: &Hash, stored: &[u8]) -> bool {
    same(&seal_mac(entry_key, seal_hash), stored)
}

fn same(expected: &Hash, stored: &[u8]) -> bool {
    let Ok(stored) = <[u8; 32]>::try_from(stored) else {
        return false;
    };
    blake3::Hash::from(*expected) == blake3::Hash::from(stored)
}

/// Deterministic record id for lifecycle entries that must exist once per resource,
/// such as the terminal state of a run. A repeated write becomes a no-op insert.
pub fn once_id(chain_id: &str, action: &str, resource_type: &str, resource_id: &str) -> String {
    let mut hasher = blake3::Hasher::new();
    hasher.update(ONCE_CONTEXT);
    for part in [chain_id, action, resource_type, resource_id] {
        hasher.update(&(part.len() as u64).to_be_bytes());
        hasher.update(part.as_bytes());
    }
    format!("once_{}", &hasher.finalize().to_hex()[..40])
}

pub fn random_salt() -> Hash {
    let mut salt = [0; 32];
    getrandom::fill(&mut salt).expect("operating system random source");
    salt
}

pub fn to_hash(bytes: &[u8]) -> Option<Hash> {
    <[u8; 32]>::try_from(bytes).ok()
}

fn keyed(key: &Hash, context: &[u8], data: &[u8]) -> Hash {
    let mut hasher = blake3::Hasher::new_keyed(key);
    hasher.update(context);
    hasher.update(data);
    *hasher.finalize().as_bytes()
}

fn digest(value: &Value) -> Hash {
    *blake3::hash(canonical_json(value).as_bytes()).as_bytes()
}

/// Sorted-key JSON with normalized numbers (an RFC 8785 subset). JSONB reorders keys,
/// drops duplicates and respells numbers, so a value read back from the database must
/// produce the same bytes as the value that was written.
pub fn canonical_json(value: &Value) -> String {
    match value {
        Value::Object(map) => {
            let sorted: BTreeMap<_, _> = map.iter().collect();
            let entries: Vec<String> = sorted
                .into_iter()
                .map(|(key, value)| {
                    format!(
                        "{}:{}",
                        serde_json::to_string(key).unwrap_or_default(),
                        canonical_json(value)
                    )
                })
                .collect();
            format!("{{{}}}", entries.join(","))
        }
        Value::Array(items) => {
            let entries: Vec<String> = items.iter().map(canonical_json).collect();
            format!("[{}]", entries.join(","))
        }
        Value::Number(number) => canonical_number(number),
        _ => serde_json::to_string(value).unwrap_or_default(),
    }
}

/// Decimal spelling without precision loss: integers are never converted to f64.
fn canonical_number(number: &serde_json::Number) -> String {
    let text = number.to_string();
    let (sign, unsigned) = text
        .strip_prefix('-')
        .map_or(("", text.as_str()), |value| ("-", value));
    let (mantissa, exponent) = match unsigned.split_once(['e', 'E']) {
        None => (unsigned, 0i64),
        Some((mantissa, exponent)) => match exponent.parse::<i64>() {
            Ok(exponent) => (mantissa, exponent),
            Err(_) => return text,
        },
    };
    let fractional_digits = mantissa
        .split_once('.')
        .map_or(0, |(_, fraction)| fraction.len()) as i64;
    let digits = mantissa.replace('.', "");
    let significant = digits.trim_start_matches('0');
    if significant.is_empty() {
        return "0".into();
    }
    let trimmed = significant.trim_end_matches('0');
    let exponent = exponent - fractional_digits + (significant.len() - trimmed.len()) as i64;
    format!("{sign}{trimmed}e{exponent}")
}

/// Replace U+0000, which PostgreSQL rejects in TEXT and JSONB, before hashing so the
/// stored value equals the hashed one.
pub fn strip_nul(text: &mut String) {
    if text.contains('\0') {
        *text = text.replace('\0', "\u{fffd}");
    }
}

pub fn strip_nul_value(value: &mut Value) {
    match value {
        Value::String(text) => strip_nul(text),
        Value::Array(items) => items.iter_mut().for_each(strip_nul_value),
        Value::Object(map) => {
            for (mut key, mut item) in std::mem::take(map) {
                strip_nul(&mut key);
                strip_nul_value(&mut item);
                map.insert(key, item);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fields<'a>(ip: Option<&'a [u8]>, details: Option<&'a [u8]>) -> RecordFields<'a> {
        RecordFields {
            id: "rec",
            chain_id: "app",
            timestamp_ms: 1_700_000_000_123,
            actor_id: "openid:user:",
            actor_type: "USER",
            action: "board.update",
            resource_type: "Board",
            resource_id: "board",
            ip_commitment: ip,
            details_commitment: details,
        }
    }

    #[test]
    fn every_record_field_changes_the_hash() {
        let base = record_hash(&fields(None, None));
        let mutations: Vec<Box<dyn Fn(&mut RecordFields<'static>)>> = vec![
            Box::new(|f| f.id = "other"),
            Box::new(|f| f.chain_id = "other"),
            Box::new(|f| f.timestamp_ms += 1),
            Box::new(|f| f.actor_id = "other"),
            Box::new(|f| f.actor_type = "SYSTEM"),
            Box::new(|f| f.action = "board.delete"),
            Box::new(|f| f.resource_type = "App"),
            Box::new(|f| f.resource_id = "other"),
            Box::new(|f| f.ip_commitment = Some(&[1; 32])),
            Box::new(|f| f.details_commitment = Some(&[2; 32])),
        ];
        for (index, mutate) in mutations.into_iter().enumerate() {
            let mut changed = fields(None, None);
            mutate(&mut changed);
            assert_ne!(record_hash(&changed), base, "mutation {index}");
        }
    }

    #[test]
    fn adjacent_fields_are_framed() {
        let mut left = fields(None, None);
        left.actor_id = "ab";
        left.action = "c";
        let mut right = fields(None, None);
        right.actor_id = "a";
        right.action = "bc";
        assert_ne!(record_hash(&left), record_hash(&right));
    }

    #[test]
    fn domains_never_collide() {
        let salt = [7; 32];
        assert_ne!(
            ip_commitment(&salt, "x"),
            details_commitment(&salt, &Value::from("x"))
        );
        assert_ne!(record_mac(&salt, &[0; 32]), ip_commitment(&salt, ""));
    }

    #[test]
    fn commitments_depend_on_salt_and_value() {
        let (a, b) = ([1; 32], [2; 32]);
        assert_ne!(
            ip_commitment(&a, "192.0.2.1"),
            ip_commitment(&b, "192.0.2.1")
        );
        assert_ne!(
            ip_commitment(&a, "192.0.2.1"),
            ip_commitment(&a, "192.0.2.2")
        );
        assert_eq!(
            ip_commitment(&a, "192.0.2.1"),
            ip_commitment(&a, "192.0.2.1")
        );
    }

    #[test]
    fn details_commitment_survives_a_jsonb_round_trip() {
        let salt = [3; 32];
        let written: Value =
            serde_json::from_str(r#"{"b":[1.0,1e18],"a":{"z":-0.0,"y":1e-7}}"#).unwrap();
        let read_back: Value =
            serde_json::from_str(r#"{"a":{"y":0.0000001,"z":0.0},"b":[1,1000000000000000000]}"#)
                .unwrap();
        assert_eq!(
            details_commitment(&salt, &written),
            details_commitment(&salt, &read_back)
        );
        let exact: Value = serde_json::from_str("9007199254740993").unwrap();
        let rounded: Value = serde_json::from_str("9007199254740992").unwrap();
        assert_ne!(
            details_commitment(&salt, &exact),
            details_commitment(&salt, &rounded)
        );
    }

    #[test]
    fn oversized_exponents_do_not_panic() {
        let huge: Value = serde_json::from_str("1e99999999999").unwrap_or(Value::Null);
        let _ = canonical_json(&huge);
        let number = serde_json::Number::from_f64(1e300).unwrap();
        assert!(!canonical_number(&number).is_empty());
    }

    #[test]
    fn mac_rejects_tampering_and_wrong_keys() {
        let key = [9; 32];
        let hash = record_hash(&fields(None, None));
        let mac = record_mac(&key, &hash);
        assert!(mac_matches(&key, &hash, &mac));
        assert!(!mac_matches(&[8; 32], &hash, &mac));
        assert!(!mac_matches(
            &key,
            &record_hash(&fields(Some(&[1; 32]), None)),
            &mac
        ));
        assert!(!mac_matches(&key, &hash, &mac[..31]));
    }

    #[test]
    fn seal_and_record_macs_are_not_interchangeable() {
        let key = [9; 32];
        let hash = [4; 32];
        let seal = seal_mac(&key, &hash);
        assert!(seal_mac_matches(&key, &hash, &seal));
        assert!(!mac_matches(&key, &hash, &seal));
        assert!(!seal_mac_matches(&key, &hash, &record_mac(&key, &hash)));
        assert!(!seal_mac_matches(&[8; 32], &hash, &seal));
    }

    #[test]
    fn watermark_batch_hash_covers_its_fields() {
        let base = watermark_batch_hash(&[1; 32], 3, 10, "key");
        assert_ne!(base, watermark_batch_hash(&[2; 32], 3, 10, "key"));
        assert_ne!(base, watermark_batch_hash(&[1; 32], 4, 10, "key"));
        assert_ne!(base, watermark_batch_hash(&[1; 32], 3, 11, "key"));
        assert_ne!(base, watermark_batch_hash(&[1; 32], 3, 10, "other"));
    }

    #[test]
    fn once_ids_are_stable_and_framed() {
        let id = once_id("app", "execution.board.complete", "ExecutionRun", "run");
        assert_eq!(
            id,
            once_id("app", "execution.board.complete", "ExecutionRun", "run")
        );
        assert_ne!(
            id,
            once_id("ap", "pexecution.board.complete", "ExecutionRun", "run")
        );
        assert!(id.starts_with("once_") && id.len() == 45);
    }

    #[test]
    fn nul_bytes_are_replaced_everywhere() {
        let mut value = serde_json::json!({"ke\u{0}y": ["va\u{0}lue", {"n": "\u{0}"}]});
        strip_nul_value(&mut value);
        assert!(!canonical_json(&value).contains("\\u0000"));
    }

    #[test]
    fn seal_epoch_and_watermark_hashes_cover_their_fields() {
        let seal = SealFields {
            id: "seal",
            chain_id: "app",
            seq: 1,
            class: "evidence",
            prev_hash: &ZERO_HASH,
            record_count: 2,
            records_root: &[5; 32],
            first_at_ms: 1,
            last_at_ms: 2,
            sealed_at_ms: 3,
        };
        let mut other = seal.clone();
        other.seq = 2;
        assert_ne!(seal_hash(&seal), seal_hash(&other));
        let epoch = EpochFields {
            seq: 1,
            prev_hash: &ZERO_HASH,
            seal_count: 1,
            seals_root: &[6; 32],
            created_at_ms: 4,
            kid: "key",
        };
        let mut rekeyed = epoch.clone();
        rekeyed.kid = "other";
        assert_ne!(epoch_hash(&epoch), epoch_hash(&rekeyed));
        assert_ne!(
            watermark_hash("app", 1, &[1; 32], 5),
            watermark_hash("app", 2, &[1; 32], 5)
        );
    }
}
