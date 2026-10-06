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
    ServiceConnect,
    /// Calls hosted models through the model gateway.
    ModelUse,
    /// Installs, configures and removes hosted models and runtimes.
    ModelManage,
    /// A capability added after this build. Signatures and digests cover the raw signed
    /// bytes, so reading it this way changes no verification; it grants nothing.
    #[serde(other)]
    Unsupported,
}

pub const MAX_GRANT_CAPABILITIES: usize = 15;
/// Includes capabilities a newer signer names which this build does not recognize.
pub const MAX_WIRE_GRANT_CAPABILITIES: usize = 64;

impl ManagementCapability {
    /// Held only with whole-device scope.
    pub fn device_only(&self) -> bool {
        matches!(
            self,
            Self::Reboot
                | Self::UpdateAgent
                | Self::ManageCertificates
                | Self::ModelUse
                | Self::ModelManage
        )
    }
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

/// Field values of a person-started run. They can hold short secrets, so they never print.
#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RunPayload(pub serde_json::Value);
impl std::fmt::Debug for RunPayload {
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
    /// Queued writes of one authorization scope without their payloads. `terminal` lists
    /// applied, skipped and superseded writes, newest first; `after_sequence` is the
    /// previous page's `next` in both directions.
    OfflineQueueOperations {
        placement_id: String,
        scope: String,
        #[serde(default)]
        after_sequence: Option<u64>,
        #[serde(default = "default_offline_operations_limit")]
        limit: u16,
        #[serde(default)]
        terminal: bool,
    },
    OfflineQueueLookup {
        placement_id: String,
        scope: String,
        queued_operation_id: String,
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
    /// Journaled operations of every principal, newest first. `after` is the previous
    /// page's `next`.
    Operations {
        #[serde(default)]
        after: Option<String>,
        #[serde(default = "default_operations_limit")]
        limit: u8,
    },
    /// The reboot or agent update that is in progress, whoever started it.
    HostOperation,
    #[serde(alias = "placement_config")]
    PlacementConfiguration {
        placement_id: String,
    },
    ServiceListeners {
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
    /// Current and finished updates of one placement, newest first. `before` is the
    /// previous page's `next`.
    RolloutHistory {
        placement_id: String,
        #[serde(default)]
        before: Option<String>,
        #[serde(default = "default_rollout_history_limit")]
        limit: u8,
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
    /// Queues one run of a person-started event of a running service. The answer carries the
    /// run id at once; the `operation` read follows the run to its end.
    RunEvent {
        placement_id: String,
        event_id: String,
        expected_revision: u64,
        #[serde(default)]
        payload: Option<RunPayload>,
    },
    /// Stops a run queued with `run_event` by the same principal, or by any for the owner.
    CancelRun {
        operation_id: String,
    },
    /// The fields of a person-started event as the running service derived them.
    EventForm {
        placement_id: String,
        event_id: String,
    },
    Metrics {
        placement_id: Option<String>,
    },
    /// Retained samples after record `after`, reduced on the device to the named fields.
    MetricsHistory {
        placement_id: Option<String>,
        after: u64,
        limit: u16,
        fields: Vec<String>,
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
    /// Model hosting; agents without the `model_host` flag answer `unsupported`.
    Models {
        request: crate::ModelsRequest,
    },
}

fn default_inspect_page_limit() -> u16 {
    2
}

fn default_certificate_page_limit() -> u16 {
    4
}

fn default_offline_operations_limit() -> u16 {
    20
}

fn default_operations_limit() -> u8 {
    20
}

fn default_rollout_history_limit() -> u8 {
    8
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
    if issued > now.saturating_add(crate::MAX_CLOCK_SKEW_SECONDS) || expires <= now {
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
            || grant.capabilities.len() > MAX_WIRE_GRANT_CAPABILITIES
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
        // Distinct newer capabilities all read as `Unsupported`, so only known ones must be unique.
        let known = grant
            .capabilities
            .iter()
            .filter(|cap| **cap != ManagementCapability::Unsupported);
        let count = known.clone().count();
        if count > MAX_GRANT_CAPABILITIES || known.collect::<HashSet<_>>().len() != count {
            return Err(ProtocolError::Invalid("duplicate capability"));
        }
        match &grant.scope {
            ManagementScope::Device => (),
            ManagementScope::Project { project_id }
            | ManagementScope::Placement { project_id, .. } => {
                validate_management_id(project_id)?;
                if grant
                    .capabilities
                    .iter()
                    .any(ManagementCapability::device_only)
                {
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
    // Re-signing a newer capability that this build read as `Unsupported` would drop it.
    if policy.grants.iter().any(|grant| {
        grant
            .capabilities
            .contains(&ManagementCapability::Unsupported)
    }) {
        return Err(ProtocolError::Invalid(
            "management policy holds a capability this build does not know",
        ));
    }
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
    fn read_commands_round_trip_with_their_wire_names_and_defaults() {
        use serde_json::json;
        let command = |value: serde_json::Value| {
            serde_json::from_value::<ManagementCommand>(value).map(|command| {
                let encoded = serde_json::to_value(&command).unwrap();
                (command, encoded)
            })
        };
        let (host, encoded) = command(json!({"type":"host_operation"})).unwrap();
        assert!(matches!(host, ManagementCommand::HostOperation));
        assert_eq!(encoded, json!({"type":"host_operation"}));

        let (history, encoded) =
            command(json!({"type":"rollout_history","placement_id":"api"})).unwrap();
        assert!(matches!(
            history,
            ManagementCommand::RolloutHistory {
                before: None,
                limit: 8,
                ..
            }
        ));
        assert_eq!(
            encoded,
            json!({"type":"rollout_history","placement_id":"api","before":null,"limit":8})
        );
        let paged =
            json!({"type":"rollout_history","placement_id":"api","before":"rollout-7","limit":16});
        assert_eq!(command(paged.clone()).unwrap().1, paged);

        let (operations, encoded) = command(json!({"type":"operations"})).unwrap();
        assert!(matches!(
            operations,
            ManagementCommand::Operations {
                after: None,
                limit: 20
            }
        ));
        assert_eq!(
            encoded,
            json!({"type":"operations","after":null,"limit":20})
        );

        let metrics = json!({"type":"metrics_history","placement_id":null,"after":7,"limit":256,"fields":["cpu_percent"]});
        assert_eq!(command(metrics.clone()).unwrap().1, metrics);
        assert!(
            command(json!({"type":"metrics_history","placement_id":"api","after":0,"limit":1}))
                .is_err()
        );

        let (queued, encoded) =
            command(json!({"type":"offline_queue_operations","placement_id":"api","scope":"a"}))
                .unwrap();
        assert!(matches!(
            queued,
            ManagementCommand::OfflineQueueOperations {
                after_sequence: None,
                limit: 20,
                terminal: false,
                ..
            }
        ));
        assert_eq!(
            encoded,
            json!({"type":"offline_queue_operations","placement_id":"api","scope":"a","after_sequence":null,"limit":20,"terminal":false})
        );
        let lookup = json!({"type":"offline_queue_lookup","placement_id":"api","scope":"a","queued_operation_id":"write-1"});
        assert_eq!(command(lookup.clone()).unwrap().1, lookup);

        for unknown in [
            json!({"type":"operations","kind":"stop"}),
            json!({"type":"rollout_histories","placement_id":"api"}),
        ] {
            assert!(command(unknown).is_err());
        }
    }

    #[test]
    fn person_started_run_commands_parse_their_literals_and_refuse_unknown_fields() {
        use serde_json::json;
        let run: ManagementCommand = serde_json::from_value(json!({"type":"run_event","placement_id":"notes","event_id":"evt_notes_form","expected_revision":7,
            "payload":{"title":"Hello","urgent":true}}))
        .unwrap();
        let ManagementCommand::RunEvent {
            placement_id,
            event_id,
            expected_revision,
            payload: Some(payload),
        } = &run
        else {
            panic!("run_event did not parse")
        };
        assert_eq!(
            (placement_id.as_str(), event_id.as_str(), *expected_revision),
            ("notes", "evt_notes_form", 7)
        );
        assert_eq!(payload.0, json!({"title":"Hello","urgent":true}));
        assert_eq!(
            serde_json::to_value(&run).unwrap(),
            json!({"type":"run_event","placement_id":"notes","event_id":"evt_notes_form","expected_revision":7,"payload":{"title":"Hello","urgent":true}})
        );
        assert!(!format!("{run:?}").contains("Hello"));

        let action: ManagementCommand = serde_json::from_value(json!({"type":"run_event","placement_id":"notes","event_id":"evt_action","expected_revision":7}))
            .unwrap();
        assert!(matches!(
            action,
            ManagementCommand::RunEvent { payload: None, .. }
        ));

        let cancel = json!({"type":"cancel_run","operation_id":"op-1"});
        let parsed: ManagementCommand = serde_json::from_value(cancel.clone()).unwrap();
        assert!(
            matches!(&parsed, ManagementCommand::CancelRun { operation_id } if operation_id == "op-1")
        );
        assert_eq!(serde_json::to_value(&parsed).unwrap(), cancel);

        let form = json!({"type":"event_form","placement_id":"notes","event_id":"evt_notes_form"});
        let parsed: ManagementCommand = serde_json::from_value(form.clone()).unwrap();
        assert!(matches!(
            &parsed,
            ManagementCommand::EventForm { placement_id, event_id }
                if placement_id == "notes" && event_id == "evt_notes_form"
        ));
        assert_eq!(serde_json::to_value(&parsed).unwrap(), form);

        for unknown in [
            json!({"type":"run_event","placement_id":"notes","event_id":"e","expected_revision":7,"inputs":{}}),
            json!({"type":"run_event","placement_id":"notes","event_id":"e"}),
            json!({"type":"cancel_run","operation_id":"op-1","run_id":"r"}),
            json!({"type":"event_form","placement_id":"notes","event_id":"e","revision":7}),
            json!({"type":"event_forms","placement_id":"notes","event_id":"e"}),
        ] {
            assert!(
                serde_json::from_value::<ManagementCommand>(unknown.clone()).is_err(),
                "{unknown}"
            );
        }

        let start = json!({"type":"start","placement_id":"notes","expected_revision":7});
        assert_eq!(
            serde_json::to_value(
                serde_json::from_value::<ManagementCommand>(start.clone()).unwrap()
            )
            .unwrap(),
            start
        );
        assert!(
            serde_json::from_value::<ManagementCommand>(
                json!({"type":"start","placement_id":"notes","expected_revision":7,"event_id":"e"})
            )
            .is_err()
        );
    }

    #[test]
    fn models_command_nests_its_request_and_refuses_unknown_shapes() {
        use serde_json::json;
        let wire = json!({"type":"models","request":{"kind":"overview"}});
        let command: ManagementCommand = serde_json::from_value(wire.clone()).unwrap();
        assert!(matches!(
            &command,
            ManagementCommand::Models {
                request: crate::ModelsRequest::Overview {}
            }
        ));
        assert_eq!(serde_json::to_value(&command).unwrap(), wire);
        for unknown in [
            json!({"type":"models","request":{"kind":"overview"},"device":"gpu-box"}),
            json!({"type":"models","request":{"kind":"benchmark"}}),
            json!({"type":"models"}),
        ] {
            assert!(
                serde_json::from_value::<ManagementCommand>(unknown.clone()).is_err(),
                "{unknown}"
            );
        }
    }

    fn policy_with(
        scope: ManagementScope,
        capabilities: Vec<ManagementCapability>,
    ) -> ManagementPolicy {
        ManagementPolicy {
            version: 1,
            device_id: "device".into(),
            policy_version: 1,
            previous_policy_digest: None,
            grants: vec![ManagementGrant {
                grant_id: "grant".into(),
                user_id: "user".into(),
                controller_key: SigningKey::generate().public_key(),
                scope,
                capabilities,
                expires_at: 150,
                group_id: None,
                group_version: None,
            }],
            issued_at: 100,
            expires_at: 200,
        }
    }

    #[test]
    fn model_capabilities_need_the_whole_device_and_fill_the_raised_cap() {
        use ManagementCapability::*;
        let owner = SigningKey::generate();
        let all = vec![
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
            ServiceConnect,
            ModelUse,
            ModelManage,
        ];
        assert_eq!(all.len(), MAX_GRANT_CAPABILITIES);
        let signed =
            sign_management_policy(&policy_with(ManagementScope::Device, all.clone()), &owner)
                .unwrap();
        assert_eq!(
            verify_management_policy(&signed, &owner.public_key(), 100)
                .unwrap()
                .grants[0]
                .capabilities,
            all
        );
        assert_eq!(
            serde_json::to_value([ModelUse, ModelManage]).unwrap(),
            serde_json::json!(["model_use", "model_manage"])
        );
        let project = ManagementScope::Project {
            project_id: "invoice-ai".into(),
        };
        for capability in [ModelUse, ModelManage] {
            assert!(capability.device_only());
            assert!(
                sign_management_policy(
                    &policy_with(project.clone(), vec![Status, capability]),
                    &owner
                )
                .is_err()
            );
        }
        let mut twice = all;
        twice[0] = ModelUse;
        assert!(
            sign_management_policy(&policy_with(ManagementScope::Device, twice), &owner).is_err()
        );
    }

    #[test]
    fn newer_capabilities_verify_grant_nothing_and_cannot_be_re_signed() {
        use ManagementCapability::*;
        let owner = SigningKey::generate();
        let project = ManagementScope::Project {
            project_id: "invoice-ai".into(),
        };
        let sign_raw = |capabilities: serde_json::Value, scope: &ManagementScope| {
            let mut policy =
                serde_json::to_value(policy_with(scope.clone(), vec![Status])).unwrap();
            policy["grants"][0]["capabilities"] = capabilities;
            sign_pinned(&policy, &owner, POLICY_TYPE).unwrap()
        };
        let signed = sign_raw(
            serde_json::json!(["status", "model_tune", "model_share"]),
            &project,
        );
        let verified = verify_management_policy(&signed, &owner.public_key(), 100).unwrap();
        assert_eq!(
            verified.grants[0].capabilities,
            vec![Status, Unsupported, Unsupported]
        );
        assert!(
            !verified.grants[0]
                .capabilities
                .iter()
                .any(ManagementCapability::device_only)
        );
        assert!(sign_management_policy(&verified, &owner).is_err());
        assert!(
            verify_management_policy(
                &sign_raw(serde_json::json!(["status", "status"]), &project),
                &owner.public_key(),
                100
            )
            .is_err()
        );
        let mut full = serde_json::to_value([
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
            ServiceConnect,
            ModelUse,
            ModelManage,
        ])
        .unwrap();
        for index in MAX_GRANT_CAPABILITIES..MAX_WIRE_GRANT_CAPABILITIES {
            full.as_array_mut()
                .unwrap()
                .push(format!("future_{index}").into());
        }
        let verified = verify_management_policy(
            &sign_raw(full.clone(), &ManagementScope::Device),
            &owner.public_key(),
            100,
        )
        .unwrap();
        assert_eq!(
            verified.grants[0].capabilities.len(),
            MAX_WIRE_GRANT_CAPABILITIES
        );
        assert!(sign_management_policy(&verified, &owner).is_err());
        full.as_array_mut().unwrap().push("one_too_many".into());
        assert!(
            verify_management_policy(
                &sign_raw(full, &ManagementScope::Device),
                &owner.public_key(),
                100,
            )
            .is_err()
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

    #[test]
    fn issued_times_tolerate_bounded_clock_skew_but_expiry_stays_strict() {
        let signer = SigningKey::generate();
        let certificate = |issued_at: i64| {
            sign_controller_certificate(
                &ControllerCertificate {
                    version: 1,
                    device_id: "device".into(),
                    grant_id: "owner".into(),
                    session_id: "session".into(),
                    management_key: x25519_dalek::x25519(
                        [7; 32],
                        x25519_dalek::X25519_BASEPOINT_BYTES,
                    ),
                    issued_at,
                    expires_at: issued_at + MANAGEMENT_SESSION_SECONDS,
                },
                &signer,
            )
            .unwrap()
        };
        let now = 1_000;
        let key = signer.public_key();
        assert!(verify_controller_certificate(&certificate(now + 60), &key, now).is_ok());
        assert!(
            verify_controller_certificate(
                &certificate(now + crate::MAX_CLOCK_SKEW_SECONDS),
                &key,
                now
            )
            .is_ok()
        );
        assert!(
            verify_controller_certificate(
                &certificate(now + crate::MAX_CLOCK_SKEW_SECONDS + 1),
                &key,
                now
            )
            .is_err()
        );
        let expired = certificate(now - MANAGEMENT_SESSION_SECONDS);
        assert!(verify_controller_certificate(&expired, &key, now).is_err());
    }
}
