use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

use crate::{Ed25519PublicKey, ProtocolError, Result};

pub const PROTOCOL_VERSION: u32 = 1;
pub const MAX_ENROLLMENT_TTL_SECONDS: i64 = 24 * 60 * 60;
pub const MAX_ASSERTION_TTL_SECONDS: i64 = 60;
pub const PROOF_CLOCK_SKEW_SECONDS: i64 = 5;
/// Bounded tolerance for server- and controller-issued times checked against a device clock.
pub const MAX_CLOCK_SKEW_SECONDS: i64 = 120;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct OnboardingManifest {
    pub version: u32,
    pub enrollment_id: String,
    pub device_id: String,
    pub owner_id: String,
    pub name: String,
    pub api_base_url: String,
    pub bootstrap_key: Ed25519PublicKey,
    pub controller_key: Ed25519PublicKey,
    pub owner_invitation_key: Ed25519PublicKey,
    pub issued_at: i64,
    pub expires_at: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct DeviceIdentity {
    pub auth_key: Ed25519PublicKey,
    pub management_key: [u8; 32],
    pub telemetry_key: Ed25519PublicKey,
}

impl DeviceIdentity {
    pub fn validate(&self) -> Result<()> {
        let auth = self.auth_key.to_bytes()?;
        let telemetry = self.telemetry_key.to_bytes()?;
        if auth == telemetry || auth == self.management_key || telemetry == self.management_key {
            return Err(ProtocolError::Invalid(
                "device keys must have separate purposes",
            ));
        }
        validate_management_key(&self.management_key)?;
        Ok(())
    }

    /// Human-comparable identity: the first 16 base64url characters of a domain-separated
    /// SHA-256 over the three raw public keys. The UI computes the same value.
    pub fn fingerprint(&self) -> Result<String> {
        let mut hasher = Sha256::new();
        hasher.update(IDENTITY_FINGERPRINT_DOMAIN);
        hasher.update(self.auth_key.to_bytes()?);
        hasher.update(self.telemetry_key.to_bytes()?);
        hasher.update(self.management_key);
        let mut encoded = URL_SAFE_NO_PAD.encode(hasher.finalize());
        encoded.truncate(IDENTITY_FINGERPRINT_CHARS);
        Ok(encoded)
    }
}

const IDENTITY_FINGERPRINT_DOMAIN: &[u8] = b"flow-like-device-identity-v1";
const IDENTITY_FINGERPRINT_CHARS: usize = 16;

/// A public test scalar detects low-order inputs without handling any private key.
pub fn validate_management_key(key: &[u8; 32]) -> Result<()> {
    if x25519_dalek::x25519([42; 32], *key) == [0; 32] {
        return Err(ProtocolError::Invalid("low-order Noise management key"));
    }
    Ok(())
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct EnrollmentBinding {
    pub version: u32,
    pub enrollment_id: String,
    pub device_id: String,
    pub identity: DeviceIdentity,
    pub manifest_digest: String,
    pub challenge_id: String,
    pub challenge_nonce: String,
    pub issued_at: i64,
    pub expires_at: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ClientAssertion {
    pub iss: String,
    pub sub: String,
    pub aud: String,
    pub iat: i64,
    pub nbf: i64,
    pub exp: i64,
    pub jti: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct EnrollmentProof {
    pub iss: String,
    pub sub: String,
    pub aud: String,
    pub iat: i64,
    pub nbf: i64,
    pub exp: i64,
    pub jti: String,
    pub challenge_id: String,
    pub challenge_nonce: String,
    pub binding_digest: String,
    pub manifest_digest: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct DpopProof {
    pub jti: String,
    pub htm: String,
    pub htu: String,
    pub iat: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ath: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub nonce: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CreateEnrollmentRequest {
    pub name: String,
    pub api_base_url: String,
    pub bootstrap_key: Ed25519PublicKey,
    pub controller_key: Ed25519PublicKey,
    pub owner_invitation_key: Ed25519PublicKey,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateEnrollmentResponse {
    pub enrollment_token: String,
    pub manifest: OnboardingManifest,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChallengeRequest {
    pub enrollment_token: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct EnrollmentChallenge {
    pub challenge_id: String,
    pub nonce: String,
    pub expires_at: i64,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RedeemEnrollmentRequest {
    pub enrollment_token: String,
    pub manifest_jws: String,
    pub binding_jws: String,
    pub proof_jws: String,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeviceTokenRequest {
    pub device_id: String,
    pub client_assertion: String,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeviceTokenResponse {
    pub access_token: String,
    pub token_type: String,
    pub expires_in: u64,
    pub expires_at: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct DeviceReceipt {
    pub enrollment_id: String,
    pub device_id: String,
    pub owner_id: String,
    pub name: String,
    pub identity: DeviceIdentity,
    pub manifest_jws: String,
    pub binding_jws: String,
    pub registered_at: i64,
    pub auth_epoch: u64,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReceiptRequest {
    pub client_assertion: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct DeviceHeartbeat {
    pub version: u32,
    pub boot_id: String,
    pub agent_version: String,
    pub uptime_seconds: u64,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DeviceRegistrationStatus {
    Active,
    Revoked,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct DeviceStatus {
    pub device_id: String,
    pub owner_id: String,
    pub name: String,
    pub identity: DeviceIdentity,
    pub status: DeviceRegistrationStatus,
    pub registered_at: i64,
    pub last_seen_at: Option<i64>,
    pub auth_epoch: u64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SigningKey;

    #[derive(Deserialize)]
    struct FingerprintVector {
        identity: DeviceIdentity,
        fingerprint: String,
    }

    #[test]
    fn identity_fingerprint_matches_shared_vector() {
        let vector: FingerprintVector =
            serde_json::from_str(include_str!("../fixtures/identity-fingerprint.json")).unwrap();
        vector.identity.validate().unwrap();
        assert_eq!(vector.identity.fingerprint().unwrap(), vector.fingerprint);
        assert_eq!(vector.fingerprint.len(), IDENTITY_FINGERPRINT_CHARS);

        let derived = DeviceIdentity {
            auth_key: SigningKey::from_bytes(&[1; 32]).public_key(),
            management_key: x25519_dalek::x25519([3; 32], x25519_dalek::X25519_BASEPOINT_BYTES),
            telemetry_key: SigningKey::from_bytes(&[2; 32]).public_key(),
        };
        assert_eq!(derived, vector.identity);
    }

    #[test]
    fn identity_fingerprint_binds_every_key() {
        let identity = DeviceIdentity {
            auth_key: SigningKey::from_bytes(&[1; 32]).public_key(),
            management_key: x25519_dalek::x25519([3; 32], x25519_dalek::X25519_BASEPOINT_BYTES),
            telemetry_key: SigningKey::from_bytes(&[2; 32]).public_key(),
        };
        let original = identity.fingerprint().unwrap();
        let swapped = DeviceIdentity {
            auth_key: identity.telemetry_key.clone(),
            telemetry_key: identity.auth_key.clone(),
            ..identity.clone()
        };
        let other_management = DeviceIdentity {
            management_key: x25519_dalek::x25519([4; 32], x25519_dalek::X25519_BASEPOINT_BYTES),
            ..identity.clone()
        };
        assert_ne!(swapped.fingerprint().unwrap(), original);
        assert_ne!(other_management.fingerprint().unwrap(), original);
    }
}
