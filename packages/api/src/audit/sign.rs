use flow_like_types::base64::Engine;
use flow_like_types::base64::engine::general_purpose::STANDARD;
use p256::ecdsa::{Signature, SigningKey, VerifyingKey, signature::Signer, signature::Verifier};
use p256::pkcs8::DecodePrivateKey;
use std::sync::LazyLock;

static SIGNING_KEY: LazyLock<Option<SigningKey>> = LazyLock::new(|| {
    let b64 = std::env::var("BACKEND_KEY").ok()?;
    let pem_bytes = STANDARD.decode(&b64).ok()?;
    let pem_str = String::from_utf8(pem_bytes).ok()?;
    SigningKey::from_pkcs8_pem(&pem_str).ok()
});

static VERIFYING_KEY: LazyLock<Option<VerifyingKey>> =
    LazyLock::new(|| SIGNING_KEY.as_ref().map(|sk| *sk.verifying_key()));

static KEY_ID: LazyLock<String> = LazyLock::new(|| {
    std::env::var("BACKEND_KID").unwrap_or_else(|_| "backend-es256-v1".to_string())
});

pub fn is_signing_configured() -> bool {
    SIGNING_KEY.is_some()
}

pub fn current_kid() -> &'static str {
    &KEY_ID
}

/// Sign an entry hash using raw P-256 ECDSA.
/// Returns the base64-encoded DER signature, or None if signing keys are not configured.
pub fn sign_entry(entry_hash: &str) -> Option<String> {
    let key = SIGNING_KEY.as_ref()?;
    let sig: Signature = key.sign(entry_hash.as_bytes());
    Some(STANDARD.encode(sig.to_der()))
}

/// Verify a raw ECDSA signature against an entry hash.
pub fn verify_entry_signature(entry_hash: &str, signature_b64: &str) -> bool {
    let Some(vk) = VERIFYING_KEY.as_ref() else {
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
