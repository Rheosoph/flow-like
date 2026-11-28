//! Credentials of an app's external databases. A [`SecretSealer`] turns a source's secret fields
//! into a [`SealedRecord`] that opens only for the same app and source, and the API opens it
//! only to build a query-service request. Every record names its sealer and key, so a key
//! service can be added next to [`DerivedKeySealer`] and stored records re-wrapped by a job.

use aes_gcm::{
    Aes256Gcm, KeyInit, Nonce,
    aead::{Aead, Payload},
};
use async_trait::async_trait;
use base64::Engine;
use flow_like_query::SecretFields;
use flow_like_types::{Result, anyhow};
use std::fmt;
use zeroize::Zeroizing;

const RECORD_PREFIX: &str = "v1:";
const NONCE_LEN: usize = 12;
const DERIVED_PROVIDER: &str = "derived";
const DERIVED_KEY_CONTEXT: &str = "flow-like data source secret v1";
const DERIVED_KEY_ID_CONTEXT: &str = "flow-like data source key id v1";

/// A sealed secret as stored: `v1:<provider>:<key id>:<ciphertext>`.
#[derive(Clone, PartialEq, Eq)]
pub struct SealedRecord {
    /// The sealer that wrote it, such as `derived` or `aws-kms`.
    pub provider: String,
    /// The key it was sealed under. It may contain `:` (a KMS key ARN); the ciphertext never does.
    pub key_id: Option<String>,
    /// Base64 of what the sealer produced.
    pub ciphertext: String,
}

impl SealedRecord {
    pub fn encode(&self) -> String {
        format!(
            "{RECORD_PREFIX}{}:{}:{}",
            self.provider,
            self.key_id.as_deref().unwrap_or_default(),
            self.ciphertext
        )
    }

    pub fn decode(stored: &str) -> Result<Self> {
        let malformed = |what: &str| anyhow!("Stored data source secret is malformed: {what}");
        let rest = stored
            .strip_prefix(RECORD_PREFIX)
            .ok_or_else(|| malformed("it has no 'v1:' prefix"))?;
        let (provider, rest) = rest
            .split_once(':')
            .ok_or_else(|| malformed("it names no sealer"))?;
        let (key_id, ciphertext) = rest
            .rsplit_once(':')
            .ok_or_else(|| malformed("it has no key id field"))?;
        let provider_ok = !provider.is_empty()
            && provider
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
        if !provider_ok {
            return Err(malformed("the sealer name is not a lowercase identifier"));
        }
        if ciphertext.is_empty() {
            return Err(malformed("the ciphertext is empty"));
        }
        Ok(Self {
            provider: provider.to_string(),
            key_id: (!key_id.is_empty()).then(|| key_id.to_string()),
            ciphertext: ciphertext.to_string(),
        })
    }

    /// `<provider>:<key id>`, kept in the `sealedBy` column so a re-wrap job can select by it.
    pub fn sealed_by(&self) -> String {
        match &self.key_id {
            Some(key_id) => format!("{}:{key_id}", self.provider),
            None => self.provider.clone(),
        }
    }
}

impl fmt::Debug for SealedRecord {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SealedRecord")
            .field("provider", &self.provider)
            .field("key_id", &self.key_id)
            .field("ciphertext_len", &self.ciphertext.len())
            .finish()
    }
}

/// The app and source a ciphertext is bound to; it opens for no other.
#[derive(Clone, Copy, Debug)]
pub struct SecretScope<'a> {
    pub app_id: &'a str,
    pub source_id: &'a str,
}

impl SecretScope<'_> {
    fn associated_data(&self) -> Vec<u8> {
        let mut aad = Vec::with_capacity(self.app_id.len() + 1 + self.source_id.len());
        aad.extend_from_slice(self.app_id.as_bytes());
        aad.push(0);
        aad.extend_from_slice(self.source_id.as_bytes());
        aad
    }
}

#[async_trait]
pub trait SecretSealer: Send + Sync {
    /// The `provider` of the records this sealer writes and opens.
    fn provider(&self) -> &'static str;

    async fn seal(&self, scope: SecretScope<'_>, plaintext: &[u8]) -> Result<SealedRecord>;

    /// Fails for a record of another sealer, key, app or source, and for any altered record.
    async fn open(
        &self,
        scope: SecretScope<'_>,
        record: &SealedRecord,
    ) -> Result<Zeroizing<Vec<u8>>>;
}

/// AES-256-GCM under a per-app key derived from the deployment key (`SINK_TOKEN_ENCRYPTION_KEY`).
/// It is the fallback of every key provider option and the only sealer until one is chosen.
pub struct DerivedKeySealer {
    master: Zeroizing<[u8; 32]>,
    key_id: String,
}

impl DerivedKeySealer {
    pub fn new(master: &[u8; 32]) -> Self {
        let fingerprint = blake3::derive_key(DERIVED_KEY_ID_CONTEXT, master);
        Self {
            master: Zeroizing::new(*master),
            key_id: hex::encode(&fingerprint[..8]),
        }
    }

    /// Names the deployment key without revealing it, so records sealed under a previous key
    /// can be told apart after a rotation.
    pub fn key_id(&self) -> &str {
        &self.key_id
    }

    fn cipher(&self, app_id: &str) -> Result<Aes256Gcm> {
        let mut material = Zeroizing::new(Vec::with_capacity(self.master.len() + app_id.len()));
        material.extend_from_slice(self.master.as_slice());
        material.extend_from_slice(app_id.as_bytes());
        let key = Zeroizing::new(blake3::derive_key(DERIVED_KEY_CONTEXT, &material));
        Aes256Gcm::new_from_slice(key.as_slice()).map_err(|_| {
            anyhow!("The data source secret key of app '{app_id}' has an invalid length")
        })
    }
}

#[async_trait]
impl SecretSealer for DerivedKeySealer {
    fn provider(&self) -> &'static str {
        DERIVED_PROVIDER
    }

    async fn seal(&self, scope: SecretScope<'_>, plaintext: &[u8]) -> Result<SealedRecord> {
        let mut nonce = [0u8; NONCE_LEN];
        getrandom::fill(&mut nonce).map_err(|error| {
            anyhow!(
                "No nonce for the secret of data source '{}': {error}",
                scope.source_id
            )
        })?;
        let ciphertext = self
            .cipher(scope.app_id)?
            .encrypt(
                Nonce::from_slice(&nonce),
                Payload {
                    msg: plaintext,
                    aad: &scope.associated_data(),
                },
            )
            .map_err(|_| {
                anyhow!(
                    "Sealing the secret of data source '{}' failed",
                    scope.source_id
                )
            })?;
        let mut sealed = nonce.to_vec();
        sealed.extend(ciphertext);
        Ok(SealedRecord {
            provider: DERIVED_PROVIDER.to_string(),
            key_id: Some(self.key_id.clone()),
            ciphertext: base64::engine::general_purpose::STANDARD.encode(sealed),
        })
    }

    async fn open(
        &self,
        scope: SecretScope<'_>,
        record: &SealedRecord,
    ) -> Result<Zeroizing<Vec<u8>>> {
        let source_id = scope.source_id;
        if record.provider != DERIVED_PROVIDER {
            return Err(anyhow!(
                "The secret of data source '{source_id}' was sealed by '{}', not by the deployment key",
                record.provider
            ));
        }
        if let Some(key_id) = &record.key_id
            && key_id != &self.key_id
        {
            return Err(anyhow!(
                "The secret of data source '{source_id}' was sealed under deployment key '{key_id}', but this deployment's key is '{}'",
                self.key_id
            ));
        }
        let sealed = base64::engine::general_purpose::STANDARD
            .decode(&record.ciphertext)
            .map_err(|_| anyhow!("The secret of data source '{source_id}' is not valid base64"))?;
        if sealed.len() <= NONCE_LEN {
            return Err(anyhow!(
                "The secret of data source '{source_id}' is too short to hold a nonce and a tag"
            ));
        }
        let (nonce, ciphertext) = sealed.split_at(NONCE_LEN);
        let plaintext = self
            .cipher(scope.app_id)?
            .decrypt(
                Nonce::from_slice(nonce),
                Payload {
                    msg: ciphertext,
                    aad: &scope.associated_data(),
                },
            )
            .map_err(|_| {
                anyhow!(
                    "The secret of data source '{source_id}' does not open for app '{}'",
                    scope.app_id
                )
            })?;
        Ok(Zeroizing::new(plaintext))
    }
}

/// The sealers this deployment opens records with, and the one it seals new secrets with.
pub struct Sealers {
    derived: DerivedKeySealer,
}

impl Sealers {
    pub fn new(master: &[u8; 32]) -> Self {
        Self {
            derived: DerivedKeySealer::new(master),
        }
    }

    pub fn active(&self) -> &dyn SecretSealer {
        &self.derived
    }

    /// The sealer that wrote `record`.
    pub fn for_record(&self, record: &SealedRecord) -> Result<&dyn SecretSealer> {
        let known: [&dyn SecretSealer; 1] = [&self.derived];
        known
            .into_iter()
            .find(|sealer| sealer.provider() == record.provider)
            .ok_or_else(|| {
                anyhow!(
                    "Data source secrets sealed by '{}' cannot be opened by this deployment",
                    record.provider
                )
            })
    }

    pub async fn seal_fields(
        &self,
        scope: SecretScope<'_>,
        fields: &SecretFields,
    ) -> Result<SealedRecord> {
        let plaintext = Zeroizing::new(serde_json::to_vec(fields).map_err(|_| {
            anyhow!(
                "The secret of data source '{}' could not be encoded",
                scope.source_id
            )
        })?);
        self.active().seal(scope, &plaintext).await
    }

    /// The fields of a stored record, wiped when dropped.
    pub async fn open_fields(&self, scope: SecretScope<'_>, stored: &str) -> Result<SecretFields> {
        let record = SealedRecord::decode(stored)?;
        let plaintext = self.for_record(&record)?.open(scope, &record).await?;
        serde_json::from_slice(&plaintext).map_err(|_| {
            anyhow!(
                "The secret of data source '{}' is not a map of named fields",
                scope.source_id
            )
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    const KEY: [u8; 32] = [7; 32];
    const SCOPE: SecretScope<'static> = SecretScope {
        app_id: "app_a",
        source_id: "source_a",
    };

    fn fields(password: &str) -> SecretFields {
        SecretFields(BTreeMap::from([
            ("username".to_string(), "reader".to_string()),
            ("password".to_string(), password.to_string()),
        ]))
    }

    #[tokio::test]
    async fn round_trips_with_a_fresh_nonce_each_time() {
        let sealer = DerivedKeySealer::new(&KEY);
        let first = sealer.seal(SCOPE, b"secret").await.unwrap();
        let second = sealer.seal(SCOPE, b"secret").await.unwrap();
        assert_ne!(first.ciphertext, second.ciphertext);
        assert_eq!(
            sealer.open(SCOPE, &first).await.unwrap().as_slice(),
            b"secret"
        );
        assert_eq!(
            sealer.open(SCOPE, &second).await.unwrap().as_slice(),
            b"secret"
        );
    }

    #[tokio::test]
    async fn stores_provider_key_id_and_base64() {
        let sealer = DerivedKeySealer::new(&KEY);
        let record = sealer.seal(SCOPE, b"").await.unwrap();
        let stored = record.encode();
        let parts: Vec<&str> = stored.splitn(4, ':').collect();
        assert_eq!(parts[0], "v1");
        assert_eq!(parts[1], "derived");
        assert_eq!(parts[2], sealer.key_id());
        assert_eq!(parts[2].len(), 16);
        let sealed = base64::engine::general_purpose::STANDARD
            .decode(parts[3])
            .unwrap();
        assert_eq!(sealed.len(), NONCE_LEN + 16);
        assert_eq!(SealedRecord::decode(&stored).unwrap(), record);
        assert_eq!(record.sealed_by(), format!("derived:{}", sealer.key_id()));
        assert_ne!(DerivedKeySealer::new(&[8; 32]).key_id(), sealer.key_id());
    }

    #[test]
    fn a_key_id_may_hold_colons_and_malformed_records_are_refused() {
        let record = SealedRecord {
            provider: "aws-kms".to_string(),
            key_id: Some("arn:aws:kms:eu-west-1:123456789012:key/abc".to_string()),
            ciphertext: "AAAA+/==".to_string(),
        };
        assert_eq!(SealedRecord::decode(&record.encode()).unwrap(), record);
        let keyless = SealedRecord {
            key_id: None,
            ..record.clone()
        };
        assert_eq!(SealedRecord::decode(&keyless.encode()).unwrap(), keyless);
        assert_eq!(keyless.sealed_by(), "aws-kms");
        for stored in [
            "derived:abc:AAAA",
            "v1:",
            "v1:derived",
            "v1:derived:AAAA",
            "v1:Derived:abc:AAAA",
            "v1:derived:abc:",
        ] {
            assert!(SealedRecord::decode(stored).is_err(), "{stored}");
        }
    }

    #[tokio::test]
    async fn a_record_does_not_open_for_another_app_source_or_key() {
        let sealer = DerivedKeySealer::new(&KEY);
        let record = sealer.seal(SCOPE, b"secret").await.unwrap();
        let other_app = SecretScope {
            app_id: "app_b",
            ..SCOPE
        };
        let other_source = SecretScope {
            source_id: "source_b",
            ..SCOPE
        };
        assert!(sealer.open(other_app, &record).await.is_err());
        assert!(sealer.open(other_source, &record).await.is_err());
        let rotated = DerivedKeySealer::new(&[8; 32]);
        let error = rotated.open(SCOPE, &record).await.unwrap_err().to_string();
        assert!(error.contains(sealer.key_id()) && error.contains(rotated.key_id()));
        let unlabeled = SealedRecord {
            key_id: None,
            ..record.clone()
        };
        assert!(rotated.open(SCOPE, &unlabeled).await.is_err());
        assert_eq!(
            sealer.open(SCOPE, &unlabeled).await.unwrap().as_slice(),
            b"secret"
        );
    }

    #[tokio::test]
    async fn a_tampered_record_does_not_open() {
        let sealer = DerivedKeySealer::new(&KEY);
        let record = sealer.seal(SCOPE, b"secret").await.unwrap();
        let mut bytes = base64::engine::general_purpose::STANDARD
            .decode(&record.ciphertext)
            .unwrap();
        for index in [0, NONCE_LEN, bytes.len() - 1] {
            bytes[index] ^= 1;
            let tampered = SealedRecord {
                ciphertext: base64::engine::general_purpose::STANDARD.encode(&bytes),
                ..record.clone()
            };
            assert!(sealer.open(SCOPE, &tampered).await.is_err(), "byte {index}");
            bytes[index] ^= 1;
        }
        let truncated = SealedRecord {
            ciphertext: base64::engine::general_purpose::STANDARD.encode(&bytes[..NONCE_LEN]),
            ..record.clone()
        };
        assert!(sealer.open(SCOPE, &truncated).await.is_err());
        let relabeled = SealedRecord {
            provider: "aws-kms".to_string(),
            ..record
        };
        assert!(sealer.open(SCOPE, &relabeled).await.is_err());
    }

    #[tokio::test]
    async fn fields_round_trip_and_errors_never_carry_values() {
        let sealers = Sealers::new(&KEY);
        let record = sealers
            .seal_fields(SCOPE, &fields("pa55-word-value"))
            .await
            .unwrap();
        let stored = record.encode();
        assert!(!stored.contains("pa55-word-value") && !stored.contains("reader"));
        let opened = sealers.open_fields(SCOPE, &stored).await.unwrap();
        assert_eq!(opened.get("username"), Some("reader"));
        assert_eq!(opened.get("password"), Some("pa55-word-value"));

        let other = SecretScope {
            source_id: "source_b",
            ..SCOPE
        };
        let error = sealers.open_fields(other, &stored).await.unwrap_err();
        assert!(!format!("{error:#}").contains("pa55-word-value"));
        let foreign = SealedRecord {
            provider: "aws-kms".to_string(),
            ..record
        };
        let error = sealers
            .open_fields(SCOPE, &foreign.encode())
            .await
            .unwrap_err()
            .to_string();
        assert!(error.contains("aws-kms"));
        let not_fields = sealers.active().seal(SCOPE, b"[1,2]").await.unwrap();
        assert!(
            sealers
                .open_fields(SCOPE, &not_fields.encode())
                .await
                .is_err()
        );
    }
}
