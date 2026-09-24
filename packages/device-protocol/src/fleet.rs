use crate::{
    proof::{sign_pinned, verify_pinned},
    *,
};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const MAX_FLEET_PLAINTEXT: usize = 256 * 1024;
pub const MAX_FLEET_READERS: usize = 64;
pub const MAX_FLEET_STREAMS: usize = 128;
pub const MAX_FLEET_READER_LIFETIME: i64 = 366 * 86400;
const READER_TYPE: &str = "flow-like-fleet-reader+jwt";
const SNAPSHOT_TYPE: &str = "flow-like-fleet-snapshot+jwt";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FleetReader {
    pub version: u8,
    pub device_id: String,
    pub api_base_url: String,
    pub user_id: String,
    pub controller_key: Ed25519PublicKey,
    pub archive_key: [u8; 32],
    pub revision: u64,
    pub issued_at: i64,
    pub expires_at: i64,
}
impl FleetReader {
    pub fn validate(&self) -> Result<()> {
        validate_management_id(&self.device_id)?;
        self.controller_key.validate()?;
        validate_management_key(&self.archive_key)?;
        if self.version != 1
            || self.revision == 0
            || self.revision > 9_007_199_254_740_991
            || self.user_id.is_empty()
            || self.user_id.len() > 512
            || self.user_id.chars().any(char::is_control)
            || canonical_api_base_url(&self.api_base_url)? != self.api_base_url
            || self.issued_at <= 0
            || self.expires_at <= self.issued_at
            || self.expires_at - self.issued_at > MAX_FLEET_READER_LIFETIME
        {
            return Err(ProtocolError::Invalid("invalid fleet reader"));
        }
        Ok(())
    }
}
pub fn sign_fleet_reader(reader: &FleetReader, key: &SigningKey) -> Result<String> {
    reader.validate()?;
    if reader.controller_key != key.public_key() {
        return Err(ProtocolError::KeyMismatch);
    }
    sign_pinned(reader, key, READER_TYPE)
}
pub fn verify_fleet_reader(compact: &str, key: &Ed25519PublicKey, now: i64) -> Result<FleetReader> {
    let reader: FleetReader = verify_pinned(compact, key, READER_TYPE)?;
    reader.validate()?;
    if &reader.controller_key != key {
        return Err(ProtocolError::KeyMismatch);
    }
    if reader.issued_at > now + 30 || reader.expires_at <= now {
        return Err(ProtocolError::InvalidTime);
    }
    Ok(reader)
}

/// The caller must still match this self-signed declaration to an authorized controller.
pub fn inspect_fleet_reader(compact: &str, now: i64) -> Result<FleetReader> {
    if compact.len() > 16_384 {
        return Err(ProtocolError::Invalid("fleet reader exceeds limit"));
    }
    let payload = compact
        .split('.')
        .nth(1)
        .ok_or(ProtocolError::Invalid("invalid fleet reader"))?;
    let reader: FleetReader = serde_json::from_slice(
        &URL_SAFE_NO_PAD
            .decode(payload)
            .map_err(|_| ProtocolError::Invalid("invalid fleet reader"))?,
    )
    .map_err(|_| ProtocolError::Invalid("invalid fleet reader"))?;
    verify_fleet_reader(compact, &reader.controller_key, now)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FleetKind {
    Status,
    Metrics,
}
impl FleetKind {
    pub fn capability(self) -> ManagementCapability {
        match self {
            Self::Status => ManagementCapability::Status,
            Self::Metrics => ManagementCapability::Metrics,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FleetAudience {
    pub reader_digest: String,
    pub grant_id: String,
    pub scope: ManagementScope,
    pub kind: FleetKind,
    pub policy_digest: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FleetManifest {
    pub version: u8,
    pub device_id: String,
    pub api_base_url: String,
    pub user_id: String,
    pub controller_key: Ed25519PublicKey,
    pub audience: FleetAudience,
    pub sequence: u64,
    pub previous_digest: Option<String>,
    pub boot_id: String,
    pub observed_at: i64,
    pub encapsulation: [u8; 32],
    pub ciphertext_digest: String,
    pub ciphertext_size: u32,
}
impl FleetManifest {
    pub fn validate(&self) -> Result<()> {
        validate_management_id(&self.device_id)?;
        validate_management_id(&self.audience.grant_id)?;
        inventory_scope_key(&self.audience.scope)?;
        self.controller_key.validate()?;
        if self.version != 1
            || self.sequence == 0
            || self.sequence > 9_007_199_254_740_991
            || (self.sequence == 1) != self.previous_digest.is_none()
            || self.boot_id.is_empty()
            || self.boot_id.len() > 128
            || self.boot_id.chars().any(char::is_control)
            || self.user_id.is_empty()
            || self.user_id.len() > 512
            || self.observed_at <= 0
            || canonical_api_base_url(&self.api_base_url)? != self.api_base_url
            || !(16..=MAX_FLEET_PLAINTEXT + 16).contains(&(self.ciphertext_size as usize))
        {
            return Err(ProtocolError::Invalid("invalid fleet manifest"));
        }
        for digest in [&self.audience.reader_digest, &self.ciphertext_digest]
            .into_iter()
            .chain(self.previous_digest.iter())
            .chain(self.audience.policy_digest.iter())
        {
            if URL_SAFE_NO_PAD.decode(digest).map_or(true, |bytes| {
                bytes.len() != 32 || URL_SAFE_NO_PAD.encode(bytes) != *digest
            }) {
                return Err(ProtocolError::Invalid("invalid fleet digest"));
            }
        }
        Ok(())
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EncryptedFleetSnapshot {
    pub manifest_jws: String,
    pub ciphertext: String,
}
pub fn fleet_digest(bytes: &[u8]) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(bytes))
}
pub fn sign_fleet_manifest(manifest: &FleetManifest, key: &SigningKey) -> Result<String> {
    manifest.validate()?;
    sign_pinned(manifest, key, SNAPSHOT_TYPE)
}
pub fn verify_fleet_manifest(compact: &str, device: &Ed25519PublicKey) -> Result<FleetManifest> {
    let manifest: FleetManifest = verify_pinned(compact, device, SNAPSHOT_TYPE)?;
    manifest.validate()?;
    Ok(manifest)
}

pub fn verify_fleet_snapshot(
    bundle: &EncryptedFleetSnapshot,
    device: &Ed25519PublicKey,
) -> Result<FleetManifest> {
    let manifest = verify_fleet_manifest(&bundle.manifest_jws, device)?;
    if bundle.ciphertext.len() > (MAX_FLEET_PLAINTEXT + 16).div_ceil(3) * 4 {
        return Err(ProtocolError::Invalid("fleet ciphertext exceeds limit"));
    }
    let bytes = URL_SAFE_NO_PAD
        .decode(&bundle.ciphertext)
        .map_err(|_| ProtocolError::Invalid("invalid fleet ciphertext"))?;
    if bytes.len() != manifest.ciphertext_size as usize
        || fleet_digest(&bytes) != manifest.ciphertext_digest
        || URL_SAFE_NO_PAD.encode(&bytes) != bundle.ciphertext
    {
        return Err(ProtocolError::BindingMismatch);
    }
    Ok(manifest)
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FleetSnapshot {
    pub version: u8,
    pub observed_at: i64,
    pub boot_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub inspection: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metrics: Option<serde_json::Value>,
}
impl FleetSnapshot {
    pub fn validate(&self, manifest: &FleetManifest) -> Result<()> {
        if self.version != 1
            || self.observed_at != manifest.observed_at
            || self.boot_id != manifest.boot_id
            || match manifest.audience.kind {
                FleetKind::Status => {
                    self.inspection.as_ref().is_none_or(|v| !v.is_object())
                        || self.metrics.is_some()
                }
                FleetKind::Metrics => {
                    self.metrics.as_ref().is_none_or(|v| !v.is_object())
                        || self.inspection.is_some()
                }
            }
        {
            return Err(ProtocolError::BindingMismatch);
        }
        Ok(())
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FleetReaderState {
    pub reader_jws: String,
    pub revision: u64,
    pub deleted: bool,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FleetHead {
    pub sequence: u64,
    pub manifest_digest: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FleetRecipients {
    pub readers: Vec<String>,
    pub policy_jws: Option<String>,
    pub head: FleetHead,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FleetView {
    pub snapshots: Vec<EncryptedFleetSnapshot>,
    pub policy_jws: Option<String>,
    pub reader_jws: String,
}

pub fn fleet_audiences(
    reader: &FleetReader,
    reader_jws: &str,
    manifest: &OnboardingManifest,
    policy: Option<(&str, &ManagementPolicy)>,
    now: i64,
) -> Vec<FleetAudience> {
    if reader.device_id != manifest.device_id
        || reader.api_base_url != manifest.api_base_url
        || reader.expires_at <= now
    {
        return vec![];
    }
    if reader.user_id == manifest.owner_id && reader.controller_key == manifest.controller_key {
        return [FleetKind::Status, FleetKind::Metrics]
            .into_iter()
            .map(|kind| FleetAudience {
                reader_digest: compact_digest(reader_jws),
                grant_id: "owner".into(),
                scope: ManagementScope::Device,
                kind,
                policy_digest: None,
            })
            .collect();
    }
    let Some((compact, policy)) = policy.filter(|(_, p)| {
        p.device_id == reader.device_id && p.issued_at <= now + 30 && p.expires_at > now
    }) else {
        return vec![];
    };
    policy
        .grants
        .iter()
        .filter(|g| {
            g.user_id == reader.user_id
                && g.controller_key == reader.controller_key
                && g.expires_at > now
        })
        .flat_map(|g| {
            [FleetKind::Status, FleetKind::Metrics]
                .into_iter()
                .filter(|kind| g.capabilities.contains(&kind.capability()))
                .map(|kind| FleetAudience {
                    reader_digest: compact_digest(reader_jws),
                    grant_id: g.grant_id.clone(),
                    scope: g.scope.clone(),
                    kind,
                    policy_digest: Some(compact_digest(compact)),
                })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    fn reader(key: &SigningKey) -> FleetReader {
        FleetReader {
            version: 1,
            device_id: "device".into(),
            api_base_url: "https://hub.example/api/v1".into(),
            user_id: "reader".into(),
            controller_key: key.public_key(),
            archive_key: [42; 32],
            revision: 1,
            issued_at: 100,
            expires_at: 200,
        }
    }
    #[test]
    fn reader_signature_binds_archive_key_account_api_and_lifetime() {
        let key = SigningKey::generate();
        let reader = reader(&key);
        let signed = sign_fleet_reader(&reader, &key).unwrap();
        assert_eq!(inspect_fleet_reader(&signed, 110).unwrap(), reader);
        assert!(verify_fleet_reader(&signed, &SigningKey::generate().public_key(), 110).is_err());
        assert!(inspect_fleet_reader(&signed, 200).is_err());
        let mut wrong = reader.clone();
        wrong.archive_key = [0; 32];
        assert!(sign_fleet_reader(&wrong, &key).is_err());
        wrong = reader;
        wrong.expires_at = wrong.issued_at + MAX_FLEET_READER_LIFETIME + 1;
        assert!(sign_fleet_reader(&wrong, &key).is_err());
        assert!(
            verify_fleet_reader(
                &sign_client_assertion(
                    &ClientAssertion {
                        iss: "device".into(),
                        sub: "device".into(),
                        aud: "https://hub.example/api/v1/devices/device/fleet/recipients".into(),
                        iat: 100,
                        nbf: 100,
                        exp: 150,
                        jti: "fleet-test-client-assertion".into()
                    },
                    &key
                )
                .unwrap(),
                &key.public_key(),
                110
            )
            .is_err()
        );
    }
    #[test]
    fn status_readers_never_receive_metrics_or_archives_and_revoked_grants_stop_publication() {
        let key = SigningKey::generate();
        let reader = reader(&key);
        let signed = sign_fleet_reader(&reader, &key).unwrap();
        let manifest = OnboardingManifest {
            version: 1,
            enrollment_id: "enrollment".into(),
            device_id: "device".into(),
            owner_id: "owner".into(),
            name: "Device".into(),
            api_base_url: reader.api_base_url.clone(),
            bootstrap_key: SigningKey::generate().public_key(),
            controller_key: SigningKey::generate().public_key(),
            owner_invitation_key: SigningKey::generate().public_key(),
            issued_at: 100,
            expires_at: 200,
        };
        let mut policy = ManagementPolicy {
            version: 1,
            device_id: "device".into(),
            policy_version: 1,
            previous_policy_digest: None,
            issued_at: 100,
            expires_at: 200,
            grants: vec![ManagementGrant {
                grant_id: "grant".into(),
                user_id: "reader".into(),
                controller_key: key.public_key(),
                scope: ManagementScope::Project {
                    project_id: "project".into(),
                },
                capabilities: vec![ManagementCapability::Status, ManagementCapability::Logs],
                expires_at: 190,
                group_id: None,
                group_version: None,
            }],
        };
        let audiences =
            fleet_audiences(&reader, &signed, &manifest, Some(("policy", &policy)), 110);
        assert_eq!(audiences.len(), 1);
        assert_eq!(audiences[0].kind, FleetKind::Status);
        assert_eq!(audiences[0].scope, policy.grants[0].scope);
        policy.grants[0].capabilities = vec![ManagementCapability::Logs];
        assert!(
            fleet_audiences(&reader, &signed, &manifest, Some(("policy", &policy)), 110).is_empty()
        );
        policy.grants[0].capabilities = vec![ManagementCapability::Metrics];
        assert_eq!(
            fleet_audiences(&reader, &signed, &manifest, Some(("policy", &policy)), 110)[0].kind,
            FleetKind::Metrics
        );
        policy.grants.clear();
        assert!(
            fleet_audiences(&reader, &signed, &manifest, Some(("policy", &policy)), 110).is_empty()
        );
    }
}
