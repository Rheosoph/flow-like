use flow_like_types::base64::Engine;
use flow_like_types::base64::engine::general_purpose::STANDARD;
use p256::ecdsa::{Signature, SigningKey, VerifyingKey, signature::Signer, signature::Verifier};
use p256::pkcs8::DecodePrivateKey;
use std::sync::OnceLock;

static SIGNING_KEY: OnceLock<SigningKey> = OnceLock::new();
static VERIFYING_KEY: OnceLock<VerifyingKey> = OnceLock::new();
static KEY_ID: OnceLock<String> = OnceLock::new();

/// Initialize the audit signing keys from pre-resolved secret values.
/// Must be called once during State construction. Subsequent calls are no-ops.
pub fn init(backend_key_b64: Option<&str>, kid: Option<String>) {
    if let Some(b64) = backend_key_b64 {
        if let Ok(pem_bytes) = STANDARD.decode(b64) {
            if let Ok(pem_str) = String::from_utf8(pem_bytes) {
                if let Ok(sk) = SigningKey::from_pkcs8_pem(&pem_str) {
                    let vk = *sk.verifying_key();
                    let _ = SIGNING_KEY.set(sk);
                    let _ = VERIFYING_KEY.set(vk);
                }
            }
        }
    }
    let _ = KEY_ID.set(kid.unwrap_or_else(|| "backend-es256-v1".to_string()));
}

pub fn is_signing_configured() -> bool {
    SIGNING_KEY.get().is_some()
}

pub fn current_kid() -> &'static str {
    KEY_ID.get().map(|s| s.as_str()).unwrap_or("backend-es256-v1")
}

/// Sign an entry hash using raw P-256 ECDSA.
/// Returns the base64-encoded DER signature, or None if signing keys are not configured.
pub fn sign_entry(entry_hash: &str) -> Option<String> {
    let key = SIGNING_KEY.get()?;
    let sig: Signature = key.sign(entry_hash.as_bytes());
    Some(STANDARD.encode(sig.to_der()))
}

/// Verify a raw ECDSA signature against an entry hash.
pub fn verify_entry_signature(entry_hash: &str, signature_b64: &str) -> bool {
    let Some(vk) = VERIFYING_KEY.get() else {
        return false;
    };
    let Ok(sig_bytes) = STANDARD.decode(signature_b64) else {
        return false;
    };
    let Ok(sig) = Signature::from_der(&sig_bytes) else {
        return false;
    };
    vk.verify(entry_hash.as_bytes(), &sig).is_ok()
}
