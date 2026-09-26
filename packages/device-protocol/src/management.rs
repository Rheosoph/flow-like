use crate::{
    Ed25519PublicKey, ProtocolError, Result, SigningKey,
    proof::{sign_pinned, verify_pinned},
};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

pub const MAX_TELEMETRY_MEMBERS: usize = 32;
pub const MAX_MANAGEMENT_POLICY_SECONDS: i64 = 31 * 86_400;
pub const MANAGEMENT_SESSION_SECONDS: i64 = 300;
const ROSTER_TYPE: &str = "flow-like-telemetry-roster+jwt";
const ENVELOPE_TYPE: &str = "flow-like-telemetry-envelope+jwt";
const RECEIPT_TYPE: &str = "flow-like-telemetry-delivery-receipt+jwt";
const POLICY_TYPE: &str = "flow-like-management-policy+jwt";
const CERTIFICATE_TYPE: &str = "flow-like-management-session+jwt";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TelemetryMember {
    pub endpoint_id: String,
    pub signing_key: Ed25519PublicKey,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TelemetryRoster {
    pub version: u32,
    pub device_id: String,
    /// "device" or the exact placement ID. These are distinct encryption groups.
    pub scope: String,
    pub policy_version: u64,
    pub previous_policy_digest: Option<String>,
    #[serde(default)]
    pub management_policy_digest: Option<String>,
    pub publisher: TelemetryMember,
    pub members: Vec<TelemetryMember>,
    pub issued_at: i64,
    pub expires_at: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TelemetryEnvelopeKind {
    Commit,
    Welcome,
    Application,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TelemetryEnvelope {
    pub device_id: String,
    pub scope: String,
    pub sequence: u64,
    pub policy_digest: String,
    pub kind: TelemetryEnvelopeKind,
    pub wire_digest: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TelemetryDeliveryReceipt {
    pub version: u32,
    pub device_id: String,
    pub scope: String,
    pub endpoint_id: String,
    pub sequence: u64,
    pub policy_digest: String,
    pub envelope_digest: String,
}

pub fn sign_telemetry_delivery_receipt(
    receipt: &TelemetryDeliveryReceipt,
    key: &SigningKey,
) -> Result<String> {
    validate_delivery_receipt(receipt)?;
    sign_pinned(receipt, key, RECEIPT_TYPE)
}
pub fn verify_telemetry_delivery_receipt(
    compact: &str,
    pinned: &Ed25519PublicKey,
) -> Result<TelemetryDeliveryReceipt> {
    let receipt = verify_pinned(compact, pinned, RECEIPT_TYPE)?;
    validate_delivery_receipt(&receipt)?;
    Ok(receipt)
}
fn validate_delivery_receipt(receipt: &TelemetryDeliveryReceipt) -> Result<()> {
    validate_management_id(&receipt.device_id)?;
    validate_management_id(&receipt.scope)?;
    validate_management_id(&receipt.endpoint_id)?;
    digest_shape(&receipt.policy_digest)?;
    digest_shape(&receipt.envelope_digest)?;
    if receipt.version != 1 || receipt.sequence == 0 {
        return Err(ProtocolError::Invalid("telemetry delivery receipt"));
    }
    Ok(())
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ManagementCapability {
    Status,
    Logs,
    Metrics,
    Deploy,
    Start,
    Stop,
    Restart,
    Remove,
    Scale,
    UpdateAgent,
    Reboot,
    ManageCertificates,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ManagementScope {
    Device,
    Project {
        project_id: String,
    },
    Placement {
        project_id: String,
        placement_id: String,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ManagementGrant {
    pub grant_id: String,
    pub user_id: String,
    pub controller_key: Ed25519PublicKey,
    pub scope: ManagementScope,
    pub capabilities: Vec<ManagementCapability>,
    pub expires_at: i64,
    /// A resolved group roster is approved by the owner; later group additions confer no rights.
    pub group_id: Option<String>,
    pub group_version: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ManagementPolicy {
    pub version: u32,
    pub device_id: String,
    pub policy_version: u64,
    pub previous_policy_digest: Option<String>,
    pub grants: Vec<ManagementGrant>,
    pub issued_at: i64,
    pub expires_at: i64,
}

/// The controller signing key certifies a fresh Noise static key for one short session.
/// The verifier obtains that signing key from onboarding or a current owner-approved grant.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ControllerCertificate {
    pub version: u32,
    pub device_id: String,
    pub grant_id: String,
    pub session_id: String,
    pub management_key: [u8; 32],
    pub issued_at: i64,
    pub expires_at: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeviceSignalingRequest {
    pub client_assertion: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ControllerSignalingRequest {
    pub participant_id: String,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeviceSignalingResponse {
    pub token: String,
    pub expires_at: i64,
    pub device_auth_epoch: u64,
    pub signaling_urls: Vec<String>,
    #[serde(default)]
    pub ice_servers: Vec<DeviceIceServer>,
    pub ice_expires_at: Option<i64>,
    pub policy_version: u64,
    pub policy_digest: Option<String>,
}
impl std::fmt::Debug for DeviceSignalingResponse {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("DeviceSignalingResponse")
            .field("expires_at", &self.expires_at)
            .field("credentials", &"[REDACTED]")
            .finish()
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeviceIceServer {
    pub urls: Vec<String>,
    pub username: Option<String>,
    pub credential: Option<String>,
}
impl std::fmt::Debug for DeviceIceServer {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("DeviceIceServer")
            .field("urls", &self.urls)
            .field("credentials", &"[REDACTED]")
            .finish()
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ManagementRequest {
    pub operation_id: String,
    pub device_id: String,
    pub issued_at: i64,
    pub expires_at: i64,
    pub command: ManagementCommand,
}

#[derive(Clone, Serialize, Deserialize, zeroize::Zeroize, zeroize::ZeroizeOnDrop)]
#[serde(transparent)]
pub struct SecretValue(pub String);
impl std::fmt::Debug for SecretValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("[REDACTED]")
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum ManagementCommand {
    Certificates {
        #[serde(default)]
        placement_id: Option<String>,
        #[serde(default)]
        after: Option<String>,
        #[serde(default = "default_certificate_page_limit")]
        limit: u16,
    },
    PutCertificate {
        certificate_id: String,
        label: String,
        expected_revision: u64,
        certificate_chain_pem: SecretValue,
        private_key_pem: SecretValue,
    },
    DeleteCertificate {
        certificate_id: String,
        expected_revision: u64,
    },
    CertificateRequests {
        #[serde(default)]
        after: Option<String>,
        #[serde(default = "default_certificate_page_limit")]
        limit: u16,
    },
    CreateCertificateRequest {
        request_id: String,
        certificate_id: String,
        label: String,
        expected_revision: u64,
        dns_names: Vec<String>,
        ip_addresses: Vec<String>,
    },
    InstallCertificateRequest {
        request_id: String,
        certificate_chain_pem: SecretValue,
    },
    DeleteCertificateRequest {
        request_id: String,
    },
    CertificateIssuers {
        #[serde(default)]
        after: Option<String>,
        #[serde(default = "default_certificate_page_limit")]
        limit: u16,
    },
    CreateCertificateIssuerRequest {
        request_id: String,
        certificate_id: String,
        expected_revision: u64,
        dns_names: Vec<String>,
        ip_addresses: Vec<String>,
        leaf_lifetime_days: u16,
    },
    InstallCertificateIssuer {
        request_id: String,
        certificate_chain_pem: SecretValue,
    },
    DeleteCertificateIssuer {
        certificate_id: String,
        expected_revision: u64,
    },
    AcmeCertificates {
        #[serde(default)]
        after: Option<String>,
        #[serde(default = "default_certificate_page_limit")]
        limit: u16,
    },
    ConfigureAcmeCertificate {
        certificate_id: String,
        label: String,
        expected_revision: u64,
        expected_certificate_revision: u64,
        dns_names: Vec<String>,
        environment: crate::AcmeEnvironment,
        http_bind: String,
        terms_of_service_agreed: bool,
    },
    DeleteAcmeCertificate {
        certificate_id: String,
        expected_revision: u64,
    },
    OfflineQueue {
        placement_id: String,
        #[serde(default)]
        after: Option<String>,
    },
    OfflineQueueRetry {
        placement_id: String,
        scope: String,
        queued_operation_id: String,
    },
    OfflineQueueSkip {
        placement_id: String,
        scope: String,
        queued_operation_id: String,
        reason: String,
        #[serde(default)]
        acknowledge_uncertain: bool,
    },
    Inspect,
    InspectPage {
        after: Option<String>,
        #[serde(default = "default_inspect_page_limit")]
        limit: u16,
    },
    Artifact {
        request: crate::ArtifactRequest,
    },
    Operation {
        operation_id: String,
    },
    #[serde(alias = "placement_config")]
    PlacementConfiguration {
        placement_id: String,
    },
    Apply {
        config: serde_json::Value,
        expected_revision: u64,
        start: bool,
    },
    StageRollout {
        config: serde_json::Value,
        expected_revision: u64,
        #[serde(default = "default_rollout_stabilization")]
        stabilization_seconds: u32,
        #[serde(default = "default_rollout_deadline")]
        deadline_seconds: u32,
    },
    RolloutSecret {
        rollout_id: String,
        name: String,
        value: SecretValue,
    },
    ActivateRollout {
        rollout_id: String,
    },
    CancelRollout {
        rollout_id: String,
    },
    Rollout {
        rollout_id: String,
    },
    Scale {
        placement_id: String,
        expected_revision: u64,
        replicas: u8,
    },
    Start {
        placement_id: String,
        expected_revision: u64,
    },
    Stop {
        placement_id: String,
        expected_revision: u64,
    },
    Restart {
        placement_id: String,
        expected_revision: u64,
    },
    Remove {
        placement_id: String,
        expected_revision: u64,
    },
    ApplyPolicy {
        policy_jws: String,
    },
    SetSecret {
        placement_id: String,
        expected_revision: u64,
        name: String,
        value: SecretValue,
    },
    Metrics {
        placement_id: Option<String>,
    },
    ProjectMetrics {
        project_id: String,
    },
    Messages {
        placement_id: Option<String>,
        project_id: Option<String>,
        after: u64,
        limit: u32,
    },
    Logs {
        placement_id: Option<String>,
        after: u64,
        limit: u32,
    },
    Reboot {
        expected_boot_id: String,
    },
    UpdateAgent {
        expected_boot_id: String,
        release_jws: String,
    },
    TelemetryPolicy {
        scope: String,
        sequence: u64,
        policy_jws: String,
        key_packages: Vec<TelemetryKeyPackage>,
    },
    TelemetryRead {
        scope: String,
        sequence: u64,
        welcome: bool,
        offset: u32,
        limit: u32,
    },
    TelemetryRosterRead {
        scope: String,
        offset: u32,
        limit: u32,
    },
    ArchivePolicy {
        policy_jws: String,
    },
    ArchiveRosterRead {
        scope: String,
        kind: crate::ArchiveKind,
        offset: u32,
        limit: u32,
    },
    ArchiveRead {
        scope: String,
        kind: crate::ArchiveKind,
        sequence: u64,
        offset: u32,
        limit: u32,
    },
    TelemetryAcknowledge {
        scope: String,
        sequence: u64,
        envelope_digest: String,
    },
    TelemetryReceipt {
        scope: String,
        endpoint_id: String,
        sequence: u64,
        receipt_jws: String,
    },
}

fn default_inspect_page_limit() -> u16 {
    2
}

fn default_certificate_page_limit() -> u16 {
    4
}

fn default_rollout_stabilization() -> u32 {
    10
}

fn default_rollout_deadline() -> u32 {
    120
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TelemetryKeyPackage {
    pub member: TelemetryMember,
    pub key_package: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ManagementResponse {
    pub operation_id: String,
    pub state: String,
    pub result: serde_json::Value,
}

pub fn validate_management_id(id: &str) -> Result<()> {
    if id.is_empty()
        || id.len() > 128
        || !id
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"-_:.".contains(&c))
    {
        return Err(ProtocolError::Invalid("management identifier"));
    }
    Ok(())
}

fn digest_shape(digest: &str) -> Result<()> {
    use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
    if digest.len() != 43 || URL_SAFE_NO_PAD.decode(digest).is_err() {
        return Err(ProtocolError::Invalid("management digest"));
    }
    Ok(())
}

fn policy_shape(
    version: u32,
    device: &str,
    revision: u64,
    previous: &Option<String>,
    issued: i64,
    expires: i64,
) -> Result<()> {
    validate_management_id(device)?;
    if version != 1
        || revision == 0
        || issued <= 0
        || expires <= issued
        || expires.saturating_sub(issued) > MAX_MANAGEMENT_POLICY_SECONDS
        || (revision == 1) != previous.is_none()
    {
        return Err(ProtocolError::Invalid(
            "management policy version or lifetime",
        ));
    }
    if let Some(digest) = previous {
        digest_shape(digest)?;
    }
    Ok(())
}

fn current(issued: i64, expires: i64, now: i64) -> Result<()> {
    if issued > now.saturating_add(5) || expires <= now {
        return Err(ProtocolError::InvalidTime);
    }
    Ok(())
}

fn validate_roster(roster: &TelemetryRoster) -> Result<()> {
    if let Some(digest) = &roster.management_policy_digest {
        digest_shape(digest)?;
    }
    policy_shape(
        roster.version,
        &roster.device_id,
        roster.policy_version,
        &roster.previous_policy_digest,
        roster.issued_at,
        roster.expires_at,
    )?;
    validate_management_id(&roster.scope)?;
    if roster.members.is_empty()
        || roster.members.len() > MAX_TELEMETRY_MEMBERS
        || !roster.members.contains(&roster.publisher)
    {
        return Err(ProtocolError::Invalid("telemetry roster"));
    }
    let mut ids = HashSet::new();
    let mut keys = HashSet::new();
    for member in &roster.members {
        validate_management_id(&member.endpoint_id)?;
        if !ids.insert(&member.endpoint_id) || !keys.insert(member.signing_key.to_bytes()?) {
            return Err(ProtocolError::Invalid("duplicate telemetry endpoint"));
        }
    }
    Ok(())
}

pub fn sign_telemetry_roster(roster: &TelemetryRoster, key: &SigningKey) -> Result<String> {
    validate_roster(roster)?;
    sign_pinned(roster, key, ROSTER_TYPE)
}

pub fn verify_telemetry_roster(
    compact: &str,
    pinned: &Ed25519PublicKey,
    now: i64,
) -> Result<TelemetryRoster> {
    let roster: TelemetryRoster = verify_pinned(compact, pinned, ROSTER_TYPE)?;
    validate_roster(&roster)?;
    current(roster.issued_at, roster.expires_at, now)?;
    Ok(roster)
}

/// Authenticate a previous policy head when constructing its successor. This
/// does not authorize new messages or membership under an expired policy.
pub fn verify_historical_telemetry_roster(
    compact: &str,
    pinned: &Ed25519PublicKey,
) -> Result<TelemetryRoster> {
    let roster = verify_pinned(compact, pinned, ROSTER_TYPE)?;
    validate_roster(&roster)?;
    Ok(roster)
}

fn validate_envelope(envelope: &TelemetryEnvelope) -> Result<()> {
    validate_management_id(&envelope.device_id)?;
    validate_management_id(&envelope.scope)?;
    digest_shape(&envelope.policy_digest)?;
    digest_shape(&envelope.wire_digest)?;
    if envelope.sequence == 0 {
        return Err(ProtocolError::Invalid("telemetry sequence"));
    }
    Ok(())
}

pub fn sign_telemetry_envelope(envelope: &TelemetryEnvelope, key: &SigningKey) -> Result<String> {
    validate_envelope(envelope)?;
    sign_pinned(envelope, key, ENVELOPE_TYPE)
}

pub fn verify_telemetry_envelope(
    compact: &str,
    pinned: &Ed25519PublicKey,
) -> Result<TelemetryEnvelope> {
    let envelope = verify_pinned(compact, pinned, ENVELOPE_TYPE)?;
    validate_envelope(&envelope)?;
    Ok(envelope)
}

fn validate_policy(policy: &ManagementPolicy) -> Result<()> {
    policy_shape(
        policy.version,
        &policy.device_id,
        policy.policy_version,
        &policy.previous_policy_digest,
        policy.issued_at,
        policy.expires_at,
    )?;
    if policy.grants.len() > 24 {
        return Err(ProtocolError::Invalid("too many management grants"));
    }
    let mut grants = HashSet::new();
    for grant in &policy.grants {
        validate_management_id(&grant.grant_id)?;
        validate_management_id(&grant.user_id)?;
        grant.controller_key.validate()?;
        if !grants.insert(&grant.grant_id)
            || grant.capabilities.is_empty()
            || grant.capabilities.len() > 13
            || grant.expires_at > policy.expires_at
            || grant.expires_at <= policy.issued_at
            || grant.group_id.is_some() != grant.group_version.is_some()
            || grant.group_version == Some(0)
        {
            return Err(ProtocolError::Invalid("management grant"));
        }
        if let Some(id) = &grant.group_id {
            validate_management_id(id)?;
        }
        let caps: HashSet<_> = grant.capabilities.iter().collect();
        if caps.len() != grant.capabilities.len() {
            return Err(ProtocolError::Invalid("duplicate capability"));
        }
        match &grant.scope {
            ManagementScope::Device => (),
            ManagementScope::Project { project_id }
            | ManagementScope::Placement { project_id, .. } => {
                validate_management_id(project_id)?;
                if grant.capabilities.iter().any(|cap| {
                    matches!(
                        cap,
                        ManagementCapability::Reboot
                            | ManagementCapability::UpdateAgent
                            | ManagementCapability::ManageCertificates
                    )
                }) {
                    return Err(ProtocolError::Invalid(
                        "host capability requires device scope",
                    ));
                }
            }
        }
        if let ManagementScope::Placement { placement_id, .. } = &grant.scope {
            validate_management_id(placement_id)?;
        }
    }
    Ok(())
}

pub fn sign_management_policy(policy: &ManagementPolicy, key: &SigningKey) -> Result<String> {
    validate_policy(policy)?;
    sign_pinned(policy, key, POLICY_TYPE)
}

pub fn verify_management_policy(
    compact: &str,
    pinned: &Ed25519PublicKey,
    now: i64,
) -> Result<ManagementPolicy> {
    let policy = verify_pinned(compact, pinned, POLICY_TYPE)?;
    validate_policy(&policy)?;
    current(policy.issued_at, policy.expires_at, now)?;
    Ok(policy)
}

/// Validate historical policy for chain recovery only. This does not grant current access.
pub fn verify_historical_management_policy(
    compact: &str,
    pinned: &Ed25519PublicKey,
) -> Result<ManagementPolicy> {
    let policy = verify_pinned(compact, pinned, POLICY_TYPE)?;
    validate_policy(&policy)?;
    Ok(policy)
}

fn validate_certificate(cert: &ControllerCertificate) -> Result<()> {
    for id in [&cert.device_id, &cert.grant_id, &cert.session_id] {
        validate_management_id(id)?;
    }
    if cert.version != 1
        || cert.issued_at <= 0
        || cert.expires_at <= cert.issued_at
        || cert.expires_at.saturating_sub(cert.issued_at) > MANAGEMENT_SESSION_SECONDS
        || x25519_dalek::x25519([0x91; 32], cert.management_key) == [0; 32]
    {
        return Err(ProtocolError::Invalid("controller session certificate"));
    }
    Ok(())
}

pub fn sign_controller_certificate(
    cert: &ControllerCertificate,
    key: &SigningKey,
) -> Result<String> {
    validate_certificate(cert)?;
    sign_pinned(cert, key, CERTIFICATE_TYPE)
}

pub fn verify_controller_certificate(
    compact: &str,
    pinned: &Ed25519PublicKey,
    now: i64,
) -> Result<ControllerCertificate> {
    let cert = verify_pinned(compact, pinned, CERTIFICATE_TYPE)?;
    validate_certificate(&cert)?;
    current(cert.issued_at, cert.expires_at, now)?;
    Ok(cert)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn placement_configuration_uses_the_current_wire_name_and_accepts_its_legacy_alias() {
        let command = ManagementCommand::PlacementConfiguration {
            placement_id: "api".into(),
        };
        assert_eq!(
            serde_json::to_value(&command).unwrap(),
            serde_json::json!({"type":"placement_configuration","placement_id":"api"})
        );
        let legacy: ManagementCommand = serde_json::from_value(
            serde_json::json!({"type":"placement_config","placement_id":"api"}),
        )
        .unwrap();
        assert!(
            matches!(legacy, ManagementCommand::PlacementConfiguration { placement_id } if placement_id == "api")
        );
    }
    #[test]
    fn roster_pins_signer_members_and_expiry() {
        let owner = SigningKey::generate();
        let publisher = TelemetryMember {
            endpoint_id: "device".into(),
            signing_key: SigningKey::generate().public_key(),
        };
        let mut roster = TelemetryRoster {
            version: 1,
            device_id: "device".into(),
            scope: "device".into(),
            policy_version: 1,
            previous_policy_digest: None,
            management_policy_digest: None,
            publisher: publisher.clone(),
            members: vec![publisher],
            issued_at: 100,
            expires_at: 200,
        };
        let signed = sign_telemetry_roster(&roster, &owner).unwrap();
        assert_eq!(
            verify_telemetry_roster(&signed, &owner.public_key(), 100).unwrap(),
            roster
        );
        assert!(
            verify_telemetry_roster(&signed, &SigningKey::generate().public_key(), 100).is_err()
        );
        assert!(verify_telemetry_roster(&signed, &owner.public_key(), 200).is_err());
        roster.members.push(roster.publisher.clone());
        assert!(sign_telemetry_roster(&roster, &owner).is_err());
    }

    #[test]
    fn session_certificates_cannot_be_rosters_or_use_low_order_keys() {
        let signer = SigningKey::generate();
        let mut cert = ControllerCertificate {
            version: 1,
            device_id: "device".into(),
            grant_id: "owner".into(),
            session_id: "session".into(),
            management_key: x25519_dalek::x25519([7; 32], x25519_dalek::X25519_BASEPOINT_BYTES),
            issued_at: 100,
            expires_at: 400,
        };
        let signed = sign_controller_certificate(&cert, &signer).unwrap();
        assert!(verify_controller_certificate(&signed, &signer.public_key(), 200).is_ok());
        assert!(verify_controller_certificate(&signed, &signer.public_key(), 400).is_err());
        assert!(verify_telemetry_roster(&signed, &signer.public_key(), 200).is_err());
        cert.management_key = [0; 32];
        assert!(sign_controller_certificate(&cert, &signer).is_err());
    }
}
