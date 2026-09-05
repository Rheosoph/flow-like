use flow_like_types::base64::Engine;
use flow_like_types::base64::engine::general_purpose::STANDARD;
use p256::ecdsa::{Signature, SigningKey, VerifyingKey, signature::Signer, signature::Verifier};
use p256::pkcs8::{DecodePrivateKey, DecodePublicKey};
use std::{collections::BTreeMap, sync::OnceLock};

struct SigningConfig {
    key: SigningKey,
    kid: String,
}

static SIGNING_CONFIG: OnceLock<SigningConfig> = OnceLock::new();
static VERIFYING_KEYS: OnceLock<BTreeMap<String, VerifyingKey>> = OnceLock::new();

/// Retain public keys for earlier signing key ids. The secret-store value is a
/// JSON object mapping each key id to its SPKI PEM public key. Private keys are
/// neither needed nor accepted here.
pub fn init_verifying_keys(keys_json: Option<&str>) -> flow_like_types::Result<()> {
    let Some(keys_json) = keys_json else {
        return Ok(());
    };
    let keys = parse_verifying_keys(keys_json)?;
    if let Some(config) = SIGNING_CONFIG.get()
        && let Some(key) = keys.get(&config.kid)
        && key != config.key.verifying_key()
    {
        return Err(flow_like_types::anyhow!(
            "audit verification key conflicts with current signing key id"
        ));
    }
    if let Some(existing) = VERIFYING_KEYS.get() {
        if existing != &keys {
            return Err(flow_like_types::anyhow!(
                "audit verification keys already initialized with different keys"
            ));
        }
        return Ok(());
    }
    VERIFYING_KEYS
        .set(keys)
        .map_err(|_| flow_like_types::anyhow!("audit verification keys initialized concurrently"))
}

fn parse_verifying_keys(
    keys_json: &str,
) -> flow_like_types::Result<BTreeMap<String, VerifyingKey>> {
    let values: BTreeMap<String, String> = serde_json::from_str(keys_json)?;
    values
        .into_iter()
        .map(|(kid, pem)| {
            if kid.trim().is_empty() {
                return Err(flow_like_types::anyhow!(
                    "audit verification key id must not be empty"
                ));
            }
            let key = VerifyingKey::from_public_key_pem(&pem).map_err(|_| {
                flow_like_types::anyhow!(
                    "audit verification keys must be P-256 SPKI PEM public keys"
                )
            })?;
            Ok((kid, key))
        })
        .collect()
}

/// Initialize the audit signing keys from pre-resolved secret values.
/// Must be called once during State construction. Subsequent calls are no-ops.
pub fn init(backend_key_b64: Option<&str>, kid: Option<String>) {
    match backend_key_b64 {
        None => {
            tracing::warn!("audit::sign::init called with no BACKEND_KEY value");
        }
        Some(b64) => match STANDARD.decode(b64) {
            Err(e) => {
                tracing::error!("BACKEND_KEY base64 decode failed: {e}");
            }
            Ok(pem_bytes) => match String::from_utf8(pem_bytes) {
                Err(e) => {
                    tracing::error!("BACKEND_KEY is not valid UTF-8 after decode: {e}");
                }
                Ok(pem_str) => match SigningKey::from_pkcs8_pem(&pem_str) {
                    Err(e) => {
                        tracing::error!("BACKEND_KEY PKCS#8 parse failed: {e}");
                    }
                    Ok(sk) => {
                        let _ = SIGNING_CONFIG.set(SigningConfig {
                            key: sk,
                            kid: kid.unwrap_or_else(|| "backend-es256-v1".to_string()),
                        });
                        tracing::info!("Audit signing key initialized successfully");
                    }
                },
            },
        },
    }
}

pub fn is_signing_configured() -> bool {
    SIGNING_CONFIG.get().is_some()
}

pub fn current_kid() -> &'static str {
    SIGNING_CONFIG
        .get()
        .map(|config| config.kid.as_str())
        .unwrap_or("backend-es256-v1")
}

/// Sign an entry hash using raw P-256 ECDSA.
/// Returns the base64-encoded DER signature, or None if signing keys are not configured.
pub fn sign_entry(entry_hash: &str) -> Option<String> {
    let config = SIGNING_CONFIG.get()?;
    let sig: Signature = config.key.sign(entry_hash.as_bytes());
    Some(STANDARD.encode(sig.to_der()))
}

/// Verify a raw ECDSA signature against an entry hash.
pub fn verify_entry_signature(entry_hash: &str, signature_b64: &str) -> bool {
    let Some(config) = SIGNING_CONFIG.get() else {
        return false;
    };
    verify_with_key(config.key.verifying_key(), entry_hash, signature_b64)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SignatureVerification {
    Valid,
    Invalid,
    Unavailable,
}

/// Historical key ids must never be checked with the current key or silently
/// accepted. A retained verification key is required to authenticate them.
pub(crate) fn verify_entry_signature_for_kid(
    entry_hash: &str,
    signature_b64: &str,
    kid: &str,
) -> SignatureVerification {
    let key = SIGNING_CONFIG
        .get()
        .filter(|config| config.kid == kid)
        .map(|config| config.key.verifying_key())
        .or_else(|| VERIFYING_KEYS.get().and_then(|keys| keys.get(kid)));
    let Some(key) = key else {
        return SignatureVerification::Unavailable;
    };
    if verify_with_key(key, entry_hash, signature_b64) {
        SignatureVerification::Valid
    } else {
        SignatureVerification::Invalid
    }
}

fn verify_with_key(vk: &VerifyingKey, entry_hash: &str, signature_b64: &str) -> bool {
    let Ok(sig_bytes) = STANDARD.decode(signature_b64) else {
        return false;
    };
    let Ok(sig) = Signature::from_der(&sig_bytes) else {
        return false;
    };
    vk.verify(entry_hash.as_bytes(), &sig).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use p256::pkcs8::{EncodePublicKey, LineEnding};

    #[test]
    fn historical_registry_accepts_only_named_p256_public_keys() {
        let key = SigningKey::from_slice(&[3; 32]).unwrap();
        let pem = key
            .verifying_key()
            .to_public_key_pem(LineEnding::LF)
            .unwrap();
        let registry = serde_json::json!({"previous-key": pem});
        let keys = parse_verifying_keys(&registry.to_string()).unwrap();
        assert_eq!(&keys["previous-key"], key.verifying_key());
        assert!(parse_verifying_keys("[]").is_err());
        assert!(parse_verifying_keys(r#"{"old":"invalid"}"#).is_err());
        assert!(parse_verifying_keys(&serde_json::json!({" ": pem}).to_string()).is_err());
    }

    #[test]
    fn signatures_reject_tampering_wrong_keys_and_malformed_encoding() {
        let key = SigningKey::from_slice(&[1; 32]).unwrap();
        let other_key = SigningKey::from_slice(&[2; 32]).unwrap();
        let sig: Signature = key.sign(b"v2:entry");
        let encoded = STANDARD.encode(sig.to_der());
        assert!(verify_with_key(key.verifying_key(), "v2:entry", &encoded));
        assert!(!verify_with_key(
            key.verifying_key(),
            "v2:tampered",
            &encoded
        ));
        assert!(!verify_with_key(
            other_key.verifying_key(),
            "v2:entry",
            &encoded
        ));
        assert!(!verify_with_key(
            key.verifying_key(),
            "v2:entry",
            "not base64"
        ));
        assert!(!verify_with_key(
            key.verifying_key(),
            "v2:entry",
            &STANDARD.encode([1, 2, 3])
        ));
    }

    #[test]
    fn an_unknown_key_is_unverifiable() {
        assert_eq!(
            verify_entry_signature_for_kid("hash", "signature", "unknown-test-key"),
            SignatureVerification::Unavailable,
        );
    }
}
