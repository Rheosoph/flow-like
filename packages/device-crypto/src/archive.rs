//! Independent per-segment keys for retained history. The caller fences roster
//! versions and sequence allocation in its durable publication transaction.
use anyhow::{Context, Result, ensure};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use chacha20poly1305::{
    XChaCha20Poly1305, XNonce,
    aead::{Aead, KeyInit, Payload},
};
use flow_like_device_protocol::*;
use openmls_rust_crypto::OpenMlsRustCrypto;
use openmls_traits::{
    OpenMlsProvider,
    crypto::OpenMlsCrypto,
    types::{HashType, HpkeAeadType, HpkeCiphertext, HpkeConfig, HpkeKdfType, HpkeKemType},
};
use rand_core::{OsRng, RngCore};
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArchivePins {
    pub device_id: String,
    pub scope: String,
    pub kind: ArchiveKind,
    pub owner_invitation_key: Ed25519PublicKey,
    pub device_signing_key: Ed25519PublicKey,
}
#[derive(Clone, Debug)]
pub struct ArchivePosition {
    pub archive_id: String,
    pub sequence: u64,
    pub previous_manifest_digest: Option<String>,
}
pub use flow_like_device_protocol::{ArchiveRecipientKey, EncryptedArchive};

fn suite() -> HpkeConfig {
    HpkeConfig(
        HpkeKemType::DhKem25519,
        HpkeKdfType::HkdfSha256,
        HpkeAeadType::ChaCha20Poly1305,
    )
}
fn digest(bytes: &[u8]) -> Result<String> {
    Ok(URL_SAFE_NO_PAD.encode(
        OpenMlsRustCrypto::default()
            .crypto()
            .hash(HashType::Sha2_256, bytes)?,
    ))
}
fn context(manifest: &ArchiveManifest) -> Result<Vec<u8>> {
    Ok(serde_json::to_vec(&(
        "flow-like-retained-history-v1",
        &manifest.device_id,
        &manifest.scope,
        &manifest.kind,
        &manifest.archive_id,
        manifest.sequence,
        &manifest.previous_manifest_digest,
        &manifest.roster_digest,
        manifest.created_at,
        &manifest.nonce,
        manifest.ciphertext_size,
    ))?)
}
fn wrapping_info(recipient: &ArchiveRecipient) -> Result<Vec<u8>> {
    Ok(serde_json::to_vec(&(
        "flow-like-retained-key-v1",
        &recipient.recipient_id,
        &recipient.user_id,
        recipient.public_key,
    ))?)
}
fn wrapping_aad(context: &[u8], ciphertext_digest: &str) -> Result<Vec<u8>> {
    Ok(serde_json::to_vec(&(context, ciphertext_digest))?)
}
fn validate_pins(pins: &ArchivePins, roster: &ArchiveRoster) -> Result<()> {
    pins.owner_invitation_key.validate()?;
    pins.device_signing_key.validate()?;
    ensure!(
        pins.owner_invitation_key != pins.device_signing_key,
        "Archive authority keys must be distinct"
    );
    ensure!(
        roster.device_id == pins.device_id
            && roster.scope == pins.scope
            && roster.kind == pins.kind,
        "Archive audience changed"
    );
    for recipient in &roster.recipients {
        ensure!(
            recipient.public_key != pins.owner_invitation_key.to_bytes()?
                && recipient.public_key != pins.device_signing_key.to_bytes()?,
            "Archive keys must be independent of signing keys"
        );
    }
    Ok(())
}

/// Create only after checking the roster against the current management policy.
/// Return this bundle to a durable outbox before sending any of its bytes.
pub fn seal_archive(
    pins: &ArchivePins,
    roster_jws: &str,
    device: &SigningKey,
    position: ArchivePosition,
    plaintext: &[u8],
    now: i64,
) -> Result<EncryptedArchive> {
    ensure!(
        plaintext.len() <= MAX_ARCHIVE_PLAINTEXT_BYTES,
        "Archive plaintext exceeds the segment limit"
    );
    ensure!(
        device.public_key() == pins.device_signing_key,
        "Archive publisher key changed"
    );
    let roster = verify_archive_roster(roster_jws, &pins.owner_invitation_key, now)?;
    validate_pins(pins, &roster)?;
    let key = Zeroizing::new({
        let mut key = [0; 32];
        OsRng.fill_bytes(&mut key);
        key
    });
    let mut nonce = [0; 24];
    OsRng.fill_bytes(&mut nonce);
    let mut manifest = ArchiveManifest {
        version: 1,
        device_id: pins.device_id.clone(),
        scope: pins.scope.clone(),
        kind: pins.kind.clone(),
        archive_id: position.archive_id,
        sequence: position.sequence,
        previous_manifest_digest: position.previous_manifest_digest,
        roster_digest: compact_digest(roster_jws),
        created_at: now,
        nonce,
        ciphertext_size: (plaintext.len() + 16) as u64,
        ciphertext_digest: String::new(),
        recipients: Vec::new(),
    };
    let context = context(&manifest)?;
    let ciphertext = XChaCha20Poly1305::new((&*key).into())
        .encrypt(
            XNonce::from_slice(&nonce),
            Payload {
                msg: plaintext,
                aad: &context,
            },
        )
        .map_err(|_| anyhow::anyhow!("Archive encryption failed"))?;
    manifest.ciphertext_digest = digest(&ciphertext)?;
    let aad = wrapping_aad(&context, &manifest.ciphertext_digest)?;
    let crypto = OpenMlsRustCrypto::default();
    let mut recipient_keys = Vec::with_capacity(roster.recipients.len());
    for recipient in &roster.recipients {
        let wrapped = crypto.crypto().hpke_seal(
            suite(),
            &recipient.public_key,
            &wrapping_info(recipient)?,
            &aad,
            &*key,
        )?;
        let encapsulation: [u8; 32] = wrapped
            .kem_output
            .as_slice()
            .try_into()
            .context("Invalid HPKE encapsulation")?;
        ensure!(
            wrapped.ciphertext.as_slice().len() == 48,
            "Invalid HPKE key envelope"
        );
        manifest.recipients.push(ArchiveWrappedKeyManifest {
            recipient_id: recipient.recipient_id.clone(),
            encapsulation,
            wrapped_key_digest: digest(wrapped.ciphertext.as_slice())?,
        });
        recipient_keys.push(ArchiveRecipientKey {
            recipient_id: recipient.recipient_id.clone(),
            wrapped_key: URL_SAFE_NO_PAD.encode(wrapped.ciphertext.as_slice()),
        });
    }
    let manifest_jws = sign_archive_manifest(&manifest, device)?;
    Ok(EncryptedArchive {
        roster_jws: roster_jws.to_owned(),
        manifest_jws,
        ciphertext: URL_SAFE_NO_PAD.encode(ciphertext),
        recipient_keys,
    })
}

/// Verify the complete immutable object without a decryption key. This does not
/// establish that its roster is the currently accepted authorization head.
pub fn verify_archive(pins: &ArchivePins, bundle: &EncryptedArchive) -> Result<ArchiveManifest> {
    let (manifest, roster) =
        verify_archive_content(bundle, &pins.owner_invitation_key, &pins.device_signing_key)?;
    ensure!(
        manifest.device_id == pins.device_id
            && manifest.scope == pins.scope
            && manifest.kind == pins.kind,
        "Archive manifest audience changed"
    );
    validate_pins(pins, &roster)?;
    Ok(manifest)
}

/// Removed recipients retain access to segments encrypted for them in the past.
/// The caller enforces sequence/checkpoint policy when displaying a history.
pub fn open_archive(
    pins: &ArchivePins,
    bundle: &EncryptedArchive,
    recipient_id: &str,
    archive_seed: &[u8; 32],
) -> Result<Zeroizing<Vec<u8>>> {
    let manifest = verify_archive(pins, bundle)?;
    let roster = verify_historical_archive_roster(
        &bundle.roster_jws,
        &pins.owner_invitation_key,
        manifest.created_at,
    )?;
    let index = roster
        .recipients
        .iter()
        .position(|recipient| recipient.recipient_id == recipient_id)
        .context("Archive does not include this recipient")?;
    let recipient = &roster.recipients[index];
    ensure!(
        x25519_dalek::x25519(*archive_seed, x25519_dalek::X25519_BASEPOINT_BYTES)
            == recipient.public_key,
        "Archive recipient key changed"
    );
    let context = context(&manifest)?;
    let aad = wrapping_aad(&context, &manifest.ciphertext_digest)?;
    let wrapped = HpkeCiphertext {
        kem_output: manifest.recipients[index].encapsulation.to_vec().into(),
        ciphertext: decode(&bundle.recipient_keys[index].wrapped_key, 48)?.into(),
    };
    let key = Zeroizing::new(OpenMlsRustCrypto::default().crypto().hpke_open(
        suite(),
        &wrapped,
        archive_seed,
        &wrapping_info(recipient)?,
        &aad,
    )?);
    ensure!(key.len() == 32, "Invalid archive content key");
    let cipher = XChaCha20Poly1305::new_from_slice(&key)
        .map_err(|_| anyhow::anyhow!("Invalid archive content key"))?;
    let ciphertext = decode(&bundle.ciphertext, MAX_ARCHIVE_PLAINTEXT_BYTES + 16)?;
    Ok(Zeroizing::new(
        cipher
            .decrypt(
                XNonce::from_slice(&manifest.nonce),
                Payload {
                    msg: &ciphertext,
                    aad: &context,
                },
            )
            .map_err(|_| anyhow::anyhow!("Archive authentication failed"))?,
    ))
}
fn decode(encoded: &str, maximum: usize) -> Result<Vec<u8>> {
    ensure!(
        encoded.len() <= maximum.div_ceil(3) * 4,
        "Archive encoding exceeds its limit"
    );
    let bytes = URL_SAFE_NO_PAD
        .decode(encoded)
        .context("Invalid archive encoding")?;
    ensure!(
        bytes.len() <= maximum && URL_SAFE_NO_PAD.encode(&bytes) == encoded,
        "Invalid archive encoding"
    );
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> (SigningKey, SigningKey, ArchivePins, ArchiveRoster) {
        let owner = SigningKey::generate();
        let device = SigningKey::generate();
        let pins = ArchivePins {
            device_id: "device".into(),
            scope: "placement".into(),
            kind: ArchiveKind::Logs,
            owner_invitation_key: owner.public_key(),
            device_signing_key: device.public_key(),
        };
        let roster = ArchiveRoster {
            version: 1,
            device_id: pins.device_id.clone(),
            scope: pins.scope.clone(),
            project_id: Some("project".into()),
            kind: pins.kind.clone(),
            policy_version: 1,
            previous_policy_digest: None,
            management_policy_digest: None,
            recipients: vec![recipient("alice", [1; 32]), recipient("bob", [2; 32])],
            issued_at: 100,
            expires_at: 200,
        };
        (owner, device, pins, roster)
    }
    fn recipient(id: &str, secret: [u8; 32]) -> ArchiveRecipient {
        ArchiveRecipient {
            recipient_id: id.into(),
            user_id: format!("user-{id}"),
            public_key: x25519_dalek::x25519(secret, x25519_dalek::X25519_BASEPOINT_BYTES),
        }
    }
    fn position() -> ArchivePosition {
        ArchivePosition {
            archive_id: "segment".into(),
            sequence: 1,
            previous_manifest_digest: None,
        }
    }
    #[test]
    fn recipients_share_history_but_removed_recipient_cannot_open_next_segment() {
        let (owner, device, pins, mut roster) = fixture();
        let first_policy = sign_archive_roster(&roster, &owner).unwrap();
        let first = seal_archive(
            &pins,
            &first_policy,
            &device,
            position(),
            b"private old log",
            150,
        )
        .unwrap();
        for (id, key) in [("alice", [1; 32]), ("bob", [2; 32])] {
            assert_eq!(
                open_archive(&pins, &first, id, &key).unwrap().as_slice(),
                b"private old log"
            );
        }
        assert!(seal_archive(&pins, &first_policy, &device, position(), b"expired", 200).is_err());
        assert!(open_archive(&pins, &first, "alice", &[2; 32]).is_err());
        roster.policy_version = 2;
        roster.previous_policy_digest = Some(compact_digest(&first_policy));
        roster.recipients.remove(0);
        roster.issued_at = 160;
        roster.expires_at = 260;
        let second_policy = sign_archive_roster(&roster, &owner).unwrap();
        let second = seal_archive(
            &pins,
            &second_policy,
            &device,
            ArchivePosition {
                archive_id: "second".into(),
                sequence: 2,
                previous_manifest_digest: Some(compact_digest(&first.manifest_jws)),
            },
            b"new log",
            170,
        )
        .unwrap();
        assert!(open_archive(&pins, &second, "alice", &[1; 32]).is_err());
        assert_eq!(
            open_archive(&pins, &second, "bob", &[2; 32])
                .unwrap()
                .as_slice(),
            b"new log"
        );
        // Historical decryption has no wall-clock gate; original authority is signed.
        assert_eq!(
            open_archive(&pins, &first, "alice", &[1; 32])
                .unwrap()
                .as_slice(),
            b"private old log"
        );
    }
    #[test]
    fn signatures_roster_ciphertext_recipient_order_and_audience_are_bound() {
        let (owner, device, pins, roster) = fixture();
        let policy = sign_archive_roster(&roster, &owner).unwrap();
        let original = seal_archive(&pins, &policy, &device, position(), b"private", 150).unwrap();
        let mut changed = original.clone();
        changed.ciphertext = URL_SAFE_NO_PAD.encode([0; 23]);
        assert!(verify_archive(&pins, &changed).is_err());
        let mut changed = original.clone();
        changed.recipient_keys.swap(0, 1);
        assert!(verify_archive(&pins, &changed).is_err());
        let mut changed = original.clone();
        changed.recipient_keys[0].wrapped_key = URL_SAFE_NO_PAD.encode([0; 48]);
        assert!(verify_archive(&pins, &changed).is_err());
        let mut changed = original.clone();
        let mut other = roster.clone();
        other.scope = "other".into();
        changed.roster_jws = sign_archive_roster(&other, &owner).unwrap();
        assert!(verify_archive(&pins, &changed).is_err());
        let mut wrong = pins.clone();
        wrong.kind = ArchiveKind::Metrics;
        assert!(verify_archive(&wrong, &original).is_err());
        let mut wrong = pins.clone();
        wrong.owner_invitation_key = SigningKey::generate().public_key();
        assert!(verify_archive(&wrong, &original).is_err());
        // Even a valid publisher signature cannot substitute AAD-bound context.
        let mut changed = original.clone();
        let mut manifest =
            verify_archive_manifest(&changed.manifest_jws, &device.public_key()).unwrap();
        manifest.archive_id = "rewritten".into();
        changed.manifest_jws = sign_archive_manifest(&manifest, &device).unwrap();
        assert!(open_archive(&pins, &changed, "alice", &[1; 32]).is_err());
        let mut changed = original.clone();
        let mut manifest =
            verify_archive_manifest(&changed.manifest_jws, &device.public_key()).unwrap();
        manifest.created_at = 200;
        changed.manifest_jws = sign_archive_manifest(&manifest, &device).unwrap();
        assert!(verify_archive(&pins, &changed).is_err());
    }
    #[test]
    fn malformed_keys_chains_duplicate_recipients_and_oversize_are_rejected() {
        let (owner, device, pins, roster) = fixture();
        let mut changed = roster.clone();
        changed.recipients[0].public_key = [0; 32];
        assert!(sign_archive_roster(&changed, &owner).is_err());
        let mut changed = roster.clone();
        changed.recipients.push(changed.recipients[0].clone());
        assert!(sign_archive_roster(&changed, &owner).is_err());
        let mut changed = roster.clone();
        changed.policy_version = 2;
        assert!(sign_archive_roster(&changed, &owner).is_err());
        let policy = sign_archive_roster(&roster, &owner).unwrap();
        assert!(
            seal_archive(
                &pins,
                &policy,
                &device,
                position(),
                &vec![0; MAX_ARCHIVE_PLAINTEXT_BYTES + 1],
                150
            )
            .is_err()
        );
        let mut bad = position();
        bad.sequence = 0;
        assert!(seal_archive(&pins, &policy, &device, bad, b"data", 150).is_err());
        let first =
            seal_archive(&pins, &policy, &device, position(), b"same content", 150).unwrap();
        let second =
            seal_archive(&pins, &policy, &device, position(), b"same content", 150).unwrap();
        assert_ne!(first.ciphertext, second.ciphertext);
        assert_ne!(
            first.recipient_keys[0].wrapped_key,
            second.recipient_keys[0].wrapped_key
        );
    }
}
