use blake3::Hasher;
use chrono::{DateTime, FixedOffset};
use flow_like_types::Value;
use std::collections::BTreeMap;

/// Previous hash for the root chain's first entry or a branch created before it.
pub const GENESIS_HASH: &str = "0000000000000000000000000000000000000000000000000000000000000000";

pub const HASH_V2_PREFIX: &str = "v2:";

/// The immutable fields covered by new entries. The signature signs this hash,
/// and the next entry also includes the signature in its own hash.
pub struct EntryHashFields<'a> {
    pub id: &'a str,
    pub sequence: i64,
    pub timestamp: &'a DateTime<FixedOffset>,
    pub actor_id: &'a str,
    pub actor_type: &'a str,
    pub actor_ip: Option<&'a str>,
    pub action: &'a str,
    pub resource_type: &'a str,
    pub resource_id: &'a str,
    pub chain_id: Option<&'a str>,
    pub summary: &'a str,
    pub details: Option<&'a Value>,
    pub prev_hash: &'a str,
    pub prev_signature: Option<&'a str>,
    pub kid: Option<&'a str>,
}

/// Hash a versioned canonical JSON object so field boundaries are unambiguous.
/// Milliseconds match the database timestamp precision. Optional details use an
/// array wrapper to distinguish SQL NULL from a stored JSON null.
pub fn compute_entry_hash_v2(fields: &EntryHashFields<'_>) -> String {
    let payload = serde_json::json!({
        "domain": "flow-like.audit-entry/v2",
        "id": fields.id,
        "sequence": fields.sequence,
        "timestamp_ms": fields.timestamp.timestamp_millis(),
        "actor_id": fields.actor_id,
        "actor_type": fields.actor_type,
        "actor_ip": fields.actor_ip,
        "action": fields.action,
        "resource_type": fields.resource_type,
        "resource_id": fields.resource_id,
        "chain_id": fields.chain_id,
        "summary": fields.summary,
        "details": fields.details.map(|value| [value]),
        "prev_hash": fields.prev_hash,
        "prev_signature": fields.prev_signature,
        "kid": fields.kid,
    });
    format!(
        "{HASH_V2_PREFIX}{}",
        blake3::hash(canonical_json_with_numbers(&payload, true).as_bytes()).to_hex()
    )
}

/// Produce a canonical (sorted-key) JSON string from a serde_json Value.
/// This ensures deterministic hashing regardless of key insertion order (RFC 8785 subset).
fn canonical_json(value: &Value) -> String {
    canonical_json_with_numbers(value, false)
}

fn canonical_json_with_numbers(value: &Value, normalize_numbers: bool) -> String {
    match value {
        Value::Object(map) => {
            let sorted: BTreeMap<_, _> = map.iter().collect();
            let entries: Vec<String> = sorted
                .into_iter()
                .map(|(k, v)| {
                    format!(
                        "{}:{}",
                        serde_json::to_string(k).unwrap_or_default(),
                        canonical_json_with_numbers(v, normalize_numbers)
                    )
                })
                .collect();
            format!("{{{}}}", entries.join(","))
        }
        Value::Array(arr) => {
            let entries: Vec<String> = arr
                .iter()
                .map(|value| canonical_json_with_numbers(value, normalize_numbers))
                .collect();
            format!("[{}]", entries.join(","))
        }
        Value::Number(number) if normalize_numbers => canonical_number(number),
        _ => serde_json::to_string(value).unwrap_or_default(),
    }
}

/// JSONB expands exponents and discards negative zero. Normalize the decimal
/// spelling without converting integers to f64, which would lose precision.
fn canonical_number(number: &serde_json::Number) -> String {
    let text = number.to_string();
    let (sign, unsigned) = text
        .strip_prefix('-')
        .map_or(("", text.as_str()), |value| ("-", value));
    let (mantissa, exponent) =
        unsigned
            .split_once(['e', 'E'])
            .map_or((unsigned, 0), |(mantissa, exponent)| {
                (
                    mantissa,
                    exponent.parse::<i32>().expect("JSON number exponent"),
                )
            });
    let fractional_digits = mantissa
        .split_once('.')
        .map_or(0, |(_, fraction)| fraction.len()) as i32;
    let digits = mantissa.replace('.', "");
    let significant = digits.trim_start_matches('0');
    if significant.is_empty() {
        return "0".into();
    }
    let trimmed = significant.trim_end_matches('0');
    let exponent = exponent - fractional_digits + (significant.len() - trimmed.len()) as i32;
    format!("{sign}{trimmed}e{exponent}")
}

/// Reproduce the legacy blake3 hash for existing audit entries.
///
/// The hash covers: sequence, timestamp, actor_id, action, resource_type,
/// resource_id, details (canonical JSON or empty), prev_hash, and prev_signature.
/// This legacy format omits metadata and concatenates fields without framing.
/// New entries must use [`compute_entry_hash_v2`].
#[allow(clippy::too_many_arguments)]
pub fn compute_entry_hash(
    sequence: i64,
    timestamp: &DateTime<FixedOffset>,
    actor_id: &str,
    action: &str,
    resource_type: &str,
    resource_id: &str,
    details: Option<&Value>,
    prev_hash: &str,
    prev_signature: Option<&str>,
) -> String {
    let mut hasher = Hasher::new();
    hasher.update(&sequence.to_le_bytes());
    hasher.update(timestamp.timestamp_millis().to_le_bytes().as_ref());
    hasher.update(actor_id.as_bytes());
    hasher.update(action.as_bytes());
    hasher.update(resource_type.as_bytes());
    hasher.update(resource_id.as_bytes());
    match details {
        Some(v) => {
            hasher.update(canonical_json(v).as_bytes());
        }
        None => {
            hasher.update(b"null");
        }
    }
    hasher.update(prev_hash.as_bytes());
    // Preserve the original encoding to keep historical entries verifiable.
    match prev_signature {
        Some(sig) => hasher.update(sig.as_bytes()),
        None => hasher.update(b"none"),
    };
    hasher.finalize().to_hex().to_string()
}

/// One audit entry as fed to [`verify_chain`]:
/// (seq, ts, actor, action, resource_type, resource_id, details, prev_hash, entry_hash, prev_signature)
pub type ChainEntryRow = (
    i64,
    DateTime<FixedOffset>,
    String,
    String,
    String,
    String,
    Option<Value>,
    String,
    String,
    Option<String>,
);

/// Verify a segment of a hash chain. Returns the index of the first broken entry,
/// or None if the chain is valid.
///
/// `entries` must be sorted by sequence ascending within the same chain.
pub fn verify_chain(entries: &[ChainEntryRow], initial_prev_hash: &str) -> Option<usize> {
    let mut expected_prev = initial_prev_hash.to_string();
    for (i, (seq, ts, actor, action, rtype, rid, details, prev_hash, entry_hash, prev_sig)) in
        entries.iter().enumerate()
    {
        if *seq < 1 || (i > 0 && entries[i - 1].0.checked_add(1) != Some(*seq)) {
            return Some(i);
        }
        if *prev_hash != expected_prev {
            return Some(i);
        }
        let computed = compute_entry_hash(
            *seq,
            ts,
            actor,
            action,
            rtype,
            rid,
            details.as_ref(),
            prev_hash,
            prev_sig.as_deref(),
        );
        if computed != *entry_hash {
            return Some(i);
        }
        expected_prev = entry_hash.clone();
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDateTime;

    #[test]
    fn test_genesis_chain() {
        let ts = NaiveDateTime::parse_from_str("2026-01-01 00:00:00", "%Y-%m-%d %H:%M:%S")
            .unwrap()
            .and_utc()
            .fixed_offset();
        let hash1 = compute_entry_hash(
            1,
            &ts,
            "user1",
            "app.create",
            "App",
            "app1",
            None,
            GENESIS_HASH,
            None,
        );
        assert!(!hash1.is_empty());
        assert_ne!(hash1, GENESIS_HASH);

        let hash2 = compute_entry_hash(
            2,
            &ts,
            "user1",
            "app.update",
            "App",
            "app1",
            None,
            &hash1,
            Some("sig1"),
        );
        assert_ne!(hash1, hash2);
    }

    #[test]
    fn test_chain_verification() {
        let ts = NaiveDateTime::parse_from_str("2026-01-01 00:00:00", "%Y-%m-%d %H:%M:%S")
            .unwrap()
            .and_utc()
            .fixed_offset();

        let h1 = compute_entry_hash(
            1,
            &ts,
            "u1",
            "app.create",
            "App",
            "a1",
            None,
            GENESIS_HASH,
            None,
        );
        let h2 = compute_entry_hash(
            2,
            &ts,
            "u1",
            "app.update",
            "App",
            "a1",
            None,
            &h1,
            Some("sig_for_h1"),
        );

        let entries = vec![
            (
                1,
                ts,
                "u1".to_string(),
                "app.create".to_string(),
                "App".to_string(),
                "a1".to_string(),
                None,
                GENESIS_HASH.to_string(),
                h1.clone(),
                None,
            ),
            (
                2,
                ts,
                "u1".to_string(),
                "app.update".to_string(),
                "App".to_string(),
                "a1".to_string(),
                None,
                h1,
                h2,
                Some("sig_for_h1".to_string()),
            ),
        ];

        assert_eq!(verify_chain(&entries, GENESIS_HASH), None);
    }

    #[test]
    fn test_tampered_chain() {
        let ts = NaiveDateTime::parse_from_str("2026-01-01 00:00:00", "%Y-%m-%d %H:%M:%S")
            .unwrap()
            .and_utc()
            .fixed_offset();

        let h1 = compute_entry_hash(
            1,
            &ts,
            "u1",
            "app.create",
            "App",
            "a1",
            None,
            GENESIS_HASH,
            None,
        );
        let _h2 = compute_entry_hash(
            2,
            &ts,
            "u1",
            "app.update",
            "App",
            "a1",
            None,
            &h1,
            Some("sig1"),
        );

        let entries = vec![
            (
                1,
                ts,
                "u1".to_string(),
                "app.create".to_string(),
                "App".to_string(),
                "a1".to_string(),
                None,
                GENESIS_HASH.to_string(),
                h1.clone(),
                None,
            ),
            (
                2,
                ts,
                "u1".to_string(),
                "app.update".to_string(),
                "App".to_string(),
                "a1".to_string(),
                None,
                h1,
                "tampered_hash".to_string(),
                Some("sig1".to_string()),
            ),
        ];

        assert_eq!(verify_chain(&entries, GENESIS_HASH), Some(1));
    }

    #[test]
    fn test_canonical_json_key_ordering() {
        let v1: Value = serde_json::json!({"b": 1, "a": 2});
        let v2: Value = serde_json::json!({"a": 2, "b": 1});
        assert_eq!(canonical_json(&v1), canonical_json(&v2));
    }

    #[test]
    fn v2_numbers_survive_jsonb_decimal_normalization() {
        for (before, after) in [
            ("-0.0", "0.0"),
            ("1.0", "1"),
            ("1e18", "1000000000000000000"),
            ("1e-7", "0.0000001"),
            ("1.2345678901234567e30", "1234567890123456700000000000000"),
        ] {
            let before: Value = serde_json::from_str(before).unwrap();
            let after: Value = serde_json::from_str(after).unwrap();
            assert_eq!(
                canonical_json_with_numbers(&before, true),
                canonical_json_with_numbers(&after, true)
            );
        }
        let exact: Value = serde_json::from_str("9007199254740993").unwrap();
        let rounded: Value = serde_json::from_str("9007199254740992").unwrap();
        assert_ne!(
            canonical_json_with_numbers(&exact, true),
            canonical_json_with_numbers(&rounded, true)
        );
        assert_ne!(
            canonical_json(&serde_json::json!(-0.0)),
            canonical_json(&serde_json::json!(0.0))
        );
    }
}
