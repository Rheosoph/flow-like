use crate::{
    controller::{ControllerPublic, unlock_controller_vault},
    vault,
};
use anyhow::{Result, ensure};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use flow_like_device_protocol::{
    Ed25519PublicKey, OnboardingManifest, SigningKey, validate_management_id, verify_manifest,
};
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecoveryContext {
    pub issuer: String,
    pub account: String,
    pub api_origin: String,
    pub device_id: String,
    pub controller_key: Ed25519PublicKey,
    pub revision: u64,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RecoveryBackup {
    pub version: u32,
    pub api_origin: String,
    pub device_id: String,
    pub controller_public: ControllerPublic,
    pub controller_vault: Vec<u8>,
    pub invitation_vault: Option<Vec<u8>>,
    pub manifest_jws: String,
    pub grant_id: String,
    pub owner_controller_key: Option<Ed25519PublicKey>,
}

fn context(value: &RecoveryContext) -> Result<Vec<u8>> {
    validate_management_id(&value.device_id)?;
    value.controller_key.validate()?;
    ensure!(
        !value.account.is_empty()
            && value.account.len() <= 1024
            && !value.issuer.is_empty()
            && value.issuer.len() <= 2048
            && !value.api_origin.is_empty()
            && value.api_origin.len() <= 2048
            && (1..=9_007_199_254_740_991).contains(&value.revision),
        "Invalid account recovery binding"
    );
    let mut aad = b"flow-like/standalone/account-recovery/v1\0".to_vec();
    aad.extend(serde_json::to_vec(value)?);
    Ok(aad)
}

fn validate(value: &RecoveryBackup, scope: &RecoveryContext, password: &[u8]) -> Result<()> {
    ensure!(
        value.version == 1
            && value.api_origin == scope.api_origin
            && value.device_id == scope.device_id
            && value.controller_public.device_id == scope.device_id
            && value.controller_public.controller_key == scope.controller_key
            && value.manifest_jws.len() <= 16_384,
        "Recovery backup belongs to another account or device"
    );
    validate_management_id(&value.grant_id)?;
    let controller = unlock_controller_vault(&scope.device_id, password, &value.controller_vault)?;
    ensure!(
        controller.public_bundle() == value.controller_public,
        "Recovery controller identity changed"
    );
    let encoded = value
        .manifest_jws
        .split('.')
        .nth(1)
        .ok_or_else(|| anyhow::anyhow!("Invalid recovery manifest"))?;
    let template: OnboardingManifest = serde_json::from_slice(&URL_SAFE_NO_PAD.decode(encoded)?)?;
    let manifest = verify_manifest(
        &value.manifest_jws,
        value
            .owner_controller_key
            .as_ref()
            .unwrap_or(&scope.controller_key),
        template.issued_at,
    )?;
    ensure!(
        manifest.device_id == scope.device_id
            && manifest.api_base_url
                == format!("{}/api/v1", scope.api_origin.trim_end_matches('/')),
        "Recovery manifest belongs to another hub or device"
    );
    if value.grant_id == "owner" {
        ensure!(
            manifest.owner_id == scope.account && manifest.controller_key == scope.controller_key,
            "Recovery owner identity changed"
        );
    } else {
        ensure!(
            value.owner_controller_key.is_some() && value.invitation_vault.is_none(),
            "Shared recovery cannot contain owner invitation authority"
        );
    }
    if let Some(ciphertext) = &value.invitation_vault {
        let seed = vault::open(
            password,
            &vault::invitation_context(&scope.device_id),
            ciphertext,
        )?;
        let seed: Zeroizing<[u8; 32]> = Zeroizing::new(
            seed.as_slice()
                .try_into()
                .map_err(|_| anyhow::anyhow!("Invalid recovery invitation key"))?,
        );
        ensure!(
            SigningKey::from_bytes(&seed).public_key() == manifest.owner_invitation_key,
            "Recovery invitation authority changed"
        );
    }
    Ok(())
}

#[derive(Serialize)]
pub struct SealedRecoveryBackup {
    pub ciphertext: Vec<u8>,
    pub proof_jws: String,
}

pub fn seal(
    scope: &RecoveryContext,
    password: &[u8],
    backup: &[u8],
) -> Result<SealedRecoveryBackup> {
    ensure!(
        backup.len() <= 64_000,
        "Account recovery backup exceeds its limit"
    );
    let aad = context(scope)?;
    let value: RecoveryBackup = serde_json::from_slice(backup)?;
    validate(&value, scope, password)?;
    let ciphertext = vault::seal(password, &aad, backup)?;
    let proof = flow_like_device_protocol::ControllerRecoveryProof {
        version: 1,
        api_base_url: format!("{}/api/v1", scope.api_origin.trim_end_matches('/')),
        user_id: scope.account.clone(),
        device_id: scope.device_id.clone(),
        revision: scope.revision,
        ciphertext_sha256: flow_like_device_protocol::recovery_ciphertext_digest(&ciphertext),
    };
    let proof_jws = unlock_controller_vault(&scope.device_id, password, &value.controller_vault)?
        .sign_recovery(&proof)?;
    Ok(SealedRecoveryBackup {
        ciphertext,
        proof_jws,
    })
}

pub fn open(scope: &RecoveryContext, password: &[u8], ciphertext: &[u8]) -> Result<RecoveryBackup> {
    let plaintext = vault::open(password, &context(scope)?, ciphertext)?;
    let value: RecoveryBackup = serde_json::from_slice(&plaintext)?;
    validate(&value, scope, password)?;
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::controller::{create_onboarding_vaults, unlock_controller_vault};

    fn fixture() -> (RecoveryContext, Vec<u8>) {
        let password = b"test recovery password";
        let initial = create_onboarding_vaults(password).unwrap();
        let mut unlocked = unlock_controller_vault(
            &initial.controller.public_bundle.device_id,
            password,
            &initial.controller.vault,
        )
        .unwrap();
        let manifest = OnboardingManifest {
            version: 1,
            enrollment_id: "enrollment".into(),
            device_id: "device".into(),
            owner_id: "owner".into(),
            name: "Recovery test".into(),
            api_base_url: "https://api.example/api/v1".into(),
            bootstrap_key: SigningKey::generate().public_key(),
            controller_key: initial.controller.public_bundle.controller_key,
            owner_invitation_key: initial.invitation.public_key,
            issued_at: 100,
            expires_at: 200,
        };
        let completed = unlocked
            .complete_onboarding(&manifest, password, &initial.invitation.vault)
            .unwrap();
        let scope = RecoveryContext {
            issuer: "https://auth.example".into(),
            account: "owner".into(),
            api_origin: "https://api.example".into(),
            device_id: "device".into(),
            controller_key: completed.controller.public_bundle.controller_key.clone(),
            revision: 1,
        };
        let backup = RecoveryBackup {
            version: 1,
            api_origin: scope.api_origin.clone(),
            device_id: scope.device_id.clone(),
            controller_public: completed.controller.public_bundle,
            controller_vault: completed.controller.vault,
            invitation_vault: Some(completed.invitation_vault),
            manifest_jws: completed.manifest_jws,
            grant_id: "owner".into(),
            owner_controller_key: None,
        };
        (scope, serde_json::to_vec(&backup).unwrap())
    }

    #[test]
    fn recovery_requires_password_and_exact_account_device_revision_binding() {
        let (scope, backup) = fixture();
        let password = b"test recovery password";
        let encrypted = seal(&scope, password, &backup).unwrap();
        let proof = flow_like_device_protocol::verify_controller_recovery(
            &encrypted.proof_jws,
            &scope.controller_key,
        )
        .unwrap();
        let ciphertext = encrypted.ciphertext;
        assert_eq!(
            proof.ciphertext_sha256,
            flow_like_device_protocol::recovery_ciphertext_digest(&ciphertext)
        );
        let recovered = open(&scope, password, &ciphertext).unwrap();
        assert_eq!(
            recovered.controller_public.controller_key,
            scope.controller_key
        );
        assert!(open(&scope, b"wrong recovery password", &ciphertext).is_err());
        for field in ["account", "issuer", "api", "device", "revision"] {
            let mut other = scope.clone();
            match field {
                "account" => other.account = "other".into(),
                "issuer" => other.issuer.push('x'),
                "api" => other.api_origin.push('x'),
                "device" => other.device_id = "other".into(),
                _ => other.revision += 1,
            }
            assert!(open(&other, password, &ciphertext).is_err());
        }
        let mut changed = ciphertext;
        *changed.last_mut().unwrap() ^= 1;
        assert!(open(&scope, password, &changed).is_err());
        assert!(seal(&scope, b"wrong recovery password", &backup).is_err());
    }
}
