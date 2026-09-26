use crate::{
    Ed25519PublicKey, ProtocolError, Result, SigningKey, canonical_api_base_url,
    proof::{sign_pinned, verify_pinned},
    validate_management_id,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ControllerRecoveryProof {
    pub version: u32,
    pub api_base_url: String,
    pub user_id: String,
    pub device_id: String,
    pub revision: u64,
    pub ciphertext_sha256: String,
}

impl ControllerRecoveryProof {
    pub fn validate(&self) -> Result<()> {
        validate_management_id(&self.device_id)?;
        if self.version != 1
            || self.user_id.is_empty()
            || self.user_id.len() > 1024
            || !(1..=9_007_199_254_740_991).contains(&self.revision)
            || canonical_api_base_url(&self.api_base_url)? != self.api_base_url
            || self.ciphertext_sha256.len() != 64
            || !self
                .ciphertext_sha256
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err(ProtocolError::Invalid("controller recovery proof"));
        }
        Ok(())
    }
}

pub fn recovery_ciphertext_digest(ciphertext: &[u8]) -> String {
    format!("{:x}", Sha256::digest(ciphertext))
}

pub fn sign_controller_recovery(
    value: &ControllerRecoveryProof,
    key: &SigningKey,
) -> Result<String> {
    value.validate()?;
    sign_pinned(value, key, "flow-like-controller-recovery+jws")
}

pub fn verify_controller_recovery(
    compact: &str,
    key: &Ed25519PublicKey,
) -> Result<ControllerRecoveryProof> {
    let value: ControllerRecoveryProof =
        verify_pinned(compact, key, "flow-like-controller-recovery+jws")?;
    value.validate()?;
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn backup_replacement_requires_the_controller_key_and_binds_ciphertext() {
        let key = SigningKey::generate();
        let value = ControllerRecoveryProof {
            version: 1,
            api_base_url: "https://api.example/api/v1".into(),
            user_id: "user".into(),
            device_id: "device".into(),
            revision: 2,
            ciphertext_sha256: recovery_ciphertext_digest(b"encrypted backup"),
        };
        let signed = sign_controller_recovery(&value, &key).unwrap();
        assert_eq!(
            verify_controller_recovery(&signed, &key.public_key()).unwrap(),
            value
        );
        assert!(verify_controller_recovery(&signed, &SigningKey::generate().public_key()).is_err());
        assert_ne!(
            value.ciphertext_sha256,
            recovery_ciphertext_digest(b"changed backup")
        );
    }
}
