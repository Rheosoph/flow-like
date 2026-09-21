//! The entry key: a 32-byte secret the API uses to MAC pending records and the audit
//! worker uses to check them before sealing. It never signs anything long-lived; once a
//! record is sealed its MAC is cleared and the epoch signature takes over.

use flow_like_types::base64::{Engine, engine::general_purpose::STANDARD};
use std::sync::OnceLock;

use super::crypto::{Hash, random_salt};

const BACKEND_DERIVE_CONTEXT: &str = "flow-like 2026-09 audit entry key v1";

static ENTRY_KEY: OnceLock<Hash> = OnceLock::new();
static PREVIOUS_ENTRY_KEY: OnceLock<Option<Hash>> = OnceLock::new();

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
    if source == EntryKeySource::Ephemeral {
        tracing::warn!(
            "AUDIT_ENTRY_KEY and BACKEND_KEY are unset: pending audit records use a per-process \
             key and can only be sealed by an audit worker running in this process"
        );
    }
    Ok(source)
}

/// `AUDIT_ENTRY_KEY_PREVIOUS`: during a rotation, pending records and seals not yet
/// signed that carry a MAC made with the old key still verify. New MACs always use the
/// current key.
pub fn init_previous_entry_key(previous_b64: Option<&str>) -> flow_like_types::Result<()> {
    let previous = previous_b64
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| decode_key("AUDIT_ENTRY_KEY_PREVIOUS", value))
        .transpose()?;
    if PREVIOUS_ENTRY_KEY.set(previous).is_err() {
        tracing::warn!("previous audit entry key already initialized; keeping the first");
    }
    Ok(())
}

/// Keys a MAC may have been made with: the current entry key, then the previous one.
pub fn accepted_entry_keys() -> Vec<Hash> {
    let mut keys = vec![*entry_key()];
    keys.extend(PREVIOUS_ENTRY_KEY.get().copied().flatten());
    keys
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
    if let Some(value) = explicit_b64
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        return Ok((
            decode_key("AUDIT_ENTRY_KEY", value)?,
            EntryKeySource::Explicit,
        ));
    }
    if let Some(value) = backend_key_b64
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        return Ok((
            blake3::derive_key(BACKEND_DERIVE_CONTEXT, value.as_bytes()),
            EntryKeySource::Derived,
        ));
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
        assert_ne!(first, resolve(None, Some("other-key")).unwrap().0);
    }

    #[test]
    fn nothing_configured_is_ephemeral() {
        let (first, source) = resolve(None, None).unwrap();
        let (second, _) = resolve(None, Some("")).unwrap();
        assert_eq!(source, EntryKeySource::Ephemeral);
        assert_ne!(first, second);
    }
}
