use blake3::Hasher;
use chrono::NaiveDateTime;
use flow_like_types::Value;
use std::collections::BTreeMap;

/// Genesis hash — used as prev_hash for the first entry in any chain
pub const GENESIS_HASH: &str = "0000000000000000000000000000000000000000000000000000000000000000";

/// Produce a canonical (sorted-key) JSON string from a serde_json Value.
/// This ensures deterministic hashing regardless of key insertion order (RFC 8785 subset).
fn canonical_json(value: &Value) -> String {
    match value {
        Value::Object(map) => {
            let sorted: BTreeMap<_, _> = map.iter().collect();
            let entries: Vec<String> = sorted
                .into_iter()
                .map(|(k, v)| {
                    format!(
                        "{}:{}",
                        serde_json::to_string(k).unwrap_or_default(),
                        canonical_json(v)
                    )
                })
                .collect();
            format!("{{{}}}", entries.join(","))
        }
        Value::Array(arr) => {
            let entries: Vec<String> = arr.iter().map(canonical_json).collect();
            format!("[{}]", entries.join(","))
        }
        _ => serde_json::to_string(value).unwrap_or_default(),
    }
}

/// Compute the blake3 hash for an audit entry.
///
/// The hash covers: sequence, timestamp, actor_id, action, resource_type,
/// resource_id, details (canonical JSON or empty), prev_hash, and prev_signature.
/// This forms a tamper-evident chain — altering any field breaks the chain.
pub fn compute_entry_hash(
    sequence: i64,
    timestamp: &NaiveDateTime,
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
    hasher.update(
        timestamp
            .and_utc()
            .timestamp_millis()
            .to_le_bytes()
            .as_ref(),
    );
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
    // Including prev_signature in the hash means a key compromise cannot forge entries
    // without also re-signing the entire chain from the rotation point.
    match prev_signature {
        Some(sig) => hasher.update(sig.as_bytes()),
        None => hasher.update(b"none"),
    };
    hasher.finalize().to_hex().to_string()
}

/// Verify a segment of a hash chain. Returns the index of the first broken entry,
/// or None if the chain is valid.
///
/// `entries` must be sorted by sequence ascending within the same chain.
/// Tuple fields: (seq, ts, actor, action, rtype, rid, details, prev_hash, entry_hash, prev_signature)
pub fn verify_chain(
    entries: &[(
        i64,
        NaiveDateTime,
        String,
        String,
        String,
        String,
        Option<Value>,
        String,
        String,
        Option<String>,
    )],
    initial_prev_hash: &str,
) -> Option<usize> {
    let mut expected_prev = initial_prev_hash.to_string();
    for (i, (seq, ts, actor, action, rtype, rid, details, prev_hash, entry_hash, prev_sig)) in
        entries.iter().enumerate()
    {
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
        let ts = NaiveDateTime::parse_from_str("2026-01-01 00:00:00", "%Y-%m-%d %H:%M:%S").unwrap();
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
        let ts = NaiveDateTime::parse_from_str("2026-01-01 00:00:00", "%Y-%m-%d %H:%M:%S").unwrap();

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
        let ts = NaiveDateTime::parse_from_str("2026-01-01 00:00:00", "%Y-%m-%d %H:%M:%S").unwrap();

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
}
