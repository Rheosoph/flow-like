use crate::{
    enrollment::{DeviceSession, unix_time},
    state::StateStore,
    supervisor,
    telemetry::TelemetryStore,
    vault,
};
use anyhow::{Context, Result, ensure};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use flow_like_device_crypto::{
    mls::MemberIdentity,
    mls_store::{
        MlsCheckpoint, MlsPins, ProtectedMlsStore, SqliteSnapshotBackend, SqliteTransactionGuard,
    },
};
use flow_like_device_protocol::*;
use rand_core::{OsRng, RngCore};
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::{Value, json};
use std::{
    fs::File,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};
use tokio_util::sync::CancellationToken;
use zeroize::Zeroizing;

/// The background publisher and owner commands hold an audience lock only for
/// one MLS transition, so contention waits briefly instead of failing.
const AUDIENCE_LOCK_WAIT: Duration = Duration::from_secs(5);

struct Audience {
    state: ProtectedMlsStore<SqliteSnapshotBackend>,
    witness: PathBuf,
    _lock: File,
}

fn storage_key(root: &Path) -> Result<Zeroizing<[u8; 32]>> {
    let path = root.join("mls.key");
    if !path.try_exists()? {
        let mut key = Zeroizing::new([0; 32]);
        OsRng.fill_bytes(key.as_mut());
        if let Err(error) = vault::write_new_private(&path, key.as_ref()) {
            if !path.try_exists()? {
                return Err(error);
            }
        }
    }
    let key = vault::read_private(&path)?;
    ensure!(key.len() == 32, "Invalid MLS storage key");
    Ok(Zeroizing::new(key.as_slice().try_into()?))
}

impl Audience {
    fn open(root: &Path, device: &DeviceSession, scope: &str) -> Result<Self> {
        Self::open_with_guard(root, device, scope, None)
    }

    fn open_with_guard(
        root: &Path,
        device: &DeviceSession,
        scope: &str,
        guard: Option<SqliteTransactionGuard>,
    ) -> Result<Self> {
        validate_management_id(scope)?;
        let digest = compact_digest(scope);
        let witnesses = supervisor::prepare_state_dir(&root.join("crypto-witnesses"))?;
        let lock = supervisor::lock_file_within(
            &witnesses.join(format!("{digest}.lock")),
            AUDIENCE_LOCK_WAIT,
        )
        .context("Telemetry audience is busy with another MLS transition; retry shortly")?;
        let witness = witnesses.join(format!("{digest}.json"));
        let signer = device.telemetry_signer();
        let member = MemberIdentity::new(
            device.manifest().device_id.as_bytes().to_vec(),
            signer.public_key().to_bytes()?,
        )?;
        let pins = MlsPins {
            device_id: device.manifest().device_id.clone(),
            scope: scope.into(),
            owner_invitation_key: device.manifest().owner_invitation_key.clone(),
            publisher: member.clone(),
            local: member,
        };
        let mut backend = SqliteSnapshotBackend::open(&root.join("management.sqlite"), &pins)?;
        if let Some(guard) = guard {
            backend = backend.with_transaction_guard(guard);
        }
        let key = storage_key(root)?;
        let state = if witness.try_exists()? {
            let checkpoint: MlsCheckpoint =
                serde_json::from_slice(&vault::read_private(&witness)?)?;
            ProtectedMlsStore::open(backend, pins, *key, checkpoint)?
        } else {
            ProtectedMlsStore::create(backend, pins, *key, &signer)?
        };
        let audience = Self {
            state,
            witness,
            _lock: lock,
        };
        audience.persist_witness()?;
        Ok(audience)
    }

    fn persist_witness(&self) -> Result<()> {
        let staging = self
            .witness
            .with_extension(format!("{}.tmp", uuid::Uuid::new_v4()));
        vault::write_new_private(&staging, &serde_json::to_vec(&self.state.checkpoint())?)?;
        if let Err(error) = std::fs::rename(&staging, &self.witness) {
            let _ = std::fs::remove_file(&staging);
            return Err(error.into());
        }
        File::open(self.witness.parent().context("Witness parent missing")?)?.sync_all()?;
        Ok(())
    }
}

fn roster_guard(
    device: &OnboardingManifest,
    compact: &str,
    require_published: bool,
) -> SqliteTransactionGuard {
    let device = device.clone();
    let compact = compact.to_owned();
    Box::new(move |connection| {
        let now = unix_time()?;
        let roster = verify_telemetry_roster(&compact, &device.owner_invitation_key, now)?;
        ensure!(
            roster.device_id == device.device_id,
            "Telemetry roster device mismatch"
        );
        let head: Option<(String, String)> = connection
            .query_row(
                "SELECT policy_digest,policy_jws FROM management_policy WHERE singleton=1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        ensure!(
            head.as_ref().map(|row| &row.0) == roster.management_policy_digest.as_ref(),
            "Telemetry membership requires renewal after management changes"
        );
        if let Some((_, policy)) = head {
            let policy = verify_management_policy(&policy, &device.owner_invitation_key, now)?;
            ensure!(
                policy.device_id == device.device_id && roster.expires_at <= policy.expires_at,
                "Telemetry policy outlives current management access"
            );
        }
        if require_published {
            ensure!(
                accepted_roster(connection, &roster.scope)?.as_deref() == Some(compact.as_str()),
                "Telemetry roster changed before publication"
            );
        }
        Ok(())
    })
}

fn accepted_roster(connection: &Connection, scope: &str) -> Result<Option<String>> {
    Ok(connection
        .query_row(
            "SELECT policy_jws FROM telemetry_audiences WHERE scope=?1",
            [scope],
            |row| row.get(0),
        )
        .optional()?)
}

fn store_roster(connection: &Connection, scope: &str, compact: &str) -> Result<()> {
    connection.execute(
        "INSERT INTO telemetry_audiences(scope,policy_jws) VALUES(?1,?2) ON CONFLICT(scope) DO UPDATE SET policy_jws=excluded.policy_jws",
        params![scope, compact],
    )?;
    Ok(())
}

/// Rows written before membership commits and roster rows became atomic may
/// lag the MLS state. The snapshot's accepted policy is authoritative, and the
/// returned roster is the one reads serve and publication fences on.
fn reconcile_roster(
    connection: &Connection,
    scope: &str,
    audience: &mut Audience,
) -> Result<Option<String>> {
    let Some(accepted) = audience.state.accepted_policy()? else {
        return accepted_roster(connection, scope);
    };
    if accepted_roster(connection, scope)?.as_deref() != Some(accepted.as_str()) {
        store_roster(connection, scope, &accepted)?;
    }
    Ok(Some(accepted))
}

fn has_witness(root: &Path, scope: &str) -> Result<bool> {
    Ok(root
        .join("crypto-witnesses")
        .join(format!("{}.json", compact_digest(scope)))
        .try_exists()?)
}

/// Agents older than atomic roster acceptance may have left a roster row behind
/// its snapshot, or none at all after genesis, so every scope that has MLS state
/// is healed at startup instead of waiting for a failed owner renewal.
fn reconcile_audiences(root: &Path, device: &DeviceSession) -> Result<()> {
    let store = StateStore::open(&root.join("management.sqlite"))?;
    let scopes: Vec<String> = store
        .connection
        .prepare(
            "SELECT 'device' UNION SELECT scope FROM telemetry_audiences UNION SELECT id FROM placement_identities",
        )?
        .query_map([], |row| row.get(0))?
        .collect::<Result<_, _>>()?;
    for scope in scopes {
        if !has_witness(root, &scope)? {
            continue;
        }
        if let Err(error) = Audience::open(root, device, &scope)
            .and_then(|mut audience| reconcile_roster(&store.connection, &scope, &mut audience))
        {
            tracing::warn!(
                scope = %scope,
                "Telemetry roster reconciliation failed; publication retries it under the audience lock: {error:#}"
            );
        }
    }
    Ok(())
}

/// Runs inside the membership commit, so the roster that reads return and
/// publication fences on never diverges from the MLS state. A retried older
/// request leaves the newer accepted roster and its reader bindings in place.
fn accept_roster(
    connection: &Connection,
    device: &OnboardingManifest,
    roster: &TelemetryRoster,
    compact: &str,
) -> Result<()> {
    if let Some(accepted) = accepted_roster(connection, &roster.scope)? {
        let settled = accepted == compact
            || verify_historical_telemetry_roster(&accepted, &device.owner_invitation_key)
                .is_ok_and(|accepted| accepted.policy_version > roster.policy_version);
        if settled {
            return Ok(());
        }
    }
    ensure_readers_granted(connection, device, roster, unix_time()?)?;
    let endpoints: Vec<&str> = roster
        .members
        .iter()
        .map(|member| member.endpoint_id.as_str())
        .collect();
    connection.execute(
        "DELETE FROM telemetry_reader_bindings WHERE scope=?1 AND endpoint_id NOT IN (SELECT value FROM json_each(?2))",
        params![roster.scope, serde_json::to_string(&endpoints)?],
    )?;
    store_roster(connection, &roster.scope, compact)
}

fn reader_bindings(connection: &Connection) -> Result<()> {
    connection.execute_batch(
        "CREATE TABLE IF NOT EXISTS telemetry_reader_bindings (
            scope TEXT NOT NULL, endpoint_id TEXT NOT NULL, controller_key TEXT NOT NULL,
            PRIMARY KEY(scope, endpoint_id)
        )",
    )?;
    Ok(())
}

fn bind_reader(connection: &Connection, scope: &str, endpoint_id: &str, relay: &str) -> Result<()> {
    reader_bindings(connection)?;
    connection.execute(
        "INSERT INTO telemetry_reader_bindings(scope,endpoint_id,controller_key) VALUES(?1,?2,?3) ON CONFLICT(scope,endpoint_id) DO UPDATE SET controller_key=excluded.controller_key",
        params![scope, endpoint_id, relay],
    )?;
    Ok(())
}

/// The single scope rule for management grants, shared by management
/// authorization and the telemetry roster check so they cannot drift apart.
pub(crate) fn scope_covers(
    scope: &ManagementScope,
    project: Option<&str>,
    placement: Option<&str>,
) -> bool {
    match scope {
        ManagementScope::Device => true,
        ManagementScope::Project { project_id } => project == Some(project_id.as_str()),
        ManagementScope::Placement {
            project_id,
            placement_id,
        } => project == Some(project_id.as_str()) && placement == Some(placement_id.as_str()),
    }
}

fn confirmed_readers(connection: &Connection, scope: &str) -> Result<Vec<String>> {
    reader_bindings(connection)?;
    Ok(connection
        .prepare(
            "SELECT endpoint_id FROM telemetry_reader_bindings WHERE scope=?1 ORDER BY endpoint_id",
        )?
        .query_map([scope], |row| row.get(0))?
        .collect::<Result<_, _>>()?)
}

/// Contract C6. A reader is bound to the controller key of the management
/// principal that relayed its delivery receipts. A roster change keeps a bound
/// reader only while that key is the owner's or holds a current Metrics grant
/// covering this scope, so after a revocation the owner must drop the reader.
/// A reader that never confirmed a delivery has no binding and stays under the
/// owner's explicit approval; roster reads list the confirmed readers, so the
/// owner can recognise unconfirmed ones and drop them.
fn ensure_readers_granted(
    connection: &Connection,
    device: &OnboardingManifest,
    roster: &TelemetryRoster,
    now: i64,
) -> Result<()> {
    reader_bindings(connection)?;
    let placement = (roster.scope != "device").then_some(roster.scope.as_str());
    let project: Option<String> = match placement {
        Some(id) => connection
            .query_row(
                "SELECT json_extract(config_json,'$.project_id') FROM placements WHERE id=?1",
                [id],
                |row| row.get::<_, Option<String>>(0),
            )
            .optional()?
            .flatten(),
        None => None,
    };
    let policy: Option<String> = connection
        .query_row(
            "SELECT policy_jws FROM management_policy WHERE singleton=1",
            [],
            |row| row.get(0),
        )
        .optional()?;
    let grants = match policy {
        Some(compact) => {
            verify_management_policy(&compact, &device.owner_invitation_key, now)?.grants
        }
        None => Vec::new(),
    };
    let mut granted = vec![device.controller_key.thumbprint()?];
    for grant in grants.iter().filter(|grant| {
        grant.expires_at > now
            && grant.capabilities.contains(&ManagementCapability::Metrics)
            && scope_covers(&grant.scope, project.as_deref(), placement)
    }) {
        granted.push(grant.controller_key.thumbprint()?);
    }
    let mut revoked = Vec::new();
    for reader in roster
        .members
        .iter()
        .filter(|member| **member != roster.publisher)
    {
        let relay: Option<String> = connection
            .query_row(
                "SELECT controller_key FROM telemetry_reader_bindings WHERE scope=?1 AND endpoint_id=?2",
                params![roster.scope, reader.endpoint_id],
                |row| row.get(0),
            )
            .optional()?;
        if relay.is_some_and(|relay| !granted.contains(&relay)) {
            revoked.push(reader.endpoint_id.as_str());
        }
    }
    if revoked.is_empty() {
        return Ok(());
    }
    Err(anyhow::Error::new(ProtocolError::Invalid(
        "telemetry reader without a current Metrics grant",
    ))
    .context(format!(
        "Telemetry readers {} no longer hold a Metrics grant for scope {}; remove them from the roster",
        revoked.join(", "),
        roster.scope
    )))
}

pub fn apply_policy(
    root: &Path,
    device: &DeviceSession,
    scope: &str,
    request_id: &str,
    sequence: u64,
    compact: &str,
    packages: &[TelemetryKeyPackage],
) -> Result<Value> {
    let now = unix_time()?;
    let policy = verify_telemetry_roster(compact, &device.manifest().owner_invitation_key, now)?;
    ensure!(
        policy.device_id == device.manifest().device_id && policy.scope == scope,
        "Telemetry policy audience mismatch"
    );
    let store = StateStore::open(&root.join("management.sqlite"))?;
    ensure!(
        store.management_policy_head()?.map(|(_, digest)| digest)
            == policy.management_policy_digest,
        "Telemetry roster requires the current management policy"
    );
    if let Some(current) = store.management_policy(&device.manifest().owner_invitation_key, now)? {
        ensure!(
            policy.expires_at <= current.expires_at,
            "Telemetry policy outlives management access"
        );
    }
    let packages: Vec<_> = packages
        .iter()
        .map(|package| {
            Ok((
                MemberIdentity::new(
                    package.member.endpoint_id.as_bytes().to_vec(),
                    package.member.signing_key.to_bytes()?,
                )?,
                URL_SAFE_NO_PAD.decode(&package.key_package)?,
            ))
        })
        .collect::<Result<_>>()?;
    let mut audience = Audience::open_with_guard(
        root,
        device,
        scope,
        Some(roster_guard(device.manifest(), compact, false)),
    )?;
    reconcile_roster(&store.connection, scope, &mut audience)?;
    let (manifest, roster, accepted) = (
        device.manifest().clone(),
        policy.clone(),
        compact.to_owned(),
    );
    audience
        .state
        .before_next_commit(Box::new(move |connection| {
            accept_roster(connection, &manifest, &roster, &accepted)
        }));
    let publication = audience
        .state
        .apply_policy(request_id, sequence, compact, &packages, now)?;
    audience.persist_witness()?;
    Ok(
        json!({"sequence":publication.sequence,"policy_version":policy.policy_version,"policy_digest":compact_digest(compact),"welcome_available":publication.welcome.is_some()}),
    )
}

pub fn read(
    root: &Path,
    device: &DeviceSession,
    scope: &str,
    sequence: u64,
    welcome: bool,
    offset: u32,
    limit: u32,
    guard: SqliteTransactionGuard,
) -> Result<Value> {
    ensure!(limit > 0 && limit <= 4096, "Invalid MLS chunk length");
    let mut audience = Audience::open_with_guard(root, device, scope, Some(guard))?;
    let publications = audience.state.pending_outbox()?;
    let latest = audience.state.publication_position()?.0;
    let publication = publications.into_iter().find(|p| {
        if sequence == 0 {
            true
        } else {
            p.sequence == sequence
        }
    });
    let Some(publication) = publication else {
        return Ok(json!({"available":false,"latest":latest}));
    };
    let delivery = if welcome {
        publication
            .welcome
            .context("This publication has no Welcome")?
    } else {
        publication.message
    };
    let bytes = serde_json::to_vec(&delivery)?;
    ensure!(offset as usize <= bytes.len(), "Invalid MLS chunk offset");
    let end = (offset as usize + limit as usize).min(bytes.len());
    audience.persist_witness()?;
    Ok(
        json!({"available":true,"sequence":publication.sequence,"latest":latest,"offset":offset,"total":bytes.len(),"digest":compact_digest(std::str::from_utf8(&bytes)?),"chunk":URL_SAFE_NO_PAD.encode(&bytes[offset as usize..end])}),
    )
}

pub fn read_roster(
    root: &Path,
    scope: &str,
    offset: u32,
    limit: u32,
    guard: SqliteTransactionGuard,
) -> Result<Value> {
    validate_management_id(scope)?;
    ensure!(limit > 0 && limit <= 4096, "Invalid roster chunk length");
    let store = StateStore::open(&root.join("management.sqlite"))?;
    let transaction = rusqlite::Transaction::new_unchecked(
        &store.connection,
        rusqlite::TransactionBehavior::Immediate,
    )?;
    guard(&transaction)?;
    let Some(compact) = accepted_roster(&transaction, scope)? else {
        guard(&transaction)?;
        transaction.commit()?;
        return Ok(json!({"available":false}));
    };
    let bytes = compact.as_bytes();
    ensure!(
        offset as usize <= bytes.len(),
        "Invalid roster chunk offset"
    );
    let end = (offset as usize + limit as usize).min(bytes.len());
    let mut result = json!({"available":true,"offset":offset,"total":bytes.len(),"digest":compact_digest(&compact),"chunk":URL_SAFE_NO_PAD.encode(&bytes[offset as usize..end])});
    if offset == 0 {
        result["confirmed_readers"] = json!(confirmed_readers(&transaction, scope)?);
    }
    guard(&transaction)?;
    transaction.commit()?;
    Ok(result)
}

pub fn acknowledge(
    root: &Path,
    device: &DeviceSession,
    scope: &str,
    sequence: u64,
    digest: &str,
) -> Result<()> {
    let mut audience = Audience::open(root, device, scope)?;
    audience.state.acknowledge(sequence, digest)?;
    audience.persist_witness()
}

pub fn receive_receipt(
    root: &Path,
    device: &DeviceSession,
    scope: &str,
    endpoint_id: &str,
    sequence: u64,
    receipt_jws: &str,
    guard: SqliteTransactionGuard,
) -> Result<Value> {
    accept_receipt(
        root,
        device,
        scope,
        endpoint_id,
        sequence,
        receipt_jws,
        None,
        guard,
    )
}

/// `relay` is the controller key of the management principal that submitted
/// the receipt. It binds the reader to that principal, so a renewed roster may
/// keep the reader only while the principal still holds a Metrics grant.
#[allow(clippy::too_many_arguments)]
pub fn receive_relayed_receipt(
    root: &Path,
    device: &DeviceSession,
    scope: &str,
    endpoint_id: &str,
    sequence: u64,
    receipt_jws: &str,
    relay: &Ed25519PublicKey,
    guard: SqliteTransactionGuard,
) -> Result<Value> {
    accept_receipt(
        root,
        device,
        scope,
        endpoint_id,
        sequence,
        receipt_jws,
        Some(relay),
        guard,
    )
}

#[allow(clippy::too_many_arguments)]
fn accept_receipt(
    root: &Path,
    device: &DeviceSession,
    scope: &str,
    endpoint_id: &str,
    sequence: u64,
    receipt_jws: &str,
    relay: Option<&Ed25519PublicKey>,
    guard: SqliteTransactionGuard,
) -> Result<Value> {
    let mut audience = Audience::open_with_guard(root, device, scope, Some(guard))?;
    if let Some(relay) = relay {
        let (scope, endpoint, relay) = (
            scope.to_owned(),
            endpoint_id.to_owned(),
            relay.thumbprint()?,
        );
        audience
            .state
            .before_next_commit(Box::new(move |connection| {
                bind_reader(connection, &scope, &endpoint, &relay)
            }));
    }
    let acknowledged =
        audience
            .state
            .accept_delivery_receipt(endpoint_id, sequence, receipt_jws)?;
    audience.persist_witness()?;
    Ok(json!({"sequence":sequence,"endpoint_id":endpoint_id,"acknowledged_through":acknowledged}))
}

pub async fn publish_live(
    root: PathBuf,
    device: Arc<DeviceSession>,
    cancel: CancellationToken,
) -> Result<()> {
    let (startup_root, startup_device) = (root.clone(), device.clone());
    let reconciled =
        tokio::task::spawn_blocking(move || reconcile_audiences(&startup_root, &startup_device))
            .await
            .context("Telemetry roster reconciliation panicked")
            .and_then(|result| result);
    if let Err(error) = reconciled {
        tracing::warn!(
            "Telemetry roster reconciliation failed; publication retries each audience: {error:#}"
        );
    }
    let mut failures = 0u32;
    loop {
        let delay = 5u64.saturating_mul(1 << failures.min(6)).min(300);
        tokio::select! {_=cancel.cancelled()=>return Ok(()),_=tokio::time::sleep(Duration::from_secs(delay))=>()}
        let (pass_root, pass_device) = (root.clone(), device.clone());
        let pass = tokio::task::spawn_blocking(move || publish_audiences(&pass_root, &pass_device))
            .await
            .context("Encrypted telemetry publication pass panicked")
            .and_then(|result| result);
        match pass {
            Ok(()) => failures = 0,
            Err(error) => {
                failures = failures.saturating_add(1);
                tracing::warn!(
                    failures,
                    "Encrypted telemetry publication pass failed; retrying with backoff: {error:#}"
                );
            }
        }
    }
}

fn publish_audiences(root: &Path, device: &DeviceSession) -> Result<()> {
    let store = StateStore::open(&root.join("management.sqlite"))?;
    let audiences: Vec<(String, String)> = store
        .connection
        .prepare("SELECT scope,policy_jws FROM telemetry_audiences")?
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<Result<_, _>>()?;
    for (scope, compact) in audiences {
        if let Err(error) = publish_audience(root, device, &store, &scope, &compact) {
            tracing::debug!(
                scope = %scope,
                "MLS audience publication paused pending policy or outbox recovery: {error:#}"
            );
        }
    }
    Ok(())
}

/// The publication commits only while `compact` is still the accepted roster
/// and current management access still covers it.
fn fence_publication(audience: &mut Audience, device: &OnboardingManifest, compact: &str) {
    let guard = roster_guard(device, compact, true);
    audience
        .state
        .before_next_commit(Box::new(move |connection| guard(connection)));
}

/// `listed` gates cheaply before any MLS state is opened, so an expired roster
/// costs no writes. Under the audience lock the row is reconciled from the
/// snapshot, and the publication fences on that accepted roster.
fn publish_audience(
    root: &Path,
    device: &DeviceSession,
    store: &StateStore,
    scope: &str,
    listed: &str,
) -> Result<()> {
    let policy = verify_telemetry_roster(
        listed,
        &device.manifest().owner_invitation_key,
        unix_time()?,
    )?;
    ensure!(
        store.management_policy_head()?.map(|(_, digest)| digest)
            == policy.management_policy_digest,
        "Telemetry membership requires renewal after management changes"
    );
    let _ = store.management_policy(&device.manifest().owner_invitation_key, unix_time()?)?;
    let telemetry = TelemetryStore::open(root)?;
    let placement = if scope == "device" { None } else { Some(scope) };
    let data = telemetry.latest_metrics(placement)?;
    if data["records"]
        .as_array()
        .is_none_or(|records| records.is_empty())
    {
        return Ok(());
    }
    let mut audience = Audience::open(root, device, scope)?;
    let accepted =
        reconcile_roster(&store.connection, scope, &mut audience)?.with_context(|| {
            format!("Telemetry roster for scope {scope} disappeared before publication")
        })?;
    let (sequence, _) = audience.state.publication_position()?;
    fence_publication(&mut audience, device.manifest(), &accepted);
    let request_id = uuid::Uuid::new_v4().to_string();
    audience.state.publish(
        &request_id,
        sequence.checked_add(1).context("MLS sequence exhausted")?,
        &serde_json::to_vec(&json!({"type":"metrics","scope":scope,"sample":data}))?,
        unix_time()?,
    )?;
    audience.persist_witness()
}

#[cfg(test)]
mod tests {
    use super::*;
    use flow_like_device_crypto::prepared_mls::PreparedMlsEndpoint;

    fn session(owner: &SigningKey) -> DeviceSession {
        DeviceSession::test_session(
            "https://example.test/api/v1".into(),
            "device".into(),
            SigningKey::generate(),
        )
        .test_with_invitation_key(owner.public_key())
    }

    fn roster(
        device: &DeviceSession,
        version: u64,
        previous: Option<&str>,
        management: &str,
        readers: &[TelemetryMember],
        now: i64,
    ) -> TelemetryRoster {
        let publisher = TelemetryMember {
            endpoint_id: "device".into(),
            signing_key: device.telemetry_signer().public_key(),
        };
        let mut members = vec![publisher.clone()];
        members.extend_from_slice(readers);
        TelemetryRoster {
            version: 1,
            device_id: "device".into(),
            scope: "device".into(),
            policy_version: version,
            previous_policy_digest: previous.map(compact_digest),
            management_policy_digest: Some(compact_digest(management)),
            publisher,
            members,
            issued_at: now,
            expires_at: now + 300,
        }
    }

    fn management(
        owner: &SigningKey,
        version: u64,
        previous: Option<&str>,
        grants: Vec<ManagementGrant>,
        now: i64,
    ) -> Result<String> {
        Ok(sign_management_policy(
            &ManagementPolicy {
                version: 1,
                device_id: "device".into(),
                policy_version: version,
                previous_policy_digest: previous.map(compact_digest),
                grants,
                issued_at: now,
                expires_at: now + 600,
            },
            owner,
        )?)
    }

    #[test]
    fn audience_waits_for_a_brief_lock_holder_instead_of_failing() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let root = directory.path().canonicalize()?;
        let device = session(&SigningKey::generate());
        StateStore::open(&root.join("management.sqlite"))?;
        let held = Audience::open(&root, &device, "device")?;
        let release = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(200));
            drop(held);
        });
        let started = std::time::Instant::now();
        let mut audience = Audience::open(&root, &device, "device")?;
        assert!(started.elapsed() >= Duration::from_millis(150));
        assert_eq!(audience.state.publication_position()?, (0, 0));
        release
            .join()
            .map_err(|_| anyhow::anyhow!("Lock holder thread panicked"))?;
        Ok(())
    }

    #[test]
    fn lost_genesis_witness_is_rebuilt_instead_of_wedging_the_scope() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let root = directory.path().canonicalize()?;
        let device = session(&SigningKey::generate());
        StateStore::open(&root.join("management.sqlite"))?;
        let witness = Audience::open(&root, &device, "device")?.witness;
        std::fs::remove_file(&witness)?;
        let mut audience = Audience::open(&root, &device, "device")?;
        assert!(witness.try_exists()?);
        assert_eq!(audience.state.publication_position()?, (0, 0));
        Ok(())
    }

    #[test]
    fn a_lagging_roster_row_is_reconciled_from_the_accepted_mls_policy() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let root = directory.path().canonicalize()?;
        let owner = SigningKey::generate();
        let device = session(&owner);
        let store = StateStore::open(&root.join("management.sqlite"))?;
        let now = unix_time()?;
        let policy = management(&owner, 1, None, vec![], now)?;
        store.accept_management_policy(&policy, &owner.public_key(), "device", now)?;
        let genesis = sign_telemetry_roster(&roster(&device, 1, None, &policy, &[], now), &owner)?;
        apply_policy(&root, &device, "device", "genesis", 1, &genesis, &[])?;
        store
            .connection
            .execute("DELETE FROM telemetry_audiences", [])?;
        let forked =
            sign_telemetry_roster(&roster(&device, 1, None, &policy, &[], now + 1), &owner)?;
        assert!(apply_policy(&root, &device, "device", "forked", 0, &forked, &[]).is_err());
        assert_eq!(
            accepted_roster(&store.connection, "device")?.as_deref(),
            Some(genesis.as_str())
        );
        Ok(())
    }

    fn renewed_publisher_only_group(
        root: &Path,
        owner: &SigningKey,
        device: &DeviceSession,
        store: &StateStore,
    ) -> Result<(String, String)> {
        let now = unix_time()?;
        let policy = management(owner, 1, None, vec![], now)?;
        store.accept_management_policy(&policy, &owner.public_key(), "device", now)?;
        let genesis = sign_telemetry_roster(&roster(device, 1, None, &policy, &[], now), owner)?;
        apply_policy(root, device, "device", "genesis", 1, &genesis, &[])?;
        let renewed =
            sign_telemetry_roster(&roster(device, 2, Some(&genesis), &policy, &[], now), owner)?;
        apply_policy(root, device, "device", "renewed", 0, &renewed, &[])?;
        Ok((genesis, renewed))
    }

    #[test]
    fn startup_heals_lagging_and_missing_roster_rows_from_the_snapshot() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let root = directory.path().canonicalize()?;
        let owner = SigningKey::generate();
        let device = session(&owner);
        let store = StateStore::open(&root.join("management.sqlite"))?;
        let (genesis, renewed) = renewed_publisher_only_group(&root, &owner, &device, &store)?;
        store.connection.execute(
            "UPDATE telemetry_audiences SET policy_jws=?1 WHERE scope='device'",
            [&genesis],
        )?;
        reconcile_audiences(&root, &device)?;
        assert_eq!(
            accepted_roster(&store.connection, "device")?.as_deref(),
            Some(renewed.as_str())
        );
        store
            .connection
            .execute("DELETE FROM telemetry_audiences", [])?;
        reconcile_audiences(&root, &device)?;
        assert_eq!(
            accepted_roster(&store.connection, "device")?.as_deref(),
            Some(renewed.as_str())
        );
        Ok(())
    }

    #[test]
    fn publication_heals_a_lagging_roster_row_and_fences_on_the_accepted_roster() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let root = directory.path().canonicalize()?;
        let owner = SigningKey::generate();
        let device = session(&owner);
        let store = StateStore::open(&root.join("management.sqlite"))?;
        let (genesis, renewed) = renewed_publisher_only_group(&root, &owner, &device, &store)?;
        store.connection.execute(
            "UPDATE telemetry_audiences SET policy_jws=?1 WHERE scope='device'",
            [&genesis],
        )?;
        TelemetryStore::open(&root)?.append(None, "metrics", &json!({"cpu":0.5}))?;
        publish_audiences(&root, &device)?;
        assert_eq!(
            accepted_roster(&store.connection, "device")?.as_deref(),
            Some(renewed.as_str())
        );
        let mut audience = Audience::open(&root, &device, "device")?;
        assert_eq!(audience.state.publication_position()?.0, 3);
        Ok(())
    }

    #[test]
    fn renewal_refuses_readers_whose_relaying_grant_was_revoked() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let root = directory.path().canonicalize()?;
        let owner = SigningKey::generate();
        let device = session(&owner);
        let store = StateStore::open(&root.join("management.sqlite"))?;
        let now = unix_time()?;
        let grantee = SigningKey::generate();
        let granted = management(
            &owner,
            1,
            None,
            vec![ManagementGrant {
                grant_id: "metrics".into(),
                user_id: "reader-user".into(),
                controller_key: grantee.public_key(),
                scope: ManagementScope::Device,
                capabilities: vec![ManagementCapability::Metrics],
                expires_at: now + 600,
                group_id: None,
                group_version: None,
            }],
            now,
        )?;
        store.accept_management_policy(&granted, &owner.public_key(), "device", now)?;
        let reader_key = SigningKey::generate();
        let reader = TelemetryMember {
            endpoint_id: "reader-endpoint".into(),
            signing_key: reader_key.public_key(),
        };
        let mut endpoint = PreparedMlsEndpoint::create(
            MlsPins {
                device_id: "device".into(),
                scope: "device".into(),
                owner_invitation_key: owner.public_key(),
                publisher: MemberIdentity::new(
                    b"device".to_vec(),
                    device.telemetry_signer().public_key().to_bytes()?,
                )?,
                local: MemberIdentity::new(
                    reader.endpoint_id.as_bytes().to_vec(),
                    reader_key.public_key().to_bytes()?,
                )?,
            },
            [7; 32],
            &reader_key,
        )?;
        endpoint.confirm_commit(&endpoint.prepared()?.checkpoint)?;
        endpoint.prepare_key_package()?;
        let output: Value =
            serde_json::from_slice(&endpoint.confirm_commit(&endpoint.prepared()?.checkpoint)?)?;
        let package = TelemetryKeyPackage {
            member: reader.clone(),
            key_package: output["wire"]
                .as_str()
                .context("Missing reader key package")?
                .into(),
        };
        let readers = std::slice::from_ref(&reader);
        let genesis =
            sign_telemetry_roster(&roster(&device, 1, None, &granted, readers, now), &owner)?;
        apply_policy(&root, &device, "device", "genesis", 1, &genesis, &[package])?;
        assert_eq!(
            accepted_roster(&store.connection, "device")?.as_deref(),
            Some(genesis.as_str())
        );
        let welcome = Audience::open(&root, &device, "device")?
            .state
            .pending_outbox()?
            .remove(0)
            .welcome
            .context("Genesis admitted no reader")?;
        endpoint.prepare_join(&welcome, now)?;
        endpoint.confirm_commit(&endpoint.prepared()?.checkpoint)?;
        let (receipt, signed) = endpoint.delivery_receipts()?.remove(0);
        receive_relayed_receipt(
            &root,
            &device,
            "device",
            &receipt.endpoint_id,
            receipt.sequence,
            &signed,
            &grantee.public_key(),
            Box::new(|_: &Connection| -> Result<()> { Ok(()) }),
        )?;
        let first = read_roster(
            &root,
            "device",
            0,
            4096,
            Box::new(|_: &Connection| -> Result<()> { Ok(()) }),
        )?;
        assert_eq!(first["confirmed_readers"], json!(["reader-endpoint"]));
        let revoked = management(&owner, 2, Some(&granted), vec![], now)?;
        store.accept_management_policy(&revoked, &owner.public_key(), "device", now)?;
        let carried = sign_telemetry_roster(
            &roster(&device, 2, Some(&genesis), &revoked, readers, now),
            &owner,
        )?;
        let error = apply_policy(&root, &device, "device", "carried", 0, &carried, &[])
            .err()
            .context("A reader without a current grant stayed in the roster")?;
        assert!(format!("{error:#}").contains("reader-endpoint"));
        assert!(error.chain().any(|cause| cause.is::<ProtocolError>()));
        assert_eq!(
            accepted_roster(&store.connection, "device")?.as_deref(),
            Some(genesis.as_str())
        );
        let renewed = sign_telemetry_roster(
            &roster(&device, 2, Some(&genesis), &revoked, &[], now),
            &owner,
        )?;
        let result = apply_policy(&root, &device, "device", "renewed", 0, &renewed, &[])?;
        assert_eq!(result["sequence"], 2);
        assert_eq!(
            accepted_roster(&store.connection, "device")?.as_deref(),
            Some(renewed.as_str())
        );
        let bindings: i64 = store.connection.query_row(
            "SELECT COUNT(*) FROM telemetry_reader_bindings",
            [],
            |row| row.get(0),
        )?;
        assert_eq!(bindings, 0);
        Ok(())
    }

    #[test]
    fn publication_rechecks_management_revocation_after_audience_was_opened() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let root = directory.path().canonicalize()?;
        let owner = SigningKey::generate();
        let device = DeviceSession::test_session(
            "https://example.test/api/v1".into(),
            "device".into(),
            SigningKey::generate(),
        )
        .test_with_invitation_key(owner.public_key());
        let store = StateStore::open(&root.join("management.sqlite"))?;
        let now = unix_time()?;
        let publisher = TelemetryMember {
            endpoint_id: "device".into(),
            signing_key: device.telemetry_signer().public_key(),
        };
        let roster = TelemetryRoster {
            version: 1,
            device_id: "device".into(),
            scope: "device".into(),
            policy_version: 1,
            previous_policy_digest: None,
            management_policy_digest: None,
            publisher: publisher.clone(),
            members: vec![publisher],
            issued_at: now,
            expires_at: now + 300,
        };
        let compact = sign_telemetry_roster(&roster, &owner)?;
        apply_policy(&root, &device, "device", "genesis", 1, &compact, &[])?;
        let mut audience = Audience::open(&root, &device, "device")?;
        fence_publication(&mut audience, device.manifest(), &compact);
        let checkpoint = audience.state.checkpoint();
        let policy = ManagementPolicy {
            version: 1,
            device_id: "device".into(),
            policy_version: 1,
            previous_policy_digest: None,
            grants: vec![],
            issued_at: now,
            expires_at: now + 600,
        };
        store.accept_management_policy(
            &sign_management_policy(&policy, &owner)?,
            &owner.public_key(),
            "device",
            now,
        )?;
        let error = audience
            .state
            .publish("stale-publisher", 2, b"post-removal sample", now)
            .err()
            .context("Revoked publisher emitted a message")?;
        assert!(
            error
                .to_string()
                .contains("renewal after management changes")
        );
        assert_eq!(audience.state.checkpoint(), checkpoint);
        drop(audience);
        let mut reopened = Audience::open(&root, &device, "device")?;
        assert_eq!(reopened.state.publication_position()?.0, 1);
        assert_eq!(reopened.state.pending_outbox()?.len(), 1);
        Ok(())
    }
}
