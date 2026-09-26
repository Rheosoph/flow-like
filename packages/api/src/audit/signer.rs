//! The audit key signs epochs, watermarks and archive manifests. Only the audit worker
//! holds it: locally as a PKCS#8 key, or in a key management service. Verifiers need
//! only public keys, registered per key id.

use flow_like_types::async_trait;
use flow_like_types::base64::{Engine, engine::general_purpose::STANDARD};
use p256::ecdsa::{Signature, SigningKey, VerifyingKey, signature::Signer, signature::Verifier};
use p256::pkcs8::{DecodePrivateKey, DecodePublicKey, EncodePublicKey, LineEnding};
use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Mutex, PoisonError, RwLock};
use std::time::{Duration, Instant};

use super::crypto::Hash;

/// Raw P-256 ECDSA signature, `r || s`, over SHA-256 of the 32-byte digest.
pub type RawSignature = [u8; 64];

#[async_trait]
pub trait AuditSigner: Send + Sync {
    fn kid(&self) -> &str;
    /// Public key, registered for verification in the same process.
    fn verifying_key(&self) -> VerifyingKey;
    async fn sign(&self, digest: &Hash) -> flow_like_types::Result<RawSignature>;
    /// Signatures still allowed in the current budget window; `None` when unlimited.
    fn remaining_budget(&self) -> Option<usize> {
        None
    }
}

/// A P-256 key held in process memory, from the `AUDIT_SIGNING_KEY` secret.
pub struct LocalSigner {
    key: SigningKey,
    kid: String,
}

impl LocalSigner {
    /// `pem_b64` is base64 of a PKCS#8 PEM P-256 private key. Without an explicit id the
    /// key's fingerprint names it, so a rotated key can never reuse an id.
    pub fn from_pem_b64(pem_b64: &str, kid: Option<String>) -> flow_like_types::Result<Self> {
        let pem = STANDARD
            .decode(pem_b64.trim())
            .map_err(|_| flow_like_types::anyhow!("audit signing key is not valid base64"))?;
        let pem = String::from_utf8(pem)
            .map_err(|_| flow_like_types::anyhow!("audit signing key is not UTF-8 PEM"))?;
        let key = SigningKey::from_pkcs8_pem(&pem).map_err(|_| {
            flow_like_types::anyhow!("audit signing key must be a PKCS#8 P-256 private key")
        })?;
        Ok(Self::new(key, kid))
    }

    pub fn new(key: SigningKey, kid: Option<String>) -> Self {
        let kid = kid
            .map(|kid| kid.trim().to_owned())
            .filter(|kid| !kid.is_empty())
            .unwrap_or_else(|| fingerprint_kid(key.verifying_key()));
        Self { key, kid }
    }
}

#[async_trait]
impl AuditSigner for LocalSigner {
    fn kid(&self) -> &str {
        &self.kid
    }

    fn verifying_key(&self) -> VerifyingKey {
        *self.key.verifying_key()
    }

    async fn sign(&self, digest: &Hash) -> flow_like_types::Result<RawSignature> {
        let signature: Signature = self.key.sign(digest);
        Ok(signature.to_bytes().into())
    }
}

pub fn fingerprint_kid(key: &VerifyingKey) -> String {
    let point = key.to_encoded_point(true);
    let fingerprint = blake3::hash(point.as_bytes()).to_hex();
    format!("audit-es256-{}", &fingerprint[..16])
}

pub fn public_key_pem(key: &VerifyingKey) -> String {
    key.to_public_key_pem(LineEnding::LF).unwrap_or_default()
}

/// Normalize a DER signature from a key management service to raw low-S form.
pub fn raw_from_der(der: &[u8]) -> flow_like_types::Result<RawSignature> {
    let signature = Signature::from_der(der)
        .map_err(|_| flow_like_types::anyhow!("key service returned an invalid ECDSA signature"))?;
    let signature = signature.normalize_s().unwrap_or(signature);
    Ok(signature.to_bytes().into())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SignatureCheck {
    Valid,
    Invalid,
    /// No public key is registered for the key id. Not proof of tampering.
    Unavailable,
}

static VERIFYING_KEYS: RwLock<BTreeMap<String, VerifyingKey>> = RwLock::new(BTreeMap::new());

/// Register public keys from the `AUDIT_VERIFYING_KEYS` secret: a JSON object mapping key
/// ids to SPKI PEM public keys. A key id may never map to two different keys.
pub fn register_verifying_keys_json(json: &str) -> flow_like_types::Result<()> {
    let values: BTreeMap<String, String> = serde_json::from_str(json).map_err(|_| {
        flow_like_types::anyhow!("AUDIT_VERIFYING_KEYS must be a JSON object of key id to PEM")
    })?;
    for (kid, pem) in values {
        if kid.trim().is_empty() {
            return Err(flow_like_types::anyhow!(
                "audit verification key id must not be empty"
            ));
        }
        let key = VerifyingKey::from_public_key_pem(&pem).map_err(|_| {
            flow_like_types::anyhow!("audit verification key {kid} is not a P-256 SPKI PEM key")
        })?;
        register_verifying_key(&kid, key)?;
    }
    Ok(())
}

pub fn register_verifying_key(kid: &str, key: VerifyingKey) -> flow_like_types::Result<()> {
    let mut keys = VERIFYING_KEYS
        .write()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    match keys.get(kid) {
        Some(existing) if existing != &key => Err(flow_like_types::anyhow!(
            "audit key id {kid} is already registered with a different public key"
        )),
        Some(_) => Ok(()),
        None => {
            keys.insert(kid.to_owned(), key);
            Ok(())
        }
    }
}

/// Public key registered for `kid`, so a key-service signer whose key is already known
/// from `AUDIT_VERIFYING_KEYS` needs no request to fetch it.
pub fn registered_key(kid: &str) -> Option<VerifyingKey> {
    VERIFYING_KEYS
        .read()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .get(kid)
        .copied()
}

/// Key ids with a registered public key.
pub fn verifying_kids() -> Vec<String> {
    VERIFYING_KEYS
        .read()
        .unwrap_or_else(PoisonError::into_inner)
        .keys()
        .cloned()
        .collect()
}

pub fn verify(kid: &str, digest: &Hash, signature: &[u8]) -> SignatureCheck {
    let Some(key) = registered_key(kid) else {
        return SignatureCheck::Unavailable;
    };
    let Ok(signature) = Signature::from_slice(signature) else {
        return SignatureCheck::Invalid;
    };
    if key.verify(digest, &signature).is_ok() {
        SignatureCheck::Valid
    } else {
        SignatureCheck::Invalid
    }
}

pub type SharedSigner = Arc<dyn AuditSigner>;

/// Signatures one worker may request per rolling hour. Normal operation needs at most
/// 12 epochs, one watermark batch and rarely a manifest per hour, plus one epoch per
/// 20,000 seals under heavy load. The cap turns a signing loop into a logged error
/// instead of a key-service bill.
pub const MAX_SIGNATURES_PER_HOUR: usize = 120;

/// Wraps the audit key for the worker: enforces the signing budget and checks every
/// signature against the public key before it is stored.
pub struct BudgetedSigner {
    inner: SharedSigner,
    limit: usize,
    window: Duration,
    issued: Mutex<VecDeque<Instant>>,
}

impl BudgetedSigner {
    pub fn new(inner: SharedSigner, limit: usize, window: Duration) -> Self {
        Self {
            inner,
            limit,
            window,
            issued: Mutex::new(VecDeque::new()),
        }
    }

    pub fn hourly(inner: SharedSigner) -> SharedSigner {
        Arc::new(Self::new(
            inner,
            MAX_SIGNATURES_PER_HOUR,
            Duration::from_secs(3600),
        ))
    }

    /// Drop requests that left the window; returns how many remain in it.
    fn in_window(&self, issued: &mut VecDeque<Instant>) -> usize {
        let now = Instant::now();
        while issued
            .front()
            .is_some_and(|at| now.duration_since(*at) >= self.window)
        {
            issued.pop_front();
        }
        issued.len()
    }

    fn reserve(&self) -> flow_like_types::Result<()> {
        let mut issued = self.issued.lock().unwrap_or_else(PoisonError::into_inner);
        if self.in_window(&mut issued) >= self.limit {
            return Err(flow_like_types::anyhow!(
                "audit signing budget of {} signatures per {} s is spent; signing resumes when the window moves",
                self.limit,
                self.window.as_secs()
            ));
        }
        issued.push_back(Instant::now());
        Ok(())
    }
}

#[async_trait]
impl AuditSigner for BudgetedSigner {
    fn kid(&self) -> &str {
        self.inner.kid()
    }

    fn verifying_key(&self) -> VerifyingKey {
        self.inner.verifying_key()
    }

    fn remaining_budget(&self) -> Option<usize> {
        let mut issued = self.issued.lock().unwrap_or_else(PoisonError::into_inner);
        Some(self.limit.saturating_sub(self.in_window(&mut issued)))
    }

    async fn sign(&self, digest: &Hash) -> flow_like_types::Result<RawSignature> {
        self.reserve()?;
        let raw = self.inner.sign(digest).await?;
        let verifies = Signature::from_slice(&raw).is_ok_and(|signature| {
            self.inner
                .verifying_key()
                .verify(digest, &signature)
                .is_ok()
        });
        if !verifies {
            return Err(flow_like_types::anyhow!(
                "audit key {} returned a signature that does not verify with its public key",
                self.inner.kid()
            ));
        }
        Ok(raw)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn signer(seed: u8, kid: Option<&str>) -> LocalSigner {
        LocalSigner::new(
            SigningKey::from_slice(&[seed; 32]).unwrap(),
            kid.map(str::to_owned),
        )
    }

    #[flow_like_types::tokio::test]
    async fn signatures_verify_only_for_their_key_and_digest() {
        let local = signer(11, Some("signer-test-a"));
        register_verifying_key(local.kid(), local.verifying_key()).unwrap();
        let digest = [1; 32];
        let signature = local.sign(&digest).await.unwrap();
        assert_eq!(
            verify("signer-test-a", &digest, &signature),
            SignatureCheck::Valid
        );
        assert_eq!(
            verify("signer-test-a", &[2; 32], &signature),
            SignatureCheck::Invalid
        );
        assert_eq!(
            verify("signer-test-a", &digest, &signature[..63]),
            SignatureCheck::Invalid
        );
        assert_eq!(
            verify("unknown-kid", &digest, &signature),
            SignatureCheck::Unavailable
        );
    }

    #[test]
    fn a_key_id_cannot_be_rebound() {
        let first = signer(12, Some("signer-test-b"));
        let second = signer(13, Some("signer-test-b"));
        register_verifying_key(first.kid(), first.verifying_key()).unwrap();
        register_verifying_key(first.kid(), first.verifying_key()).unwrap();
        assert!(register_verifying_key(second.kid(), second.verifying_key()).is_err());
    }

    #[test]
    fn default_key_id_is_the_fingerprint() {
        let one = signer(14, None);
        let two = signer(15, Some("  "));
        assert!(one.kid().starts_with("audit-es256-"));
        assert_ne!(one.kid(), two.kid());
    }

    #[test]
    fn registry_json_accepts_public_keys_only() {
        let local = signer(16, None);
        let pem = public_key_pem(&local.verifying_key());
        let json = serde_json::json!({ "signer-test-c": pem }).to_string();
        register_verifying_keys_json(&json).unwrap();
        assert!(register_verifying_keys_json("[]").is_err());
        assert!(register_verifying_keys_json(r#"{"signer-test-d":"invalid"}"#).is_err());
        assert!(register_verifying_keys_json(&serde_json::json!({" ": pem}).to_string()).is_err());
    }

    #[flow_like_types::tokio::test]
    async fn budget_caps_signatures_per_window() {
        let budgeted = BudgetedSigner::new(
            Arc::new(signer(18, Some("signer-test-e"))),
            2,
            Duration::from_secs(3600),
        );
        assert_eq!(budgeted.remaining_budget(), Some(2));
        assert!(budgeted.sign(&[1; 32]).await.is_ok());
        assert_eq!(budgeted.remaining_budget(), Some(1));
        assert!(budgeted.sign(&[2; 32]).await.is_ok());
        assert!(budgeted.sign(&[3; 32]).await.is_err());
        assert_eq!(budgeted.remaining_budget(), Some(0));

        let refilled = BudgetedSigner::new(
            Arc::new(signer(19, Some("signer-test-f"))),
            1,
            Duration::ZERO,
        );
        assert!(refilled.sign(&[1; 32]).await.is_ok());
        assert!(refilled.sign(&[2; 32]).await.is_ok());
    }

    struct WrongKey(LocalSigner, VerifyingKey);

    #[async_trait]
    impl AuditSigner for WrongKey {
        fn kid(&self) -> &str {
            self.0.kid()
        }
        fn verifying_key(&self) -> VerifyingKey {
            self.1
        }
        async fn sign(&self, digest: &Hash) -> flow_like_types::Result<RawSignature> {
            self.0.sign(digest).await
        }
    }

    #[flow_like_types::tokio::test]
    async fn budget_rejects_signatures_from_the_wrong_key() {
        let other = signer(21, None).verifying_key();
        let budgeted = BudgetedSigner::new(
            Arc::new(WrongKey(signer(20, None), other)),
            10,
            Duration::from_secs(3600),
        );
        assert!(budgeted.sign(&[1; 32]).await.is_err());
    }

    #[test]
    fn der_signatures_normalize_to_raw() {
        let key = SigningKey::from_slice(&[17; 32]).unwrap();
        let signature: Signature = key.sign(&[3u8; 32]);
        let raw = raw_from_der(signature.to_der().as_bytes()).unwrap();
        assert!(
            key.verifying_key()
                .verify(&[3u8; 32], &Signature::from_slice(&raw).unwrap())
                .is_ok()
        );
        assert!(raw_from_der(&[1, 2, 3]).is_err());
    }
}
