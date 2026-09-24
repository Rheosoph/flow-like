use crate::{noise, vault};
use anyhow::{Context, Result, ensure};
use flow_like_device_protocol::{
    ControllerCertificate, Ed25519PublicKey, ManagementPolicy, SigningKey, TelemetryMember,
    TelemetryRoster, sign_controller_certificate, sign_management_policy, sign_telemetry_roster,
    validate_management_id,
};
use rand_core::{OsRng, RngCore};
use serde::{Deserialize, Serialize};
use zeroize::{Zeroize, Zeroizing};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ControllerPublic {
    pub device_id: String,
    pub endpoint_id: String,
    pub controller_key: Ed25519PublicKey,
    pub archive_key: [u8; 32],
    pub telemetry_member: TelemetryMember,
}

#[derive(Serialize)]
pub struct ControllerVault {
    pub public_bundle: ControllerPublic,
    pub vault: Vec<u8>,
}

#[derive(Serialize)]
pub struct InvitationVault {
    pub public_key: Ed25519PublicKey,
    pub vault: Vec<u8>,
}

#[derive(Serialize)]
pub struct RewrappedControllerVaults {
    pub controller: ControllerVault,
    pub invitation: Option<InvitationVault>,
}

#[derive(Serialize)]
pub struct OnboardingVaults {
    pub controller: ControllerVault,
    pub invitation: InvitationVault,
}

#[derive(Serialize)]
pub struct CompletedOnboarding {
    pub controller: ControllerVault,
    pub invitation_vault: Vec<u8>,
    pub manifest_jws: String,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ControllerSecrets {
    version: u32,
    #[serde(default)]
    onboarding: bool,
    device_id: String,
    endpoint_id: String,
    controller_seed: [u8; 32],
    archive_seed: [u8; 32],
    telemetry_seed: [u8; 32],
    storage_key: [u8; 32],
}

impl Drop for ControllerSecrets {
    fn drop(&mut self) {
        self.controller_seed.zeroize();
        self.archive_seed.zeroize();
        self.telemetry_seed.zeroize();
        self.storage_key.zeroize();
    }
}

impl ControllerSecrets {
    fn public(&self) -> ControllerPublic {
        ControllerPublic {
            device_id: self.device_id.clone(),
            endpoint_id: self.endpoint_id.clone(),
            controller_key: SigningKey::from_bytes(&self.controller_seed).public_key(),
            archive_key: x25519_dalek::x25519(
                self.archive_seed,
                x25519_dalek::X25519_BASEPOINT_BYTES,
            ),
            telemetry_member: TelemetryMember {
                endpoint_id: self.endpoint_id.clone(),
                signing_key: SigningKey::from_bytes(&self.telemetry_seed).public_key(),
            },
        }
    }
}

/// Dropping this session clears its owned seeds. An invitation key is never part
/// of an ordinary unlocked controller session.
pub struct UnlockedController {
    secrets: ControllerSecrets,
}

pub fn create_controller_vault(device_id: &str, password: &[u8]) -> Result<ControllerVault> {
    create_controller(device_id, password, false)
}

fn create_controller(
    device_id: &str,
    password: &[u8],
    onboarding: bool,
) -> Result<ControllerVault> {
    validate_management_id(device_id)?;
    let secrets = ControllerSecrets {
        version: 1,
        onboarding,
        device_id: device_id.into(),
        endpoint_id: random_id("endpoint"),
        controller_seed: random_key(),
        archive_seed: random_key(),
        telemetry_seed: random_key(),
        storage_key: random_key(),
    };
    let plaintext = Zeroizing::new(serde_json::to_vec(&secrets)?);
    Ok(ControllerVault {
        public_bundle: secrets.public(),
        vault: vault::seal(password, &vault::controller_context(device_id), &plaintext)?,
    })
}

pub fn create_onboarding_vaults(password: &[u8]) -> Result<OnboardingVaults> {
    let device_id = random_id("pending");
    Ok(OnboardingVaults {
        controller: create_controller(&device_id, password, true)?,
        invitation: create_invitation_vault(&device_id, password)?,
    })
}

pub fn unlock_controller_vault(
    device_id: &str,
    password: &[u8],
    ciphertext: &[u8],
) -> Result<UnlockedController> {
    validate_management_id(device_id)?;
    let plaintext = vault::open(password, &vault::controller_context(device_id), ciphertext)?;
    let secrets: ControllerSecrets = serde_json::from_slice(&plaintext)
        .map_err(|_| anyhow::anyhow!("Invalid controller vault payload"))?;
    ensure!(
        secrets.version == 1 && secrets.device_id == device_id,
        "Controller vault binding failed"
    );
    validate_management_id(&secrets.endpoint_id)?;
    ensure!(
        secrets.controller_seed != secrets.telemetry_seed
            && secrets.archive_seed != secrets.controller_seed
            && secrets.archive_seed != secrets.telemetry_seed,
        "Controller keys must be independent"
    );
    Ok(UnlockedController { secrets })
}

/// Re-encrypt an existing endpoint without rotating its authority or MLS state.
/// The caller must atomically replace both ciphertexts before reporting success.
/// Previously exported ciphertext remains usable with its original password.
pub fn rewrap_controller_vaults(
    device_id: &str,
    current_password: &[u8],
    new_password: &[u8],
    controller_ciphertext: &[u8],
    invitation_ciphertext: Option<&[u8]>,
) -> Result<RewrappedControllerVaults> {
    ensure!(
        (12..=4096).contains(&new_password.len()),
        "Use a password between 12 and 4096 bytes"
    );
    let controller = unlock_controller_vault(device_id, current_password, controller_ciphertext)?;
    ensure!(
        !controller.secrets.onboarding,
        "Finish onboarding before changing the password"
    );
    // Authenticate both old envelopes before preparing either replacement.
    let invitation = invitation_ciphertext
        .map(|ciphertext| invitation_key(device_id, current_password, ciphertext))
        .transpose()?;
    let plaintext = Zeroizing::new(serde_json::to_vec(&controller.secrets)?);
    let controller = ControllerVault {
        public_bundle: controller.public_bundle(),
        vault: vault::seal(
            new_password,
            &vault::controller_context(device_id),
            &plaintext,
        )?,
    };
    let invitation = invitation
        .map(|key| {
            let seed = Zeroizing::new(key.to_bytes());
            Ok::<_, anyhow::Error>(InvitationVault {
                public_key: key.public_key(),
                vault: vault::seal(
                    new_password,
                    &vault::invitation_context(device_id),
                    seed.as_ref(),
                )?,
            })
        })
        .transpose()?;
    Ok(RewrappedControllerVaults {
        controller,
        invitation,
    })
}

impl UnlockedController {
    pub fn open_archive(
        &self,
        pins: &crate::archive::ArchivePins,
        bundle: &crate::archive::EncryptedArchive,
        recipient_id: &str,
    ) -> Result<Zeroizing<Vec<u8>>> {
        ensure!(
            pins.device_id == self.secrets.device_id,
            "Archive belongs to another device"
        );
        crate::archive::open_archive(pins, bundle, recipient_id, &self.secrets.archive_seed)
    }
    pub fn public_bundle(&self) -> ControllerPublic {
        self.secrets.public()
    }

    pub(crate) fn sign_recovery(
        &self,
        proof: &flow_like_device_protocol::ControllerRecoveryProof,
    ) -> Result<String> {
        ensure!(
            proof.device_id == self.secrets.device_id && !self.secrets.onboarding,
            "Recovery controller mismatch"
        );
        Ok(flow_like_device_protocol::sign_controller_recovery(
            proof,
            &SigningKey::from_bytes(&self.secrets.controller_seed),
        )?)
    }

    pub fn fresh_endpoint_vault(&mut self, password: &[u8]) -> Result<ControllerVault> {
        ensure!(
            !self.secrets.onboarding,
            "Finish onboarding before restoring a controller"
        );
        let next = ControllerSecrets {
            version: 1,
            onboarding: false,
            device_id: self.secrets.device_id.clone(),
            endpoint_id: random_id("endpoint"),
            controller_seed: self.secrets.controller_seed,
            archive_seed: self.secrets.archive_seed,
            telemetry_seed: random_key(),
            storage_key: random_key(),
        };
        let payload = Zeroizing::new(serde_json::to_vec(&next)?);
        let result = ControllerVault {
            public_bundle: next.public(),
            vault: vault::seal(
                password,
                &vault::controller_context(&next.device_id),
                &payload,
            )?,
        };
        self.secrets = next;
        Ok(result)
    }

    pub fn complete_onboarding(
        &mut self,
        manifest: &flow_like_device_protocol::OnboardingManifest,
        password: &[u8],
        invitation_ciphertext: &[u8],
    ) -> Result<CompletedOnboarding> {
        ensure!(
            self.secrets.onboarding,
            "Controller is already bound to a device"
        );
        ensure!(
            manifest.controller_key == self.public_bundle().controller_key,
            "Onboarding controller key changed"
        );
        let invitation = invitation_key(&self.secrets.device_id, password, invitation_ciphertext)?;
        ensure!(
            invitation.public_key() == manifest.owner_invitation_key,
            "Onboarding invitation key changed"
        );
        let manifest_jws = flow_like_device_protocol::sign_manifest(
            manifest,
            &SigningKey::from_bytes(&self.secrets.controller_seed),
        )?;
        let next = ControllerSecrets {
            version: 1,
            onboarding: false,
            device_id: manifest.device_id.clone(),
            endpoint_id: self.secrets.endpoint_id.clone(),
            controller_seed: self.secrets.controller_seed,
            archive_seed: self.secrets.archive_seed,
            telemetry_seed: self.secrets.telemetry_seed,
            storage_key: self.secrets.storage_key,
        };
        let payload = Zeroizing::new(serde_json::to_vec(&next)?);
        let seed = Zeroizing::new(invitation.to_bytes());
        let result = CompletedOnboarding {
            controller: ControllerVault {
                public_bundle: next.public(),
                vault: vault::seal(
                    password,
                    &vault::controller_context(&manifest.device_id),
                    &payload,
                )?,
            },
            invitation_vault: vault::seal(
                password,
                &vault::invitation_context(&manifest.device_id),
                seed.as_ref(),
            )?,
            manifest_jws,
        };
        self.secrets = next;
        Ok(result)
    }

    pub fn sign_onboarding_manifest(
        &self,
        manifest: &flow_like_device_protocol::OnboardingManifest,
    ) -> Result<String> {
        ensure!(
            manifest.device_id == self.secrets.device_id
                && manifest.controller_key == self.public_bundle().controller_key,
            "Onboarding manifest does not match this controller"
        );
        Ok(flow_like_device_protocol::sign_manifest(
            manifest,
            &SigningKey::from_bytes(&self.secrets.controller_seed),
        )?)
    }

    pub fn begin_noise(
        &self,
        grant_id: &str,
        expected_device: [u8; 32],
        now: i64,
    ) -> Result<CertifiedHandshake> {
        validate_management_id(grant_id)?;
        let secret = Zeroizing::new(random_key());
        let certificate = ControllerCertificate {
            version: 1,
            device_id: self.secrets.device_id.clone(),
            grant_id: grant_id.into(),
            session_id: random_id("session"),
            management_key: x25519_dalek::x25519(*secret, x25519_dalek::X25519_BASEPOINT_BYTES),
            issued_at: now,
            expires_at: now
                .checked_add(300)
                .ok_or_else(|| anyhow::anyhow!("Invalid session time"))?,
        };
        let certificate_jws = sign_controller_certificate(
            &certificate,
            &SigningKey::from_bytes(&self.secrets.controller_seed),
        )?;
        let handshake = noise::Handshake::initiator(
            &secret,
            expected_device,
            &certificate.device_id,
            &certificate.session_id,
        )?;
        Ok(CertifiedHandshake {
            certificate,
            certificate_jws,
            handshake,
        })
    }

    pub fn create_fleet_reader(
        &self,
        api_base_url: String,
        user_id: String,
        revision: u64,
        issued_at: i64,
        expires_at: i64,
    ) -> Result<String> {
        let public = self.public_bundle();
        let reader = flow_like_device_protocol::FleetReader {
            version: 1,
            device_id: public.device_id,
            api_base_url,
            user_id,
            controller_key: public.controller_key,
            archive_key: public.archive_key,
            revision,
            issued_at,
            expires_at,
        };
        Ok(flow_like_device_protocol::sign_fleet_reader(
            &reader,
            &SigningKey::from_bytes(&self.secrets.controller_seed),
        )?)
    }

    pub fn verify_fleet_reader_history(
        &self,
        trusted: &crate::fleet::FleetTrustedContext,
        receipt: &flow_like_device_protocol::DeviceReceipt,
        compact: &str,
    ) -> Result<flow_like_device_protocol::FleetReader> {
        ensure!(
            receipt.device_id == self.secrets.device_id,
            "Fleet device changed"
        );
        crate::fleet::verify_fleet_reader_history(
            trusted,
            receipt,
            compact,
            &self.public_bundle().controller_key,
            &self.secrets.archive_seed,
        )
    }

    pub fn verify_fleet_view(
        &self,
        trusted: &crate::fleet::FleetTrustedContext,
        receipt: &flow_like_device_protocol::DeviceReceipt,
        view: &flow_like_device_protocol::FleetView,
        now: i64,
    ) -> Result<crate::fleet::VerifiedFleetView> {
        ensure!(
            receipt.device_id == self.secrets.device_id,
            "Fleet device changed"
        );
        crate::fleet::verify_fleet_view(
            trusted,
            receipt,
            view,
            &self.public_bundle().controller_key,
            &self.secrets.archive_seed,
            now,
        )
    }

    pub fn open_fleet(
        &self,
        trusted: &crate::fleet::FleetTrustedContext,
        receipt: &flow_like_device_protocol::DeviceReceipt,
        view: &flow_like_device_protocol::FleetView,
        bundle: &flow_like_device_protocol::EncryptedFleetSnapshot,
        now: i64,
    ) -> Result<Zeroizing<Vec<u8>>> {
        ensure!(
            receipt.device_id == self.secrets.device_id,
            "Fleet device changed"
        );
        crate::fleet::open_fleet(
            trusted,
            receipt,
            view,
            bundle,
            &self.public_bundle().controller_key,
            &self.secrets.archive_seed,
            now,
        )
    }

    pub fn seal_inventory(
        &self,
        binding: flow_like_device_protocol::InventoryBinding,
        plaintext: &[u8],
    ) -> Result<flow_like_device_protocol::EncryptedInventory> {
        ensure!(
            binding.device_id == self.secrets.device_id
                && binding.controller_key == self.public_bundle().controller_key,
            "Inventory controller mismatch"
        );
        crate::inventory::seal(&self.secrets.archive_seed, binding, plaintext)
    }

    pub fn open_inventory(
        &self,
        expected: &flow_like_device_protocol::InventoryBinding,
        encrypted: &flow_like_device_protocol::EncryptedInventory,
    ) -> Result<Zeroizing<Vec<u8>>> {
        ensure!(
            expected.device_id == self.secrets.device_id
                && expected.controller_key == self.public_bundle().controller_key,
            "Inventory controller mismatch"
        );
        crate::inventory::open(&self.secrets.archive_seed, expected, encrypted)
    }

    pub(crate) fn storage_key(&self) -> [u8; 32] {
        self.secrets.storage_key
    }

    pub(crate) fn telemetry_key(&self) -> SigningKey {
        SigningKey::from_bytes(&self.secrets.telemetry_seed)
    }
}

pub struct CertifiedHandshake {
    pub certificate: ControllerCertificate,
    pub certificate_jws: String,
    pub handshake: noise::Handshake,
}

pub fn create_invitation_vault(device_id: &str, password: &[u8]) -> Result<InvitationVault> {
    validate_management_id(device_id)?;
    let key = SigningKey::generate();
    let seed = Zeroizing::new(key.to_bytes());
    Ok(InvitationVault {
        public_key: key.public_key(),
        vault: vault::seal(
            password,
            &vault::invitation_context(device_id),
            seed.as_ref(),
        )?,
    })
}

fn invitation_key(device_id: &str, password: &[u8], ciphertext: &[u8]) -> Result<SigningKey> {
    validate_management_id(device_id)?;
    let plaintext = vault::open(password, &vault::invitation_context(device_id), ciphertext)?;
    let seed = Zeroizing::new(
        <[u8; 32]>::try_from(plaintext.as_slice())
            .map_err(|_| anyhow::anyhow!("Invalid invitation vault payload"))?,
    );
    Ok(SigningKey::from_bytes(&seed))
}

pub fn approve_management_policy(
    policy: &ManagementPolicy,
    password: &[u8],
    invitation_vault: &[u8],
) -> Result<String> {
    let key = invitation_key(&policy.device_id, password, invitation_vault)?;
    Ok(sign_management_policy(policy, &key)?)
}

pub fn approve_telemetry_roster(
    roster: &TelemetryRoster,
    password: &[u8],
    invitation_vault: &[u8],
) -> Result<String> {
    let key = invitation_key(&roster.device_id, password, invitation_vault)?;
    Ok(sign_telemetry_roster(roster, &key)?)
}

pub fn approve_archive_roster(
    roster: &flow_like_device_protocol::ArchiveRoster,
    password: &[u8],
    invitation_vault: &[u8],
) -> Result<String> {
    let key = invitation_key(&roster.device_id, password, invitation_vault)?;
    Ok(flow_like_device_protocol::sign_archive_roster(
        roster, &key,
    )?)
}

pub(crate) fn random_key() -> [u8; 32] {
    let mut key = [0; 32];
    OsRng.fill_bytes(&mut key);
    key
}

pub fn verify_device_receipt(
    receipt: &flow_like_device_protocol::DeviceReceipt,
    expected_manifest: &str,
    controller_key: &Ed25519PublicKey,
) -> Result<flow_like_device_protocol::OnboardingManifest> {
    use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
    use flow_like_device_protocol::{
        EnrollmentBinding, OnboardingManifest, compact_digest, verify_binding, verify_manifest,
    };
    ensure!(
        receipt.manifest_jws == expected_manifest
            && expected_manifest.len() <= 16_384
            && receipt.binding_jws.len() <= 16_384,
        "Device identity changed from its trusted onboarding manifest"
    );
    fn payload<T: serde::de::DeserializeOwned>(compact: &str) -> Result<T> {
        let encoded = compact
            .split('.')
            .nth(1)
            .context("Invalid signed identity")?;
        Ok(serde_json::from_slice(&URL_SAFE_NO_PAD.decode(encoded)?)
            .map_err(|_| anyhow::anyhow!("Invalid signed identity"))?)
    }
    let template: OnboardingManifest = payload(expected_manifest)?;
    let manifest = verify_manifest(expected_manifest, controller_key, template.issued_at)?;
    let unsigned_binding: EnrollmentBinding = payload(&receipt.binding_jws)?;
    let binding = verify_binding(
        &receipt.binding_jws,
        &manifest.bootstrap_key,
        unsigned_binding.issued_at,
    )?;
    for authority in [
        &manifest.bootstrap_key,
        &manifest.controller_key,
        &manifest.owner_invitation_key,
    ] {
        let key = authority.to_bytes()?;
        ensure!(
            binding.identity.auth_key.to_bytes()? != key
                && binding.identity.telemetry_key.to_bytes()? != key
                && binding.identity.management_key != key,
            "Device permanent keys must be separate from onboarding authorities"
        );
    }
    ensure!(
        binding.device_id == manifest.device_id
            && binding.enrollment_id == manifest.enrollment_id
            && binding.manifest_digest == compact_digest(expected_manifest)
            && binding.identity == receipt.identity
            && receipt.device_id == manifest.device_id
            && receipt.enrollment_id == manifest.enrollment_id
            && receipt.owner_id == manifest.owner_id
            && receipt.name == manifest.name
            && receipt.auth_epoch > 0,
        "Device receipt does not match its signed identity binding"
    );
    Ok(manifest)
}

fn random_id(prefix: &str) -> String {
    use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
    format!("{prefix}-{}", URL_SAFE_NO_PAD.encode(random_key()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use flow_like_device_protocol::{verify_controller_certificate, verify_telemetry_roster};

    #[test]
    fn password_change_keeps_authority_and_opens_the_same_mls_snapshot() {
        use crate::{mls::MemberIdentity, mls_store::MlsPins, prepared_mls::PreparedMlsEndpoint};
        let old_password = b"the previous local password";
        let new_password = b"the replacement local password";
        let original = create_controller_vault("device", old_password).unwrap();
        let invitation = create_invitation_vault("device", old_password).unwrap();
        let unlocked = unlock_controller_vault("device", old_password, &original.vault).unwrap();
        let local = &original.public_bundle.telemetry_member;
        let pins = MlsPins {
            device_id: "device".into(),
            scope: "device".into(),
            owner_invitation_key: invitation.public_key.clone(),
            publisher: MemberIdentity::new(
                b"publisher".to_vec(),
                SigningKey::generate().public_key().to_bytes().unwrap(),
            )
            .unwrap(),
            local: MemberIdentity::new(
                local.endpoint_id.as_bytes().to_vec(),
                local.signing_key.to_bytes().unwrap(),
            )
            .unwrap(),
        };
        let endpoint = PreparedMlsEndpoint::create(
            pins.clone(),
            unlocked.storage_key(),
            &unlocked.telemetry_key(),
        )
        .unwrap();
        let persisted = endpoint.prepared().unwrap();
        let changed = rewrap_controller_vaults(
            "device",
            old_password,
            new_password,
            &original.vault,
            Some(&invitation.vault),
        )
        .unwrap();
        assert_eq!(changed.controller.public_bundle, original.public_bundle);
        assert_ne!(changed.controller.vault, original.vault);
        let next =
            unlock_controller_vault("device", new_password, &changed.controller.vault).unwrap();
        assert_eq!(next.storage_key(), unlocked.storage_key());
        assert_eq!(
            next.telemetry_key().public_key(),
            unlocked.telemetry_key().public_key()
        );
        let reopened = PreparedMlsEndpoint::open(
            pins,
            next.storage_key(),
            persisted.snapshot,
            persisted.checkpoint.clone(),
        )
        .unwrap();
        assert_eq!(reopened.checkpoint(), Some(persisted.checkpoint));
        let changed_invitation = changed.invitation.unwrap();
        assert_eq!(changed_invitation.public_key, invitation.public_key);
        assert_ne!(changed_invitation.vault, invitation.vault);
        assert_eq!(
            invitation_key("device", new_password, &changed_invitation.vault)
                .unwrap()
                .public_key(),
            invitation.public_key
        );
        assert!(
            unlock_controller_vault("device", old_password, &changed.controller.vault).is_err()
        );
        assert!(invitation_key("device", old_password, &changed_invitation.vault).is_err());
        // Exported backups are not revoked by rewrapping the local copy.
        assert!(unlock_controller_vault("device", old_password, &original.vault).is_ok());
        assert!(invitation_key("device", old_password, &invitation.vault).is_ok());
    }

    #[test]
    fn password_change_authenticates_both_envelopes_and_supports_shared_users() {
        let old = b"the previous local password";
        let new = b"the replacement local password";
        let original = create_controller_vault("device", old).unwrap();
        let invitation = create_invitation_vault("device", old).unwrap();
        let changed = rewrap_controller_vaults("device", old, new, &original.vault, None).unwrap();
        assert!(changed.invitation.is_none());
        assert_eq!(
            unlock_controller_vault("device", new, &changed.controller.vault)
                .unwrap()
                .public_bundle(),
            original.public_bundle
        );
        for (device, password, next, invitation_bytes) in [
            (
                "device",
                b"incorrect password".as_slice(),
                new.as_slice(),
                Some(invitation.vault.as_slice()),
            ),
            (
                "other-device",
                old.as_slice(),
                new.as_slice(),
                Some(invitation.vault.as_slice()),
            ),
            (
                "device",
                old.as_slice(),
                b"short".as_slice(),
                Some(invitation.vault.as_slice()),
            ),
            (
                "device",
                old.as_slice(),
                new.as_slice(),
                Some(original.vault.as_slice()),
            ),
        ] {
            assert!(
                rewrap_controller_vaults(device, password, next, &original.vault, invitation_bytes)
                    .is_err()
            );
        }
        let mut damaged = invitation.vault.clone();
        *damaged.last_mut().unwrap() ^= 1;
        assert!(
            rewrap_controller_vaults("device", old, new, &original.vault, Some(&damaged)).is_err()
        );
        assert!(unlock_controller_vault("device", old, &original.vault).is_ok());
        let pending = create_onboarding_vaults(old).unwrap();
        assert!(
            rewrap_controller_vaults(
                &pending.controller.public_bundle.device_id,
                old,
                new,
                &pending.controller.vault,
                None
            )
            .is_err()
        );
    }

    #[test]
    fn fresh_browser_endpoints_have_independent_keys_and_separate_invitation_unlock() {
        let password = b"a memorable local password";
        let first = create_controller_vault("device", password).unwrap();
        let second = create_controller_vault("device", password).unwrap();
        assert_ne!(
            first.public_bundle.endpoint_id,
            second.public_bundle.endpoint_id
        );
        assert_ne!(
            first.public_bundle.telemetry_member,
            second.public_bundle.telemetry_member
        );
        let controller = unlock_controller_vault("device", password, &first.vault).unwrap();
        assert_eq!(controller.public_bundle(), first.public_bundle);
        assert!(unlock_controller_vault("different-device", password, &first.vault).is_err());
        let invitation = create_invitation_vault("device", password).unwrap();
        assert!(unlock_controller_vault("device", password, &invitation.vault).is_err());
        let roster = TelemetryRoster {
            version: 1,
            device_id: "device".into(),
            scope: "device".into(),
            policy_version: 1,
            previous_policy_digest: None,
            management_policy_digest: None,
            publisher: first.public_bundle.telemetry_member.clone(),
            members: vec![first.public_bundle.telemetry_member.clone()],
            issued_at: 100,
            expires_at: 200,
        };
        assert!(approve_telemetry_roster(&roster, password, &first.vault).is_err());
        assert!(approve_telemetry_roster(&roster, b"wrong password", &invitation.vault).is_err());
        let signed = approve_telemetry_roster(&roster, password, &invitation.vault).unwrap();
        assert_eq!(
            verify_telemetry_roster(&signed, &invitation.public_key, 100).unwrap(),
            roster
        );
    }

    #[test]
    fn session_certificate_binds_fresh_noise_key_and_pinned_device() {
        let password = b"a memorable local password";
        let sealed = create_controller_vault("device", password).unwrap();
        let controller = unlock_controller_vault("device", password, &sealed.vault).unwrap();
        let device_secret = random_key();
        let device_public =
            x25519_dalek::x25519(device_secret, x25519_dalek::X25519_BASEPOINT_BYTES);
        let first = controller.begin_noise("owner", device_public, 100).unwrap();
        let second = controller.begin_noise("owner", device_public, 100).unwrap();
        assert_ne!(
            first.certificate.management_key,
            second.certificate.management_key
        );
        let certificate = verify_controller_certificate(
            &first.certificate_jws,
            &sealed.public_bundle.controller_key,
            100,
        )
        .unwrap();
        let mut reader = first.handshake;
        let mut device = noise::Handshake::responder(
            &device_secret,
            certificate.management_key,
            "device",
            &certificate.session_id,
        )
        .unwrap();
        device.read(&reader.write().unwrap()).unwrap();
        reader.read(&device.write().unwrap()).unwrap();
        device.read(&reader.write().unwrap()).unwrap();
        let mut reader = reader.finish().unwrap();
        let mut device = device.finish().unwrap();
        assert_eq!(
            device
                .decrypt(&reader.encrypt(b"inspect").unwrap())
                .unwrap(),
            b"inspect"
        );
    }

    #[test]
    fn restored_controller_keeps_authority_but_creates_fresh_private_mls_state() {
        let password = b"a memorable local password";
        let original = create_controller_vault("device", password).unwrap();
        let mut restored = unlock_controller_vault("device", password, &original.vault).unwrap();
        let old_storage = restored.storage_key();
        let inventory_binding = flow_like_device_protocol::InventoryBinding {
            issuer: "issuer".into(),
            api_origin: "https://hub.example".into(),
            account_id: "owner".into(),
            device_id: "device".into(),
            controller_key: original.public_bundle.controller_key.clone(),
            scope: flow_like_device_protocol::ManagementScope::Device,
            revision: 1,
        };
        let inventory = restored
            .seal_inventory(inventory_binding.clone(), b"retained deployment")
            .unwrap();
        let fresh = restored.fresh_endpoint_vault(password).unwrap();
        assert_eq!(
            &**restored
                .open_inventory(&inventory_binding, &inventory)
                .unwrap(),
            b"retained deployment"
        );
        assert_eq!(
            fresh.public_bundle.controller_key,
            original.public_bundle.controller_key
        );
        assert_eq!(
            fresh.public_bundle.archive_key,
            original.public_bundle.archive_key
        );
        assert_ne!(
            fresh.public_bundle.endpoint_id,
            original.public_bundle.endpoint_id
        );
        assert_ne!(
            fresh.public_bundle.telemetry_member,
            original.public_bundle.telemetry_member
        );
        assert_ne!(restored.storage_key(), old_storage);
        assert_eq!(
            unlock_controller_vault("device", password, &fresh.vault)
                .unwrap()
                .public_bundle(),
            fresh.public_bundle
        );
    }

    #[test]
    fn provisional_vaults_bind_only_once_to_exact_onboarding_authorities() {
        let password = b"a memorable local password";
        let vaults = create_onboarding_vaults(password).unwrap();
        let temporary = vaults.controller.public_bundle.device_id.clone();
        let mut controller =
            unlock_controller_vault(&temporary, password, &vaults.controller.vault).unwrap();
        let mut manifest = flow_like_device_protocol::OnboardingManifest {
            version: 1,
            enrollment_id: "enrollment".into(),
            device_id: "registered-device".into(),
            owner_id: "owner".into(),
            name: "test device".into(),
            api_base_url: "https://example.com/api/v1".into(),
            bootstrap_key: SigningKey::generate().public_key(),
            controller_key: vaults.controller.public_bundle.controller_key.clone(),
            owner_invitation_key: vaults.invitation.public_key.clone(),
            issued_at: 100,
            expires_at: 200,
        };
        let saved = manifest.controller_key.clone();
        manifest.controller_key = SigningKey::generate().public_key();
        assert!(
            controller
                .complete_onboarding(&manifest, password, &vaults.invitation.vault)
                .is_err()
        );
        manifest.controller_key = saved;
        let complete = controller
            .complete_onboarding(&manifest, password, &vaults.invitation.vault)
            .unwrap();
        assert_eq!(
            flow_like_device_protocol::verify_manifest(
                &complete.manifest_jws,
                &manifest.controller_key,
                100
            )
            .unwrap(),
            manifest
        );
        assert!(unlock_controller_vault(&temporary, password, &complete.controller.vault).is_err());
        assert_eq!(
            unlock_controller_vault("registered-device", password, &complete.controller.vault)
                .unwrap()
                .public_bundle(),
            complete.controller.public_bundle
        );
        assert!(
            controller
                .complete_onboarding(&manifest, password, &vaults.invitation.vault)
                .is_err()
        );
        assert!(
            vault::open(
                password,
                &vault::controller_context("registered-device"),
                &complete.invitation_vault
            )
            .is_err()
        );
    }
}
