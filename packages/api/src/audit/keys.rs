//! The entry key: a 32-byte secret the API uses to MAC pending records and the audit
//! worker uses to check them before sealing. It never signs anything long-lived; once a
//! record is sealed its MAC is cleared and the epoch signature takes over.
//!
//! Each record carries the kid of the key that made its MAC, so a worker holding a
//! different set of keys can tell a rotation it has not caught up with from tampering.

use flow_like_types::base64::{Engine, engine::general_purpose::STANDARD};
use std::sync::OnceLock;

use super::crypto::{Hash, random_salt};

const BACKEND_DERIVE_CONTEXT: &str = "flow-like 2026-09 audit entry key v1";
const KID_DERIVE_CONTEXT: &str = "flow-like 2026-09 audit entry kid v1";

static ENTRY_KEY: OnceLock<Hash> = OnceLock::new();
/// The key derived from `BACKEND_KEY` while the current key is explicit, so records
/// written by a process that only holds `BACKEND_KEY` still verify.
static BACKEND_ENTRY_KEY: OnceLock<Option<Hash>> = OnceLock::new();
static PREVIOUS_ENTRY_KEY: OnceLock<Option<Hash>> = OnceLock::new();
static PREVIOUS_BACKEND_ENTRY_KEY: OnceLock<Option<Hash>> = OnceLock::new();

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EntryKeySource {
    /// `AUDIT_ENTRY_KEY`, base64 of 32 random bytes.
    Explicit,
    /// Derived from `BACKEND_KEY`; every process holding it derives the same key.
    Derived,
    /// Random per process. Only a worker in the same process can seal these records.
    Ephemeral,
}

/// Initialize once during state construction. Later calls keep the first key.
pub fn init_entry_key(
    explicit_b64: Option<&str>,
    backend_key_b64: Option<&str>,
) -> flow_like_types::Result<EntryKeySource> {
    let (key, source) = resolve(explicit_b64, backend_key_b64)?;
    if ENTRY_KEY.set(key).is_err() && ENTRY_KEY.get() != Some(&key) {
        tracing::warn!("audit entry key already initialized; keeping the first key");
    }
    let backend = (source == EntryKeySource::Explicit)
        .then(|| non_empty(backend_key_b64).map(derive_entry_key))
        .flatten();
    let _ = BACKEND_ENTRY_KEY.set(backend);
    if source == EntryKeySource::Ephemeral {
        tracing::warn!(
            "AUDIT_ENTRY_KEY and BACKEND_KEY are unset: pending audit records use a per-process \
             key and can only be sealed by an audit worker running in this process"
        );
    }
    Ok(source)
}

/// `AUDIT_ENTRY_KEY_PREVIOUS` and `BACKEND_KEY_PREVIOUS`: during a rotation, pending
/// records and seals not yet signed that carry a MAC made with the old explicit key, or
/// with the key derived from the old backend key, still verify. New MACs always use the
/// current key.
pub fn init_previous_entry_key(
    previous_b64: Option<&str>,
    backend_key_previous_b64: Option<&str>,
) -> flow_like_types::Result<()> {
    let previous = non_empty(previous_b64)
        .map(|value| decode_key("AUDIT_ENTRY_KEY_PREVIOUS", value))
        .transpose()?;
    if PREVIOUS_ENTRY_KEY.set(previous).is_err() {
        tracing::warn!("previous audit entry key already initialized; keeping the first");
    }
    let _ =
        PREVIOUS_BACKEND_ENTRY_KEY.set(non_empty(backend_key_previous_b64).map(derive_entry_key));
    Ok(())
}

/// Keys a MAC may have been made with, each with its kid, current key first: the
/// current key, `AUDIT_ENTRY_KEY_PREVIOUS`, the key derived from `BACKEND_KEY_PREVIOUS`
/// and, while the current key is explicit, the key derived from `BACKEND_KEY`.
pub fn accepted_entry_keys() -> Vec<(String, Hash)> {
    union(
        *entry_key(),
        [
            &PREVIOUS_ENTRY_KEY,
            &PREVIOUS_BACKEND_ENTRY_KEY,
            &BACKEND_ENTRY_KEY,
        ]
        .map(|slot| slot.get().copied().flatten()),
    )
}

fn union(current: Hash, extras: [Option<Hash>; 3]) -> Vec<(String, Hash)> {
    let mut keys = vec![current];
    for key in extras.into_iter().flatten() {
        if !keys.contains(&key) {
            keys.push(key);
        }
    }
    keys.into_iter().map(|key| (entry_kid(&key), key)).collect()
}

/// Public identifier of an entry key, stored on every record so the worker knows which
/// key made its MAC. Derived through a separate context, so it reveals nothing about
/// the key.
pub fn entry_kid(key: &Hash) -> String {
    hex::encode(&blake3::derive_key(KID_DERIVE_CONTEXT, key)[..8])
}

/// The entry key every process holding `backend_key` derives from it.
pub fn derive_entry_key(backend_key: &str) -> Hash {
    blake3::derive_key(BACKEND_DERIVE_CONTEXT, backend_key.as_bytes())
}

fn non_empty(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|value| !value.is_empty())
}

fn decode_key(name: &str, value: &str) -> flow_like_types::Result<Hash> {
    let bytes = STANDARD
        .decode(value)
        .map_err(|_| flow_like_types::anyhow!("{name} is not valid base64"))?;
    <[u8; 32]>::try_from(bytes.as_slice())
        .map_err(|_| flow_like_types::anyhow!("{name} must decode to 32 bytes"))
}

fn resolve(
    explicit_b64: Option<&str>,
    backend_key_b64: Option<&str>,
) -> flow_like_types::Result<(Hash, EntryKeySource)> {
    if let Some(value) = non_empty(explicit_b64) {
        return Ok((
            decode_key("AUDIT_ENTRY_KEY", value)?,
            EntryKeySource::Explicit,
        ));
    }
    if let Some(value) = non_empty(backend_key_b64) {
        return Ok((derive_entry_key(value), EntryKeySource::Derived));
    }
    Ok((random_salt(), EntryKeySource::Ephemeral))
}

/// The process entry key, falling back to an ephemeral key when state construction
/// never ran (unit tests, tools).
pub fn entry_key() -> &'static Hash {
    ENTRY_KEY.get_or_init(random_salt)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_key_wins_and_must_be_32_bytes() {
        let explicit = STANDARD.encode([4u8; 32]);
        let (key, source) = resolve(Some(&explicit), Some("backend")).unwrap();
        assert_eq!((key, source), ([4; 32], EntryKeySource::Explicit));
        assert!(resolve(Some(&STANDARD.encode([4u8; 16])), None).is_err());
        assert!(resolve(Some("not base64!"), None).is_err());
    }

    #[test]
    fn backend_key_derivation_is_stable_and_distinct_from_the_input() {
        let (first, source) = resolve(None, Some("backend-key")).unwrap();
        let (second, _) = resolve(Some("  "), Some("backend-key")).unwrap();
        assert_eq!(source, EntryKeySource::Derived);
        assert_eq!(first, second);
        assert_eq!(first, derive_entry_key("backend-key"));
        assert_ne!(first, resolve(None, Some("other-key")).unwrap().0);
    }

    #[test]
    fn nothing_configured_is_ephemeral() {
        let (first, source) = resolve(None, None).unwrap();
        let (second, _) = resolve(None, Some("")).unwrap();
        assert_eq!(source, EntryKeySource::Ephemeral);
        assert_ne!(first, second);
    }

    #[test]
    fn kids_are_short_stable_and_reveal_nothing_of_the_key() {
        let kid = entry_kid(&[4; 32]);
        assert_eq!(kid.len(), 16);
        assert!(kid.bytes().all(|byte| byte.is_ascii_hexdigit()));
        assert_eq!(kid, entry_kid(&[4; 32]));
        assert_ne!(kid, entry_kid(&[5; 32]));
        assert_ne!(kid, hex::encode(&[4u8; 8]));
        assert_ne!(
            kid,
            hex::encode(&blake3::derive_key(BACKEND_DERIVE_CONTEXT, &[4; 32])[..8])
        );
    }

    #[test]
    fn accepted_keys_start_with_the_current_key_and_drop_duplicates() {
        let current = [1; 32];
        let previous = [2; 32];
        let derived = [3; 32];
        let keys = union(current, [Some(previous), None, Some(derived)]);
        assert_eq!(
            keys,
            vec![
                (entry_kid(&current), current),
                (entry_kid(&previous), previous),
                (entry_kid(&derived), derived),
            ]
        );
        let same = union(current, [Some(current), Some(previous), Some(current)]);
        assert_eq!(same.len(), 2);
        assert_eq!(union(current, [None, None, None]).len(), 1);
    }
}
