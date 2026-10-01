use crate::{
    config::PlacementConfig,
    crypto::noise,
    enrollment::{DeviceSession, unix_time},
    state::StateStore,
};
use anyhow::{Context, Result, ensure};
use flow_like_device_protocol::*;
use rusqlite::{OptionalExtension, params};
use serde_json::{Value, json};
use std::{
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicI64, Ordering},
    },
};

/// Terminal rows stay replayable and readable by status lookups for this long.
const JOURNAL_RETENTION_SECONDS: i64 = 86_400;
const MAX_GRANT_JOURNAL_ENTRIES: u64 = 4_096;
const MAX_JOURNAL_ENTRIES: u64 = 1_000_000;
const MAX_REJECTION_TEXT: usize = 1_024;
const REMOTE_HOST_OPERATIONS: bool = cfg!(target_os = "linux");

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RejectionCode {
    Unauthorized,
    RevisionConflict,
    Invalid,
    HostPolicy,
    Unsupported,
    Limit,
    Busy,
    Failed,
}

impl RejectionCode {
    fn as_str(self) -> &'static str {
        match self {
            Self::Unauthorized => "unauthorized",
            Self::RevisionConflict => "revision_conflict",
            Self::Invalid => "invalid",
            Self::HostPolicy => "host_policy",
            Self::Unsupported => "unsupported",
            Self::Limit => "limit",
            Self::Busy => "busy",
            Self::Failed => "failed",
        }
    }

    fn retryable(self) -> bool {
        matches!(self, Self::Busy | Self::Failed)
    }

    fn public_message(self) -> &'static str {
        match self {
            Self::Unauthorized => "The current management grant does not allow this command.",
            Self::RevisionConflict => {
                "The placement or device changed since it was read. Reload it before retrying."
            }
            Self::Invalid => "The device rejected this request as invalid.",
            Self::HostPolicy => {
                "The device's host isolation policy does not allow this configuration."
            }
            Self::Unsupported => {
                "The device agent does not support this command. Update the device agent."
            }
            Self::Limit => "A device capacity limit was reached.",
            Self::Busy => "The device is busy with another operation. Retry shortly.",
            Self::Failed => "The device could not complete this command.",
        }
    }
}

/// Keeps the wrapped error's message and causes while recording why the command was refused.
#[derive(Debug)]
struct Rejection {
    code: RejectionCode,
    error: anyhow::Error,
}

impl std::fmt::Display for Rejection {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(&self.error, formatter)
    }
}

impl std::error::Error for Rejection {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        std::error::Error::source(&*self.error)
    }
}

fn refusal(
    code: RejectionCode,
    message: impl std::fmt::Display + std::fmt::Debug + Send + Sync + 'static,
) -> anyhow::Error {
    Rejection {
        code,
        error: anyhow::Error::msg(message),
    }
    .into()
}

fn refuse_unless(
    condition: bool,
    code: RejectionCode,
    message: impl std::fmt::Display + std::fmt::Debug + Send + Sync + 'static,
) -> Result<()> {
    if condition {
        Ok(())
    } else {
        Err(refusal(code, message))
    }
}

trait RejectAs<T> {
    fn reject_as(self, code: RejectionCode) -> Result<T>;
}

impl<T, E: Into<anyhow::Error>> RejectAs<T> for std::result::Result<T, E> {
    fn reject_as(self, code: RejectionCode) -> Result<T> {
        self.map_err(|error| {
            let error = error.into();
            if error.downcast_ref::<Rejection>().is_some() {
                error
            } else {
                Rejection { code, error }.into()
            }
        })
    }
}

pub(crate) fn rejection_code(error: &anyhow::Error) -> RejectionCode {
    if let Some(rejection) = error.downcast_ref::<Rejection>() {
        return rejection.code;
    }
    error
        .chain()
        .find_map(|cause| {
            if cause
                .downcast_ref::<std::io::Error>()
                .is_some_and(|io| io.kind() == std::io::ErrorKind::WouldBlock)
            {
                return Some(RejectionCode::Busy);
            }
            if let Some(rusqlite::Error::SqliteFailure(failure, _)) =
                cause.downcast_ref::<rusqlite::Error>()
                && matches!(
                    failure.code,
                    rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked
                )
            {
                return Some(RejectionCode::Busy);
            }
            cause
                .downcast_ref::<ProtocolError>()
                .map(|_| RejectionCode::Invalid)
        })
        .unwrap_or(RejectionCode::Failed)
}

fn bounded_text(mut text: String) -> String {
    if text.len() > MAX_REJECTION_TEXT {
        let mut end = MAX_REJECTION_TEXT;
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        text.truncate(end);
    }
    text
}

/// Only the device owner sees the local cause; other principals get a fixed sentence per code.
fn rejected(
    authority: &Authority,
    operation_id: &str,
    code: RejectionCode,
    detail: String,
) -> ManagementResponse {
    let detail = bounded_text(detail);
    tracing::warn!(
        operation_id = if validate_management_id(operation_id).is_ok() {
            operation_id
        } else {
            "<invalid>"
        },
        principal = %authority.principal,
        code = code.as_str(),
        error = %detail,
        "Management command rejected"
    );
    let message = if authority.grant.is_none() {
        detail
    } else {
        code.public_message().to_owned()
    };
    ManagementResponse {
        operation_id: operation_id.to_owned(),
        state: "rejected".into(),
        result: json!({"error":message,"code":code.as_str(),"retryable":code.retryable()}),
    }
}

/// Schema names help an owner see which newer field or command this agent lacks; value
/// errors are reduced to their position so request payloads never reach logs.
fn unsupported_request_detail(error: &serde_json::Error) -> String {
    let message = error.to_string();
    if ["unknown variant", "unknown field", "missing field"]
        .iter()
        .any(|prefix| message.starts_with(prefix))
    {
        format!("This device agent does not support the request: {message}")
    } else {
        format!(
            "This device agent could not decode the request ({:?} error at line {} column {})",
            error.classify(),
            error.line(),
            error.column()
        )
    }
}

pub struct ManagementService {
    state_dir: PathBuf,
    device: Arc<DeviceSession>,
    authority_until: AtomicI64,
    boot_id: String,
}

pub struct ManagementConnection {
    service: Arc<ManagementService>,
    certificate: ControllerCertificate,
    signer: Ed25519PublicKey,
    handshake: Option<noise::Handshake>,
    session: Option<noise::Session>,
    handshake_step: u8,
}

#[derive(Clone)]
struct Authority {
    principal: String,
    key: Ed25519PublicKey,
    grant: Option<ManagementGrant>,
}

impl Authority {
    fn require_current(
        &self,
        store: &StateStore,
        manifest: &OnboardingManifest,
        now: i64,
    ) -> Result<()> {
        self.require_current_connection(&store.connection, manifest, now)
    }
    fn require_current_connection(
        &self,
        connection: &rusqlite::Connection,
        manifest: &OnboardingManifest,
        now: i64,
    ) -> Result<()> {
        if let Some(grant) = &self.grant {
            let compact: String = connection.query_row(
                "SELECT policy_jws FROM management_policy WHERE singleton=1",
                [],
                |row| row.get(0),
            )?;
            let current = verify_management_policy(&compact, &manifest.owner_invitation_key, now)
                .reject_as(RejectionCode::Unauthorized)?;
            refuse_unless(
                current.device_id == manifest.device_id
                    && current
                        .grants
                        .iter()
                        .any(|value| value == grant && value.expires_at > now),
                RejectionCode::Unauthorized,
                "Management grant changed",
            )?;
        }
        Ok(())
    }
    fn require_owner(&self, message: &'static str) -> Result<()> {
        refuse_unless(self.grant.is_none(), RejectionCode::Unauthorized, message)
    }
    fn read_guard(
        &self,
        manifest: &OnboardingManifest,
        request: &ManagementRequest,
        now: i64,
        capability: Option<ManagementCapability>,
        placement: Option<&str>,
    ) -> flow_like_device_crypto::mls_store::SqliteTransactionGuard {
        let authority = self.clone();
        let manifest = manifest.clone();
        let request = request.clone();
        let placement = placement.map(str::to_owned);
        let started = std::time::Instant::now();
        Box::new(move |connection| {
            let now = now.saturating_add(started.elapsed().as_secs() as i64);
            validate_request(&request, &manifest.device_id, now)?;
            authority.require_current_connection(connection, &manifest, now)?;
            if let Some(capability) = &capability {
                let project: Option<String> = placement.as_deref().map(|id| {
                    connection.query_row("SELECT json_extract(config_json,'$.project_id') FROM placements WHERE id=?1", [id], |row| row.get(0))
                }).transpose()?;
                authority.require(capability.clone(), project.as_deref(), placement.as_deref())?;
            }
            Ok(())
        })
    }
    fn permits(
        &self,
        capability: ManagementCapability,
        project: Option<&str>,
        placement: Option<&str>,
    ) -> bool {
        let Some(grant) = &self.grant else {
            return true;
        };
        grant.capabilities.contains(&capability)
            && match &grant.scope {
                ManagementScope::Device => true,
                ManagementScope::Project { project_id } => project == Some(project_id.as_str()),
                ManagementScope::Placement {
                    project_id,
                    placement_id,
                } => {
                    project == Some(project_id.as_str()) && placement == Some(placement_id.as_str())
                }
            }
    }
    fn require(
        &self,
        capability: ManagementCapability,
        project: Option<&str>,
        placement: Option<&str>,
    ) -> Result<()> {
        refuse_unless(
            self.permits(capability.clone(), project, placement),
            RejectionCode::Unauthorized,
            format!("Management capability {capability:?} denied"),
        )
    }

    fn require_certificate_assignment(
        &self,
        store: &StateStore,
        config: &PlacementConfig,
    ) -> Result<()> {
        let previous = store.get_placement(&config.id)?;
        let previous_id = previous
            .as_ref()
            .and_then(|record| record.config.get("tls_certificate_id"))
            .and_then(Value::as_str);
        if previous_id != config.tls_certificate_id.as_deref() {
            // Assigning a certificate gives the selected workload access to its private key.
            // Project deployment authority cannot expand that device-level delegation.
            refuse_unless(
                self.permits(ManagementCapability::ManageCertificates, None, None),
                RejectionCode::Unauthorized,
                "Changing a certificate assignment requires device certificate administration",
            )?;
        }
        Ok(())
    }
}

fn authorized_read<R>(
    store: &StateStore,
    guard: flow_like_device_crypto::mls_store::SqliteTransactionGuard,
    operation: impl FnOnce() -> Result<R>,
) -> Result<R> {
    let transaction = rusqlite::Transaction::new_unchecked(
        &store.connection,
        rusqlite::TransactionBehavior::Immediate,
    )?;
    guard(&transaction)?;
    let result = operation()?;
    guard(&transaction)?;
    transaction.commit()?;
    Ok(result)
}

impl ManagementService {
    pub fn new(state_dir: PathBuf, device: Arc<DeviceSession>, boot_id: String) -> Arc<Self> {
        Arc::new(Self {
            state_dir,
            device,
            authority_until: AtomicI64::new(0),
            boot_id,
        })
    }

    /// Cloud admission can suspend management; it cannot create a controller key or capability.
    /// Returns the admission's expiry on the device clock, which a skewed clock cannot extend
    /// beyond the longest admission lifetime.
    pub fn refresh_authority(&self, expires_at: i64) -> Result<i64> {
        let now = unix_time()?;
        ensure!(
            expires_at > now
                && expires_at
                    <= now + crate::enrollment::ADMISSION_SECONDS + MAX_CLOCK_SKEW_SECONDS,
            "Invalid management authority lease: expires at {expires_at}, device clock reads {now}"
        );
        let local = expires_at.min(now + crate::enrollment::ADMISSION_SECONDS);
        self.authority_until.store(local, Ordering::Release);
        Ok(local)
    }

    pub async fn synchronize_policy(&self, admission: &DeviceSignalingResponse) -> Result<()> {
        let store = StateStore::open(&self.state_dir.join("management.sqlite"))?;
        let mut head = store.management_policy_head()?;
        // A stale or rolled-back control plane cannot extend access from an older roster.
        ensure!(
            admission.policy_version >= head.as_ref().map_or(0, |p| p.0),
            "Control plane management policy is stale"
        );
        for _ in 0..32 {
            let version = head.as_ref().map_or(0, |p| p.0);
            if version == admission.policy_version {
                break;
            }
            let policies = self.device.management_policies(version).await?;
            ensure!(
                !policies.is_empty() && policies.len() <= 32,
                "Management policy history is incomplete"
            );
            for compact in policies {
                let policy = verify_historical_management_policy(
                    &compact,
                    &self.device.manifest().owner_invitation_key,
                )?;
                ensure!(
                    policy.policy_version <= admission.policy_version,
                    "Management policy changed during synchronization"
                );
                store.accept_management_policy(
                    &compact,
                    &self.device.manifest().owner_invitation_key,
                    &self.device.manifest().device_id,
                    policy.issued_at,
                )?;
            }
            head = store.management_policy_head()?;
        }
        ensure!(
            head.as_ref().map_or(0, |p| p.0) == admission.policy_version
                && head.as_ref().map(|p| p.1.clone()) == admission.policy_digest,
            "Management policy head mismatch"
        );
        if let Some((version, digest)) = head {
            self.device.acknowledge_policy(version, &digest).await?;
        }
        Ok(())
    }

    fn current_authority(&self, store: &StateStore, grant_id: &str, now: i64) -> Result<Authority> {
        ensure!(
            now < self.authority_until.load(Ordering::Acquire),
            "Management admission needs renewal"
        );
        let manifest = self.device.manifest();
        if grant_id == "owner" {
            return Ok(Authority {
                principal: format!("{}:owner", manifest.owner_id),
                key: manifest.controller_key.clone(),
                grant: None,
            });
        }
        let policy = store
            .management_policy(&manifest.owner_invitation_key, now)?
            .context("No owner-approved management policy")?;
        ensure!(
            policy.device_id == manifest.device_id,
            "Management policy device mismatch"
        );
        let grant = policy
            .grants
            .into_iter()
            .find(|g| g.grant_id == grant_id && g.expires_at > now)
            .context("Management grant is missing or expired")?;
        Ok(Authority {
            principal: format!(
                "{}:{}:{}",
                grant.user_id,
                grant.grant_id,
                grant.controller_key.thumbprint()?
            ),
            key: grant.controller_key.clone(),
            grant: Some(grant),
        })
    }

    pub fn connect(
        self: &Arc<Self>,
        certificate_jws: &str,
        grant_id: &str,
        session_id: &str,
    ) -> Result<ManagementConnection> {
        let store = StateStore::open(&self.state_dir.join("management.sqlite"))?;
        let now = unix_time()?;
        let authority = self.current_authority(&store, grant_id, now)?;
        let certificate = verify_controller_certificate(certificate_jws, &authority.key, now)?;
        ensure!(
            certificate.device_id == self.device.manifest().device_id
                && certificate.grant_id == grant_id
                && certificate.session_id == session_id,
            "Controller certificate binding mismatch"
        );
        let handshake = self
            .device
            .noise_responder(&certificate.management_key, &certificate.session_id)?;
        Ok(ManagementConnection {
            service: self.clone(),
            certificate,
            signer: authority.key,
            handshake: Some(handshake),
            session: None,
            handshake_step: 0,
        })
    }
}

impl ManagementConnection {
    /// One reliable, ordered transport owns this state. Any error closes it permanently.
    pub async fn receive(&mut self, bytes: &[u8]) -> Result<Vec<u8>> {
        let result = self.receive_inner(bytes).await;
        if result.is_err() {
            self.handshake.take();
            self.session.take();
        }
        result
    }

    async fn receive_inner(&mut self, bytes: &[u8]) -> Result<Vec<u8>> {
        let now = unix_time()?;
        ensure!(
            now < self.certificate.expires_at,
            "Management session expired"
        );
        let mut store = StateStore::open(&self.service.state_dir.join("management.sqlite"))?;
        let authority = self
            .service
            .current_authority(&store, &self.certificate.grant_id, now)?;
        ensure!(authority.key == self.signer, "Controller key was revoked");
        if let Some(mut handshake) = self.handshake.take() {
            handshake.read(bytes)?;
            if self.handshake_step == 0 {
                let response = handshake.write()?;
                self.handshake = Some(handshake);
                self.handshake_step = 1;
                return Ok(response);
            }
            self.session = Some(handshake.finish()?);
            return Ok(self.session.as_mut().context("Missing Noise session")?.encrypt(&serde_json::to_vec(&json!({"ready":true,"device_id":self.certificate.device_id,"boot_id":self.service.boot_id,"expires_at":self.certificate.expires_at}))?)?);
        }
        let session = self
            .session
            .as_mut()
            .context("Management connection is closed")?;
        let plaintext = zeroize::Zeroizing::new(session.decrypt(bytes)?);
        let request: ManagementRequest = match serde_json::from_slice(&plaintext) {
            Ok(request) => request,
            Err(error) => {
                // A JSON object with an operation ID is a request from a newer controller;
                // anything else is not a management request and closes the session.
                // Other fields are skipped unread, so no secret is copied out of the buffer.
                #[derive(serde::Deserialize)]
                struct Envelope {
                    operation_id: String,
                }
                let envelope = plaintext
                    .trim_ascii_start()
                    .starts_with(b"{")
                    .then(|| serde_json::from_slice::<Envelope>(&plaintext).ok())
                    .flatten();
                let Some(envelope) = envelope else {
                    return Err(anyhow::Error::from(error).context("Decode management request"));
                };
                let response = rejected(
                    &authority,
                    &envelope.operation_id,
                    RejectionCode::Unsupported,
                    unsupported_request_detail(&error),
                );
                return Ok(session.encrypt(&serde_json::to_vec(&response)?)?);
            }
        };
        let response = if matches!(request.command, ManagementCommand::Artifact { .. }) {
            drop(store);
            let service = self.service.clone();
            let request = request.clone();
            let grant = self.certificate.grant_id.clone();
            let signer = self.signer.clone();
            tokio::task::spawn_blocking(move || {
                let store = StateStore::open(&service.state_dir.join("management.sqlite"))?;
                let now = unix_time()?;
                let authority = service.current_authority(&store, &grant, now)?;
                refuse_unless(
                    authority.key == signer,
                    RejectionCode::Unauthorized,
                    "Artifact controller key changed",
                )?;
                execute_artifact(&store, &authority, &request, &service, now)
            })
            .await
            .map_err(anyhow::Error::from)
            .and_then(|result| result)
        } else if matches!(
            request.command,
            ManagementCommand::TelemetryPolicy { .. }
                | ManagementCommand::TelemetryRead { .. }
                | ManagementCommand::TelemetryRosterRead { .. }
                | ManagementCommand::TelemetryAcknowledge { .. }
                | ManagementCommand::TelemetryReceipt { .. }
        ) {
            execute_telemetry_group(&store, &authority, &request, &self.service, now)
        } else {
            execute(
                &mut store,
                &authority,
                &request,
                self.service.device.manifest(),
                &self.service.boot_id,
                &self.service.state_dir,
                now,
            )
        };
        let response = response.unwrap_or_else(|error| {
            rejected(
                &authority,
                &request.operation_id,
                rejection_code(&error),
                format!("{error:#}"),
            )
        });
        Ok(session.encrypt(&serde_json::to_vec(&response)?)?)
    }
}

impl StateStore {
    pub(crate) fn management_policy_head(&self) -> Result<Option<(u64, String)>> {
        Ok(self
            .connection
            .query_row(
                "SELECT policy_version,policy_digest FROM management_policy WHERE singleton=1",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?)
    }
    pub fn management_policy(
        &self,
        owner: &Ed25519PublicKey,
        now: i64,
    ) -> Result<Option<ManagementPolicy>> {
        let compact: Option<String> = self
            .connection
            .query_row(
                "SELECT policy_jws FROM management_policy WHERE singleton=1",
                [],
                |row| row.get(0),
            )
            .optional()?;
        compact
            .map(|compact| verify_management_policy(&compact, owner, now).map_err(Into::into))
            .transpose()
    }

    pub fn accept_management_policy(
        &self,
        compact: &str,
        owner: &Ed25519PublicKey,
        device_id: &str,
        now: i64,
    ) -> Result<()> {
        let own_transaction = self.connection.is_autocommit();
        if own_transaction {
            self.connection.execute_batch("BEGIN IMMEDIATE")?;
        }
        let result = self.accept_management_policy_transaction(compact, owner, device_id, now);
        if own_transaction {
            match result {
                Ok(()) => {
                    self.connection.execute_batch("COMMIT")?;
                    return Ok(());
                }
                Err(error) => {
                    let _ = self.connection.execute_batch("ROLLBACK");
                    return Err(error);
                }
            }
        }
        result
    }

    fn accept_management_policy_transaction(
        &self,
        compact: &str,
        owner: &Ed25519PublicKey,
        device_id: &str,
        now: i64,
    ) -> Result<()> {
        let policy = verify_management_policy(compact, owner, now)?;
        ensure!(
            policy.device_id == device_id,
            "Management policy device mismatch"
        );
        let previous: Option<(u64, String)> = self
            .connection
            .query_row(
                "SELECT policy_version,policy_digest FROM management_policy WHERE singleton=1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        let digest = compact_digest(compact);
        if previous
            .as_ref()
            .is_some_and(|(version, old)| *version == policy.policy_version && *old == digest)
        {
            return Ok(());
        }
        match previous {
            Some((version, old)) => ensure!(
                policy.policy_version
                    == version.checked_add(1).context("Policy revision overflow")?
                    && policy.previous_policy_digest.as_ref() == Some(&old),
                "Management policy chain mismatch"
            ),
            None => ensure!(
                policy.policy_version == 1 && policy.previous_policy_digest.is_none(),
                "Management policy genesis required"
            ),
        }
        self.connection.execute("INSERT INTO management_policy(singleton,policy_version,policy_digest,policy_jws) VALUES(1,?1,?2,?3) ON CONFLICT(singleton) DO UPDATE SET policy_version=excluded.policy_version,policy_digest=excluded.policy_digest,policy_jws=excluded.policy_jws", params![policy.policy_version,digest,compact])?;
        Ok(())
    }
}

fn placement_scope(
    store: &StateStore,
    id: &str,
) -> Result<(crate::state::PlacementRecord, String)> {
    validate_management_id(id).reject_as(RejectionCode::Invalid)?;
    let record = store.get_placement(id)?.ok_or_else(|| {
        refusal(
            RejectionCode::RevisionConflict,
            format!("Unknown placement {id}"),
        )
    })?;
    let project = record
        .config
        .get("project_id")
        .and_then(Value::as_str)
        .context("Invalid placement identity")?
        .to_owned();
    Ok((record, project))
}

fn known_rollout(store: &StateStore, rollout_id: &str) -> Result<crate::rollout::RolloutRecord> {
    store.rollout(rollout_id)?.ok_or_else(|| {
        refusal(
            RejectionCode::Invalid,
            format!("Unknown workflow update {rollout_id}"),
        )
    })
}

/// Controller clocks may differ from the device by a bounded skew. Noise already rejects
/// replays, and mutating operation IDs stay journaled well beyond this window.
fn validate_request(request: &ManagementRequest, device_id: &str, now: i64) -> Result<()> {
    validate_management_id(&request.operation_id).reject_as(RejectionCode::Invalid)?;
    refuse_unless(
        request.device_id == device_id
            && request.issued_at > 0
            && request.issued_at <= now + MAX_CLOCK_SKEW_SECONDS
            && request.expires_at > now - MAX_CLOCK_SKEW_SECONDS
            && request.expires_at > request.issued_at
            && request.expires_at.saturating_sub(request.issued_at) <= 300,
        RejectionCode::Invalid,
        format!(
            "Invalid management request binding or lifetime: issued {}, expires {}, device clock {now}",
            request.issued_at, request.expires_at
        ),
    )
}

/// Prunes rows past their replay and status window, then admits one more row for this
/// principal. A grantee's quota cannot exhaust the journal for the owner or other grants.
fn reserve_journal_entry(
    connection: &rusqlite::Connection,
    authority: &Authority,
    now: i64,
) -> Result<()> {
    connection.execute(
        "DELETE FROM management_operations WHERE accepted_at<?1
            AND operation_id NOT IN (SELECT operation_id FROM host_operations WHERE state IN ('pending','staging','draining','requesting','requested','unknown'))
            AND operation_id NOT IN (SELECT operation_id FROM secret_operations WHERE state='pending')",
        [now.saturating_sub(JOURNAL_RETENTION_SECONDS)],
    )?;
    let (total, own): (u64, u64) = connection.query_row(
        "SELECT COUNT(*),COALESCE(SUM(principal=?1),0) FROM management_operations",
        [&authority.principal],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    refuse_unless(
        total < MAX_JOURNAL_ENTRIES,
        RejectionCode::Limit,
        format!("Management journal is full ({total} operations in the last day)"),
    )?;
    refuse_unless(
        authority.grant.is_none() || own < MAX_GRANT_JOURNAL_ENTRIES,
        RejectionCode::Limit,
        format!(
            "Management grant reached its limit of {MAX_GRANT_JOURNAL_ENTRIES} recorded operations per day"
        ),
    )
}

fn previous_operation(
    connection: &rusqlite::Connection,
    operation_id: &str,
    digest: &str,
    authority: &Authority,
) -> Result<Option<ManagementResponse>> {
    let previous: Option<(String, String, String)> = connection
        .query_row(
            "SELECT request_digest,principal,result_json FROM management_operations WHERE operation_id=?1",
            [operation_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()?;
    previous
        .map(|(old, principal, result)| {
            refuse_unless(
                old == digest && principal == authority.principal,
                RejectionCode::Invalid,
                format!("Operation ID {operation_id} already belongs to a different request"),
            )?;
            Ok(serde_json::from_str(&result)?)
        })
        .transpose()
}

fn execute_artifact(
    store: &StateStore,
    authority: &Authority,
    request: &ManagementRequest,
    service: &ManagementService,
    now: i64,
) -> Result<ManagementResponse> {
    validate_request(request, &service.device.manifest().device_id, now)?;
    let ManagementCommand::Artifact { request: artifact } = &request.command else {
        anyhow::bail!("Invalid artifact command")
    };
    authority.require(
        ManagementCapability::Deploy,
        Some(artifact.project_id()),
        None,
    )?;
    let digest = compact_digest(&serde_json::to_string(request)?);
    if artifact.journaled() {
        match previous_operation(&store.connection, &request.operation_id, &digest, authority)? {
            Some(result) if result.state != "pending" => return Ok(result),
            Some(_) => {}
            None => {
                reserve_journal_entry(&store.connection, authority, now)?;
                let pending = ManagementResponse {
                    operation_id: request.operation_id.clone(),
                    state: "pending".into(),
                    result: json!({"project_id":artifact.project_id()}),
                };
                let inserted=store.connection.execute("INSERT INTO management_operations(operation_id,request_digest,principal,project_id,accepted_at,result_json) VALUES(?1,?2,?3,?4,?5,?6) ON CONFLICT(operation_id) DO NOTHING",params![request.operation_id,digest,authority.principal,artifact.project_id(),now,serde_json::to_string(&pending)?])?;
                refuse_unless(
                    inserted == 1,
                    RejectionCode::Busy,
                    "Artifact operation was claimed concurrently; retry its ID",
                )?;
            }
        }
    }
    use crate::project_artifacts as artifacts;
    let root = &service.state_dir;
    let principal = &authority.principal;
    let result = match artifact {
        ArtifactRequest::Begin { descriptor } => serde_json::to_value(artifacts::begin(
            store,
            root,
            &request.operation_id,
            principal,
            descriptor,
        )?)?,
        ArtifactRequest::Chunk {
            project_id,
            transfer_id,
            file_index,
            offset,
            data,
        } => serde_json::to_value(artifacts::chunk(
            store,
            root,
            project_id,
            transfer_id,
            principal,
            *file_index,
            *offset,
            data,
        )?)?,
        ArtifactRequest::Status {
            project_id,
            transfer_id,
            file_index,
        } => serde_json::to_value(artifacts::status(
            store,
            root,
            project_id,
            transfer_id,
            principal,
            *file_index,
        )?)?,
        ArtifactRequest::Commit {
            project_id,
            transfer_id,
        } => serde_json::to_value(artifacts::commit(
            store,
            root,
            project_id,
            transfer_id,
            principal,
        )?)?,
        ArtifactRequest::Abort {
            project_id,
            transfer_id,
        } => serde_json::to_value(artifacts::abort(
            store,
            root,
            project_id,
            transfer_id,
            principal,
        )?)?,
        ArtifactRequest::PrepareOnline { project_id } => {
            json!({"project_id":project_id,"project_path":artifacts::prepare_online_cache(root,project_id)?})
        }
        ArtifactRequest::Describe {
            project_id,
            revision,
            event_id,
            after,
        } => {
            #[cfg(feature = "runtime")]
            {
                // Parsing stays on the artifact blocking worker. Fence the final read against
                // policy changes, including expiry while local metadata was being loaded.
                let guard =
                    authority.read_guard(service.device.manifest(), request, now, None, None);
                let result =
                    tokio::runtime::Handle::current().block_on(crate::deployment::describe(
                        root,
                        project_id,
                        revision,
                        event_id.as_deref(),
                        after.as_deref(),
                    ))?;
                authorized_read(store, guard, || {
                    authority.require(ManagementCapability::Deploy, Some(project_id), None)?;
                    Ok(result)
                })?
            }
            #[cfg(not(feature = "runtime"))]
            {
                let _ = (project_id, revision, event_id, after);
                return Err(refusal(
                    RejectionCode::Unsupported,
                    "Project discovery requires the runtime build",
                ));
            }
        }
    };
    let response = ManagementResponse {
        operation_id: request.operation_id.clone(),
        state: "completed".into(),
        result,
    };
    if artifact.journaled() {
        store.connection.execute("UPDATE management_operations SET result_json=?2 WHERE operation_id=?1 AND request_digest=?3 AND principal=?4",params![request.operation_id,serde_json::to_string(&response)?,digest,authority.principal])?;
    }
    Ok(response)
}

fn host_isolation_mode(capabilities: &crate::isolation::Capabilities) -> &'static str {
    if capabilities.require_isolation {
        "required"
    } else if capabilities.sandbox_available {
        "optional"
    } else {
        "none"
    }
}

/// `host_isolation` tells a granting owner whether project deployments run sandboxed
/// ("required"), may choose to ("optional"), or run as the agent's OS account ("none").
fn inspection_result(
    authority: &Authority,
    manifest: &OnboardingManifest,
    boot_id: &str,
    state_dir: &Path,
    placements: Vec<Value>,
) -> Value {
    let device_status = authority.permits(ManagementCapability::Status, None, None);
    let isolation = device_status.then(|| crate::isolation::capabilities(state_dir));
    let mut result = json!({
        "device_id":manifest.device_id,
        "agent_version":env!("CARGO_PKG_VERSION"),
        "certificate_management":1,
        "certificate_issuance":1,
        "certificate_acme":1,
        "features":crate::diagnostics::features(),
        "can_delegate_certificate_renewal":authority.grant.is_none(),
        "can_manage_certificates":authority.permits(ManagementCapability::ManageCertificates,None,None),
        "host_operations":{"reboot":REMOTE_HOST_OPERATIONS,"update_agent":REMOTE_HOST_OPERATIONS},
        "boot_id":device_status.then_some(boot_id),
        "host_isolation":isolation.as_ref().map(host_isolation_mode),
        "isolation":isolation,
        "placements":placements
    });
    if device_status && let Value::Object(object) = &mut result {
        object.extend(crate::diagnostics::global().device_facts(state_dir, false));
    }
    result
}

/// A reply that fits one encrypted message: a two-row page first drops to one row,
/// then the row sheds detail. `page` carries the cursor of a paged read.
fn fitted_inspection(
    authority: &Authority,
    manifest: &OnboardingManifest,
    boot_id: &str,
    state_dir: &Path,
    operation_id: &str,
    records: &[crate::state::PlacementRecord],
    page: Option<Option<String>>,
) -> Result<ManagementResponse> {
    use crate::diagnostics::{DETAILS, Detail, Rows};
    let rows = Rows::new(crate::diagnostics::global(), state_dir, false);
    let error_text: Vec<bool> = records
        .iter()
        .map(|record| {
            authority.permits(
                ManagementCapability::Logs,
                record.config.get("project_id").and_then(Value::as_str),
                Some(&record.id),
            )
        })
        .collect();
    let build = |count: usize, detail: Detail, next: Option<&str>| {
        let placements = records[..count]
            .iter()
            .zip(&error_text)
            .map(|(record, error_text)| rows.placement(record, *error_text, detail))
            .collect();
        let mut result = inspection_result(authority, manifest, boot_id, state_dir, placements);
        if page.is_some() {
            result["next"] = json!(next);
        }
        ManagementResponse {
            operation_id: operation_id.to_owned(),
            state: "completed".into(),
            result,
        }
    };
    let next = page.clone().flatten();
    let mut attempts = Vec::new();
    if page.is_some() && records.len() > 1 {
        attempts.push((records.len(), Detail::Full, next.as_deref()));
        let first = records.first().map(|record| record.id.as_str());
        attempts.extend(DETAILS.map(|detail| (1, detail, first)));
    } else {
        attempts.extend(DETAILS.map(|detail| (records.len(), detail, next.as_deref())));
    }
    for (count, detail, next) in attempts {
        let response = build(count, detail, next);
        if serde_json::to_vec(&response)?.len() <= noise::MAX_PLAINTEXT {
            return Ok(response);
        }
    }
    Err(refusal(
        RejectionCode::Limit,
        "Inspection exceeds the encrypted message limit even at minimal detail",
    ))
}

fn execute_telemetry_group(
    store: &StateStore,
    authority: &Authority,
    request: &ManagementRequest,
    service: &ManagementService,
    now: i64,
) -> Result<ManagementResponse> {
    validate_request(request, &service.device.manifest().device_id, now)?;
    let scope = match &request.command {
        ManagementCommand::TelemetryPolicy { scope, .. }
        | ManagementCommand::TelemetryRead { scope, .. }
        | ManagementCommand::TelemetryRosterRead { scope, .. }
        | ManagementCommand::TelemetryAcknowledge { scope, .. }
        | ManagementCommand::TelemetryReceipt { scope, .. } => scope,
        _ => anyhow::bail!("Invalid telemetry group command"),
    };
    let project = if scope == "device" {
        None
    } else {
        Some(placement_scope(store, scope)?.1)
    };
    let placement = if scope == "device" {
        None
    } else {
        Some(scope.as_str())
    };
    let mutating = !matches!(
        request.command,
        ManagementCommand::TelemetryRead { .. }
            | ManagementCommand::TelemetryRosterRead { .. }
            | ManagementCommand::TelemetryReceipt { .. }
    );
    if mutating {
        authority.require_owner(
            "Only the owner may change telemetry membership or acknowledge the shared outbox",
        )?;
    } else {
        refuse_unless(
            authority.permits(ManagementCapability::Metrics, project.as_deref(), placement),
            RejectionCode::Unauthorized,
            "Telemetry access denied",
        )?;
    }
    let digest = compact_digest(&serde_json::to_string(request)?);
    if mutating {
        match previous_operation(&store.connection, &request.operation_id, &digest, authority)? {
            Some(response) if response.state != "pending" => return Ok(response),
            Some(_) => {}
            None => {
                reserve_journal_entry(&store.connection, authority, now)?;
                let pending = ManagementResponse {
                    operation_id: request.operation_id.clone(),
                    state: "pending".into(),
                    result: json!({"scope":scope}),
                };
                let inserted=store.connection.execute("INSERT INTO management_operations(operation_id,request_digest,principal,project_id,placement_id,accepted_at,result_json) VALUES(?1,?2,?3,?4,?5,?6,?7) ON CONFLICT(operation_id) DO NOTHING",params![request.operation_id,digest,authority.principal,project,placement,now,serde_json::to_string(&pending)?])?;
                refuse_unless(
                    inserted == 1,
                    RejectionCode::Busy,
                    "Operation was claimed concurrently; retry its ID",
                )?;
            }
        }
    }
    let result = match &request.command {
        ManagementCommand::TelemetryPolicy {
            scope,
            sequence,
            policy_jws,
            key_packages,
        } => crate::telemetry_groups::apply_policy(
            &service.state_dir,
            &service.device,
            scope,
            &request.operation_id,
            *sequence,
            policy_jws,
            key_packages,
        )?,
        ManagementCommand::TelemetryRead {
            scope,
            sequence,
            welcome,
            offset,
            limit,
        } => crate::telemetry_groups::read(
            &service.state_dir,
            &service.device,
            scope,
            *sequence,
            *welcome,
            *offset,
            *limit,
            authority.read_guard(
                service.device.manifest(),
                request,
                now,
                Some(ManagementCapability::Metrics),
                placement,
            ),
        )?,
        ManagementCommand::TelemetryRosterRead {
            scope,
            offset,
            limit,
        } => crate::telemetry_groups::read_roster(
            &service.state_dir,
            scope,
            *offset,
            *limit,
            authority.read_guard(
                service.device.manifest(),
                request,
                now,
                Some(ManagementCapability::Metrics),
                placement,
            ),
        )?,
        ManagementCommand::TelemetryAcknowledge {
            scope,
            sequence,
            envelope_digest,
        } => {
            crate::telemetry_groups::acknowledge(
                &service.state_dir,
                &service.device,
                scope,
                *sequence,
                envelope_digest,
            )?;
            json!({"acknowledged":sequence})
        }
        ManagementCommand::TelemetryReceipt {
            scope,
            endpoint_id,
            sequence,
            receipt_jws,
        } => crate::telemetry_groups::receive_receipt(
            &service.state_dir,
            &service.device,
            scope,
            endpoint_id,
            *sequence,
            receipt_jws,
            authority.read_guard(
                service.device.manifest(),
                request,
                now,
                Some(ManagementCapability::Metrics),
                placement,
            ),
        )?,
        _ => unreachable!(),
    };
    let response = ManagementResponse {
        operation_id: request.operation_id.clone(),
        state: "completed".into(),
        result,
    };
    if mutating {
        store.connection.execute("UPDATE management_operations SET result_json=?2 WHERE operation_id=?1 AND request_digest=?3 AND principal=?4",params![request.operation_id,serde_json::to_string(&response)?,digest,authority.principal])?;
    }
    Ok(response)
}

fn execute(
    store: &mut StateStore,
    authority: &Authority,
    request: &ManagementRequest,
    manifest: &OnboardingManifest,
    boot_id: &str,
    state_dir: &Path,
    now: i64,
) -> Result<ManagementResponse> {
    validate_request(request, &manifest.device_id, now)?;
    match &request.command {
        ManagementCommand::AcmeCertificates { after, limit } => {
            refuse_unless(
                (1..=8).contains(limit),
                RejectionCode::Invalid,
                "ACME policy page limit must be between 1 and 8",
            )?;
            if let Some(after) = after {
                validate_certificate_id(after).reject_as(RejectionCode::Invalid)?;
            }
            return authorized_read(
                store,
                authority.read_guard(manifest, request, now, None, None),
                || {
                    authority.require_owner("Only the device owner can manage ACME renewal")?;
                    let mut policies = Vec::new();
                    let mut next = None;
                    for policy in crate::acme::list(store)? {
                        if after
                            .as_deref()
                            .is_some_and(|after| policy.certificate_id.as_str() <= after)
                        {
                            continue;
                        }
                        if policies.len() >= usize::from(*limit) {
                            next = policies
                                .last()
                                .map(|item: &AcmeCertificateMetadata| item.certificate_id.clone());
                            break;
                        }
                        policies.push(policy);
                        if serde_json::to_vec(&policies)?.len() > noise::MAX_PLAINTEXT - 1024 {
                            policies.pop();
                            ensure!(
                                !policies.is_empty(),
                                "ACME policy exceeds the encrypted response limit"
                            );
                            next = policies.last().map(|item| item.certificate_id.clone());
                            break;
                        }
                    }
                    Ok(ManagementResponse {
                        operation_id: request.operation_id.clone(),
                        state: "completed".into(),
                        result: json!({"policies":policies,"next":next}),
                    })
                },
            );
        }
        ManagementCommand::CertificateIssuers { after, limit } => {
            refuse_unless(
                (1..=8).contains(limit),
                RejectionCode::Invalid,
                "Certificate issuer page limit must be between 1 and 8",
            )?;
            if let Some(after) = after {
                validate_certificate_id(after).reject_as(RejectionCode::Invalid)?;
            }
            return authorized_read(
                store,
                authority.read_guard(manifest, request, now, None, None),
                || {
                    authority.require_owner(
                        "Only the device owner can manage certificate renewal authorities",
                    )?;
                    let mut issuers = Vec::new();
                    let mut next = None;
                    for item in crate::certificate_issuers::list(store)? {
                        if after
                            .as_deref()
                            .is_some_and(|after| item.certificate_id.as_str() <= after)
                        {
                            continue;
                        }
                        if issuers.len() >= usize::from(*limit) {
                            next = issuers.last().map(|item: &CertificateIssuerMetadata| {
                                item.certificate_id.clone()
                            });
                            break;
                        }
                        issuers.push(item);
                        if serde_json::to_vec(&issuers)?.len() > noise::MAX_PLAINTEXT - 1024 {
                            issuers.pop();
                            ensure!(
                                !issuers.is_empty(),
                                "Certificate issuer exceeds the encrypted response limit"
                            );
                            next = issuers.last().map(|item| item.certificate_id.clone());
                            break;
                        }
                    }
                    Ok(ManagementResponse {
                        operation_id: request.operation_id.clone(),
                        state: "completed".into(),
                        result: json!({"issuers":issuers,"next":next}),
                    })
                },
            );
        }
        ManagementCommand::CertificateRequests { after, limit } => {
            refuse_unless(
                (1..=8).contains(limit),
                RejectionCode::Invalid,
                "Certificate request page limit must be between 1 and 8",
            )?;
            if let Some(after) = after {
                validate_certificate_id(after).reject_as(RejectionCode::Invalid)?;
            }
            return authorized_read(
                store,
                authority.read_guard(manifest, request, now, None, None),
                || {
                    authority.require(ManagementCapability::ManageCertificates, None, None)?;
                    let mut requests = Vec::new();
                    let mut next = None;
                    for item in crate::certificate_requests::list(store)? {
                        if item.purpose == CertificateRequestPurpose::Issuer
                            && authority.grant.is_some()
                        {
                            continue;
                        }
                        if after
                            .as_deref()
                            .is_some_and(|after| item.request_id.as_str() <= after)
                        {
                            continue;
                        }
                        if requests.len() >= usize::from(*limit) {
                            next = requests
                                .last()
                                .map(|item: &CertificateSigningRequest| item.request_id.clone());
                            break;
                        }
                        requests.push(item);
                        if serde_json::to_vec(&requests)?.len() > noise::MAX_PLAINTEXT - 1024 {
                            requests.pop();
                            ensure!(
                                !requests.is_empty(),
                                "Certificate request exceeds the encrypted response limit"
                            );
                            next = requests.last().map(|item| item.request_id.clone());
                            break;
                        }
                    }
                    Ok(ManagementResponse {
                        operation_id: request.operation_id.clone(),
                        state: "completed".into(),
                        result: json!({"requests":requests,"next":next}),
                    })
                },
            );
        }
        ManagementCommand::Certificates {
            placement_id,
            after,
            limit,
        } => {
            refuse_unless(
                (1..=8).contains(limit),
                RejectionCode::Invalid,
                "Certificate page limit must be between 1 and 8",
            )?;
            if let Some(after) = after {
                validate_certificate_id(after).reject_as(RejectionCode::Invalid)?;
            }
            return authorized_read(
                store,
                authority.read_guard(manifest, request, now, None, None),
                || {
                    if let Some(id) = placement_id {
                        let (_, project) = placement_scope(store, id)?;
                        authority.require(
                            ManagementCapability::Status,
                            Some(&project),
                            Some(id),
                        )?;
                    } else if let Some(grant) = &authority.grant {
                        refuse_unless(
                            grant.capabilities.contains(&ManagementCapability::Status),
                            RejectionCode::Unauthorized,
                            "Status access denied",
                        )?;
                    }
                    let mut certificates = Vec::new();
                    let mut next = None;
                    let inventory_revision = crate::certificates::inventory_revision(store)?;
                    for mut certificate in crate::certificates::list(store)? {
                        if after
                            .as_deref()
                            .is_some_and(|after| certificate.certificate_id.as_str() <= after)
                        {
                            continue;
                        }
                        certificate.bindings.retain(|binding| {
                            placement_id
                                .as_deref()
                                .is_none_or(|id| binding.placement_id == id)
                                && authority.permits(
                                    ManagementCapability::Status,
                                    Some(&binding.project_id),
                                    Some(&binding.placement_id),
                                )
                        });
                        if (placement_id.is_some()
                            || !authority.permits(ManagementCapability::Status, None, None))
                            && certificate.bindings.is_empty()
                        {
                            continue;
                        }
                        if certificates.len() >= usize::from(*limit) {
                            next = certificates
                                .last()
                                .map(|value: &CertificateMetadata| value.certificate_id.clone());
                            break;
                        }
                        certificates.push(crate::certificates::bounded_metadata(certificate)?);
                        // Leave room for the response envelope and continuation cursor.
                        if serde_json::to_vec(&certificates)?.len() > noise::MAX_PLAINTEXT - 1024 {
                            certificates.pop();
                            ensure!(
                                !certificates.is_empty(),
                                "Certificate bindings exceed the encrypted response limit"
                            );
                            next = certificates
                                .last()
                                .map(|value| value.certificate_id.clone());
                            break;
                        }
                    }
                    Ok(ManagementResponse {
                        operation_id: request.operation_id.clone(),
                        state: "completed".into(),
                        result: json!({"certificates":certificates,"inventory_revision":inventory_revision,"next":next}),
                    })
                },
            );
        }
        ManagementCommand::OfflineQueue {
            placement_id,
            after,
        } => {
            return authorized_read(
                store,
                authority.read_guard(
                    manifest,
                    request,
                    now,
                    Some(ManagementCapability::Status),
                    Some(placement_id),
                ),
                || {
                    let (record, project_id) = placement_scope(store, placement_id)?;
                    authority.require(
                        ManagementCapability::Status,
                        Some(&project_id),
                        Some(placement_id),
                    )?;
                    let config: PlacementConfig = serde_json::from_value(record.config)?;
                    let mut available = crate::outbox::for_placement(state_dir, &config)?;
                    available.sort_by(|a, b| a.scope().cmp(b.scope()));
                    let mut page = available
                        .iter()
                        .filter(|queue| after.as_deref().is_none_or(|after| queue.scope() > after))
                        .take(3)
                        .collect::<Vec<_>>();
                    let next = if page.len() > 2 {
                        page.pop();
                        page.last().map(|queue| queue.scope().to_owned())
                    } else {
                        None
                    };
                    let queues = page
                        .into_iter()
                        .map(crate::outbox::Outbox::status)
                        .collect::<Result<Vec<_>>>()?;
                    let response = ManagementResponse {
                        operation_id: request.operation_id.clone(),
                        state: "completed".into(),
                        result: json!({"placement_id":placement_id,"queues":queues,"next":next}),
                    };
                    refuse_unless(
                        serde_json::to_vec(&response)?.len() <= noise::MAX_PLAINTEXT,
                        RejectionCode::Limit,
                        "Offline queue status exceeds the encrypted message limit",
                    )?;
                    Ok(response)
                },
            );
        }
        ManagementCommand::InspectPage { after, limit } => {
            refuse_unless(
                (1..=2).contains(limit),
                RejectionCode::Invalid,
                "Inspection page limit must be between 1 and 2",
            )?;
            if let Some(after) = after {
                validate_management_id(after).reject_as(RejectionCode::Invalid)?;
            }
            let (project, placement) = if let Some(grant) = &authority.grant {
                refuse_unless(
                    grant.capabilities.contains(&ManagementCapability::Status),
                    RejectionCode::Unauthorized,
                    "Status access denied",
                )?;
                match &grant.scope {
                    ManagementScope::Device => (None, None),
                    ManagementScope::Project { project_id } => (Some(project_id.as_str()), None),
                    ManagementScope::Placement {
                        project_id,
                        placement_id,
                    } => (Some(project_id.as_str()), Some(placement_id.as_str())),
                }
            } else {
                (None, None)
            };
            let guard = authority.read_guard(manifest, request, now, None, None);
            let transaction = rusqlite::Transaction::new_unchecked(
                &store.connection,
                rusqlite::TransactionBehavior::Immediate,
            )?;
            guard(&transaction)?;
            let ids = store.connection.prepare("SELECT id FROM placements WHERE id>?1 AND (?2 IS NULL OR json_extract(config_json,'$.project_id')=?2) AND (?3 IS NULL OR id=?3) ORDER BY id LIMIT ?4")?
                .query_map(params![after.as_deref().unwrap_or(""), project, placement, u32::from(*limit)+1], |r| r.get::<_, String>(0))?
                .collect::<Result<Vec<_>, _>>()?;
            let more = ids.len() > usize::from(*limit);
            let mut records = Vec::new();
            for id in ids.into_iter().take(usize::from(*limit)) {
                if let Some(record) = store.get_placement(&id)? {
                    authority.require(
                        ManagementCapability::Status,
                        record.config.get("project_id").and_then(Value::as_str),
                        Some(&record.id),
                    )?;
                    records.push(record);
                }
            }
            let next = if more {
                records.last().map(|record| record.id.clone())
            } else {
                None
            };
            guard(&transaction)?;
            transaction.commit()?;
            return fitted_inspection(
                authority,
                manifest,
                boot_id,
                state_dir,
                &request.operation_id,
                &records,
                Some(next),
            );
        }
        ManagementCommand::Inspect => {
            return authorized_read(
                store,
                authority.read_guard(manifest, request, now, None, None),
                || {
                    let records: Vec<_> = store
                        .list_placements()?
                        .into_iter()
                        .filter(|p| {
                            authority.permits(
                                ManagementCapability::Status,
                                p.config.get("project_id").and_then(Value::as_str),
                                Some(&p.id),
                            )
                        })
                        .collect();
                    refuse_unless(
                        authority.permits(ManagementCapability::Status, None, None)
                            || !records.is_empty(),
                        RejectionCode::Unauthorized,
                        "Status access denied",
                    )?;
                    fitted_inspection(
                        authority,
                        manifest,
                        boot_id,
                        state_dir,
                        &request.operation_id,
                        &records,
                        None,
                    )
                },
            );
        }
        ManagementCommand::Operation { operation_id } => {
            return authorized_read(
                store,
                authority.read_guard(manifest, request, now, None, None),
                || {
                    let result: Option<String> = store.connection.query_row("SELECT result_json FROM management_operations WHERE operation_id=?1 AND principal=?2",params![operation_id,authority.principal],|r|r.get(0)).optional()?;
                    let result = result.ok_or_else(|| {
                        refusal(
                            RejectionCode::Invalid,
                            format!("Unknown operation {operation_id}; it was never accepted or its record expired"),
                        )
                    })?;
                    Ok(serde_json::from_str(&result)?)
                },
            );
        }
        ManagementCommand::PlacementConfiguration { placement_id } => {
            return authorized_read(
                store,
                authority.read_guard(
                    manifest,
                    request,
                    now,
                    Some(ManagementCapability::Deploy),
                    Some(placement_id),
                ),
                || {
                    let (record, project_id) = placement_scope(store, placement_id)?;
                    authority.require(
                        ManagementCapability::Deploy,
                        Some(&project_id),
                        Some(placement_id),
                    )?;
                    let config: PlacementConfig = serde_json::from_value(record.config)?;
                    let rollout_sources: &[&str] = if cfg!(feature = "runtime") {
                        &["offline", "online"]
                    } else {
                        &[]
                    };
                    let response = ManagementResponse {
                        operation_id: request.operation_id.clone(),
                        state: "completed".into(),
                        result: json!({"placement_id":record.id,"project_id":project_id,"deployment_id":config.deployment_id,"config_revision":record.config_revision,"desired_state":record.desired_state,"rollout_sources":rollout_sources,"rollout":store.latest_rollout(placement_id)?.map(|r|r.status()),"config":config}),
                    };
                    refuse_unless(
                        serde_json::to_vec(&response)?.len() <= noise::MAX_PLAINTEXT,
                        RejectionCode::Limit,
                        "Placement configuration exceeds the remote read limit",
                    )?;
                    Ok(response)
                },
            );
        }
        ManagementCommand::Rollout { rollout_id } => {
            return authorized_read(
                store,
                authority.read_guard(manifest, request, now, None, None),
                || {
                    let rollout = known_rollout(store, rollout_id)?;
                    authority.require(
                        ManagementCapability::Deploy,
                        Some(&rollout.project_id),
                        Some(&rollout.placement_id),
                    )?;
                    Ok(ManagementResponse {
                        operation_id: request.operation_id.clone(),
                        state: "completed".into(),
                        result: rollout.status(),
                    })
                },
            );
        }
        ManagementCommand::ArchiveRead { scope, kind, .. }
        | ManagementCommand::ArchiveRosterRead { scope, kind, .. } => {
            return authorized_read(
                store,
                authority.read_guard(
                    manifest,
                    request,
                    now,
                    Some(match kind {
                        ArchiveKind::Logs => ManagementCapability::Logs,
                        ArchiveKind::Metrics => ManagementCapability::Metrics,
                    }),
                    if scope == "device" { None } else { Some(scope) },
                ),
                || {
                    let project = if scope == "device" {
                        None
                    } else {
                        Some(placement_scope(store, scope)?.1)
                    };
                    authority.require(
                        match kind {
                            ArchiveKind::Logs => ManagementCapability::Logs,
                            ArchiveKind::Metrics => ManagementCapability::Metrics,
                        },
                        project.as_deref(),
                        if scope == "device" { None } else { Some(scope) },
                    )?;
                    let result = match &request.command {
                        ManagementCommand::ArchiveRead {
                            sequence,
                            offset,
                            limit,
                            ..
                        } => crate::archives::read_from_store(
                            store, scope, kind, *sequence, *offset, *limit,
                        )?,
                        ManagementCommand::ArchiveRosterRead { offset, limit, .. } => {
                            crate::archives::read_roster_from_store(
                                store, scope, kind, *offset, *limit,
                            )?
                        }
                        _ => unreachable!(),
                    };
                    Ok(ManagementResponse {
                        operation_id: request.operation_id.clone(),
                        state: "completed".into(),
                        result,
                    })
                },
            );
        }
        ManagementCommand::ProjectMetrics { project_id } => {
            flow_like_device_protocol::validate_management_id(project_id)?;
            let telemetry = crate::telemetry::TelemetryStore::open(state_dir)?;
            return authorized_read(
                store,
                authority.read_guard(manifest, request, now, None, None),
                || {
                    let known:bool=store.connection.query_row("SELECT EXISTS(SELECT 1 FROM placement_identities WHERE project_id=?1 COLLATE BINARY)",[project_id],|r|r.get(0))?;
                    refuse_unless(
                        known,
                        RejectionCode::Invalid,
                        "Project has no device placement history",
                    )?;
                    authority.require(ManagementCapability::Metrics, Some(project_id), None)?;
                    Ok(ManagementResponse {
                        operation_id: request.operation_id.clone(),
                        state: "completed".into(),
                        result: telemetry.project_metrics(project_id)?,
                    })
                },
            );
        }
        ManagementCommand::Messages {
            placement_id,
            project_id,
            after,
            limit,
        } => {
            refuse_unless(
                placement_id.is_none() || project_id.is_none(),
                RejectionCode::Invalid,
                "Choose a project or placement message scope",
            )?;
            for id in placement_id.iter().chain(project_id.iter()) {
                flow_like_device_protocol::validate_management_id(id)?;
            }
            let telemetry = crate::telemetry::TelemetryStore::open(state_dir)?;
            return authorized_read(
                store,
                authority.read_guard(manifest, request, now, None, None),
                || {
                    let project = if let Some(placement) = placement_id {
                        Some(store.connection.query_row("SELECT project_id FROM placement_identities WHERE id=?1 COLLATE BINARY",[placement],|r|r.get::<_,String>(0))?)
                    } else {
                        project_id.clone()
                    };
                    if let Some(project) = &project {
                        let known:bool=store.connection.query_row("SELECT EXISTS(SELECT 1 FROM placement_identities WHERE project_id=?1 COLLATE BINARY)",[project],|r|r.get(0))?;
                        refuse_unless(
                            known,
                            RejectionCode::Invalid,
                            "Project has no device placement history",
                        )?;
                    }
                    authority.require(
                        ManagementCapability::Logs,
                        project.as_deref(),
                        placement_id.as_deref(),
                    )?;
                    Ok(ManagementResponse {
                        operation_id: request.operation_id.clone(),
                        state: "completed".into(),
                        result: telemetry.messages(
                            placement_id.as_deref(),
                            project_id.as_deref(),
                            *after,
                            *limit,
                        )?,
                    })
                },
            );
        }
        ManagementCommand::Metrics { placement_id }
        | ManagementCommand::Logs { placement_id, .. } => {
            let telemetry = crate::telemetry::TelemetryStore::open(state_dir)?;
            return authorized_read(
                store,
                authority.read_guard(
                    manifest,
                    request,
                    now,
                    Some(
                        if matches!(request.command, ManagementCommand::Metrics { .. }) {
                            ManagementCapability::Metrics
                        } else {
                            ManagementCapability::Logs
                        },
                    ),
                    placement_id.as_deref(),
                ),
                || {
                    let project = placement_id
                        .as_ref()
                        .map(|id| placement_scope(store, id).map(|(_, project)| project))
                        .transpose()?;
                    let capability = if matches!(request.command, ManagementCommand::Metrics { .. })
                    {
                        ManagementCapability::Metrics
                    } else {
                        ManagementCapability::Logs
                    };
                    authority.require(capability, project.as_deref(), placement_id.as_deref())?;
                    let result = match &request.command {
                        ManagementCommand::Metrics { .. } => {
                            telemetry.latest_metrics(placement_id.as_deref())?
                        }
                        ManagementCommand::Logs { after, limit, .. } => {
                            telemetry.read(placement_id.as_deref(), "log", *after, *limit)?
                        }
                        _ => unreachable!(),
                    };
                    Ok(ManagementResponse {
                        operation_id: request.operation_id.clone(),
                        state: "completed".into(),
                        result,
                    })
                },
            );
        }
        _ => (),
    }
    // Intent changes and their durable result share one write transaction. A caller can
    // reconnect and replay an operation ID, but cannot substitute another request.
    store.connection.execute_batch("BEGIN IMMEDIATE")?;
    let result = execute_transaction(store, authority, request, manifest, boot_id, state_dir, now);
    match result {
        Ok(value) => {
            store.connection.execute_batch("COMMIT")?;
            if matches!(
                request.command,
                ManagementCommand::PutCertificate { .. }
                    | ManagementCommand::DeleteCertificate { .. }
                    | ManagementCommand::CreateCertificateRequest { .. }
                    | ManagementCommand::InstallCertificateRequest { .. }
                    | ManagementCommand::DeleteCertificateRequest { .. }
                    | ManagementCommand::CreateCertificateIssuerRequest { .. }
                    | ManagementCommand::InstallCertificateIssuer { .. }
                    | ManagementCommand::DeleteCertificateIssuer { .. }
                    | ManagementCommand::ConfigureAcmeCertificate { .. }
                    | ManagementCommand::DeleteAcmeCertificate { .. }
            ) {
                let _ = crate::certificates::collect_unused(state_dir);
            }
            Ok(value)
        }
        Err(error) => {
            let _ = store.connection.execute_batch("ROLLBACK");
            if matches!(
                request.command,
                ManagementCommand::PutCertificate { .. }
                    | ManagementCommand::DeleteCertificate { .. }
                    | ManagementCommand::CreateCertificateRequest { .. }
                    | ManagementCommand::InstallCertificateRequest { .. }
                    | ManagementCommand::DeleteCertificateRequest { .. }
                    | ManagementCommand::CreateCertificateIssuerRequest { .. }
                    | ManagementCommand::InstallCertificateIssuer { .. }
                    | ManagementCommand::DeleteCertificateIssuer { .. }
                    | ManagementCommand::ConfigureAcmeCertificate { .. }
                    | ManagementCommand::DeleteAcmeCertificate { .. }
            ) {
                let _ = crate::certificates::collect_unused(state_dir);
            }
            Err(error)
        }
    }
}

fn validate_remote_project_path(state_dir: &Path, config: &PlacementConfig) -> Result<()> {
    // Remote callers may use only a device-owned imported project store.
    let project_root =
        crate::project_artifacts::managed_project_root(state_dir, &config.project_id)?;
    let selected = config
        .project_path
        .canonicalize()
        .context("Import the project on this device before deployment")?;
    let allowed = match config.source {
        crate::config::ProjectSource::Online => {
            selected
                == crate::project_artifacts::prepare_online_cache(state_dir, &config.project_id)?
                || selected
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|digest| {
                        crate::project_artifacts::managed_revision_for_source(
                            state_dir,
                            &config.project_id,
                            digest,
                            flow_like_device_protocol::ProjectArtifactSource::Online,
                        )
                        .is_ok_and(|path| selected == path)
                    })
        }
        crate::config::ProjectSource::Offline => {
            if selected == project_root.canonicalize()? {
                true
            } else {
                let digest = selected
                    .file_name()
                    .and_then(|value| value.to_str())
                    .context("Invalid imported revision")?;
                selected
                    == crate::project_artifacts::managed_revision(
                        state_dir,
                        &config.project_id,
                        digest,
                    )?
            }
        }
    };
    ensure!(
        allowed,
        "Remote placements must use their imported project store"
    );
    if config.source == crate::config::ProjectSource::Online {
        #[cfg(feature = "runtime")]
        crate::online::validate_approved_metadata(config)?;
        #[cfg(not(feature = "runtime"))]
        return Err(refusal(
            RejectionCode::Unsupported,
            "This device binary does not include the online project runtime",
        ));
    }
    Ok(())
}

fn require_host_operation_slot(
    store: &StateStore,
    expected_boot_id: &str,
    boot_id: &str,
) -> Result<()> {
    refuse_unless(
        !store.has_active_rollouts()?,
        RejectionCode::RevisionConflict,
        "A workflow update is active",
    )?;
    refuse_unless(
        expected_boot_id == boot_id,
        RejectionCode::RevisionConflict,
        "Device has already rebooted",
    )?;
    let pending: u64 = store.connection.query_row(
        "SELECT COUNT(*) FROM host_operations WHERE state IN ('pending','staging','draining','requesting','requested','unknown')",
        [],
        |r| r.get(0),
    )?;
    refuse_unless(
        pending == 0,
        RejectionCode::RevisionConflict,
        "Another host operation is pending",
    )
}

fn require_revision(current: u64, expected: u64, placement_id: &str) -> Result<()> {
    refuse_unless(
        current == expected,
        RejectionCode::RevisionConflict,
        format!(
            "Placement revision changed: {placement_id} is at {current}, request expected {expected}"
        ),
    )
}

fn requested_config(value: &Value) -> Result<PlacementConfig> {
    let config: PlacementConfig =
        serde_json::from_value(value.clone()).reject_as(RejectionCode::Invalid)?;
    config.validate().reject_as(RejectionCode::Invalid)?;
    Ok(config)
}

/// Turning buffering off stops replay, so queued writes the cloud has not applied would be
/// stranded on disk. They must be replayed or skipped before the buffer can be removed.
fn require_buffered_writes_drained(
    state_dir: &Path,
    previous: Option<&Value>,
    next: &PlacementConfig,
) -> Result<()> {
    let Some(previous) = previous else {
        return Ok(());
    };
    let previous: PlacementConfig = serde_json::from_value(previous.clone())?;
    if previous.offline_writes.is_none() || next.offline_writes.is_some() {
        return Ok(());
    }
    let mut pending = 0u64;
    for queue in crate::outbox::for_placement(state_dir, &previous)? {
        pending = pending.saturating_add(queue.status()?.pending_count);
    }
    refuse_unless(
        pending == 0,
        RejectionCode::Invalid,
        format!(
            "Offline write buffering for placement {} still holds {pending} queued writes; replay or skip them before removing buffering",
            previous.id
        ),
    )
}

fn execute_transaction(
    store: &mut StateStore,
    authority: &Authority,
    request: &ManagementRequest,
    manifest: &OnboardingManifest,
    boot_id: &str,
    state_dir: &Path,
    now: i64,
) -> Result<ManagementResponse> {
    authority.require_current(store, manifest, now)?;
    if matches!(
        request.command,
        ManagementCommand::PutCertificate { .. }
            | ManagementCommand::DeleteCertificate { .. }
            | ManagementCommand::CreateCertificateRequest { .. }
            | ManagementCommand::InstallCertificateRequest { .. }
            | ManagementCommand::DeleteCertificateRequest { .. }
            | ManagementCommand::CreateCertificateIssuerRequest { .. }
            | ManagementCommand::InstallCertificateIssuer { .. }
            | ManagementCommand::DeleteCertificateIssuer { .. }
            | ManagementCommand::ConfigureAcmeCertificate { .. }
            | ManagementCommand::DeleteAcmeCertificate { .. }
    ) {
        authority.require(ManagementCapability::ManageCertificates, None, None)?;
    }
    if matches!(
        request.command,
        ManagementCommand::CreateCertificateIssuerRequest { .. }
            | ManagementCommand::InstallCertificateIssuer { .. }
            | ManagementCommand::DeleteCertificateIssuer { .. }
            | ManagementCommand::ConfigureAcmeCertificate { .. }
            | ManagementCommand::DeleteAcmeCertificate { .. }
    ) {
        authority
            .require_owner("Only the device owner can manage certificate renewal authorities")?;
    }
    if let ManagementCommand::DeleteCertificateRequest { request_id } = &request.command {
        let issuer: bool = store.connection.query_row("SELECT EXISTS(SELECT 1 FROM certificate_requests WHERE request_id=?1 AND json_extract(metadata_json,'$.purpose')='issuer')", [request_id], |row| row.get(0))?;
        if issuer {
            authority
                .require_owner("Only the device owner can cancel an issuing authority request")?;
        }
    }
    let digest = if matches!(
        request.command,
        ManagementCommand::SetSecret { .. }
            | ManagementCommand::RolloutSecret { .. }
            | ManagementCommand::PutCertificate { .. }
            | ManagementCommand::InstallCertificateRequest { .. }
            | ManagementCommand::InstallCertificateIssuer { .. }
    ) {
        crate::secrets::request_digest(state_dir, request)?
    } else {
        compact_digest(&serde_json::to_string(request)?)
    };
    if let Some(previous) =
        previous_operation(&store.connection, &request.operation_id, &digest, authority)?
    {
        return Ok(previous);
    }
    reserve_journal_entry(&store.connection, authority, now)?;
    let mut project = None;
    let mut placement = None;
    let result = match &request.command {
        ManagementCommand::ConfigureAcmeCertificate {
            certificate_id,
            label,
            expected_revision,
            expected_certificate_revision,
            dns_names,
            environment,
            http_bind,
            terms_of_service_agreed,
        } => {
            authority.require_owner("Only the device owner can manage ACME renewal")?;
            let acme = crate::acme::configure(
                store,
                certificate_id,
                label,
                *expected_revision,
                *expected_certificate_revision,
                dns_names,
                *environment,
                http_bind,
                *terms_of_service_agreed,
                now,
            )?;
            json!({"acme":acme})
        }
        ManagementCommand::DeleteAcmeCertificate {
            certificate_id,
            expected_revision,
        } => {
            authority.require_owner("Only the device owner can manage ACME renewal")?;
            crate::acme::delete(store, certificate_id, *expected_revision)?;
            json!({"certificate_id":certificate_id,"deleted":true})
        }
        ManagementCommand::CreateCertificateIssuerRequest {
            request_id,
            certificate_id,
            expected_revision,
            dns_names,
            ip_addresses,
            leaf_lifetime_days,
        } => {
            let request = crate::certificate_issuers::create_request(
                store,
                state_dir,
                request_id,
                certificate_id,
                *expected_revision,
                dns_names,
                ip_addresses,
                *leaf_lifetime_days,
                now,
            )?;
            json!({"request":request})
        }
        ManagementCommand::InstallCertificateIssuer {
            request_id,
            certificate_chain_pem,
        } => {
            let issuer = crate::certificate_issuers::install(
                store,
                state_dir,
                request_id,
                &certificate_chain_pem.0,
                now,
            )?;
            json!({"issuer":issuer})
        }
        ManagementCommand::DeleteCertificateIssuer {
            certificate_id,
            expected_revision,
        } => {
            crate::certificate_issuers::delete(store, certificate_id, *expected_revision)?;
            json!({"certificate_id":certificate_id,"deleted":true})
        }
        ManagementCommand::CreateCertificateRequest {
            request_id,
            certificate_id,
            label,
            expected_revision,
            dns_names,
            ip_addresses,
        } => {
            let request = crate::certificate_requests::create(
                store,
                state_dir,
                request_id,
                certificate_id,
                label,
                *expected_revision,
                dns_names,
                ip_addresses,
                now,
            )?;
            json!({"request":request})
        }
        ManagementCommand::InstallCertificateRequest {
            request_id,
            certificate_chain_pem,
        } => {
            let certificate = crate::certificate_requests::install(
                store,
                state_dir,
                request_id,
                &certificate_chain_pem.0,
                now,
            )?;
            let certificate = crate::certificates::bounded_metadata(certificate)?;
            json!({"certificate":certificate,"inventory_revision":crate::certificates::inventory_revision(store)?})
        }
        ManagementCommand::DeleteCertificateRequest { request_id } => {
            crate::certificate_requests::delete(store, request_id)?;
            json!({"request_id":request_id,"deleted":true})
        }
        ManagementCommand::PutCertificate {
            certificate_id,
            label,
            expected_revision,
            certificate_chain_pem,
            private_key_pem,
        } => {
            let certificate = crate::certificates::put(
                store,
                state_dir,
                certificate_id,
                label,
                *expected_revision,
                &certificate_chain_pem.0,
                &private_key_pem.0,
                now,
            )?;
            let certificate = crate::certificates::bounded_metadata(certificate)?;
            json!({"certificate":certificate,"inventory_revision":crate::certificates::inventory_revision(store)?})
        }
        ManagementCommand::DeleteCertificate {
            certificate_id,
            expected_revision,
        } => {
            let revision = crate::certificates::delete(store, certificate_id, *expected_revision)?;
            json!({"certificate_id":certificate_id,"deleted":true,"inventory_revision":revision})
        }
        ManagementCommand::OfflineQueueRetry {
            placement_id,
            scope,
            queued_operation_id,
        }
        | ManagementCommand::OfflineQueueSkip {
            placement_id,
            scope,
            queued_operation_id,
            ..
        } => {
            let (record, project_id) = placement_scope(store, placement_id)?;
            authority.require(
                ManagementCapability::Deploy,
                Some(&project_id),
                Some(placement_id),
            )?;
            let config: PlacementConfig = serde_json::from_value(record.config)?;
            let queues = crate::outbox::for_placement(state_dir, &config)?;
            let queue = queues
                .iter()
                .find(|queue| queue.scope() == scope)
                .ok_or_else(|| {
                    refusal(
                        RejectionCode::Invalid,
                        format!("Unknown offline authorization scope {scope}"),
                    )
                })?;
            match &request.command {
                ManagementCommand::OfflineQueueSkip {
                    reason,
                    acknowledge_uncertain,
                    ..
                } => queue.request_skip(
                    queued_operation_id,
                    &format!("{}: {reason}", authority.principal),
                    *acknowledge_uncertain,
                )?,
                _ => queue.retry(queued_operation_id)?,
            }
            project = Some(project_id);
            placement = Some(placement_id.clone());
            json!({"placement_id":placement_id,"queued_operation_id":queued_operation_id,"queue":queue.status()?})
        }
        ManagementCommand::StageRollout {
            config,
            expected_revision,
            stabilization_seconds,
            deadline_seconds,
        } => {
            let config = requested_config(config)?;
            crate::isolation::enforce_host_policy(&config, state_dir)
                .reject_as(RejectionCode::HostPolicy)?;
            authority.require(
                ManagementCapability::Deploy,
                Some(&config.project_id),
                Some(&config.id),
            )?;
            // Rollout activation starts services. A deploy-only invitation must not
            // acquire the separately granted ability to start a placement.
            authority.require(
                ManagementCapability::Start,
                Some(&config.project_id),
                Some(&config.id),
            )?;
            store
                .require_no_active_rollout(&config.id)
                .reject_as(RejectionCode::RevisionConflict)?;
            let existing = store.get_placement(&config.id)?;
            refuse_unless(
                existing.as_ref().is_some_and(|current| {
                    current.config_revision == *expected_revision
                        && current.desired_state == crate::state::DesiredState::Running
                }),
                RejectionCode::RevisionConflict,
                format!(
                    "Rollout requires running placement {} at revision {expected_revision}",
                    config.id
                ),
            )?;
            authority.require_certificate_assignment(store, &config)?;
            require_buffered_writes_drained(
                state_dir,
                existing.as_ref().map(|current| &current.config),
                &config,
            )?;
            validate_remote_project_path(state_dir, &config).reject_as(RejectionCode::Invalid)?;
            if let Some(id) = &config.tls_certificate_id {
                crate::certificates::validate_binding(store, state_dir, id, now)
                    .reject_as(RejectionCode::Invalid)?;
            }
            let rollout = store.stage_rollout(
                &request.operation_id,
                &config,
                *expected_revision,
                *stabilization_seconds,
                *deadline_seconds,
                now,
            )?;
            crate::secrets::preserve_for_revision(&rollout.previous_config, &config)?;
            project = Some(config.project_id);
            placement = Some(config.id);
            rollout.status()
        }
        ManagementCommand::RolloutSecret {
            rollout_id,
            name,
            value,
        } => {
            let rollout = known_rollout(store, rollout_id)?;
            authority.require(
                ManagementCapability::Deploy,
                Some(&rollout.project_id),
                Some(&rollout.placement_id),
            )?;
            let config = store.rollout_secret_config(rollout_id, name)?;
            crate::secrets::install(&config, name, value.0.as_bytes())?;
            project = Some(rollout.project_id);
            placement = Some(rollout.placement_id.clone());
            json!({"rollout_id":rollout_id,"placement_id":rollout.placement_id,"name":name,"secret":"completed"})
        }
        ManagementCommand::ActivateRollout { rollout_id } => {
            let rollout = known_rollout(store, rollout_id)?;
            authority.require(
                ManagementCapability::Deploy,
                Some(&rollout.project_id),
                Some(&rollout.placement_id),
            )?;
            authority.require(
                ManagementCapability::Start,
                Some(&rollout.project_id),
                Some(&rollout.placement_id),
            )?;
            project = Some(rollout.project_id);
            placement = Some(rollout.placement_id);
            store.begin_rollout_validation(rollout_id, now)?.status()
        }
        ManagementCommand::CancelRollout { rollout_id } => {
            let rollout = known_rollout(store, rollout_id)?;
            authority.require(
                ManagementCapability::Deploy,
                Some(&rollout.project_id),
                Some(&rollout.placement_id),
            )?;
            refuse_unless(
                matches!(rollout.state.as_str(), "staged" | "validating"),
                RejectionCode::RevisionConflict,
                format!(
                    "Only a staged or validating update can be discarded; {rollout_id} is {}",
                    rollout.state
                ),
            )?;
            store.connection.execute(
                "UPDATE placement_rollouts SET state='cancelled',failure_code='discarded',updated_at=?2,stable_since=NULL,cohort=NULL WHERE rollout_id=?1",
                params![rollout_id, now],
            )?;
            project = Some(rollout.project_id);
            placement = Some(rollout.placement_id);
            store
                .rollout(rollout_id)?
                .context("Missing workflow update")?
                .status()
        }
        ManagementCommand::Start {
            placement_id,
            expected_revision,
        }
        | ManagementCommand::Stop {
            placement_id,
            expected_revision,
        }
        | ManagementCommand::Restart {
            placement_id,
            expected_revision,
        }
        | ManagementCommand::Remove {
            placement_id,
            expected_revision,
        } => {
            let (record, project_id) = placement_scope(store, placement_id)?;
            require_revision(record.config_revision, *expected_revision, placement_id)?;
            let capability = match request.command {
                ManagementCommand::Start { .. } => ManagementCapability::Start,
                ManagementCommand::Stop { .. } => ManagementCapability::Stop,
                ManagementCommand::Restart { .. } => ManagementCapability::Restart,
                _ => ManagementCapability::Remove,
            };
            authority.require(capability, Some(&project_id), Some(placement_id))?;
            // Restarting a stopped placement starts it, which Restart alone does not grant.
            if matches!(request.command, ManagementCommand::Restart { .. })
                && record.desired_state != crate::state::DesiredState::Running
            {
                authority.require(
                    ManagementCapability::Start,
                    Some(&project_id),
                    Some(placement_id),
                )?;
            }
            if !matches!(request.command, ManagementCommand::Stop { .. }) {
                store
                    .require_no_active_rollout(placement_id)
                    .reject_as(RejectionCode::RevisionConflict)?;
            }
            if matches!(request.command, ManagementCommand::Remove { .. }) {
                store.remove_placement(placement_id)?;
            } else {
                store.set_desired_state(
                    placement_id,
                    if matches!(request.command, ManagementCommand::Stop { .. }) {
                        crate::state::DesiredState::Stopped
                    } else {
                        crate::state::DesiredState::Running
                    },
                )?;
            }
            project = Some(project_id);
            placement = Some(placement_id.clone());
            json!({"placement_id":placement_id,"intent_recorded":true})
        }
        ManagementCommand::Scale {
            placement_id,
            expected_revision,
            replicas,
        } => {
            let (record, project_id) = placement_scope(store, placement_id)?;
            authority.require(
                ManagementCapability::Scale,
                Some(&project_id),
                Some(placement_id),
            )?;
            require_revision(record.config_revision, *expected_revision, placement_id)?;
            store
                .require_no_active_rollout(placement_id)
                .reject_as(RejectionCode::RevisionConflict)?;
            let max_replicas = record
                .config
                .get("max_replicas")
                .and_then(Value::as_u64)
                .unwrap_or(1);
            refuse_unless(
                (1..=max_replicas.min(32)).contains(&u64::from(*replicas)),
                RejectionCode::Invalid,
                format!("Replica count {replicas} must be between 1 and {max_replicas}"),
            )?;
            store.set_replica_count(placement_id, *expected_revision, *replicas)?;
            project = Some(project_id);
            placement = Some(placement_id.clone());
            json!({"placement_id":placement_id,"desired_replicas":replicas,"intent_recorded":true})
        }
        ManagementCommand::Apply {
            config,
            expected_revision,
            start,
        } => {
            let config = requested_config(config)?;
            store
                .check_placement_identity(&config.id, &serde_json::to_value(&config)?)
                .reject_as(RejectionCode::RevisionConflict)?;
            authority.require(
                ManagementCapability::Deploy,
                Some(&config.project_id),
                Some(&config.id),
            )?;
            crate::isolation::enforce_host_policy(&config, state_dir)
                .reject_as(RejectionCode::HostPolicy)?;
            store
                .require_no_active_rollout(&config.id)
                .reject_as(RejectionCode::RevisionConflict)?;
            let existing = store.get_placement(&config.id)?;
            require_revision(
                existing.as_ref().map_or(0, |p| p.config_revision),
                *expected_revision,
                &config.id,
            )?;
            if let Some(old) = &existing {
                refuse_unless(
                    old.config.get("project_id").and_then(Value::as_str)
                        == Some(&config.project_id)
                        && old.config.get("deployment_id").and_then(Value::as_str)
                            == Some(&config.deployment_id),
                    RejectionCode::RevisionConflict,
                    "Placement identity is immutable",
                )?;
            }
            // Deploy changes configuration only. Starting a placement that is not running,
            // or stopping one that is, needs the separately granted capability.
            let running = existing
                .as_ref()
                .is_some_and(|old| old.desired_state == crate::state::DesiredState::Running);
            if *start && !running {
                authority.require(
                    ManagementCapability::Start,
                    Some(&config.project_id),
                    Some(&config.id),
                )?;
            }
            if !*start && running {
                authority.require(
                    ManagementCapability::Stop,
                    Some(&config.project_id),
                    Some(&config.id),
                )?;
            }
            authority.require_certificate_assignment(store, &config)?;
            require_buffered_writes_drained(
                state_dir,
                existing.as_ref().map(|old| &old.config),
                &config,
            )?;
            validate_remote_project_path(state_dir, &config).reject_as(RejectionCode::Invalid)?;
            if let Some(id) = &config.tls_certificate_id {
                crate::certificates::validate_binding(store, state_dir, id, now)
                    .reject_as(RejectionCode::Invalid)?;
            }
            if let Some(old) = &existing {
                crate::secrets::preserve_for_revision(
                    &serde_json::from_value(old.config.clone())?,
                    &config,
                )?;
            }
            let config_value = serde_json::to_value(&config)?;
            let changed = existing.as_ref().is_none_or(|p| p.config != config_value);
            let desired_state = if *start {
                crate::state::DesiredState::Running
            } else {
                crate::state::DesiredState::Stopped
            };
            if changed {
                let revision = expected_revision
                    .checked_add(1)
                    .context("Placement revision overflow")?;
                let encoded = serde_json::to_string(&config_value)?;
                store.connection.execute("INSERT INTO placements(id,config_json,desired_state,intent_revision,observed_state,config_revision) VALUES(?1,?2,?3,1,'unknown',?4) ON CONFLICT(id) DO UPDATE SET config_json=excluded.config_json,config_revision=excluded.config_revision",params![config.id,encoded,desired_state.as_str(),revision])?;
            }
            if existing
                .as_ref()
                .is_some_and(|old| changed || old.desired_state != desired_state)
            {
                store.set_desired_state(&config.id, desired_state)?;
            }
            store.clamp_replica_count(&config.id, config.max_replicas)?;
            project = Some(config.project_id);
            placement = Some(config.id);
            json!({"placement_id":placement,"config_revision":if changed{expected_revision+1}else{*expected_revision}})
        }
        ManagementCommand::SetSecret {
            placement_id,
            expected_revision,
            name,
            value,
        } => {
            let (record, project_id) = placement_scope(store, placement_id)?;
            authority.require(
                ManagementCapability::Deploy,
                Some(&project_id),
                Some(placement_id),
            )?;
            require_revision(record.config_revision, *expected_revision, placement_id)?;
            store
                .require_no_active_rollout(placement_id)
                .reject_as(RejectionCode::RevisionConflict)?;
            crate::secrets::enqueue(
                store,
                state_dir,
                &request.operation_id,
                placement_id,
                *expected_revision,
                name,
                &value.0,
            )?;
            project = Some(project_id);
            placement = Some(placement_id.clone());
            json!({"placement_id":placement_id,"name":name,"secret":"pending"})
        }
        ManagementCommand::ArchivePolicy { policy_jws } => {
            authority.require_owner("Only the owner may change retained-history recipients")?;
            crate::archives::apply_policy(store, manifest, policy_jws, now)?
        }
        ManagementCommand::ApplyPolicy { policy_jws } => {
            authority.require_owner("Only the owner can apply sharing policy")?;
            store
                .accept_management_policy(
                    policy_jws,
                    &manifest.owner_invitation_key,
                    &manifest.device_id,
                    now,
                )
                .reject_as(RejectionCode::Invalid)?;
            json!({"policy_digest":compact_digest(policy_jws)})
        }
        ManagementCommand::Reboot { expected_boot_id } => {
            authority.require(ManagementCapability::Reboot, None, None)?;
            refuse_unless(
                REMOTE_HOST_OPERATIONS,
                RejectionCode::Unsupported,
                "Remote reboot requires Linux systemd; restart this device locally",
            )?;
            require_host_operation_slot(store, expected_boot_id, boot_id)?;
            store.connection.execute("INSERT INTO host_operations(operation_id,kind,boot_id,state,created_at) VALUES(?1,'reboot',?2,'pending',?3)",params![request.operation_id,boot_id,now])?;
            json!({"boot_id":boot_id,"reboot":"pending"})
        }
        ManagementCommand::UpdateAgent {
            expected_boot_id,
            release_jws,
        } => {
            authority.require(ManagementCapability::UpdateAgent, None, None)?;
            refuse_unless(
                REMOTE_HOST_OPERATIONS,
                RejectionCode::Unsupported,
                "Automatic updates require Linux systemd",
            )?;
            refuse_unless(
                uuid::Uuid::parse_str(&request.operation_id)
                    .is_ok_and(|id| id.to_string() == request.operation_id),
                RejectionCode::Invalid,
                "Update operation ID must be a canonical UUID",
            )?;
            let trust = crate::release::ReleaseTrust::load(&state_dir.join("release-trust.json"))?;
            let release = crate::release::VerifiedRelease::verify(release_jws.clone(), &trust)
                .reject_as(RejectionCode::Invalid)?;
            require_host_operation_slot(store, expected_boot_id, boot_id)?;
            store.connection.execute("INSERT INTO host_operations(operation_id,kind,boot_id,state,created_at,payload_json) VALUES(?1,'update',?2,'pending',?3,?4)",params![request.operation_id,boot_id,now,serde_json::to_string(&json!({"release_jws":release_jws}))?])?;
            json!({"boot_id":boot_id,"update":"pending","release_version":release.manifest().release_version})
        }
        _ => {
            return Err(refusal(
                RejectionCode::Unsupported,
                "Unsupported management command",
            ));
        }
    };
    let response = ManagementResponse {
        operation_id: request.operation_id.clone(),
        state: if matches!(
            request.command,
            ManagementCommand::PutCertificate { .. }
                | ManagementCommand::DeleteCertificate { .. }
                | ManagementCommand::CreateCertificateRequest { .. }
                | ManagementCommand::InstallCertificateRequest { .. }
                | ManagementCommand::DeleteCertificateRequest { .. }
                | ManagementCommand::CreateCertificateIssuerRequest { .. }
                | ManagementCommand::InstallCertificateIssuer { .. }
                | ManagementCommand::DeleteCertificateIssuer { .. }
                | ManagementCommand::ConfigureAcmeCertificate { .. }
                | ManagementCommand::DeleteAcmeCertificate { .. }
        ) {
            "completed"
        } else {
            "accepted"
        }
        .into(),
        result,
    };
    store.connection.execute("INSERT INTO management_operations(operation_id,request_digest,principal,project_id,placement_id,accepted_at,result_json) VALUES(?1,?2,?3,?4,?5,?6,?7)",params![request.operation_id,digest,authority.principal,project,placement,now,serde_json::to_string(&response)?])?;
    Ok(response)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest(owner_key: &SigningKey) -> OnboardingManifest {
        OnboardingManifest {
            version: 1,
            enrollment_id: "enrollment".into(),
            device_id: "device".into(),
            owner_id: "owner-user".into(),
            name: "Test device".into(),
            api_base_url: "https://example.test/api/v1".into(),
            bootstrap_key: SigningKey::generate().public_key(),
            controller_key: SigningKey::generate().public_key(),
            owner_invitation_key: owner_key.public_key(),
            issued_at: 100,
            expires_at: 1000,
        }
    }
    fn request(id: &str, command: ManagementCommand) -> ManagementRequest {
        ManagementRequest {
            operation_id: id.into(),
            device_id: "device".into(),
            issued_at: 100,
            expires_at: 300,
            command,
        }
    }
    fn placement(root: &Path) -> Result<Value> {
        let root = crate::supervisor::prepare_state_dir(root)?;
        let path = crate::project_artifacts::managed_project_root(&root, "project")?;
        Ok(
            json!({"id":"api","project_id":"project","deployment_id":"deployment","revision":"release-1","source":"offline","project_path":path,"events":[{"event_id":"event","event_version":[1,0,0],"board_version":[1,0,0]}],"variables":{"public-listen-port":8080}}),
        )
    }
    fn owner(manifest: &OnboardingManifest) -> Authority {
        Authority {
            principal: "owner-user:owner".into(),
            key: manifest.controller_key.clone(),
            grant: None,
        }
    }
    fn project_grant(
        id: &str,
        capabilities: Vec<ManagementCapability>,
        expires_at: i64,
    ) -> Authority {
        let key = SigningKey::generate().public_key();
        Authority {
            principal: format!("{id}:{id}"),
            key: key.clone(),
            grant: Some(ManagementGrant {
                grant_id: id.into(),
                user_id: id.into(),
                controller_key: key,
                scope: ManagementScope::Project {
                    project_id: "project".into(),
                },
                capabilities,
                expires_at,
                group_id: None,
                group_version: None,
            }),
        }
    }
    fn accept_grants(
        store: &StateStore,
        signing: &SigningKey,
        grantees: &[&Authority],
        issued_at: i64,
        expires_at: i64,
    ) -> Result<()> {
        let policy = ManagementPolicy {
            version: 1,
            device_id: "device".into(),
            policy_version: 1,
            previous_policy_digest: None,
            grants: grantees
                .iter()
                .filter_map(|authority| authority.grant.clone())
                .collect(),
            issued_at,
            expires_at,
        };
        store.accept_management_policy(
            &sign_management_policy(&policy, signing)?,
            &signing.public_key(),
            "device",
            issued_at,
        )
    }

    #[test]
    fn apply_cannot_start_or_stop_a_placement_without_those_capabilities() -> Result<()> {
        use crate::state::DesiredState;
        let temp = tempfile::tempdir()?;
        let root = temp.path().canonicalize()?;
        let mut store = StateStore::open(&root.join("management.sqlite"))?;
        let signing = SigningKey::generate();
        let manifest = manifest(&signing);
        let owner = owner(&manifest);
        let deployer = project_grant("deployer", vec![ManagementCapability::Deploy], 1000);
        let restarter = project_grant("restarter", vec![ManagementCapability::Restart], 1000);
        accept_grants(&store, &signing, &[&deployer, &restarter], 100, 1000)?;
        let mut config = placement(&root)?;
        let mut run = |authority: &Authority, id: &str, command| {
            execute(
                &mut store,
                authority,
                &request(id, command),
                &manifest,
                "boot",
                &root,
                101,
            )
        };
        let apply = |config: &Value, expected_revision, start| ManagementCommand::Apply {
            config: config.clone(),
            expected_revision,
            start,
        };
        let placement_id = || "api".to_owned();
        let denied = run(&deployer, "start-new", apply(&config, 0, true)).unwrap_err();
        assert_eq!(rejection_code(&denied), RejectionCode::Unauthorized);
        run(&deployer, "create-stopped", apply(&config, 0, false))?;
        let denied = run(&deployer, "start-existing", apply(&config, 1, true)).unwrap_err();
        assert_eq!(rejection_code(&denied), RejectionCode::Unauthorized);
        run(
            &owner,
            "owner-start",
            ManagementCommand::Start {
                placement_id: placement_id(),
                expected_revision: 1,
            },
        )?;
        let denied = run(&deployer, "stop-running", apply(&config, 1, false)).unwrap_err();
        assert_eq!(rejection_code(&denied), RejectionCode::Unauthorized);
        config["variables"]["public-listen-port"] = json!(9090);
        run(&deployer, "update-running", apply(&config, 1, true))?;
        run(
            &owner,
            "owner-stop",
            ManagementCommand::Stop {
                placement_id: placement_id(),
                expected_revision: 2,
            },
        )?;
        let restart = |expected_revision| ManagementCommand::Restart {
            placement_id: placement_id(),
            expected_revision,
        };
        let denied = run(&restarter, "restart-stopped", restart(2)).unwrap_err();
        assert_eq!(rejection_code(&denied), RejectionCode::Unauthorized);
        let stale = run(&owner, "stale-restart", restart(1)).unwrap_err();
        assert_eq!(rejection_code(&stale), RejectionCode::RevisionConflict);
        run(
            &owner,
            "owner-restart-start",
            ManagementCommand::Start {
                placement_id: placement_id(),
                expected_revision: 2,
            },
        )?;
        run(&restarter, "restart-running", restart(2))?;
        let current = store.get_placement("api")?.unwrap();
        assert_eq!(current.config_revision, 2);
        assert_eq!(current.desired_state, DesiredState::Running);
        let denied: u64 = store.connection.query_row("SELECT COUNT(*) FROM management_operations WHERE operation_id IN ('start-new','start-existing','stop-running','restart-stopped','stale-restart')", [], |r| r.get(0))?;
        assert_eq!(denied, 0);
        Ok(())
    }

    #[test]
    fn rejections_carry_codes_and_hide_local_causes_from_grantees() {
        let manifest = manifest(&SigningKey::generate());
        let owner = owner(&manifest);
        let grantee = project_grant("grantee", vec![ManagementCapability::Status], 1000);
        let cause = refusal(
            RejectionCode::HostPolicy,
            "This device requires linux_sandbox under /var/lib/flow-like",
        );
        let shown = rejected(&owner, "op", rejection_code(&cause), format!("{cause:#}"));
        assert_eq!(shown.operation_id, "op");
        assert_eq!(shown.state, "rejected");
        assert_eq!(
            shown.result,
            json!({"error":"This device requires linux_sandbox under /var/lib/flow-like","code":"host_policy","retryable":false})
        );
        let hidden = rejected(&grantee, "op", rejection_code(&cause), format!("{cause:#}"));
        assert_eq!(
            hidden.result["error"],
            RejectionCode::HostPolicy.public_message()
        );
        assert!(!hidden.result.to_string().contains("/var/lib"));
        let wrapped = cause.context("Apply placement api");
        assert_eq!(rejection_code(&wrapped), RejectionCode::HostPolicy);
        assert!(format!("{wrapped:#}").contains("linux_sandbox"));
        let first: Result<()> =
            Err(refusal(RejectionCode::Unauthorized, "denied")).reject_as(RejectionCode::Invalid);
        assert_eq!(
            rejection_code(&first.unwrap_err()),
            RejectionCode::Unauthorized
        );
        let lock = anyhow::Error::from(std::io::Error::from(std::io::ErrorKind::WouldBlock))
            .context("Acquire artifact lock");
        assert_eq!(rejection_code(&lock), RejectionCode::Busy);
        let sqlite = anyhow::Error::from(rusqlite::Error::SqliteFailure(
            rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_BUSY),
            None,
        ));
        assert_eq!(rejection_code(&sqlite), RejectionCode::Busy);
        let identifier = validate_management_id("../escape").map_err(anyhow::Error::from);
        assert_eq!(
            rejection_code(&identifier.unwrap_err()),
            RejectionCode::Invalid
        );
        assert_eq!(
            rejection_code(&anyhow::anyhow!("disk failure")),
            RejectionCode::Failed
        );
        for code in [
            RejectionCode::Unauthorized,
            RejectionCode::RevisionConflict,
            RejectionCode::Invalid,
            RejectionCode::HostPolicy,
            RejectionCode::Unsupported,
            RejectionCode::Limit,
        ] {
            assert!(!code.retryable(), "{code:?}");
        }
        assert!(RejectionCode::Busy.retryable() && RejectionCode::Failed.retryable());
        let long = rejected(&owner, "op", RejectionCode::Failed, "é".repeat(2048));
        assert!(long.result["error"].as_str().unwrap().len() <= MAX_REJECTION_TEXT);
    }

    #[test]
    fn journal_prunes_expired_rows_and_bounds_each_grant() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let root = temp.path().canonicalize()?;
        let mut store = StateStore::open(&root.join("management.sqlite"))?;
        let signing = SigningKey::generate();
        let manifest = manifest(&signing);
        let owner = owner(&manifest);
        let now = 1_000_000;
        let horizon = now + 30 * 86_400;
        let deployer = project_grant("deployer", vec![ManagementCapability::Deploy], horizon);
        accept_grants(&store, &signing, &[&deployer], now - 1, horizon)?;
        let expired = now - JOURNAL_RETENTION_SECONDS - 1;
        for id in ["expired", "host-pending", "secret-pending"] {
            store.connection.execute("INSERT INTO management_operations(operation_id,request_digest,principal,accepted_at,result_json) VALUES(?1,'digest','owner-user:owner',?2,'{}')", params![id, expired])?;
        }
        store.connection.execute("INSERT INTO host_operations(operation_id,kind,boot_id,state,created_at) VALUES('host-pending','reboot','boot','unknown',?1)", [expired])?;
        store.connection.execute("INSERT INTO secret_operations(operation_id,placement_id,expected_revision,name,ciphertext,created_at,state) VALUES('secret-pending','api',1,'name',x'',?1,'pending')", [expired])?;
        store.connection.execute_batch("BEGIN")?;
        for index in 0..MAX_GRANT_JOURNAL_ENTRIES {
            store.connection.execute("INSERT INTO management_operations(operation_id,request_digest,principal,accepted_at,result_json) VALUES(?1,'digest','deployer:deployer',?2,'{}')", params![format!("flood-{index}"), now - 10])?;
        }
        store.connection.execute_batch("COMMIT")?;
        let config = placement(&root)?;
        let at = |id: &str, command, time: i64| ManagementRequest {
            operation_id: id.into(),
            device_id: "device".into(),
            issued_at: time,
            expires_at: time + 60,
            command,
        };
        let apply = |expected_revision| ManagementCommand::Apply {
            config: config.clone(),
            expected_revision,
            start: false,
        };
        execute(
            &mut store,
            &owner,
            &at("owner-create", apply(0), now),
            &manifest,
            "boot",
            &root,
            now,
        )?;
        let kept = |store: &StateStore, id: &str| -> Result<bool> {
            Ok(store.connection.query_row(
                "SELECT EXISTS(SELECT 1 FROM management_operations WHERE operation_id=?1)",
                [id],
                |r| r.get(0),
            )?)
        };
        assert!(!kept(&store, "expired")?);
        assert!(kept(&store, "host-pending")? && kept(&store, "secret-pending")?);
        let full = execute(
            &mut store,
            &deployer,
            &at("grantee-noop", apply(1), now),
            &manifest,
            "boot",
            &root,
            now,
        )
        .unwrap_err();
        assert_eq!(rejection_code(&full), RejectionCode::Limit, "{full:#}");
        execute(
            &mut store,
            &owner,
            &at(
                "owner-still-managed",
                ManagementCommand::Stop {
                    placement_id: "api".into(),
                    expected_revision: 1,
                },
                now,
            ),
            &manifest,
            "boot",
            &root,
            now,
        )?;
        let later = now + JOURNAL_RETENTION_SECONDS + 20;
        execute(
            &mut store,
            &deployer,
            &at("grantee-after-retention", apply(1), later),
            &manifest,
            "boot",
            &root,
            later,
        )?;
        assert!(!kept(&store, "flood-0")?);
        assert!(kept(&store, "host-pending")? && kept(&store, "secret-pending")?);
        Ok(())
    }

    #[test]
    fn buffering_cannot_be_removed_while_offline_writes_are_queued() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let root = temp.path().canonicalize()?;
        let mut store = StateStore::open(&root.join("management.sqlite"))?;
        let manifest = manifest(&SigningKey::generate());
        let owner = owner(&manifest);
        let mut config = placement(&root)?;
        config["source"] = json!("online");
        config["resource_grant"] = json!({"grant_id":"grant","authz_version":1});
        config["online_metadata_sha256"] = json!("a".repeat(64));
        config["offline_writes"] = json!({"tables":[{"purpose":"storage","database":"db","table":"measurements","primary_key":"id"}]});
        store.upsert_placement("api", &config, crate::state::DesiredState::Running)?;
        let mut data = root.clone();
        for part in ["placement-data", "api", "current", "store"] {
            data.push(part);
            crate::outbox::private_directory(&data)?;
        }
        let queue = crate::outbox::Outbox::open(&data, "api", &"a".repeat(64), Default::default())?;
        queue.enqueue("table", json!({"row":1}), None, 100)?;
        drop(queue);
        let mut unbuffered = config.clone();
        unbuffered["offline_writes"] = Value::Null;
        for (id, command) in [
            (
                "apply-unbuffered",
                ManagementCommand::Apply {
                    config: unbuffered.clone(),
                    expected_revision: 1,
                    start: true,
                },
            ),
            (
                "stage-unbuffered",
                ManagementCommand::StageRollout {
                    config: unbuffered.clone(),
                    expected_revision: 1,
                    stabilization_seconds: 2,
                    deadline_seconds: 30,
                },
            ),
        ] {
            let error = execute(
                &mut store,
                &owner,
                &request(id, command),
                &manifest,
                "boot",
                &root,
                101,
            )
            .unwrap_err();
            assert_eq!(rejection_code(&error), RejectionCode::Invalid, "{error:#}");
            assert!(error.to_string().contains("1 queued writes"), "{error:#}");
        }
        assert_eq!(store.get_placement("api")?.unwrap().config_revision, 1);
        assert!(store.latest_rollout("api")?.is_none());
        Ok(())
    }

    #[test]
    fn inspection_reports_host_isolation_and_refuses_undispatchable_reboots() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let root = crate::supervisor::prepare_state_dir(temp.path())?;
        crate::vault::write_new_private(
            &root.join("agent.env"),
            b"FLOW_LIKE_DEVICE_ISOLATION_POLICY=required\n",
        )?;
        let mut store = StateStore::open(&root.join("management.sqlite"))?;
        let signing = SigningKey::generate();
        let manifest = manifest(&signing);
        let owner = owner(&manifest);
        let inspected = execute(
            &mut store,
            &owner,
            &request("inspect", ManagementCommand::Inspect),
            &manifest,
            "boot",
            &root,
            101,
        )?;
        assert_eq!(inspected.result["host_isolation"], "required");
        assert_eq!(
            inspected.result["host_operations"]["reboot"],
            cfg!(target_os = "linux")
        );
        assert_eq!(inspected.result["agent_version"], env!("CARGO_PKG_VERSION"));
        let reader = project_grant("reader", vec![ManagementCapability::Status], 1000);
        accept_grants(&store, &signing, &[&reader], 100, 1000)?;
        let mut config = placement(&root)?;
        config["resources"] = json!({"profile":"trusted_process"});
        store.upsert_placement("api", &config, crate::state::DesiredState::Stopped)?;
        let page = execute(
            &mut store,
            &reader,
            &request(
                "reader-inspect",
                ManagementCommand::InspectPage {
                    after: None,
                    limit: 2,
                },
            ),
            &manifest,
            "boot",
            &root,
            101,
        )?;
        assert!(page.result["host_isolation"].is_null());
        assert!(page.result["isolation"].is_null());
        if !cfg!(target_os = "linux") {
            let error = execute(
                &mut store,
                &owner,
                &request(
                    "reboot",
                    ManagementCommand::Reboot {
                        expected_boot_id: "boot".into(),
                    },
                ),
                &manifest,
                "boot",
                &root,
                101,
            )
            .unwrap_err();
            assert_eq!(rejection_code(&error), RejectionCode::Unsupported);
            let queued: u64 =
                store
                    .connection
                    .query_row("SELECT COUNT(*) FROM host_operations", [], |r| r.get(0))?;
            assert_eq!(queued, 0);
        }
        Ok(())
    }

    async fn owner_noise_session(
        service: &Arc<ManagementService>,
        controller: &SigningKey,
    ) -> Result<(ManagementConnection, noise::Session)> {
        let now = unix_time()?;
        let certificate = sign_controller_certificate(
            &ControllerCertificate {
                version: 1,
                device_id: "device".into(),
                grant_id: "owner".into(),
                session_id: "session".into(),
                management_key: x25519_dalek::x25519([7; 32], x25519_dalek::X25519_BASEPOINT_BYTES),
                issued_at: now,
                expires_at: now + 300,
            },
            controller,
        )?;
        let mut initiator = noise::Handshake::initiator(
            &[7; 32],
            x25519_dalek::x25519([42; 32], x25519_dalek::X25519_BASEPOINT_BYTES),
            "device",
            "session",
        )?;
        let mut connection = service.connect(&certificate, "owner", "session")?;
        let reply = connection.receive(&initiator.write()?).await?;
        initiator.read(&reply)?;
        let ready = connection.receive(&initiator.write()?).await?;
        let mut session = initiator.finish()?;
        let ready: Value = serde_json::from_slice(&session.decrypt(&ready)?)?;
        ensure!(ready["ready"] == true, "Management session was not ready");
        Ok((connection, session))
    }

    #[tokio::test]
    async fn undecodable_requests_with_an_operation_id_are_rejected_and_keep_the_session()
    -> Result<()> {
        let temp = tempfile::tempdir()?;
        let controller = SigningKey::generate();
        let device = Arc::new(DeviceSession::test_management_session(
            "https://example.test/api/v1".into(),
            "device".into(),
            SigningKey::generate(),
            controller.public_key(),
        ));
        let service = ManagementService::new(temp.path().canonicalize()?, device, "boot".into());
        service.refresh_authority(unix_time()? + 300)?;
        let (mut connection, mut session) = owner_noise_session(&service, &controller).await?;
        let now = unix_time()?;
        let newer = json!({"operation_id":"from-newer-controller","device_id":"device","issued_at":now,"expires_at":now+60,"command":{"type":"future_command","value":"private-request-value"}});
        let reply = connection
            .receive(&session.encrypt(&serde_json::to_vec(&newer)?)?)
            .await?;
        let reply: Value = serde_json::from_slice(&session.decrypt(&reply)?)?;
        assert_eq!(reply["operation_id"], "from-newer-controller");
        assert_eq!(reply["state"], "rejected");
        assert_eq!(reply["result"]["code"], "unsupported");
        assert_eq!(reply["result"]["retryable"], false);
        assert!(
            reply["result"]["error"]
                .as_str()
                .unwrap()
                .contains("future_command")
        );
        assert!(!reply.to_string().contains("private-request-value"));
        let inspect = ManagementRequest {
            operation_id: "inspect-after-unsupported".into(),
            device_id: "device".into(),
            issued_at: now,
            expires_at: now + 60,
            command: ManagementCommand::Inspect,
        };
        let reply = connection
            .receive(&session.encrypt(&serde_json::to_vec(&inspect)?)?)
            .await?;
        let reply: Value = serde_json::from_slice(&session.decrypt(&reply)?)?;
        assert_eq!(reply["state"], "completed");
        let expired = ManagementRequest {
            operation_id: "expired".into(),
            device_id: "device".into(),
            issued_at: now - 1000,
            expires_at: now - 900,
            command: ManagementCommand::Inspect,
        };
        let reply = connection
            .receive(&session.encrypt(&serde_json::to_vec(&expired)?)?)
            .await?;
        let reply: Value = serde_json::from_slice(&session.decrypt(&reply)?)?;
        assert_eq!(reply["result"]["code"], "invalid");
        assert!(
            connection
                .receive(&session.encrypt(b"[\"no-operation-id\"]")?)
                .await
                .is_err()
        );
        Ok(())
    }

    #[test]
    fn remote_deploy_cannot_omit_or_downgrade_required_host_isolation() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let root = crate::supervisor::prepare_state_dir(temp.path())?;
        crate::vault::write_new_private(
            &root.join("agent.env"),
            b"FLOW_LIKE_DEVICE_ISOLATION_POLICY=required\n",
        )?;
        let mut store = StateStore::open(&root.join("management.sqlite"))?;
        let manifest = manifest(&SigningKey::generate());
        let authority = owner(&manifest);
        let mut config = placement(&root)?;
        for (index, resources) in [Value::Null, json!({"profile":"trusted_process"})]
            .into_iter()
            .enumerate()
        {
            config["resources"] = resources;
            let request = request(
                &format!("downgrade-{index}"),
                ManagementCommand::Apply {
                    config: config.clone(),
                    expected_revision: 0,
                    start: false,
                },
            );
            let error = execute(
                &mut store, &authority, &request, &manifest, "boot", &root, 101,
            )
            .unwrap_err();
            assert!(
                error.to_string().contains("requires linux_sandbox"),
                "{error:#}"
            );
            assert!(store.get_placement("api")?.is_none());
            let typed = serde_json::from_value(config.clone())?;
            assert!(crate::isolation::enforce_host_policy(&typed, &root).is_err());
        }
        Ok(())
    }

    #[test]
    fn offline_queue_management_pages_metadata_and_fences_uncertain_skip() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let root = temp.path().canonicalize()?;
        let mut store = StateStore::open(&root.join("management.sqlite"))?;
        let signing = SigningKey::generate();
        let manifest = manifest(&signing);
        let owner = owner(&manifest);
        let mut config = placement(&root)?;
        config["source"] = json!("online");
        config["resource_grant"] = json!({"grant_id":"grant","authz_version":1});
        config["offline_writes"] = json!({"tables":[{"purpose":"storage","database":"db","table":"measurements","primary_key":"id"}]});
        store.upsert_placement("api", &config, crate::state::DesiredState::Running)?;
        let mut data = root.clone();
        for part in ["placement-data", "api", "current", "store"] {
            data.push(part);
            crate::outbox::private_directory(&data)?;
        }
        let mut queues = Vec::new();
        for scope in ["a", "b", "c"] {
            let queue =
                crate::outbox::Outbox::open(&data, "api", &scope.repeat(64), Default::default())?;
            queue.enqueue(
                "table",
                json!({"private_body":"must-not-reach-management"}),
                None,
                100,
            )?;
            queues.push(queue);
        }
        let first = execute(
            &mut store,
            &owner,
            &request(
                "queue-page-1",
                ManagementCommand::OfflineQueue {
                    placement_id: "api".into(),
                    after: None,
                },
            ),
            &manifest,
            "boot",
            &root,
            101,
        )?;
        assert_eq!(first.result["queues"].as_array().unwrap().len(), 2);
        assert!(!serde_json::to_string(&first)?.contains("must-not-reach-management"));
        assert_eq!(first.result["next"], "b".repeat(64));
        let second = execute(
            &mut store,
            &owner,
            &request(
                "queue-page-2",
                ManagementCommand::OfflineQueue {
                    placement_id: "api".into(),
                    after: Some("b".repeat(64)),
                },
            ),
            &manifest,
            "boot",
            &root,
            101,
        )?;
        assert_eq!(second.result["queues"].as_array().unwrap().len(), 1);
        assert!(second.result["next"].is_null());
        let head = queues[0].head()?.unwrap();
        queues[0].mark_local(&head.operation_id, 1)?;
        queues[0].prepare_attempt(&head.operation_id, &head.payload)?;
        queues[0].block(
            &head.operation_id,
            "outcome_unknown",
            "Cloud acknowledgement lost",
        )?;
        let skip = |acknowledge_uncertain| ManagementCommand::OfflineQueueSkip {
            placement_id: "api".into(),
            scope: "a".repeat(64),
            queued_operation_id: head.operation_id.clone(),
            reason: "Discard the local pending effect".into(),
            acknowledge_uncertain,
        };
        assert!(
            execute(
                &mut store,
                &owner,
                &request("unsafe-skip", skip(false)),
                &manifest,
                "boot",
                &root,
                102
            )
            .is_err()
        );
        assert!(queues[0].requested_skip()?.is_none());
        let accepted = execute(
            &mut store,
            &owner,
            &request("acknowledged-skip", skip(true)),
            &manifest,
            "boot",
            &root,
            103,
        )?;
        assert_eq!(accepted.state, "accepted");
        assert_eq!(queues[0].requested_skip()?.unwrap().0, head.operation_id);
        let duplicate = execute(
            &mut store,
            &owner,
            &request("acknowledged-skip", skip(true)),
            &manifest,
            "boot",
            &root,
            104,
        )?;
        assert_eq!(accepted.result, duplicate.result);
        Ok(())
    }

    #[test]
    fn rollout_commands_preserve_secrets_fence_mutations_and_replay_receipts() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let root = temp.path().canonicalize()?;
        let mut store = StateStore::open(&root.join("management.sqlite"))?;
        let manifest = manifest(&SigningKey::generate());
        let owner = owner(&manifest);
        let mut config: PlacementConfig = serde_json::from_value(placement(&root)?)?;
        config.hosting = Some(serde_json::from_value(
            json!({"host":"127.0.0.1","port":8080,"max_in_flight":8,"request_timeout_secs":30,"auth_secret":"service-token"}),
        )?);
        crate::secrets::install(&config, "service-token", b"private-service-token")?;
        config
            .secret_overrides
            .insert("password".into(), "old-secret".into());
        crate::secrets::install(&config, "old-secret", b"original-value")?;
        store.upsert_placement(
            "api",
            &serde_json::to_value(&config)?,
            crate::state::DesiredState::Running,
        )?;
        let mut candidate = config.clone();
        candidate.revision = "release-2".into();
        candidate
            .secret_overrides
            .insert("password".into(), "next-secret".into());
        let stage = request(
            "rollout-1",
            ManagementCommand::StageRollout {
                config: serde_json::to_value(&candidate)?,
                expected_revision: 1,
                stabilization_seconds: 2,
                deadline_seconds: 30,
            },
        );
        let first = execute(&mut store, &owner, &stage, &manifest, "boot", &root, 100)?;
        let duplicate = execute(&mut store, &owner, &stage, &manifest, "boot", &root, 101)?;
        assert_eq!(
            serde_json::to_value(first)?,
            serde_json::to_value(duplicate)?
        );
        assert_eq!(store.get_placement("api")?.unwrap().config_revision, 1);
        for command in [
            ManagementCommand::Apply {
                config: serde_json::to_value(&candidate)?,
                expected_revision: 1,
                start: false,
            },
            ManagementCommand::Scale {
                placement_id: "api".into(),
                expected_revision: 1,
                replicas: 1,
            },
            ManagementCommand::Start {
                placement_id: "api".into(),
                expected_revision: 1,
            },
            ManagementCommand::Restart {
                placement_id: "api".into(),
                expected_revision: 1,
            },
            ManagementCommand::Remove {
                placement_id: "api".into(),
                expected_revision: 1,
            },
            ManagementCommand::SetSecret {
                placement_id: "api".into(),
                expected_revision: 1,
                name: "old-secret".into(),
                value: flow_like_device_protocol::SecretValue("changed".into()),
            },
            ManagementCommand::RolloutSecret {
                rollout_id: "rollout-1".into(),
                name: "old-secret".into(),
                value: flow_like_device_protocol::SecretValue("changed".into()),
            },
            ManagementCommand::Reboot {
                expected_boot_id: "boot".into(),
            },
        ] {
            assert!(
                execute(
                    &mut store,
                    &owner,
                    &request("conflict", command),
                    &manifest,
                    "boot",
                    &root,
                    101
                )
                .is_err()
            );
            assert!(store.connection.is_autocommit());
        }
        let secret = request(
            "candidate-secret",
            ManagementCommand::RolloutSecret {
                rollout_id: "rollout-1".into(),
                name: "next-secret".into(),
                value: flow_like_device_protocol::SecretValue("next-private-value".into()),
            },
        );
        let receipt = execute(&mut store, &owner, &secret, &manifest, "boot", &root, 101)?;
        assert_eq!(receipt.result["secret"], "completed");
        let journal: String = store.connection.query_row(
            "SELECT result_json FROM management_operations WHERE operation_id='candidate-secret'",
            [],
            |r| r.get(0),
        )?;
        assert!(!journal.contains("next-private-value"));
        assert_eq!(
            crate::vault::read_private(
                &crate::secrets::directory(&config)?.join("old-secret.secret")
            )?
            .as_slice(),
            b"original-value"
        );
        let activate = request(
            "activate",
            ManagementCommand::ActivateRollout {
                rollout_id: "rollout-1".into(),
            },
        );
        execute(&mut store, &owner, &activate, &manifest, "boot", &root, 102)?;
        let read = request(
            "status",
            ManagementCommand::Rollout {
                rollout_id: "rollout-1".into(),
            },
        );
        let status = execute(&mut store, &owner, &read, &manifest, "boot", &root, 102)?;
        assert_eq!(status.result["state"], "validating");
        assert!(status.result.get("candidate_config").is_none());
        execute(
            &mut store,
            &owner,
            &request(
                "stop",
                ManagementCommand::Stop {
                    placement_id: "api".into(),
                    expected_revision: 1,
                },
            ),
            &manifest,
            "boot",
            &root,
            103,
        )?;
        store.complete_rollout_validation("rollout-1", true, 104)?;
        store.reconcile_rollouts(105)?;
        assert_eq!(store.rollout("rollout-1")?.unwrap().state, "cancelled");
        let current = store.get_placement("api")?.unwrap();
        assert_eq!(current.config_revision, 1);
        assert_eq!(current.desired_state, crate::state::DesiredState::Stopped);
        store.set_desired_state("api", crate::state::DesiredState::Running)?;
        let mut stage_again = stage.clone();
        stage_again.operation_id = "rollout-discard".into();
        execute(
            &mut store,
            &owner,
            &stage_again,
            &manifest,
            "boot",
            &root,
            106,
        )?;
        let begin = request(
            "validate-discard",
            ManagementCommand::ActivateRollout {
                rollout_id: "rollout-discard".into(),
            },
        );
        execute(&mut store, &owner, &begin, &manifest, "boot", &root, 107)?;
        let discard = request(
            "discard",
            ManagementCommand::CancelRollout {
                rollout_id: "rollout-discard".into(),
            },
        );
        let receipt = execute(&mut store, &owner, &discard, &manifest, "boot", &root, 108)?;
        assert_eq!(receipt.result["failure_code"], "discarded");
        store.complete_rollout_validation("rollout-discard", true, 109)?;
        let current = store.get_placement("api")?.unwrap();
        assert_eq!(current.config_revision, 1);
        assert_eq!(current.desired_state, crate::state::DesiredState::Running);
        stage_again.operation_id = "rollout-active".into();
        execute(
            &mut store,
            &owner,
            &stage_again,
            &manifest,
            "boot",
            &root,
            110,
        )?;
        store.begin_rollout_validation("rollout-active", 110)?;
        store.complete_rollout_validation("rollout-active", true, 111)?;
        let discard = request(
            "discard-active",
            ManagementCommand::CancelRollout {
                rollout_id: "rollout-active".into(),
            },
        );
        assert!(execute(&mut store, &owner, &discard, &manifest, "boot", &root, 112).is_err());
        assert_eq!(
            store.rollout("rollout-active")?.unwrap().state,
            "activating"
        );
        Ok(())
    }

    #[test]
    fn rollout_read_and_activation_require_current_scoped_deploy_and_start_grants() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let root = temp.path().canonicalize()?;
        let mut store = StateStore::open(&root.join("management.sqlite"))?;
        let signer = SigningKey::generate();
        let manifest = manifest(&signer);
        let mut config: PlacementConfig = serde_json::from_value(placement(&root)?)?;
        config.hosting = Some(serde_json::from_value(
            json!({"host":"127.0.0.1","port":8080,"max_in_flight":8,"request_timeout_secs":30,"auth_secret":"service-token"}),
        )?);
        crate::secrets::install(&config, "service-token", b"private-service-token")?;
        store.upsert_placement(
            "api",
            &serde_json::to_value(&config)?,
            crate::state::DesiredState::Running,
        )?;
        let mut candidate = config.clone();
        candidate.revision = "release-2".into();
        let stage = request(
            "rollout-scope",
            ManagementCommand::StageRollout {
                config: serde_json::to_value(&candidate)?,
                expected_revision: 1,
                stabilization_seconds: 2,
                deadline_seconds: 30,
            },
        );
        execute(
            &mut store,
            &owner(&manifest),
            &stage,
            &manifest,
            "boot",
            &root,
            100,
        )?;
        let grant =
            |id: &str, project: &str, capabilities: Vec<ManagementCapability>| ManagementGrant {
                grant_id: id.into(),
                user_id: id.into(),
                controller_key: SigningKey::generate().public_key(),
                scope: ManagementScope::Project {
                    project_id: project.into(),
                },
                capabilities,
                expires_at: 1000,
                group_id: None,
                group_version: None,
            };
        let deploy = grant("deploy", "project", vec![ManagementCapability::Deploy]);
        let other = grant(
            "other",
            "other-project",
            vec![ManagementCapability::Deploy, ManagementCapability::Start],
        );
        let policy = ManagementPolicy {
            version: 1,
            device_id: "device".into(),
            policy_version: 1,
            previous_policy_digest: None,
            grants: vec![deploy.clone(), other.clone()],
            issued_at: 100,
            expires_at: 1000,
        };
        store.accept_management_policy(
            &sign_management_policy(&policy, &signer)?,
            &signer.public_key(),
            "device",
            100,
        )?;
        let authority = |grant: ManagementGrant| Authority {
            principal: format!("{}:{}", grant.user_id, grant.grant_id),
            key: grant.controller_key.clone(),
            grant: Some(grant),
        };
        let read = request(
            "read-rollout",
            ManagementCommand::Rollout {
                rollout_id: "rollout-scope".into(),
            },
        );
        assert!(
            execute(
                &mut store,
                &authority(other),
                &read,
                &manifest,
                "boot",
                &root,
                101
            )
            .is_err()
        );
        assert!(
            execute(
                &mut store,
                &authority(deploy.clone()),
                &read,
                &manifest,
                "boot",
                &root,
                101
            )
            .is_ok()
        );
        let activate = request(
            "activate-scope",
            ManagementCommand::ActivateRollout {
                rollout_id: "rollout-scope".into(),
            },
        );
        assert!(
            execute(
                &mut store,
                &authority(deploy),
                &activate,
                &manifest,
                "boot",
                &root,
                101
            )
            .is_err()
        );
        assert_eq!(store.rollout("rollout-scope")?.unwrap().state, "staged");
        Ok(())
    }

    #[test]
    fn inspection_error_text_follows_logs_and_device_facts_follow_device_scope() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let root = temp.path().canonicalize()?;
        let mut store = StateStore::open(&root.join("management.sqlite"))?;
        let signing = SigningKey::generate();
        let manifest = manifest(&signing);
        store.upsert_placement(
            "api",
            &placement(&root)?,
            crate::state::DesiredState::Running,
        )?;
        let error = format!("Cannot open {}/projects/api", root.display());
        store.connection.execute(
            "UPDATE placements SET observed_state='backoff',last_error=?1 WHERE id='api'",
            [&error],
        )?;
        store.connection.execute(
            "INSERT INTO placement_replicas(placement_id,slot,config_revision,intent_revision,observed_state,last_error) VALUES('api',0,1,1,'backoff',?1)",
            [&error],
        )?;
        let logs = project_grant(
            "logs",
            vec![ManagementCapability::Status, ManagementCapability::Logs],
            1000,
        );
        let status = project_grant("status", vec![ManagementCapability::Status], 1000);
        let mut device = project_grant("device", vec![ManagementCapability::Status], 1000);
        if let Some(grant) = &mut device.grant {
            grant.scope = ManagementScope::Device;
        }
        accept_grants(&store, &signing, &[&logs, &status, &device], 100, 1000)?;
        let mut inspect = |authority: &Authority| {
            execute(
                &mut store,
                authority,
                &request(
                    &format!("inspect-{}", authority.principal.replace(':', "-")),
                    ManagementCommand::InspectPage {
                        after: None,
                        limit: 2,
                    },
                ),
                &manifest,
                "boot",
                &root,
                101,
            )
            .map(|response| response.result)
        };
        let owned = inspect(&owner(&manifest))?;
        for flag in ["placement_diagnostics", "task_health", "placement_events"] {
            assert_eq!(owned["features"][flag], 1, "{flag}");
        }
        let row = &owned["placements"][0];
        assert_eq!(row["last_error"], "Cannot open <state>/projects/api");
        assert_eq!(
            row["replicas"][0]["last_error"],
            "Cannot open <state>/projects/api"
        );
        assert_eq!(row["source"], "offline");
        assert_eq!(row["events"][0]["event_id"], "event");
        assert!(!row.to_string().contains("public-listen-port"));
        assert_eq!(owned["agent"]["version"], env!("CARGO_PKG_VERSION"));
        assert!(owned["agent"]["release_sequence"].is_null());
        assert!(owned["host"]["agent_started_at"].is_i64());
        assert!(owned["tasks"].is_array());

        let with_logs = inspect(&logs)?;
        assert_eq!(with_logs["placements"][0]["last_error"], row["last_error"]);
        let status_only = inspect(&status)?;
        let row = &status_only["placements"][0];
        assert!(row.get("last_error").is_none());
        assert_eq!(row["has_error"], true);
        assert!(row["replicas"][0].get("last_error").is_none());
        assert_eq!(row["replicas"][0]["has_error"], true);
        for project_reader in [&with_logs, &status_only] {
            assert_eq!(project_reader["features"]["task_health"], 1);
            for device_fact in ["agent", "host", "tasks"] {
                assert!(project_reader.get(device_fact).is_none(), "{device_fact}");
            }
        }
        let device_reader = inspect(&device)?;
        assert!(device_reader["placements"][0].get("last_error").is_none());
        assert!(device_reader["agent"].is_object());
        assert!(device_reader["tasks"].is_array());
        Ok(())
    }

    #[test]
    fn worst_case_inspection_rows_fit_one_encrypted_message() -> Result<()> {
        use crate::diagnostics::test_support::{worst_case_id, worst_case_placement};
        let temp = tempfile::tempdir()?;
        let root = temp.path().canonicalize()?;
        let mut store = StateStore::open(&root.join("management.sqlite"))?;
        let manifest = manifest(&SigningKey::generate());
        let ids = ["worst-inspect-a", "worst-inspect-b"].map(worst_case_id);
        for id in &ids {
            worst_case_placement(&mut store, &root, crate::diagnostics::global(), id)?;
        }
        let owner = owner(&manifest);
        let mut inspect = |after: Option<&String>| {
            execute(
                &mut store,
                &owner,
                &request(
                    "inspect",
                    ManagementCommand::InspectPage {
                        after: after.cloned(),
                        limit: 2,
                    },
                ),
                &manifest,
                "boot",
                &root,
                101,
            )
        };
        let first = inspect(None)?;
        assert_eq!(first.state, "completed");
        assert!(serde_json::to_vec(&first)?.len() <= noise::MAX_PLAINTEXT);
        let rows = first.result["placements"].as_array().unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["id"], ids[0]);
        assert_eq!(first.result["next"], ids[0]);
        assert_eq!(rows[0]["replicas"].as_array().unwrap().len(), 32);
        assert!(rows[0]["replicas"][31]["restarts"].is_object());
        assert!(
            rows[0]["last_error"]
                .as_str()
                .unwrap()
                .starts_with("<state>/projects failed")
        );
        assert_eq!(rows[0]["events_truncated"], true);
        let second = inspect(Some(&ids[0]))?;
        assert!(serde_json::to_vec(&second)?.len() <= noise::MAX_PLAINTEXT);
        assert_eq!(second.result["placements"][0]["id"], ids[1]);
        assert!(second.result["next"].is_null());

        for index in 0..6 {
            let id = worst_case_id(&format!("worst-inspect-more-{index}"));
            worst_case_placement(&mut store, &root, crate::diagnostics::global(), &id)?;
        }
        let unpaged = execute(
            &mut store,
            &owner,
            &request("inspect", ManagementCommand::Inspect),
            &manifest,
            "boot",
            &root,
            101,
        )
        .unwrap_err();
        assert_eq!(rejection_code(&unpaged), RejectionCode::Limit);
        Ok(())
    }

    #[test]
    fn inspection_pages_filter_scope_before_limiting_and_finish_with_empty_pages() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let root = temp.path().canonicalize()?;
        let mut store = StateStore::open(&root.join("management.sqlite"))?;
        let signing = SigningKey::generate();
        let manifest = manifest(&signing);
        let config = placement(&root)?;
        for (id, project) in [
            ("a-hidden", "other"),
            ("b-hidden", "other"),
            ("c-hidden", "other"),
            ("d-visible", "project"),
            ("e-hidden", "other"),
            ("f-visible", "project"),
            ("g-hidden", "other"),
            ("h-visible", "project"),
            ("i-hidden", "other"),
        ] {
            let mut config = config.clone();
            config["id"] = json!(id);
            config["project_id"] = json!(project);
            store.upsert_placement(id, &config, crate::state::DesiredState::Stopped)?;
        }
        let reader = |id: &str, scope: ManagementScope, capability| {
            let key = SigningKey::generate().public_key();
            Authority {
                principal: format!("reader:{id}"),
                key: key.clone(),
                grant: Some(ManagementGrant {
                    grant_id: id.into(),
                    user_id: "reader".into(),
                    controller_key: key,
                    scope,
                    capabilities: vec![capability],
                    expires_at: 1000,
                    group_id: None,
                    group_version: None,
                }),
            }
        };
        let project_reader = reader(
            "project-reader",
            ManagementScope::Project {
                project_id: "project".into(),
            },
            ManagementCapability::Status,
        );
        let placement_reader = reader(
            "placement-reader",
            ManagementScope::Placement {
                project_id: "project".into(),
                placement_id: "f-visible".into(),
            },
            ManagementCapability::Status,
        );
        let empty_reader = reader(
            "empty-project-reader",
            ManagementScope::Project {
                project_id: "undeployed".into(),
            },
            ManagementCapability::Status,
        );
        let metrics_reader = reader(
            "metrics-reader",
            ManagementScope::Device,
            ManagementCapability::Metrics,
        );
        let policy = ManagementPolicy {
            version: 1,
            device_id: manifest.device_id.clone(),
            policy_version: 1,
            previous_policy_digest: None,
            grants: [
                &project_reader,
                &placement_reader,
                &empty_reader,
                &metrics_reader,
            ]
            .into_iter()
            .filter_map(|authority| authority.grant.clone())
            .collect(),
            issued_at: 100,
            expires_at: 1000,
        };
        store.accept_management_policy(
            &sign_management_policy(&policy, &signing)?,
            &signing.public_key(),
            &manifest.device_id,
            100,
        )?;
        let mut inspect = |authority: &Authority, after: Option<&str>, limit| {
            execute(
                &mut store,
                authority,
                &request(
                    "inspect",
                    ManagementCommand::InspectPage {
                        after: after.map(str::to_owned),
                        limit,
                    },
                ),
                &manifest,
                "boot",
                &root,
                101,
            )
        };
        let first = inspect(&project_reader, None, 2)?;
        assert_eq!(first.state, "completed");
        assert_eq!(first.result["device_id"], "device");
        assert!(first.result["boot_id"].is_null());
        let rows = first.result["placements"].as_array().unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0]["id"], "d-visible");
        assert_eq!(rows[1]["id"], "f-visible");
        assert!(rows.iter().all(|row| row["project_id"] == "project"));
        assert!(rows[0]["applied_revision"].is_null());
        assert_eq!(first.result["next"], "f-visible");
        let second = inspect(&project_reader, Some("f-visible"), 2)?;
        assert_eq!(second.result["placements"].as_array().unwrap().len(), 1);
        assert_eq!(second.result["placements"][0]["id"], "h-visible");
        assert!(second.result["next"].is_null());
        let smaller = inspect(&project_reader, None, 1)?;
        assert_eq!(smaller.result["placements"].as_array().unwrap().len(), 1);
        assert_eq!(smaller.result["next"], "d-visible");
        let only_placement = inspect(&placement_reader, None, 2)?;
        assert_eq!(
            only_placement.result["placements"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        assert_eq!(only_placement.result["placements"][0]["id"], "f-visible");
        assert!(only_placement.result["next"].is_null());
        for (authority, after) in [
            (&project_reader, Some("h-visible")),
            (&placement_reader, Some("f-visible")),
            (&empty_reader, None),
        ] {
            let empty = inspect(authority, after, 2)?;
            assert_eq!(empty.state, "completed");
            assert_eq!(empty.result["placements"], json!([]));
            assert!(empty.result["next"].is_null());
        }
        let owner = owner(&manifest);
        let global = inspect(&owner, None, 2)?;
        assert_eq!(global.result["boot_id"], "boot");
        assert_eq!(global.result["placements"][0]["id"], "a-hidden");
        assert_eq!(global.result["next"], "b-hidden");
        for limit in [0, 3, u16::MAX] {
            assert!(inspect(&owner, None, limit).is_err());
        }
        assert!(inspect(&owner, Some("invalid/cursor"), 2).is_err());
        assert!(inspect(&metrics_reader, None, 2).is_err());
        assert!(store.connection.is_autocommit());
        Ok(())
    }

    #[test]
    fn operations_commit_once_and_replay_across_database_reopen() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let root = temp.path().canonicalize()?;
        let mut store = StateStore::open(&root.join("management.sqlite"))?;
        let manifest = manifest(&SigningKey::generate());
        let owner = owner(&manifest);
        let apply = request(
            "apply",
            ManagementCommand::Apply {
                config: placement(&root)?,
                expected_revision: 0,
                start: true,
            },
        );
        execute(&mut store, &owner, &apply, &manifest, "boot", &root, 100)?;
        let stop = request(
            "stop",
            ManagementCommand::Stop {
                placement_id: "api".into(),
                expected_revision: 1,
            },
        );
        let accepted = execute(&mut store, &owner, &stop, &manifest, "boot", &root, 101)?;
        assert_eq!(store.get_placement("api")?.unwrap().intent_revision, 2);
        drop(store);
        let mut store = StateStore::open(&root.join("management.sqlite"))?;
        assert_eq!(
            serde_json::to_value(execute(
                &mut store, &owner, &stop, &manifest, "boot", &root, 102
            )?)?,
            serde_json::to_value(accepted)?
        );
        assert_eq!(store.get_placement("api")?.unwrap().intent_revision, 2);
        let substituted = request(
            "stop",
            ManagementCommand::Restart {
                placement_id: "api".into(),
                expected_revision: 1,
            },
        );
        assert!(
            execute(
                &mut store,
                &owner,
                &substituted,
                &manifest,
                "boot",
                &root,
                102
            )
            .is_err()
        );
        assert_eq!(store.get_placement("api")?.unwrap().intent_revision, 2);
        let status = execute(
            &mut store,
            &owner,
            &request("inspect", ManagementCommand::Inspect),
            &manifest,
            "boot",
            &root,
            103,
        )?;
        let encoded = serde_json::to_string(&status)?;
        assert!(!encoded.contains("public-listen-port"));
        assert!(!encoded.contains("project_path"));
        Ok(())
    }

    #[test]
    fn revision_conflicts_and_unmanaged_project_paths_do_not_record_operations() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let root = temp.path().canonicalize()?;
        let mut store = StateStore::open(&root.join("management.sqlite"))?;
        let manifest = manifest(&SigningKey::generate());
        let owner = owner(&manifest);
        let mut config = placement(&root)?;
        config["project_path"] = json!(root);
        assert!(
            execute(
                &mut store,
                &owner,
                &request(
                    "outside",
                    ManagementCommand::Apply {
                        config,
                        expected_revision: 0,
                        start: true
                    }
                ),
                &manifest,
                "boot",
                &root,
                100
            )
            .is_err()
        );
        assert!(store.list_placements()?.is_empty());
        execute(
            &mut store,
            &owner,
            &request(
                "apply",
                ManagementCommand::Apply {
                    config: placement(&root)?,
                    expected_revision: 0,
                    start: true,
                },
            ),
            &manifest,
            "boot",
            &root,
            100,
        )?;
        assert!(
            execute(
                &mut store,
                &owner,
                &request(
                    "stale",
                    ManagementCommand::Stop {
                        placement_id: "api".into(),
                        expected_revision: 2
                    }
                ),
                &manifest,
                "boot",
                &root,
                100
            )
            .is_err()
        );
        let count: u64 =
            store
                .connection
                .query_row("SELECT COUNT(*) FROM management_operations", [], |r| {
                    r.get(0)
                })?;
        assert_eq!(count, 1);
        assert_eq!(store.get_placement("api")?.unwrap().intent_revision, 1);
        Ok(())
    }

    #[test]
    fn placement_configuration_reads_require_scoped_deploy_without_resolving_secrets() -> Result<()>
    {
        let temp = tempfile::tempdir()?;
        let root = temp.path().canonicalize()?;
        let mut store = StateStore::open(&root.join("management.sqlite"))?;
        let signing = SigningKey::generate();
        let manifest = manifest(&signing);
        let owner = owner(&manifest);
        let mut config = placement(&root)?;
        config["secret_overrides"] = json!({"private-variable":"private-ref"});
        crate::secrets::install(
            &serde_json::from_value(config.clone())?,
            "private-ref",
            b"private-secret-contents",
        )?;
        execute(
            &mut store,
            &owner,
            &request(
                "apply",
                ManagementCommand::Apply {
                    config: config.clone(),
                    expected_revision: 0,
                    start: true,
                },
            ),
            &manifest,
            "boot",
            &root,
            100,
        )?;
        let read = |id: &str, placement: &str| {
            request(
                id,
                ManagementCommand::PlacementConfiguration {
                    placement_id: placement.into(),
                },
            )
        };
        let current = execute(
            &mut store,
            &owner,
            &read("read-1", "api"),
            &manifest,
            "boot",
            &root,
            101,
        )?;
        assert_eq!(current.state, "completed");
        assert_eq!(current.result["placement_id"], "api");
        assert_eq!(current.result["project_id"], "project");
        assert_eq!(current.result["deployment_id"], "deployment");
        assert_eq!(current.result["config_revision"], 1);
        assert_eq!(
            current.result["config"]["secret_overrides"]["private-variable"],
            "private-ref"
        );
        assert!(!serde_json::to_string(&current)?.contains("private-secret-contents"));
        assert_eq!(
            current.result["config"]["variables"]["public-listen-port"],
            8080
        );
        assert!(
            execute(
                &mut store,
                &owner,
                &read("read-missing", "missing"),
                &manifest,
                "boot",
                &root,
                101
            )
            .is_err()
        );

        let mut updated = current.result["config"].clone();
        updated["variables"]["public-listen-port"] = json!(9090);
        let applied = execute(
            &mut store,
            &owner,
            &request(
                "update",
                ManagementCommand::Apply {
                    config: updated,
                    expected_revision: 1,
                    start: false,
                },
            ),
            &manifest,
            "boot",
            &root,
            102,
        )?;
        assert_eq!(applied.result["config_revision"], 2);
        let after = execute(
            &mut store,
            &owner,
            &read("read-2", "api"),
            &manifest,
            "boot",
            &root,
            103,
        )?;
        assert_eq!(after.result["config_revision"], 2);
        assert_eq!(
            after.result["config"]["variables"]["public-listen-port"],
            9090
        );

        let operator = SigningKey::generate();
        let deployer = SigningKey::generate();
        let grant =
            |id: &str, key: &SigningKey, capabilities: Vec<ManagementCapability>| ManagementGrant {
                grant_id: id.into(),
                user_id: id.into(),
                controller_key: key.public_key(),
                scope: ManagementScope::Project {
                    project_id: "project".into(),
                },
                capabilities,
                expires_at: 1000,
                group_id: None,
                group_version: None,
            };
        let status_only = grant(
            "operator",
            &operator,
            vec![ManagementCapability::Status, ManagementCapability::Stop],
        );
        let deploy = grant("deployer", &deployer, vec![ManagementCapability::Deploy]);
        let policy = ManagementPolicy {
            version: 1,
            device_id: "device".into(),
            policy_version: 1,
            previous_policy_digest: None,
            grants: vec![status_only.clone(), deploy.clone()],
            issued_at: 100,
            expires_at: 1000,
        };
        store.accept_management_policy(
            &sign_management_policy(&policy, &signing)?,
            &signing.public_key(),
            "device",
            100,
        )?;
        let as_grant = |grant: &ManagementGrant, key: &SigningKey| Authority {
            principal: format!("{}:{}", grant.user_id, grant.grant_id),
            key: key.public_key(),
            grant: Some(grant.clone()),
        };
        assert!(
            execute(
                &mut store,
                &as_grant(&status_only, &operator),
                &read("read-status", "api"),
                &manifest,
                "boot",
                &root,
                104
            )
            .is_err()
        );
        let deployed = execute(
            &mut store,
            &as_grant(&deploy, &deployer),
            &read("read-deploy", "api"),
            &manifest,
            "boot",
            &root,
            104,
        )?;
        assert_eq!(deployed.result["config_revision"], 2);
        let mut other = config;
        other["id"] = json!("other-api");
        other["project_id"] = json!("other-project");
        store.upsert_placement("other-api", &other, crate::state::DesiredState::Stopped)?;
        assert!(
            execute(
                &mut store,
                &as_grant(&deploy, &deployer),
                &read("read-other-project", "other-api"),
                &manifest,
                "boot",
                &root,
                104,
            )
            .is_err()
        );
        let journal: u64 =
            store
                .connection
                .query_row("SELECT COUNT(*) FROM management_operations", [], |r| {
                    r.get(0)
                })?;
        assert_eq!(journal, 2);
        Ok(())
    }

    #[test]
    fn placement_configuration_bounds_the_entire_noise_response_without_journaling() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let root = temp.path().canonicalize()?;
        let mut store = StateStore::open(&root.join("management.sqlite"))?;
        let manifest = manifest(&SigningKey::generate());
        let owner = owner(&manifest);
        let mut config: PlacementConfig = serde_json::from_value(placement(&root)?)?;
        config.variables.insert("padding".into(), json!(""));
        store.upsert_placement(
            "api",
            &serde_json::to_value(&config)?,
            crate::state::DesiredState::Stopped,
        )?;
        let read = request(
            "read",
            ManagementCommand::PlacementConfiguration {
                placement_id: "api".into(),
            },
        );
        let initial = execute(&mut store, &owner, &read, &manifest, "boot", &root, 100)?;
        let padding = noise::MAX_PLAINTEXT - serde_json::to_vec(&initial)?.len();
        config
            .variables
            .insert("padding".into(), json!("x".repeat(padding)));
        store.connection.execute(
            "UPDATE placements SET config_json=?1 WHERE id='api'",
            [serde_json::to_string(&config)?],
        )?;
        let response = execute(&mut store, &owner, &read, &manifest, "boot", &root, 100)?;
        assert_eq!(serde_json::to_vec(&response)?.len(), noise::MAX_PLAINTEXT);
        config
            .variables
            .insert("padding".into(), json!("x".repeat(padding + 1)));
        assert!(serde_json::to_vec(&config)?.len() < noise::MAX_PLAINTEXT);
        store.connection.execute(
            "UPDATE placements SET config_json=?1 WHERE id='api'",
            [serde_json::to_string(&config)?],
        )?;
        let error = execute(&mut store, &owner, &read, &manifest, "boot", &root, 100).unwrap_err();
        assert!(error.to_string().contains("remote read limit"));
        let journal: u64 =
            store
                .connection
                .query_row("SELECT COUNT(*) FROM management_operations", [], |r| {
                    r.get(0)
                })?;
        assert_eq!(journal, 0);
        assert!(store.connection.is_autocommit());
        Ok(())
    }

    #[test]
    fn apply_updates_state_and_intent_once_preserves_secrets_and_fences_revisions() -> Result<()> {
        use crate::state::DesiredState;
        let temp = tempfile::tempdir()?;
        let root = temp.path().canonicalize()?;
        let mut store = StateStore::open(&root.join("management.sqlite"))?;
        let manifest = manifest(&SigningKey::generate());
        let owner = owner(&manifest);
        let mut config = placement(&root)?;
        config["max_replicas"] = json!(4);
        config["hosting"] = json!({"host":"127.0.0.1","port":8080,"max_in_flight":8,"request_timeout_secs":30,"auth_secret":"service-token"});
        config["secret_overrides"] = json!({"private-variable":"private-ref"});
        let old: PlacementConfig = serde_json::from_value(config.clone())?;
        crate::secrets::install(&old, "private-ref", b"private-secret-contents")?;
        crate::secrets::install(&old, "service-token", b"private-service-token")?;
        let apply = |store: &mut StateStore, id: &str, config: &Value, revision, start| {
            execute(
                store,
                &owner,
                &request(
                    id,
                    ManagementCommand::Apply {
                        config: config.clone(),
                        expected_revision: revision,
                        start,
                    },
                ),
                &manifest,
                "boot",
                &root,
                100,
            )
        };
        apply(&mut store, "create", &config, 0, true)?;
        store.set_replica_count("api", 1, 3)?;
        let initial = store.get_placement("api")?.unwrap();
        assert_eq!((initial.config_revision, initial.intent_revision), (1, 1));
        assert_eq!(initial.desired_state, DesiredState::Running);

        let source = tempfile::tempdir()?;
        std::fs::create_dir_all(source.path().join("apps/project"))?;
        std::fs::write(
            source.path().join("apps/project/manifest.app"),
            b"new-project-snapshot",
        )?;
        let imported =
            crate::project_artifacts::import_local(&store, &root, "project", source.path())?;
        config["project_path"] = json!(
            imported
                .project_path
                .context("Missing imported project path")?
        );
        config["revision"] = json!("release-2");
        config["variables"]["public-listen-port"] = json!(9090);
        config["max_replicas"] = json!(2);
        let accepted = apply(&mut store, "update", &config, 1, false)?;
        assert_eq!(accepted.result["config_revision"], 2);
        let stopped = store.get_placement("api")?.unwrap();
        assert_eq!((stopped.config_revision, stopped.intent_revision), (2, 2));
        assert_eq!(stopped.desired_state, DesiredState::Stopped);
        assert_eq!(stopped.desired_replicas, 2);
        let updated: PlacementConfig = serde_json::from_value(stopped.config.clone())?;
        for (name, value) in [
            ("private-ref", b"private-secret-contents".as_slice()),
            ("service-token", b"private-service-token".as_slice()),
        ] {
            assert_eq!(
                &**crate::vault::read_private(&crate::config::private_secret_path(
                    &updated, name
                )?)?,
                value
            );
        }
        let current = execute(
            &mut store,
            &owner,
            &request(
                "read",
                ManagementCommand::PlacementConfiguration {
                    placement_id: "api".into(),
                },
            ),
            &manifest,
            "boot",
            &root,
            100,
        )?;
        let encoded = serde_json::to_string(&current)?;
        assert!(!encoded.contains("private-secret-contents"));
        assert!(!encoded.contains("private-service-token"));

        let set_secret = |id: &str, expected_revision| {
            request(
                id,
                ManagementCommand::SetSecret {
                    placement_id: "api".into(),
                    expected_revision,
                    name: "private-ref".into(),
                    value: SecretValue("replacement-private-secret".into()),
                },
            )
        };
        assert!(
            execute(
                &mut store,
                &owner,
                &set_secret("stale-secret", 1),
                &manifest,
                "boot",
                &root,
                100
            )
            .is_err()
        );
        let replacement = execute(
            &mut store,
            &owner,
            &set_secret("new-secret", 2),
            &manifest,
            "boot",
            &root,
            100,
        )?;
        assert_eq!(replacement.result["secret"], "pending");
        assert!(crate::secrets::publish_one(&root)?);
        assert_eq!(
            &**crate::vault::read_private(&crate::config::private_secret_path(
                &updated,
                "private-ref"
            )?)?,
            b"replacement-private-secret"
        );

        apply(&mut store, "same-stopped", &config, 2, false)?;
        let same = store.get_placement("api")?.unwrap();
        assert_eq!(
            (
                same.config_revision,
                same.intent_revision,
                same.desired_replicas
            ),
            (2, 2, 2)
        );
        apply(&mut store, "start-current", &config, 2, true)?;
        let started = store.get_placement("api")?.unwrap();
        assert_eq!((started.config_revision, started.intent_revision), (2, 3));
        assert_eq!(started.desired_state, DesiredState::Running);
        apply(&mut store, "same-running", &config, 2, true)?;
        assert_eq!(store.get_placement("api")?.unwrap().intent_revision, 3);
        config["variables"]["public-listen-port"] = json!(9091);
        apply(&mut store, "update-running", &config, 2, true)?;
        let running = store.get_placement("api")?.unwrap();
        assert_eq!(
            (
                running.config_revision,
                running.intent_revision,
                running.desired_replicas
            ),
            (3, 4, 2)
        );
        assert_eq!(running.desired_state, DesiredState::Running);
        assert!(apply(&mut store, "stale-apply", &config, 2, false).is_err());
        let after_stale = store.get_placement("api")?.unwrap();
        assert_eq!(
            (after_stale.config_revision, after_stale.intent_revision),
            (3, 4)
        );
        assert_eq!(after_stale.desired_state, DesiredState::Running);
        let failed_journal: u64 = store.connection.query_row("SELECT COUNT(*) FROM management_operations WHERE operation_id IN ('stale-apply','stale-secret')", [], |r| r.get(0))?;
        assert_eq!(failed_journal, 0);
        Ok(())
    }

    #[test]
    fn revoked_grants_cannot_use_a_previous_authority_snapshot() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let root = temp.path().canonicalize()?;
        let mut store = StateStore::open(&root.join("management.sqlite"))?;
        let signing = SigningKey::generate();
        let manifest = manifest(&signing);
        let owner = owner(&manifest);
        execute(
            &mut store,
            &owner,
            &request(
                "apply",
                ManagementCommand::Apply {
                    config: placement(&root)?,
                    expected_revision: 0,
                    start: true,
                },
            ),
            &manifest,
            "boot",
            &root,
            100,
        )?;
        let reader = SigningKey::generate();
        let grant = ManagementGrant {
            grant_id: "project-operator".into(),
            user_id: "reader".into(),
            controller_key: reader.public_key(),
            scope: ManagementScope::Project {
                project_id: "project".into(),
            },
            capabilities: vec![
                ManagementCapability::Deploy,
                ManagementCapability::Stop,
                ManagementCapability::Status,
                ManagementCapability::Logs,
                ManagementCapability::Metrics,
            ],
            expires_at: 1000,
            group_id: None,
            group_version: None,
        };
        let mut policy = ManagementPolicy {
            version: 1,
            device_id: "device".into(),
            policy_version: 1,
            previous_policy_digest: None,
            grants: vec![grant.clone()],
            issued_at: 100,
            expires_at: 1000,
        };
        let signed = sign_management_policy(&policy, &signing)?;
        store.accept_management_policy(&signed, &signing.public_key(), "device", 100)?;
        let authority = Authority {
            principal: "reader:project-operator".into(),
            key: reader.public_key(),
            grant: Some(grant),
        };
        let inspection = execute(
            &mut store,
            &authority,
            &request(
                "inspect-before-revocation",
                ManagementCommand::InspectPage {
                    after: None,
                    limit: 2,
                },
            ),
            &manifest,
            "boot",
            &root,
            101,
        )?;
        assert_eq!(inspection.result["placements"][0]["id"], "api");
        let telemetry = crate::telemetry::TelemetryStore::open(&root)?;
        telemetry.append(Some("api"), "log", &json!({"message":"private-log"}))?;
        telemetry.append(Some("api"), "metrics", &json!({"cpu":12}))?;
        let read_commands = vec![
            ManagementCommand::Inspect,
            ManagementCommand::PlacementConfiguration {
                placement_id: "api".into(),
            },
            ManagementCommand::Logs {
                placement_id: Some("api".into()),
                after: 0,
                limit: 10,
            },
            ManagementCommand::Metrics {
                placement_id: Some("api".into()),
            },
            ManagementCommand::ProjectMetrics {
                project_id: "project".into(),
            },
            ManagementCommand::Messages {
                placement_id: Some("api".into()),
                project_id: None,
                after: 0,
                limit: 10,
            },
            ManagementCommand::Messages {
                placement_id: None,
                project_id: Some("project".into()),
                after: 0,
                limit: 10,
            },
            ManagementCommand::ArchiveRead {
                scope: "api".into(),
                kind: ArchiveKind::Logs,
                sequence: 0,
                offset: 0,
                limit: 1024,
            },
            ManagementCommand::ArchiveRosterRead {
                scope: "api".into(),
                kind: ArchiveKind::Logs,
                offset: 0,
                limit: 1024,
            },
        ];
        for command in &read_commands {
            assert_eq!(
                execute(
                    &mut store,
                    &authority,
                    &request("read-before", command.clone()),
                    &manifest,
                    "boot",
                    &root,
                    101
                )?
                .state,
                "completed"
            );
        }
        let service = ManagementService::new(
            root.clone(),
            Arc::new(
                DeviceSession::test_session(
                    "https://example.test/api/v1".into(),
                    "device".into(),
                    SigningKey::generate(),
                )
                .test_with_invitation_key(signing.public_key()),
            ),
            "boot".into(),
        );
        let roster_read = request(
            "roster-before",
            ManagementCommand::TelemetryRosterRead {
                scope: "api".into(),
                offset: 0,
                limit: 1024,
            },
        );
        assert_eq!(
            execute_telemetry_group(&store, &authority, &roster_read, &service, 101)?.state,
            "completed"
        );
        assert!(
            execute(
                &mut store,
                &authority,
                &request(
                    "reboot",
                    ManagementCommand::Reboot {
                        expected_boot_id: "boot".into()
                    }
                ),
                &manifest,
                "boot",
                &root,
                101
            )
            .is_err()
        );
        policy.policy_version = 2;
        policy.previous_policy_digest = Some(compact_digest(&signed));
        policy.grants.clear();
        policy.issued_at = 102;
        let removal = sign_management_policy(&policy, &signing)?;
        store.accept_management_policy(&removal, &signing.public_key(), "device", 102)?;
        for command in read_commands {
            let error = execute(
                &mut store,
                &authority,
                &request("read-after", command),
                &manifest,
                "boot",
                &root,
                103,
            )
            .err()
            .context("Revoked reader was admitted")?;
            assert!(error.to_string().contains("Management grant changed"));
            assert!(store.connection.is_autocommit());
        }
        for command in [
            roster_read.command,
            ManagementCommand::TelemetryRead {
                scope: "api".into(),
                sequence: 0,
                welcome: false,
                offset: 0,
                limit: 1024,
            },
            ManagementCommand::TelemetryReceipt {
                scope: "api".into(),
                endpoint_id: "old-reader".into(),
                sequence: 1,
                receipt_jws: "invalid-proof-must-not-be-parsed".into(),
            },
        ] {
            let error = execute_telemetry_group(
                &store,
                &authority,
                &request("group-after", command),
                &service,
                103,
            )
            .err()
            .context("Revoked MLS reader was admitted")?;
            assert!(error.to_string().contains("Management grant changed"));
            assert!(store.connection.is_autocommit());
        }
        for after in [None, Some("api".into())] {
            assert!(
                execute(
                    &mut store,
                    &authority,
                    &request(
                        "inspect-after-revocation",
                        ManagementCommand::InspectPage { after, limit: 2 },
                    ),
                    &manifest,
                    "boot",
                    &root,
                    103,
                )
                .is_err()
            );
            assert!(store.connection.is_autocommit());
        }
        assert!(
            execute(
                &mut store,
                &authority,
                &request(
                    "stop",
                    ManagementCommand::Stop {
                        placement_id: "api".into(),
                        expected_revision: 1
                    }
                ),
                &manifest,
                "boot",
                &root,
                103
            )
            .is_err()
        );
        assert!(
            store
                .accept_management_policy(&signed, &signing.public_key(), "device", 103)
                .is_err()
        );
        assert_eq!(store.get_placement("api")?.unwrap().intent_revision, 1);
        Ok(())
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn reboot_completion_requires_a_new_os_boot_identity() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let root = temp.path().canonicalize()?;
        let mut store = StateStore::open(&root.join("management.sqlite"))?;
        let manifest = manifest(&SigningKey::generate());
        let owner = owner(&manifest);
        let reboot = request(
            "reboot",
            ManagementCommand::Reboot {
                expected_boot_id: "boot-a".into(),
            },
        );
        execute(&mut store, &owner, &reboot, &manifest, "boot-a", &root, 100)?;
        crate::host::reconcile_boot(&root, "boot-a")?;
        let state: String =
            store
                .connection
                .query_row("SELECT state FROM host_operations", [], |r| r.get(0))?;
        assert_eq!(state, "pending");
        crate::host::reconcile_boot(&root, "boot-b")?;
        let response = execute(&mut store, &owner, &reboot, &manifest, "boot-b", &root, 101)?;
        assert_eq!(response.state, "completed");
        let count: u64 =
            store
                .connection
                .query_row("SELECT COUNT(*) FROM host_operations", [], |r| r.get(0))?;
        assert_eq!(count, 1);
        Ok(())
    }

    #[tokio::test]
    #[ignore = "requires Node and generated browser WASM assets from build:device-crypto"]
    async fn browser_wasm_and_native_agent_exchange_authenticated_noise_commands() -> Result<()> {
        use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
        use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .canonicalize()?;
        let mut child = tokio::process::Command::new("node")
            .arg(root.join("packages/device-crypto/tests/native-peer.mjs"))
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::inherit())
            .kill_on_drop(true)
            .spawn()?;
        let mut input = child.stdin.take().context("Missing browser input")?;
        let mut lines =
            BufReader::new(child.stdout.take().context("Missing browser output")?).lines();
        async fn line(
            lines: &mut tokio::io::Lines<BufReader<tokio::process::ChildStdout>>,
        ) -> Result<Value> {
            let line = tokio::time::timeout(std::time::Duration::from_secs(60), lines.next_line())
                .await??
                .context("Browser peer closed")?;
            ensure!(line.len() <= 65536, "Browser peer frame exceeds bound");
            Ok(serde_json::from_str(&line)?)
        }
        async fn send(input: &mut tokio::process::ChildStdin, value: Value) -> Result<()> {
            let mut bytes = serde_json::to_vec(&value)?;
            bytes.push(b'\n');
            input.write_all(&bytes).await?;
            input.flush().await?;
            Ok(())
        }
        let public = line(&mut lines).await?;
        let controller_key: Ed25519PublicKey =
            serde_json::from_value(public["controller_key"].clone())?;
        let directory = tempfile::tempdir()?;
        let state = directory.path().canonicalize()?;
        let device = Arc::new(DeviceSession::test_management_session(
            "https://example.test/api/v1".into(),
            "device".into(),
            SigningKey::generate(),
            controller_key,
        ));
        let service = ManagementService::new(state, device, "test-boot".into());
        service.refresh_authority(unix_time()? + 300)?;
        send(&mut input,json!({"management_key":x25519_dalek::PublicKey::from(&x25519_dalek::StaticSecret::from([42;32])).to_bytes()})).await?;
        let hello = line(&mut lines).await?;
        let mut connection = service.connect(
            hello["certificate"]
                .as_str()
                .context("Missing certificate")?,
            "owner",
            hello["session_id"].as_str().context("Missing session")?,
        )?;
        let response = connection
            .receive(&URL_SAFE_NO_PAD.decode(hello["data"].as_str().context("Missing handshake")?)?)
            .await?;
        send(&mut input, json!({"data":URL_SAFE_NO_PAD.encode(response)})).await?;
        for _ in 0..2 {
            let frame = line(&mut lines).await?;
            let response = connection
                .receive(&URL_SAFE_NO_PAD.decode(frame["data"].as_str().context("Missing frame")?)?)
                .await?;
            send(&mut input, json!({"data":URL_SAFE_NO_PAD.encode(response)})).await?;
        }
        let replay = line(&mut lines).await?;
        assert!(
            connection
                .receive(
                    &URL_SAFE_NO_PAD.decode(replay["data"].as_str().context("Missing replay")?)?
                )
                .await
                .is_err()
        );
        send(&mut input, json!({"closed":true})).await?;
        drop(input);
        ensure!(
            tokio::time::timeout(std::time::Duration::from_secs(5), child.wait())
                .await??
                .success(),
            "Browser peer verification failed"
        );
        Ok(())
    }

    #[test]
    fn certificate_commands_are_private_idempotent_scoped_and_bound() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let root = crate::supervisor::prepare_state_dir(temp.path())?;
        let mut store = StateStore::open(&root.join("management.sqlite"))?;
        let signing = SigningKey::generate();
        let manifest = manifest(&signing);
        let owner = owner(&manifest);
        let now = unix_time()?;
        let request = |id: &str, command| ManagementRequest {
            operation_id: id.into(),
            device_id: manifest.device_id.clone(),
            issued_at: now,
            expires_at: now + 100,
            command,
        };
        let id = uuid::Uuid::new_v4().to_string();
        let unbound = uuid::Uuid::new_v4().to_string();
        let identity = rcgen::generate_simple_self_signed(vec!["service.example.test".into()])?;
        let certificate_chain_pem = identity.cert.pem();
        let private_key_pem = identity.signing_key.serialize_pem();
        let put = |id: &str, revision| ManagementCommand::PutCertificate {
            certificate_id: id.into(),
            label: "Service HTTPS".into(),
            expected_revision: revision,
            certificate_chain_pem: SecretValue(certificate_chain_pem.clone()),
            private_key_pem: SecretValue(private_key_pem.clone()),
        };
        let create = request("certificate-create", put(&id, 0));
        let first = execute(&mut store, &owner, &create, &manifest, "boot", &root, now)?;
        assert_eq!(first.state, "completed");
        assert_eq!(first.result["certificate"]["revision"], 1);
        assert_eq!(
            execute(&mut store, &owner, &create, &manifest, "boot", &root, now)?.result,
            first.result
        );
        assert_eq!(crate::certificates::inventory_revision(&store)?, 2);
        let journal: String = store.connection.query_row(
            "SELECT result_json FROM management_operations WHERE operation_id='certificate-create'",
            [],
            |r| r.get(0),
        )?;
        assert!(!journal.contains("PRIVATE KEY") && !journal.contains("BEGIN CERTIFICATE"));
        assert!(!format!("{create:?}").contains("PRIVATE KEY"));
        assert!(
            execute(
                &mut store,
                &owner,
                &request("stale", put(&id, 0)),
                &manifest,
                "boot",
                &root,
                now
            )
            .is_err()
        );
        execute(
            &mut store,
            &owner,
            &request("unbound", put(&unbound, 0)),
            &manifest,
            "boot",
            &root,
            now,
        )?;
        let mut config = placement(&root)?;
        config["tls_certificate_id"] = json!(id);
        store.upsert_placement("api", &config, crate::state::DesiredState::Stopped)?;
        assert!(
            execute(
                &mut store,
                &owner,
                &request(
                    "delete-in-use",
                    ManagementCommand::DeleteCertificate {
                        certificate_id: id.clone(),
                        expected_revision: 1
                    }
                ),
                &manifest,
                "boot",
                &root,
                now
            )
            .is_err()
        );
        let controller = SigningKey::generate();
        let grant = ManagementGrant {
            grant_id: "reader".into(),
            user_id: "reader".into(),
            controller_key: controller.public_key(),
            scope: ManagementScope::Project {
                project_id: "project".into(),
            },
            capabilities: vec![ManagementCapability::Status],
            expires_at: now + 1000,
            group_id: None,
            group_version: None,
        };
        let policy = ManagementPolicy {
            version: 1,
            device_id: manifest.device_id.clone(),
            policy_version: 1,
            previous_policy_digest: None,
            grants: vec![grant.clone()],
            issued_at: now - 1,
            expires_at: now + 1000,
        };
        store.accept_management_policy(
            &sign_management_policy(&policy, &signing)?,
            &signing.public_key(),
            &manifest.device_id,
            now,
        )?;
        let reader = Authority {
            principal: "reader:reader".into(),
            key: controller.public_key(),
            grant: Some(grant),
        };
        let list = request(
            "certificate-list",
            ManagementCommand::Certificates {
                placement_id: None,
                after: None,
                limit: 4,
            },
        );
        let response = execute(&mut store, &reader, &list, &manifest, "boot", &root, now)?;
        assert_eq!(response.result["certificates"].as_array().unwrap().len(), 1);
        assert_eq!(response.result["certificates"][0]["certificate_id"], id);
        assert_eq!(
            response.result["certificates"][0]["bindings"][0]["placement_id"],
            "api"
        );
        assert!(!serde_json::to_string(&response)?.contains("PRIVATE KEY"));
        assert!(
            execute(
                &mut store,
                &reader,
                &request("denied-write", put(&id, 1)),
                &manifest,
                "boot",
                &root,
                now
            )
            .is_err()
        );
        assert!(
            execute(
                &mut store,
                &reader,
                &request(
                    "denied-delete",
                    ManagementCommand::DeleteCertificate {
                        certificate_id: unbound.clone(),
                        expected_revision: 1
                    }
                ),
                &manifest,
                "boot",
                &root,
                now
            )
            .is_err()
        );
        let mut invalid_policy = policy;
        invalid_policy.grants[0]
            .capabilities
            .push(ManagementCapability::ManageCertificates);
        assert!(sign_management_policy(&invalid_policy, &signing).is_err());
        let deleted = execute(
            &mut store,
            &owner,
            &request(
                "delete-unbound",
                ManagementCommand::DeleteCertificate {
                    certificate_id: unbound,
                    expected_revision: 1,
                },
            ),
            &manifest,
            "boot",
            &root,
            now,
        )?;
        assert_eq!(deleted.state, "completed");
        assert_eq!(deleted.result["deleted"], true);
        Ok(())
    }

    #[test]
    fn project_deploy_cannot_acquire_or_change_a_device_certificate_assignment() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let root = crate::supervisor::prepare_state_dir(temp.path())?;
        let mut store = StateStore::open(&root.join("management.sqlite"))?;
        let signing = SigningKey::generate();
        let manifest = manifest(&signing);
        let owner = owner(&manifest);
        let now = unix_time()?;
        let request = |id: &str, command| ManagementRequest {
            operation_id: id.into(),
            device_id: manifest.device_id.clone(),
            issued_at: now,
            expires_at: now + 100,
            command,
        };
        let certificate = rcgen::generate_simple_self_signed(vec!["service.example.test".into()])?;
        let first = uuid::Uuid::new_v4().to_string();
        let other = uuid::Uuid::new_v4().to_string();
        for id in [&first, &other] {
            crate::certificates::put(
                &store,
                &root,
                id,
                "Device identity",
                0,
                &certificate.cert.pem(),
                &certificate.signing_key.serialize_pem(),
                now,
            )?;
        }
        let controller = SigningKey::generate();
        let grant = ManagementGrant {
            grant_id: "deployer".into(),
            user_id: "deployer".into(),
            controller_key: controller.public_key(),
            scope: ManagementScope::Project {
                project_id: "project".into(),
            },
            capabilities: vec![
                ManagementCapability::Status,
                ManagementCapability::Deploy,
                ManagementCapability::Start,
            ],
            expires_at: now + 1000,
            group_id: None,
            group_version: None,
        };
        let policy = ManagementPolicy {
            version: 1,
            device_id: manifest.device_id.clone(),
            policy_version: 1,
            previous_policy_digest: None,
            grants: vec![grant.clone()],
            issued_at: now - 1,
            expires_at: now + 1000,
        };
        store.accept_management_policy(
            &sign_management_policy(&policy, &signing)?,
            &signing.public_key(),
            &manifest.device_id,
            now,
        )?;
        let deployer = Authority {
            principal: "deployer:deployer".into(),
            key: controller.public_key(),
            grant: Some(grant),
        };
        let mut config = placement(&root)?;
        config["tls_certificate_id"] = json!(other);
        let denied = execute(
            &mut store,
            &deployer,
            &request(
                "new-binding",
                ManagementCommand::Apply {
                    config: config.clone(),
                    expected_revision: 0,
                    start: true,
                },
            ),
            &manifest,
            "boot",
            &root,
            now,
        )
        .unwrap_err();
        assert!(denied.to_string().contains("certificate assignment"));
        assert!(store.get_placement("api")?.is_none());
        config["tls_certificate_id"] = Value::Null;
        execute(
            &mut store,
            &deployer,
            &request(
                "plain-deploy",
                ManagementCommand::Apply {
                    config: config.clone(),
                    expected_revision: 0,
                    start: true,
                },
            ),
            &manifest,
            "boot",
            &root,
            now,
        )?;
        config["tls_certificate_id"] = json!(first);
        execute(
            &mut store,
            &owner,
            &request(
                "owner-assign",
                ManagementCommand::Apply {
                    config: config.clone(),
                    expected_revision: 1,
                    start: true,
                },
            ),
            &manifest,
            "boot",
            &root,
            now,
        )?;
        config["variables"]["public-listen-port"] = json!(9090);
        execute(
            &mut store,
            &deployer,
            &request(
                "retain-binding",
                ManagementCommand::Apply {
                    config: config.clone(),
                    expected_revision: 2,
                    start: true,
                },
            ),
            &manifest,
            "boot",
            &root,
            now,
        )?;
        for (label, next) in [("steal", json!(other)), ("remove", Value::Null)] {
            let mut changed = config.clone();
            changed["tls_certificate_id"] = next;
            for (name, command) in [
                (
                    "apply",
                    ManagementCommand::Apply {
                        config: changed.clone(),
                        expected_revision: 3,
                        start: true,
                    },
                ),
                (
                    "stage",
                    ManagementCommand::StageRollout {
                        config: changed,
                        expected_revision: 3,
                        stabilization_seconds: 2,
                        deadline_seconds: 15,
                    },
                ),
            ] {
                let error = execute(
                    &mut store,
                    &deployer,
                    &request(&format!("{label}-{name}"), command),
                    &manifest,
                    "boot",
                    &root,
                    now,
                )
                .unwrap_err();
                assert!(error.to_string().contains("certificate assignment"));
            }
        }
        let retained = execute(
            &mut store,
            &deployer,
            &request(
                "retained-stage",
                ManagementCommand::StageRollout {
                    config: config.clone(),
                    expected_revision: 3,
                    stabilization_seconds: 2,
                    deadline_seconds: 15,
                },
            ),
            &manifest,
            "boot",
            &root,
            now,
        )?;
        assert_eq!(retained.result["state"], "staged");
        execute(
            &mut store,
            &deployer,
            &request(
                "cancel-retained",
                ManagementCommand::CancelRollout {
                    rollout_id: "retained-stage".into(),
                },
            ),
            &manifest,
            "boot",
            &root,
            now,
        )?;
        let inspect = request("deployer-inspect", ManagementCommand::Inspect);
        assert_eq!(
            execute(
                &mut store, &deployer, &inspect, &manifest, "boot", &root, now
            )?
            .result["can_manage_certificates"],
            false
        );
        let inspect = request("owner-inspect", ManagementCommand::Inspect);
        assert_eq!(
            execute(&mut store, &owner, &inspect, &manifest, "boot", &root, now)?.result["can_manage_certificates"],
            true
        );
        assert_eq!(
            store.get_placement("api")?.unwrap().config["tls_certificate_id"],
            first
        );
        let rejected: u64 = store.connection.query_row("SELECT COUNT(*) FROM management_operations WHERE operation_id IN ('new-binding','steal-apply','steal-stage','remove-apply','remove-stage')", [], |r| r.get(0))?;
        assert_eq!(rejected, 0);
        Ok(())
    }

    #[test]
    fn certificate_requests_are_idempotent_and_issuer_delegation_is_owner_only() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let root = crate::supervisor::prepare_state_dir(temp.path())?;
        let mut store = StateStore::open(&root.join("management.sqlite"))?;
        let signing = SigningKey::generate();
        let manifest = manifest(&signing);
        let owner = owner(&manifest);
        let now = unix_time()?;
        let request = |id: &str, command| ManagementRequest {
            operation_id: id.into(),
            device_id: manifest.device_id.clone(),
            issued_at: now,
            expires_at: now + 100,
            command,
        };
        let controller = SigningKey::generate();
        let grant = ManagementGrant {
            grant_id: "certificate-admin".into(),
            user_id: "certificate-admin".into(),
            controller_key: controller.public_key(),
            scope: ManagementScope::Device,
            capabilities: vec![
                ManagementCapability::Status,
                ManagementCapability::ManageCertificates,
            ],
            expires_at: now + 1000,
            group_id: None,
            group_version: None,
        };
        let policy = ManagementPolicy {
            version: 1,
            device_id: manifest.device_id.clone(),
            policy_version: 1,
            previous_policy_digest: None,
            grants: vec![grant.clone()],
            issued_at: now - 1,
            expires_at: now + 1000,
        };
        store.accept_management_policy(
            &sign_management_policy(&policy, &signing)?,
            &signing.public_key(),
            &manifest.device_id,
            now,
        )?;
        let shared = Authority {
            principal: "certificate-admin:certificate-admin".into(),
            key: controller.public_key(),
            grant: Some(grant),
        };
        let id = uuid::Uuid::new_v4().to_string();
        let pending_id = uuid::Uuid::new_v4().to_string();
        let create = request(
            "create-device-csr",
            ManagementCommand::CreateCertificateRequest {
                request_id: pending_id.clone(),
                certificate_id: id.clone(),
                label: "Enterprise REST".into(),
                expected_revision: 0,
                dns_names: vec!["service.example.test".into()],
                ip_addresses: vec![],
            },
        );
        let first = execute(&mut store, &shared, &create, &manifest, "boot", &root, now)?;
        assert_eq!(first.state, "completed");
        assert_eq!(first.result["request"]["purpose"], "service");
        assert_eq!(
            first.result,
            execute(&mut store, &shared, &create, &manifest, "boot", &root, now)?.result
        );
        assert!(!serde_json::to_string(&first)?.contains("PRIVATE KEY"));
        assert_eq!(crate::certificate_requests::list(&store)?.len(), 1);
        let certificate_id = uuid::Uuid::new_v4().to_string();
        let identity = rcgen::generate_simple_self_signed(vec!["service.example.test".into()])?;
        crate::certificates::put(
            &store,
            &root,
            &certificate_id,
            "REST",
            0,
            &identity.cert.pem(),
            &identity.signing_key.serialize_pem(),
            now,
        )?;
        let issuer_request_id = uuid::Uuid::new_v4().to_string();
        let issuer_request = request(
            "create-device-issuer",
            ManagementCommand::CreateCertificateIssuerRequest {
                request_id: issuer_request_id.clone(),
                certificate_id: certificate_id.clone(),
                expected_revision: 1,
                dns_names: vec!["service.example.test".into()],
                ip_addresses: vec![],
                leaf_lifetime_days: 30,
            },
        );
        assert!(
            execute(
                &mut store,
                &shared,
                &issuer_request,
                &manifest,
                "boot",
                &root,
                now
            )
            .is_err()
        );
        let created = execute(
            &mut store,
            &owner,
            &issuer_request,
            &manifest,
            "boot",
            &root,
            now,
        )?;
        assert_eq!(created.result["request"]["purpose"], "issuer");
        let list = request(
            "list-csrs",
            ManagementCommand::CertificateRequests {
                after: None,
                limit: 8,
            },
        );
        assert_eq!(
            execute(&mut store, &shared, &list, &manifest, "boot", &root, now)?.result["requests"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        assert_eq!(
            execute(&mut store, &owner, &list, &manifest, "boot", &root, now)?.result["requests"]
                .as_array()
                .unwrap()
                .len(),
            2
        );
        let issuers = request(
            "list-issuers",
            ManagementCommand::CertificateIssuers {
                after: None,
                limit: 8,
            },
        );
        assert!(execute(&mut store, &shared, &issuers, &manifest, "boot", &root, now).is_err());
        let cancel = request(
            "cancel-issuer-csr",
            ManagementCommand::DeleteCertificateRequest {
                request_id: issuer_request_id,
            },
        );
        assert!(execute(&mut store, &shared, &cancel, &manifest, "boot", &root, now).is_err());
        execute(&mut store, &owner, &cancel, &manifest, "boot", &root, now)?;
        assert!(execute(&mut store, &shared, &cancel, &manifest, "boot", &root, now).is_err());
        let configure = request(
            "configure-public-certificate",
            ManagementCommand::ConfigureAcmeCertificate {
                certificate_id: certificate_id.clone(),
                label: "Public REST".into(),
                expected_revision: 0,
                expected_certificate_revision: 1,
                dns_names: vec!["service.example.test".into()],
                environment: AcmeEnvironment::LetsEncryptStaging,
                http_bind: "127.0.0.1:8080".into(),
                terms_of_service_agreed: true,
            },
        );
        assert!(
            execute(
                &mut store, &shared, &configure, &manifest, "boot", &root, now
            )
            .is_err()
        );
        let configured = execute(
            &mut store, &owner, &configure, &manifest, "boot", &root, now,
        )?;
        assert_eq!(configured.state, "completed");
        assert_eq!(
            configured.result,
            execute(
                &mut store, &owner, &configure, &manifest, "boot", &root, now
            )?
            .result
        );
        let list = request(
            "list-acme",
            ManagementCommand::AcmeCertificates {
                after: None,
                limit: 8,
            },
        );
        assert!(execute(&mut store, &shared, &list, &manifest, "boot", &root, now).is_err());
        assert_eq!(
            execute(&mut store, &owner, &list, &manifest, "boot", &root, now)?.result["policies"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        let revoke = request(
            "stop-acme",
            ManagementCommand::DeleteAcmeCertificate {
                certificate_id: certificate_id.clone(),
                expected_revision: 1,
            },
        );
        assert!(execute(&mut store, &shared, &revoke, &manifest, "boot", &root, now).is_err());
        execute(&mut store, &owner, &revoke, &manifest, "boot", &root, now)?;
        assert_eq!(
            crate::certificates::metadata(&store, &certificate_id)?.revision,
            1
        );
        let journal = store
            .connection
            .prepare("SELECT result_json FROM management_operations")?
            .query_map([], |row| row.get::<_, String>(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        assert!(journal.iter().all(|entry| !entry.contains("PRIVATE KEY")));
        Ok(())
    }

    #[tokio::test]
    async fn owner_created_certificate_and_delegated_renewal_complete_a_trusted_tls_cycle()
    -> Result<()> {
        use flow_like_device_crypto::certificate_authority as ca;
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        async fn handshake(root: &Path, certificate_id: &str, root_pem: &str) -> Result<Vec<u8>> {
            let (_, identity) = crate::certificates::load_certified_key(root, certificate_id)?;
            let mut resolver = rustls::server::ResolvesServerCertUsingSni::new();
            resolver.add("api.example.test", (*identity).clone())?;
            let server = rustls::ServerConfig::builder_with_provider(std::sync::Arc::new(
                crate::crypto::tls_provider(),
            ))
            .with_safe_default_protocol_versions()?
            .with_no_client_auth()
            .with_cert_resolver(std::sync::Arc::new(resolver));
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
            let address = listener.local_addr()?;
            let task = tokio::spawn(async move {
                tokio::time::timeout(std::time::Duration::from_secs(5), async move {
                    let (stream, _) = listener.accept().await?;
                    let acceptor = tokio_rustls::TlsAcceptor::from(std::sync::Arc::new(server));
                    let mut stream = acceptor.accept(stream).await?;
                    let mut request = [0_u8; 4];
                    stream.read_exact(&mut request).await?;
                    ensure!(&request == b"ping", "Unexpected test TLS request");
                    stream.write_all(b"pong").await?;
                    stream.shutdown().await?;
                    Ok::<_, anyhow::Error>(())
                })
                .await?
            });
            let mut trusted = rustls::RootCertStore::empty();
            for cert in rustls_pemfile::certs(&mut std::io::Cursor::new(root_pem.as_bytes())) {
                trusted.add(cert?)?;
            }
            ensure!(
                trusted.len() == 1,
                "TLS test must trust only the exported organisation root"
            );
            let client = rustls::ClientConfig::builder_with_provider(std::sync::Arc::new(
                crate::crypto::tls_provider(),
            ))
            .with_safe_default_protocol_versions()?
            .with_root_certificates(trusted)
            .with_no_client_auth();
            let stream = tokio::net::TcpStream::connect(address).await?;
            let connector = tokio_rustls::TlsConnector::from(std::sync::Arc::new(client));
            let mut stream = connector
                .connect(
                    rustls::pki_types::ServerName::try_from("api.example.test")?,
                    stream,
                )
                .await?;
            let presented = stream
                .get_ref()
                .1
                .peer_certificates()
                .context("Missing TLS certificate")?[0]
                .as_ref()
                .to_vec();
            stream.write_all(b"ping").await?;
            let mut response = [0_u8; 4];
            stream.read_exact(&mut response).await?;
            assert_eq!(&response, b"pong");
            task.await??;
            Ok(presented)
        }

        let temp = tempfile::tempdir()?;
        let root = crate::supervisor::prepare_state_dir(temp.path())?;
        let mut store = StateStore::open(&root.join("management.sqlite"))?;
        let signing = SigningKey::generate();
        let manifest = manifest(&signing);
        let owner = owner(&manifest);
        let now = unix_time()?;
        let command = |command| ManagementRequest {
            operation_id: uuid::Uuid::new_v4().to_string(),
            device_id: manifest.device_id.clone(),
            issued_at: now,
            expires_at: now + 100,
            command,
        };
        let authority_id = uuid::Uuid::new_v4().to_string();
        let password = b"owner-local-password-for-issuance";
        let authority = ca::create_certificate_authority_vault(
            &ca::CertificateAuthoritySpec {
                account_binding: "owner-account".into(),
                authority_id: authority_id.clone(),
                label: "Customer organisation".into(),
                dns_suffixes: vec!["example.test".into()],
                ip_addresses: vec![],
                validity_days: 365,
            },
            password,
            now,
        )?;
        let id = uuid::Uuid::new_v4().to_string();
        let created = execute(
            &mut store,
            &owner,
            &command(ManagementCommand::CreateCertificateRequest {
                request_id: uuid::Uuid::new_v4().to_string(),
                certificate_id: id.clone(),
                label: "Private organisation API".into(),
                expected_revision: 0,
                dns_names: vec!["api.example.test".into()],
                ip_addresses: vec![],
            }),
            &manifest,
            "boot",
            &root,
            now,
        )?;
        let csr: CertificateSigningRequest =
            serde_json::from_value(created.result["request"].clone())?;
        let signed = ca::sign_service_certificate(
            "owner-account",
            &authority_id,
            password,
            &authority.vault,
            &ca::CertificateSigningRequest {
                csr_pem: csr.csr_pem,
                dns_names: csr.dns_names,
                ip_addresses: csr.ip_addresses,
                validity_days: 30,
            },
            now,
        )?;
        let installed = execute(
            &mut store,
            &owner,
            &command(ManagementCommand::InstallCertificateRequest {
                request_id: csr.request_id,
                certificate_chain_pem: SecretValue(signed.certificate_chain_pem),
            }),
            &manifest,
            "boot",
            &root,
            now,
        )?;
        assert_eq!(installed.result["certificate"]["revision"], 1);
        let initial = handshake(&root, &id, &authority.public_bundle.root_certificate_pem).await?;
        let initial_key = crate::certificates::load_identity(&root, &id)?.private_key_pem;
        let created = execute(
            &mut store,
            &owner,
            &command(ManagementCommand::CreateCertificateIssuerRequest {
                request_id: uuid::Uuid::new_v4().to_string(),
                certificate_id: id.clone(),
                expected_revision: 1,
                dns_names: vec!["api.example.test".into()],
                ip_addresses: vec![],
                leaf_lifetime_days: 30,
            }),
            &manifest,
            "boot",
            &root,
            now,
        )?;
        let csr: CertificateSigningRequest =
            serde_json::from_value(created.result["request"].clone())?;
        let signed = ca::sign_device_certificate_issuer(
            "owner-account",
            &authority_id,
            password,
            &authority.vault,
            &ca::CertificateSigningRequest {
                csr_pem: csr.csr_pem,
                dns_names: csr.dns_names,
                ip_addresses: csr.ip_addresses,
                validity_days: 180,
            },
            now,
        )?;
        let installed = execute(
            &mut store,
            &owner,
            &command(ManagementCommand::InstallCertificateIssuer {
                request_id: csr.request_id,
                certificate_chain_pem: SecretValue(signed.certificate_chain_pem),
            }),
            &manifest,
            "boot",
            &root,
            now,
        )?;
        let mut policy: CertificateIssuerMetadata =
            serde_json::from_value(installed.result["issuer"].clone())?;
        policy.next_renewal_at = now;
        store.connection.execute(
            "UPDATE certificate_issuers SET metadata_json=?2 WHERE certificate_id=?1",
            params![id, serde_json::to_string(&policy)?],
        )?;
        assert_eq!(crate::certificate_issuers::renew_due(&root, now)?, 1);
        let renewed = handshake(&root, &id, &authority.public_bundle.root_certificate_pem).await?;
        assert_ne!(initial, renewed);
        assert_eq!(crate::certificates::metadata(&store, &id)?.revision, 2);
        assert_ne!(
            initial_key.0,
            crate::certificates::load_identity(&root, &id)?
                .private_key_pem
                .0
        );
        let journal = store
            .connection
            .prepare("SELECT result_json FROM management_operations")?
            .query_map([], |row| row.get::<_, String>(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        assert!(journal.iter().all(|entry| !entry.contains("PRIVATE KEY")));
        Ok(())
    }
}
