use anyhow::{Result, ensure};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use flow_like_device_protocol::*;
use openmls_rust_crypto::OpenMlsRustCrypto;
use openmls_traits::{
    OpenMlsProvider,
    crypto::OpenMlsCrypto,
    types::{HpkeAeadType, HpkeCiphertext, HpkeConfig, HpkeKdfType, HpkeKemType},
};
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FleetTrustedContext {
    pub api_base_url: String,
    pub user_id: String,
    pub onboarding_manifest_jws: String,
    pub owner_controller_key: Ed25519PublicKey,
}
fn suite() -> HpkeConfig {
    HpkeConfig(
        HpkeKemType::DhKem25519,
        HpkeKdfType::HkdfSha256,
        HpkeAeadType::ChaCha20Poly1305,
    )
}
fn context(manifest: &FleetManifest) -> Result<Vec<u8>> {
    Ok(serde_json::to_vec(&(
        "flow-like-fleet-snapshot-v1",
        &manifest.device_id,
        &manifest.api_base_url,
        &manifest.user_id,
        &manifest.controller_key,
        &manifest.audience,
        manifest.sequence,
        &manifest.previous_digest,
        &manifest.boot_id,
        manifest.observed_at,
    ))?)
}
pub fn seal_fleet(
    mut manifest: FleetManifest,
    reader: &FleetReader,
    signer: &SigningKey,
    snapshot: &FleetSnapshot,
) -> Result<EncryptedFleetSnapshot> {
    ensure!(
        manifest.device_id == reader.device_id
            && manifest.api_base_url == reader.api_base_url
            && manifest.user_id == reader.user_id
            && manifest.controller_key == reader.controller_key,
        "Fleet reader scope changed"
    );
    snapshot.validate(&manifest)?;
    let plaintext = Zeroizing::new(serde_json::to_vec(snapshot)?);
    ensure!(
        plaintext.len() <= MAX_FLEET_PLAINTEXT,
        "Fleet snapshot exceeds retention limit; reduce the number of deployments in this scope"
    );
    let context = context(&manifest)?;
    let ciphertext = OpenMlsRustCrypto::default().crypto().hpke_seal(
        suite(),
        &reader.archive_key,
        &context,
        &context,
        &plaintext,
    )?;
    manifest.encapsulation = ciphertext.kem_output.as_slice().try_into()?;
    manifest.ciphertext_size = ciphertext.ciphertext.as_slice().len().try_into()?;
    manifest.ciphertext_digest = fleet_digest(ciphertext.ciphertext.as_slice());
    Ok(EncryptedFleetSnapshot {
        manifest_jws: sign_fleet_manifest(&manifest, signer)?,
        ciphertext: URL_SAFE_NO_PAD.encode(ciphertext.ciphertext.as_slice()),
    })
}
pub fn verify_fleet_reader_history(
    trusted: &FleetTrustedContext,
    receipt: &DeviceReceipt,
    compact: &str,
    controller_key: &Ed25519PublicKey,
    archive_seed: &[u8; 32],
) -> Result<FleetReader> {
    let onboarding = crate::controller::verify_device_receipt(
        receipt,
        &trusted.onboarding_manifest_jws,
        &trusted.owner_controller_key,
    )?;
    ensure!(compact.len() <= 16_384, "Fleet reader exceeds limit");
    let raw: FleetReader = serde_json::from_slice(
        &URL_SAFE_NO_PAD.decode(
            compact
                .split('.')
                .nth(1)
                .ok_or_else(|| anyhow::anyhow!("Invalid fleet reader"))?,
        )?,
    )?;
    let reader = verify_fleet_reader(compact, controller_key, raw.issued_at)?;
    ensure!(
        onboarding.api_base_url == trusted.api_base_url
            && reader.device_id == onboarding.device_id
            && reader.api_base_url == trusted.api_base_url
            && reader.user_id == trusted.user_id
            && reader.archive_key
                == x25519_dalek::x25519(*archive_seed, x25519_dalek::X25519_BASEPOINT_BYTES),
        "Fleet reader identity changed"
    );
    Ok(reader)
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VerifiedFleetView {
    pub reader_revision: u64,
    pub reader_expires_at: i64,
    pub audiences: Vec<FleetAudience>,
}
pub fn verify_fleet_view(
    trusted: &FleetTrustedContext,
    receipt: &DeviceReceipt,
    view: &FleetView,
    controller_key: &Ed25519PublicKey,
    archive_seed: &[u8; 32],
    now: i64,
) -> Result<VerifiedFleetView> {
    let onboarding = crate::controller::verify_device_receipt(
        receipt,
        &trusted.onboarding_manifest_jws,
        &trusted.owner_controller_key,
    )?;
    ensure!(
        onboarding.api_base_url == trusted.api_base_url,
        "Fleet API authority changed"
    );
    let reader = verify_fleet_reader(&view.reader_jws, controller_key, now)?;
    ensure!(
        reader.device_id == onboarding.device_id
            && reader.user_id == trusted.user_id
            && reader.api_base_url == trusted.api_base_url
            && reader.archive_key
                == x25519_dalek::x25519(*archive_seed, x25519_dalek::X25519_BASEPOINT_BYTES),
        "Fleet reader identity changed"
    );
    let policy = view
        .policy_jws
        .as_ref()
        .map(|jws| {
            verify_management_policy(jws, &onboarding.owner_invitation_key, now)
                .map(|p| (jws.as_str(), p))
        })
        .transpose()?;
    let audiences = fleet_audiences(
        &reader,
        &view.reader_jws,
        &onboarding,
        policy.as_ref().map(|(j, p)| (*j, p)),
        now,
    );
    ensure!(
        !audiences.is_empty(),
        "Fleet reader has no current status or metrics grant"
    );
    Ok(VerifiedFleetView {
        reader_revision: reader.revision,
        reader_expires_at: reader.expires_at,
        audiences,
    })
}
#[allow(clippy::too_many_arguments)]
pub fn open_fleet(
    trusted: &FleetTrustedContext,
    receipt: &DeviceReceipt,
    view: &FleetView,
    bundle: &EncryptedFleetSnapshot,
    controller_key: &Ed25519PublicKey,
    archive_seed: &[u8; 32],
    now: i64,
) -> Result<Zeroizing<Vec<u8>>> {
    let verified = verify_fleet_view(trusted, receipt, view, controller_key, archive_seed, now)?;
    let manifest = verify_fleet_snapshot(bundle, &receipt.identity.telemetry_key)?;
    ensure!(
        manifest.device_id == receipt.device_id
            && manifest.api_base_url == trusted.api_base_url
            && manifest.user_id == trusted.user_id
            && &manifest.controller_key == controller_key
            && manifest.observed_at <= now + 30
            && verified.audiences.contains(&manifest.audience),
        "Fleet snapshot is outside the current reader authorization"
    );
    let context = context(&manifest)?;
    let ciphertext = HpkeCiphertext {
        kem_output: manifest.encapsulation.to_vec().into(),
        ciphertext: URL_SAFE_NO_PAD.decode(&bundle.ciphertext)?.into(),
    };
    let plaintext = Zeroizing::new(OpenMlsRustCrypto::default().crypto().hpke_open(
        suite(),
        &ciphertext,
        archive_seed,
        &context,
        &context,
    )?);
    let snapshot: FleetSnapshot = serde_json::from_slice(&plaintext)?;
    snapshot.validate(&manifest)?;
    Ok(plaintext)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pinned_bootstrap_hpke_scope_tamper_and_membership_fences() -> Result<()> {
        let owner = SigningKey::generate();
        let bootstrap = SigningKey::generate();
        let invitation = SigningKey::generate();
        let device = SigningKey::generate();
        let controller = SigningKey::generate();
        let archive_seed = [42; 32];
        let onboarding = OnboardingManifest {
            version: 1,
            enrollment_id: "enrollment".into(),
            device_id: "device".into(),
            owner_id: "owner".into(),
            name: "Device".into(),
            api_base_url: "https://hub.example/api/v1".into(),
            bootstrap_key: bootstrap.public_key(),
            controller_key: owner.public_key(),
            owner_invitation_key: invitation.public_key(),
            issued_at: 100,
            expires_at: 200,
        };
        let manifest_jws = sign_manifest(&onboarding, &owner)?;
        let identity = DeviceIdentity {
            auth_key: SigningKey::generate().public_key(),
            management_key: [13; 32],
            telemetry_key: device.public_key(),
        };
        let binding = EnrollmentBinding {
            version: 1,
            enrollment_id: "enrollment".into(),
            device_id: "device".into(),
            identity: identity.clone(),
            manifest_digest: compact_digest(&manifest_jws),
            challenge_id: "challenge".into(),
            challenge_nonce: "abcdefghijklmnopqrstuvwx12345678".into(),
            issued_at: 100,
            expires_at: 150,
        };
        let receipt = DeviceReceipt {
            enrollment_id: "enrollment".into(),
            device_id: "device".into(),
            owner_id: "owner".into(),
            name: "Device".into(),
            identity,
            manifest_jws: manifest_jws.clone(),
            binding_jws: sign_binding(&binding, &bootstrap)?,
            registered_at: 110,
            auth_epoch: 1,
        };
        let trusted = FleetTrustedContext {
            api_base_url: onboarding.api_base_url.clone(),
            user_id: "reader".into(),
            onboarding_manifest_jws: manifest_jws,
            owner_controller_key: owner.public_key(),
        };
        let reader = FleetReader {
            version: 1,
            device_id: "device".into(),
            api_base_url: onboarding.api_base_url.clone(),
            user_id: "reader".into(),
            controller_key: controller.public_key(),
            archive_key: x25519_dalek::x25519(archive_seed, x25519_dalek::X25519_BASEPOINT_BYTES),
            revision: 1,
            issued_at: 100,
            expires_at: 300,
        };
        let reader_jws = sign_fleet_reader(&reader, &controller)?;
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
                controller_key: controller.public_key(),
                scope: ManagementScope::Project {
                    project_id: "project".into(),
                },
                capabilities: vec![ManagementCapability::Status],
                expires_at: 190,
                group_id: None,
                group_version: None,
            }],
        };
        let policy_jws = sign_management_policy(&policy, &invitation)?;
        let audience = fleet_audiences(
            &reader,
            &reader_jws,
            &onboarding,
            Some((&policy_jws, &policy)),
            110,
        )
        .remove(0);
        let snapshot = FleetSnapshot {
            version: 1,
            observed_at: 110,
            boot_id: "boot".into(),
            inspection: Some(
                serde_json::json!({"placements":[{"id":"placement","project_id":"project"}]}),
            ),
            metrics: None,
        };
        let header = FleetManifest {
            version: 1,
            device_id: "device".into(),
            api_base_url: onboarding.api_base_url.clone(),
            user_id: "reader".into(),
            controller_key: controller.public_key(),
            audience,
            sequence: 1,
            previous_digest: None,
            boot_id: "boot".into(),
            observed_at: 110,
            encapsulation: [0; 32],
            ciphertext_digest: String::new(),
            ciphertext_size: 16,
        };
        let bundle = seal_fleet(header, &reader, &device, &snapshot)?;
        let mut view = FleetView {
            snapshots: vec![bundle.clone()],
            reader_jws: reader_jws.clone(),
            policy_jws: Some(policy_jws),
        };
        let open = |context: &FleetTrustedContext,
                    receipt: &DeviceReceipt,
                    view: &FleetView,
                    bundle: &EncryptedFleetSnapshot,
                    seed: &[u8; 32]| {
            open_fleet(
                context,
                receipt,
                view,
                bundle,
                &controller.public_key(),
                seed,
                120,
            )
        };
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&open(
                &trusted,
                &receipt,
                &view,
                &bundle,
                &archive_seed
            )?)?["inspection"]["placements"][0]["id"],
            "placement"
        );
        assert!(open(&trusted, &receipt, &view, &bundle, &[9; 32]).is_err());
        let mut wrong = trusted.clone();
        wrong.user_id = "other".into();
        assert!(open(&wrong, &receipt, &view, &bundle, &archive_seed).is_err());
        wrong = trusted.clone();
        wrong.api_base_url = "https://other.example/api/v1".into();
        assert!(open(&wrong, &receipt, &view, &bundle, &archive_seed).is_err());
        let mut forged = receipt.clone();
        forged.identity.telemetry_key = SigningKey::generate().public_key();
        assert!(open(&trusted, &forged, &view, &bundle, &archive_seed).is_err());
        let mut tampered = bundle.clone();
        let mut bytes = URL_SAFE_NO_PAD.decode(&tampered.ciphertext)?;
        bytes[0] ^= 1;
        tampered.ciphertext = URL_SAFE_NO_PAD.encode(bytes);
        assert!(open(&trusted, &receipt, &view, &tampered, &archive_seed).is_err());
        let mut retyped = verify_fleet_snapshot(&bundle, &device.public_key())?;
        retyped.audience.kind = FleetKind::Metrics;
        tampered = bundle.clone();
        tampered.manifest_jws = sign_fleet_manifest(&retyped, &device)?;
        assert!(open(&trusted, &receipt, &view, &tampered, &archive_seed).is_err());
        view.snapshots.clear();
        assert_eq!(
            verify_fleet_view(
                &trusted,
                &receipt,
                &view,
                &controller.public_key(),
                &archive_seed,
                120
            )?
            .audiences
            .len(),
            1
        );
        policy.grants.clear();
        view.policy_jws = Some(sign_management_policy(&policy, &invitation)?);
        assert!(open(&trusted, &receipt, &view, &bundle, &archive_seed).is_err());
        assert_eq!(
            verify_fleet_reader_history(
                &trusted,
                &receipt,
                &reader_jws,
                &controller.public_key(),
                &archive_seed
            )?
            .revision,
            1
        );
        Ok(())
    }
}
