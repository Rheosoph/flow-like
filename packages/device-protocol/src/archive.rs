use crate::{
    Ed25519PublicKey, ProtocolError, Result, SigningKey,
    proof::{sign_pinned, verify_pinned},
};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashSet;

pub const MAX_ARCHIVE_RECIPIENTS: usize = 32;
pub const MAX_ARCHIVE_PLAINTEXT_BYTES: usize = 1024 * 1024;
pub const MAX_ARCHIVE_ROSTER_SECONDS: i64 = 31 * 86_400;
const ROSTER_TYPE: &str = "flow-like-archive-roster+jwt";
const MANIFEST_TYPE: &str = "flow-like-archive-manifest+jwt";

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArchiveRecipientKey {
    pub recipient_id: String,
    pub wrapped_key: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EncryptedArchive {
    pub roster_jws: String,
    pub manifest_jws: String,
    pub ciphertext: String,
    pub recipient_keys: Vec<ArchiveRecipientKey>,
}

/// Integrity validation for an opaque stored object. Callers separately enforce
/// the active roster head, upload authority, sequence continuity, and quota.
pub fn verify_archive_content(
    bundle: &EncryptedArchive,
    owner: &Ed25519PublicKey,
    device: &Ed25519PublicKey,
) -> Result<(ArchiveManifest, ArchiveRoster)> {
    let manifest = verify_archive_manifest(&bundle.manifest_jws, device)?;
    if manifest.roster_digest != crate::compact_digest(&bundle.roster_jws) || owner == device {
        return Err(ProtocolError::BindingMismatch);
    }
    let roster = verify_historical_archive_roster(&bundle.roster_jws, owner, manifest.created_at)?;
    if manifest.device_id != roster.device_id
        || manifest.scope != roster.scope
        || manifest.kind != roster.kind
        || bundle.recipient_keys.len() != roster.recipients.len()
        || manifest.recipients.len() != roster.recipients.len()
    {
        return Err(ProtocolError::BindingMismatch);
    }
    for ((recipient, sealed), envelope) in roster
        .recipients
        .iter()
        .zip(&manifest.recipients)
        .zip(&bundle.recipient_keys)
    {
        if recipient.recipient_id != sealed.recipient_id
            || recipient.recipient_id != envelope.recipient_id
            || recipient.public_key == owner.to_bytes()?
            || recipient.public_key == device.to_bytes()?
        {
            return Err(ProtocolError::BindingMismatch);
        }
        let key = decode_archive_bytes(&envelope.wrapped_key, 48)?;
        if key.len() != 48 || hash_archive_bytes(&key) != sealed.wrapped_key_digest {
            return Err(ProtocolError::BindingMismatch);
        }
    }
    let ciphertext = decode_archive_bytes(&bundle.ciphertext, MAX_ARCHIVE_PLAINTEXT_BYTES + 16)?;
    if ciphertext.len() as u64 != manifest.ciphertext_size
        || hash_archive_bytes(&ciphertext) != manifest.ciphertext_digest
    {
        return Err(ProtocolError::BindingMismatch);
    }
    Ok((manifest, roster))
}
fn hash_archive_bytes(bytes: &[u8]) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(bytes))
}
fn decode_archive_bytes(value: &str, maximum: usize) -> Result<Vec<u8>> {
    if value.len() > maximum.div_ceil(3) * 4 {
        return Err(ProtocolError::Invalid("archive encoding size"));
    }
    let bytes = URL_SAFE_NO_PAD
        .decode(value)
        .map_err(|_| ProtocolError::Invalid("archive encoding"))?;
    if bytes.len() > maximum || URL_SAFE_NO_PAD.encode(&bytes) != value {
        return Err(ProtocolError::Invalid("archive encoding"));
    }
    Ok(bytes)
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArchiveKind {
    Logs,
    Metrics,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArchiveRecipient {
    pub recipient_id: String,
    pub user_id: String,
    pub public_key: [u8; 32],
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArchiveRoster {
    pub version: u32,
    pub device_id: String,
    /// "device" or an exact placement ID; logs and metrics have separate rosters.
    pub scope: String,
    /// The owner binds placement archives to their project for server-side read authorization.
    #[serde(default)]
    pub project_id: Option<String>,
    pub kind: ArchiveKind,
    pub policy_version: u64,
    pub previous_policy_digest: Option<String>,
    pub management_policy_digest: Option<String>,
    pub recipients: Vec<ArchiveRecipient>,
    pub issued_at: i64,
    pub expires_at: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArchiveWrappedKeyManifest {
    pub recipient_id: String,
    pub encapsulation: [u8; 32],
    pub wrapped_key_digest: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArchiveManifest {
    pub version: u32,
    pub device_id: String,
    pub scope: String,
    pub kind: ArchiveKind,
    pub archive_id: String,
    pub sequence: u64,
    pub previous_manifest_digest: Option<String>,
    pub roster_digest: String,
    pub created_at: i64,
    pub nonce: [u8; 24],
    pub ciphertext_size: u64,
    pub ciphertext_digest: String,
    pub recipients: Vec<ArchiveWrappedKeyManifest>,
}

fn id(value: &str) -> Result<()> {
    crate::validate_management_id(value)
}
fn digest(value: &str) -> Result<()> {
    let bytes = URL_SAFE_NO_PAD
        .decode(value)
        .map_err(|_| ProtocolError::Invalid("archive digest"))?;
    if bytes.len() != 32 || URL_SAFE_NO_PAD.encode(bytes) != value {
        return Err(ProtocolError::Invalid("archive digest"));
    }
    Ok(())
}
fn chain(sequence: u64, previous: &Option<String>) -> Result<()> {
    if sequence == 0 || (sequence == 1) != previous.is_none() {
        return Err(ProtocolError::Invalid("archive chain"));
    }
    if let Some(previous) = previous {
        digest(previous)?;
    }
    Ok(())
}
pub fn validate_archive_roster(roster: &ArchiveRoster) -> Result<()> {
    id(&roster.device_id)?;
    id(&roster.scope)?;
    if let Some(project) = &roster.project_id {
        id(project)?;
    }
    if (roster.scope == "device") != roster.project_id.is_none() {
        return Err(ProtocolError::Invalid("archive project scope"));
    }
    chain(roster.policy_version, &roster.previous_policy_digest)?;
    if roster.version != 1
        || roster.issued_at <= 0
        || roster.expires_at <= roster.issued_at
        || roster.expires_at - roster.issued_at > MAX_ARCHIVE_ROSTER_SECONDS
        || roster.recipients.is_empty()
        || roster.recipients.len() > MAX_ARCHIVE_RECIPIENTS
    {
        return Err(ProtocolError::Invalid("archive roster"));
    }
    if let Some(head) = &roster.management_policy_digest {
        digest(head)?;
    }
    let mut ids = HashSet::new();
    let mut keys = HashSet::new();
    for recipient in &roster.recipients {
        id(&recipient.recipient_id)?;
        id(&recipient.user_id)?;
        crate::validate_management_key(&recipient.public_key)?;
        if !ids.insert(&recipient.recipient_id) || !keys.insert(recipient.public_key) {
            return Err(ProtocolError::Invalid("duplicate archive recipient"));
        }
    }
    Ok(())
}
pub fn sign_archive_roster(roster: &ArchiveRoster, owner: &SigningKey) -> Result<String> {
    validate_archive_roster(roster)?;
    sign_pinned(roster, owner, ROSTER_TYPE)
}
pub fn verify_archive_roster(
    compact: &str,
    owner: &Ed25519PublicKey,
    now: i64,
) -> Result<ArchiveRoster> {
    let roster: ArchiveRoster = verify_pinned(compact, owner, ROSTER_TYPE)?;
    validate_archive_roster(&roster)?;
    if now < roster.issued_at || now >= roster.expires_at {
        return Err(ProtocolError::InvalidTime);
    }
    Ok(roster)
}
/// Historical access checks authorization at the device-signed creation time.
/// It does not authorize publishing under an expired or superseded roster.
pub fn verify_historical_archive_roster(
    compact: &str,
    owner: &Ed25519PublicKey,
    created_at: i64,
) -> Result<ArchiveRoster> {
    verify_archive_roster(compact, owner, created_at)
}

/// Authenticate an old policy head for renewal without authorizing publication.
pub fn verify_archive_roster_head(
    compact: &str,
    owner: &Ed25519PublicKey,
) -> Result<ArchiveRoster> {
    let roster = verify_pinned(compact, owner, ROSTER_TYPE)?;
    validate_archive_roster(&roster)?;
    Ok(roster)
}
pub fn validate_archive_manifest(manifest: &ArchiveManifest) -> Result<()> {
    id(&manifest.device_id)?;
    id(&manifest.scope)?;
    id(&manifest.archive_id)?;
    chain(manifest.sequence, &manifest.previous_manifest_digest)?;
    digest(&manifest.roster_digest)?;
    digest(&manifest.ciphertext_digest)?;
    if manifest.version != 1
        || manifest.created_at <= 0
        || manifest.ciphertext_size < 16
        || manifest.ciphertext_size > (MAX_ARCHIVE_PLAINTEXT_BYTES + 16) as u64
        || manifest.recipients.is_empty()
        || manifest.recipients.len() > MAX_ARCHIVE_RECIPIENTS
    {
        return Err(ProtocolError::Invalid("archive manifest"));
    }
    let mut ids = HashSet::new();
    for recipient in &manifest.recipients {
        id(&recipient.recipient_id)?;
        digest(&recipient.wrapped_key_digest)?;
        crate::validate_management_key(&recipient.encapsulation)?;
        if !ids.insert(&recipient.recipient_id) {
            return Err(ProtocolError::Invalid("duplicate archive recipient"));
        }
    }
    Ok(())
}
pub fn sign_archive_manifest(manifest: &ArchiveManifest, device: &SigningKey) -> Result<String> {
    validate_archive_manifest(manifest)?;
    sign_pinned(manifest, device, MANIFEST_TYPE)
}
pub fn verify_archive_manifest(
    compact: &str,
    device: &Ed25519PublicKey,
) -> Result<ArchiveManifest> {
    let manifest = verify_pinned(compact, device, MANIFEST_TYPE)?;
    validate_archive_manifest(&manifest)?;
    Ok(manifest)
}
