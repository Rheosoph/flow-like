//! A device binds each process to an approved placement. The process key can
//! renew resource leases without receiving device or human credentials.

use serde::{Deserialize, Serialize};

use crate::{
    ClientAssertion, Ed25519PublicKey, MAX_ASSERTION_TTL_SECONDS, PROTOCOL_VERSION, ProtocolError,
    Result, SigningKey, compact_digest,
    proof::{
        bounded_text, check_assertion_request, decode_exact, sign_pinned, validate_assertion,
        verify_pinned,
    },
};

pub const INSTANCE_REGISTRATION_JWS_TYPE: &str = "flow-like-instance-registration+jwt";
pub const INSTANCE_POSSESSION_JWS_TYPE: &str = "flow-like-instance-possession+jwt";
pub const WORKLOAD_ASSERTION_JWS_TYPE: &str = "flow-like-instance-client-assertion+jwt";
pub const INSTANCE_MODELS_AUDIENCE: &str = "flow-like-model-proxy";
pub const INSTANCE_MODELS_SCOPE: &str = "models:invoke";
pub const MAX_INSTANCE_TOKEN_SECONDS: i64 = 300;
pub const INSTANCE_LEASE_SECONDS: i64 = 600;
pub const INSTANCE_VALIDATION_SECONDS: i64 = 600;
pub const INSTANCE_PROJECT_METADATA_SCOPE: &str = "project:metadata:read";

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum InstancePurpose {
    #[default]
    Workload,
    RolloutValidation,
}

impl InstancePurpose {
    pub fn is_workload(&self) -> bool {
        *self == Self::Workload
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Workload => "workload",
            Self::RolloutValidation => "rollout_validation",
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum OnlineProjectAccess {
    ReadOnly,
    ReadWrite,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CreateResourceGrantRequest {
    pub placement_id: String,
    pub deployment_id: String,
    pub project_id: String,
    pub app_id: Option<String>,
    pub model_ids: Vec<String>,
    pub max_instances: u32,
    pub expires_at: i64,
    #[serde(default)]
    pub online_access: Option<OnlineProjectAccess>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ApproveBillingGrantRequest {
    /// One fixed allowance, in micro EUR, across every instance in the placement.
    pub limit_micros: i64,
    pub expires_at: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ResourceGrantResponse {
    pub grant_id: String,
    pub device_id: String,
    pub placement_id: String,
    pub deployment_id: String,
    pub project_id: String,
    pub app_id: Option<String>,
    pub delegating_user_id: String,
    pub authz_version: u64,
    pub model_ids: Vec<String>,
    pub max_instances: u32,
    pub expires_at: i64,
    pub status: String,
    #[serde(default)]
    pub online_access: Option<OnlineProjectAccess>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct BillingGrantResponse {
    pub billing_grant_id: String,
    pub grant_id: String,
    pub payer_id: String,
    pub authz_version: u64,
    pub limit_micros: i64,
    pub used_micros: i64,
    pub reserved_micros: i64,
    pub expires_at: i64,
    pub status: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct InstanceRegistration {
    pub version: u32,
    #[serde(default, skip_serializing_if = "InstancePurpose::is_workload")]
    pub purpose: InstancePurpose,
    pub device_id: String,
    pub device_auth_epoch: u64,
    pub instance_id: String,
    pub placement_id: String,
    pub deployment_id: String,
    pub project_id: String,
    pub grant_id: String,
    pub authz_version: u64,
    #[serde(default)]
    pub billing_grant_id: Option<String>,
    #[serde(default)]
    pub billing_authz_version: Option<u64>,
    pub workload_key: Ed25519PublicKey,
    pub aud: String,
    pub iat: i64,
    pub nbf: i64,
    pub exp: i64,
    pub jti: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct InstancePossession {
    pub iss: String,
    pub sub: String,
    pub aud: String,
    pub iat: i64,
    pub nbf: i64,
    pub exp: i64,
    pub jti: String,
    pub registration_digest: String,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InstanceRegistrationRequest {
    pub registration_jws: String,
    pub possession_jws: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct InstanceReceipt {
    pub instance_id: String,
    #[serde(default, skip_serializing_if = "InstancePurpose::is_workload")]
    pub purpose: InstancePurpose,
    pub device_id: String,
    pub grant_id: String,
    #[serde(default)]
    pub billing_grant_id: Option<String>,
    pub workload_key: Ed25519PublicKey,
    pub key_epoch: u64,
    pub registered_at: i64,
    pub lease_expires_at: i64,
    pub registration_jws: String,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InstanceTokenRequest {
    pub client_assertion: String,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InstanceTokenResponse {
    pub access_token: String,
    pub token_type: String,
    pub expires_in: u64,
    pub expires_at: i64,
    pub dpop_nonce: String,
    pub lease_expires_at: i64,
}

fn registration_assertion(value: &InstanceRegistration) -> ClientAssertion {
    ClientAssertion {
        iss: value.device_id.clone(),
        sub: value.device_id.clone(),
        aud: value.aud.clone(),
        iat: value.iat,
        nbf: value.nbf,
        exp: value.exp,
        jti: value.jti.clone(),
    }
}

fn possession_assertion(value: &InstancePossession) -> ClientAssertion {
    ClientAssertion {
        iss: value.iss.clone(),
        sub: value.sub.clone(),
        aud: value.aud.clone(),
        iat: value.iat,
        nbf: value.nbf,
        exp: value.exp,
        jti: value.jti.clone(),
    }
}

/// IDs become URL path segments and database keys. Do not accept encoded paths.
pub fn validate_instance_identifier(value: &str) -> Result<()> {
    bounded_text(value, 128)?;
    if value == "."
        || value == ".."
        || !value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
    {
        return Err(ProtocolError::Invalid("instance identifier"));
    }
    Ok(())
}

pub fn validate_instance_registration(value: &InstanceRegistration) -> Result<()> {
    if value.version != PROTOCOL_VERSION
        || value.device_auth_epoch == 0
        || value.authz_version == 0
        || value.billing_grant_id.is_some() != value.billing_authz_version.is_some()
        || value.billing_authz_version == Some(0)
        || (value.purpose == InstancePurpose::RolloutValidation && value.billing_grant_id.is_some())
        || value.nbf != value.iat
    {
        return Err(ProtocolError::Invalid("instance registration version"));
    }
    for id in [
        &value.device_id,
        &value.instance_id,
        &value.placement_id,
        &value.deployment_id,
        &value.project_id,
        &value.grant_id,
    ] {
        validate_instance_identifier(id)?;
    }
    if let Some(id) = &value.billing_grant_id {
        validate_instance_identifier(id)?;
    }
    value.workload_key.validate()?;
    validate_assertion(&registration_assertion(value))
}

pub fn sign_instance_registration(
    value: &InstanceRegistration,
    key: &SigningKey,
) -> Result<String> {
    validate_instance_registration(value)?;
    if value.workload_key == key.public_key() {
        return Err(ProtocolError::Invalid(
            "workload and device keys must differ",
        ));
    }
    sign_pinned(value, key, INSTANCE_REGISTRATION_JWS_TYPE)
}

pub fn verify_instance_registration(
    compact: &str,
    device_key: &Ed25519PublicKey,
    device_id: &str,
    endpoint: &str,
    now: i64,
) -> Result<InstanceRegistration> {
    let value: InstanceRegistration =
        verify_pinned(compact, device_key, INSTANCE_REGISTRATION_JWS_TYPE)?;
    validate_instance_registration(&value)?;
    if &value.workload_key == device_key {
        return Err(ProtocolError::KeyMismatch);
    }
    check_assertion_request(&registration_assertion(&value), device_id, endpoint, now)?;
    Ok(value)
}

pub fn sign_instance_possession(value: &InstancePossession, key: &SigningKey) -> Result<String> {
    validate_assertion(&possession_assertion(value))?;
    validate_instance_identifier(&value.sub)?;
    decode_exact::<32>(&value.registration_digest)?;
    sign_pinned(value, key, INSTANCE_POSSESSION_JWS_TYPE)
}

/// Possession authenticates the complete device-signed document, including its
/// placement and grant versions. DPoP alone does not authenticate a request body.
pub fn verify_instance_possession(
    compact: &str,
    workload_key: &Ed25519PublicKey,
    instance_id: &str,
    endpoint: &str,
    registration_jws: &str,
    now: i64,
) -> Result<InstancePossession> {
    let value: InstancePossession =
        verify_pinned(compact, workload_key, INSTANCE_POSSESSION_JWS_TYPE)?;
    validate_assertion(&possession_assertion(&value))?;
    validate_instance_identifier(&value.sub)?;
    decode_exact::<32>(&value.registration_digest)?;
    check_assertion_request(&possession_assertion(&value), instance_id, endpoint, now)?;
    if value.registration_digest != compact_digest(registration_jws) {
        return Err(ProtocolError::BindingMismatch);
    }
    Ok(value)
}

pub fn sign_workload_assertion(value: &ClientAssertion, key: &SigningKey) -> Result<String> {
    validate_assertion(value)?;
    validate_instance_identifier(&value.sub)?;
    sign_pinned(value, key, WORKLOAD_ASSERTION_JWS_TYPE)
}

pub fn verify_workload_assertion(
    compact: &str,
    workload_key: &Ed25519PublicKey,
    instance_id: &str,
    endpoint: &str,
    now: i64,
) -> Result<ClientAssertion> {
    let value: ClientAssertion = verify_pinned(compact, workload_key, WORKLOAD_ASSERTION_JWS_TYPE)?;
    validate_assertion(&value)?;
    validate_instance_identifier(&value.sub)?;
    check_assertion_request(&value, instance_id, endpoint, now)?;
    Ok(value)
}

const _: () = assert!(MAX_ASSERTION_TTL_SECONDS < MAX_INSTANCE_TOKEN_SECONDS);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{sign_client_assertion, verify_client_assertion};

    const NOW: i64 = 1_800_000_000;
    const ENDPOINT: &str = "https://api.example.test/api/v1/devices/device-1/instances";

    fn registration(key: &SigningKey) -> InstanceRegistration {
        InstanceRegistration {
            version: PROTOCOL_VERSION,
            purpose: InstancePurpose::Workload,
            device_id: "device-1".into(),
            device_auth_epoch: 1,
            instance_id: "instance-1".into(),
            placement_id: "placement-1".into(),
            deployment_id: "deployment-1".into(),
            project_id: "project-1".into(),
            grant_id: "grant-1".into(),
            authz_version: 2,
            billing_grant_id: Some("billing-1".into()),
            billing_authz_version: Some(3),
            workload_key: key.public_key(),
            aud: ENDPOINT.into(),
            iat: NOW,
            nbf: NOW,
            exp: NOW + 60,
            jti: "registration-unique-id".into(),
        }
    }

    #[test]
    fn registration_and_possession_bind_the_same_complete_document() {
        let device = SigningKey::generate();
        let workload = SigningKey::generate();
        let mut document = registration(&workload);
        let signed = sign_instance_registration(&document, &device).unwrap();
        assert_eq!(
            verify_instance_registration(&signed, &device.public_key(), "device-1", ENDPOINT, NOW)
                .unwrap(),
            document
        );
        let possession = InstancePossession {
            iss: "instance-1".into(),
            sub: "instance-1".into(),
            aud: ENDPOINT.into(),
            iat: NOW,
            nbf: NOW,
            exp: NOW + 60,
            jti: "possession-unique-id".into(),
            registration_digest: compact_digest(&signed),
        };
        let proof = sign_instance_possession(&possession, &workload).unwrap();
        assert!(
            verify_instance_possession(
                &proof,
                &workload.public_key(),
                "instance-1",
                ENDPOINT,
                &signed,
                NOW
            )
            .is_ok()
        );
        document.billing_grant_id = Some("different-payer-consent".into());
        let changed = sign_instance_registration(&document, &device).unwrap();
        assert!(
            verify_instance_possession(
                &proof,
                &workload.public_key(),
                "instance-1",
                ENDPOINT,
                &changed,
                NOW
            )
            .is_err()
        );
        assert!(
            verify_instance_registration(
                &signed,
                &device.public_key(),
                "other-device",
                ENDPOINT,
                NOW
            )
            .is_err()
        );
        assert!(
            verify_instance_registration(
                &signed,
                &device.public_key(),
                "device-1",
                "https://other.example/instances",
                NOW
            )
            .is_err()
        );
        assert!(
            verify_instance_registration(
                &signed,
                &device.public_key(),
                "device-1",
                ENDPOINT,
                NOW + 60
            )
            .is_err()
        );
        assert!(
            verify_instance_possession(
                &proof,
                &device.public_key(),
                "instance-1",
                ENDPOINT,
                &signed,
                NOW
            )
            .is_err()
        );
        assert!(
            verify_instance_possession(
                &proof,
                &workload.public_key(),
                "instance-2",
                ENDPOINT,
                &signed,
                NOW
            )
            .is_err()
        );
    }

    #[test]
    fn workload_assertion_cannot_be_used_as_a_device_assertion() {
        let key = SigningKey::generate();
        let assertion = ClientAssertion {
            iss: "instance-1".into(),
            sub: "instance-1".into(),
            aud: "https://api.example.test/api/v1/instances/instance-1/token".into(),
            iat: NOW,
            nbf: NOW,
            exp: NOW + 60,
            jti: "workload-assertion-id".into(),
        };
        let signed = sign_workload_assertion(&assertion, &key).unwrap();
        assert!(
            verify_workload_assertion(
                &signed,
                &key.public_key(),
                "instance-1",
                &assertion.aud,
                NOW
            )
            .is_ok()
        );
        assert!(
            verify_client_assertion(
                &signed,
                &key.public_key(),
                "instance-1",
                &assertion.aud,
                NOW
            )
            .is_err()
        );
        let device_signed = sign_client_assertion(&assertion, &key).unwrap();
        assert!(
            verify_workload_assertion(
                &device_signed,
                &key.public_key(),
                "instance-1",
                &assertion.aud,
                NOW
            )
            .is_err()
        );
        assert!(
            verify_workload_assertion(
                &signed,
                &key.public_key(),
                "instance-1",
                "https://api.example.test/api/v1/instances/instance-1/receipt",
                NOW
            )
            .is_err()
        );
    }

    #[test]
    fn validation_purpose_is_signed_and_cannot_bind_billing() {
        let workload = SigningKey::generate();
        let device = SigningKey::generate();
        let mut document = registration(&workload);
        let legacy = serde_json::to_value(&document).unwrap();
        assert!(legacy.get("purpose").is_none());
        assert_eq!(
            serde_json::from_value::<InstanceRegistration>(legacy)
                .unwrap()
                .purpose,
            InstancePurpose::Workload
        );
        document.purpose = InstancePurpose::RolloutValidation;
        assert!(sign_instance_registration(&document, &device).is_err());
        document.billing_grant_id = None;
        document.billing_authz_version = None;
        let signed = sign_instance_registration(&document, &device).unwrap();
        assert_eq!(
            verify_instance_registration(&signed, &device.public_key(), "device-1", ENDPOINT, NOW)
                .unwrap()
                .purpose,
            InstancePurpose::RolloutValidation
        );
        let mut unknown = serde_json::to_value(&document).unwrap();
        unknown["purpose"] = "unrestricted".into();
        assert!(serde_json::from_value::<InstanceRegistration>(unknown).is_err());
    }

    #[test]
    fn storage_only_registration_has_no_implied_billing_consent() {
        let workload = SigningKey::generate();
        let device = SigningKey::generate();
        let mut document = registration(&workload);
        document.billing_grant_id = None;
        document.billing_authz_version = None;
        let signed = sign_instance_registration(&document, &device).unwrap();
        let verified = verify_instance_registration(
            &signed,
            &device.public_key(),
            &document.device_id,
            &document.aud,
            NOW,
        )
        .unwrap();
        assert!(verified.billing_grant_id.is_none());
        document.billing_grant_id = Some("billing".into());
        assert!(sign_instance_registration(&document, &device).is_err());
        document.billing_grant_id = None;
        document.billing_authz_version = Some(1);
        assert!(sign_instance_registration(&document, &device).is_err());
    }

    #[test]
    fn reject_key_reuse_unbounded_proofs_and_ambiguous_identifiers() {
        let key = SigningKey::generate();
        let mut document = registration(&key);
        assert!(sign_instance_registration(&document, &key).is_err());
        let device = SigningKey::generate();
        document.exp += 1;
        assert!(sign_instance_registration(&document, &device).is_err());
        document.exp -= 1;
        document.authz_version = 0;
        assert!(sign_instance_registration(&document, &device).is_err());
        for id in [
            "", "..", ".", "a/b", "a%2fb", "a?b", "a#b", "a\\b", "a\nb", "a b",
        ] {
            assert!(validate_instance_identifier(id).is_err(), "accepted {id:?}");
        }
        let mut json = serde_json::to_value(registration(&key)).unwrap();
        json["payer_id"] = "caller-selected-payer".into();
        assert!(serde_json::from_value::<InstanceRegistration>(json).is_err());
    }
}
