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
use rusqlite::{OptionalExtension, params};
use serde_json::{Value, json};
use std::{
    fs::File,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};
use tokio_util::sync::CancellationToken;
use zeroize::Zeroizing;

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
        let lock = supervisor::lock_file(&witnesses.join(format!("{digest}.lock")))?;
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
            let accepted: String = connection.query_row(
                "SELECT policy_jws FROM telemetry_audiences WHERE scope=?1",
                [&roster.scope],
                |row| row.get(0),
            )?;
            ensure!(
                accepted == compact,
                "Telemetry roster changed before publication"
            );
        }
        Ok(())
    })
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
    let publication = audience
        .state
        .apply_policy(request_id, sequence, compact, &packages, now)?;
    audience.persist_witness()?;
    let store = StateStore::open(&root.join("management.sqlite"))?;
    store.connection.execute("INSERT INTO telemetry_audiences(scope,policy_jws) VALUES(?1,?2) ON CONFLICT(scope) DO UPDATE SET policy_jws=excluded.policy_jws",params![scope,compact])?;
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
    let compact: Option<String> = store
        .connection
        .query_row(
            "SELECT policy_jws FROM telemetry_audiences WHERE scope=?1",
            [scope],
            |r| r.get(0),
        )
        .optional()?;
    let Some(compact) = compact else {
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
    guard(&transaction)?;
    transaction.commit()?;
    Ok(
        json!({"available":true,"offset":offset,"total":bytes.len(),"digest":compact_digest(&compact),"chunk":URL_SAFE_NO_PAD.encode(&bytes[offset as usize..end])}),
    )
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
    let mut audience = Audience::open_with_guard(root, device, scope, Some(guard))?;
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
    let mut tick = tokio::time::interval(Duration::from_secs(5));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        tokio::select! {_=cancel.cancelled()=>return Ok(()),_=tick.tick()=>()}
        let store = StateStore::open(&root.join("management.sqlite"))?;
        let audiences: Vec<(String, String)> = store
            .connection
            .prepare("SELECT scope,policy_jws FROM telemetry_audiences")?
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<Result<_, _>>()?;
        for (scope, compact) in audiences {
            let result = (|| -> Result<()> {
                let policy = verify_telemetry_roster(
                    &compact,
                    &device.manifest().owner_invitation_key,
                    unix_time()?,
                )?;
                ensure!(
                    store.management_policy_head()?.map(|(_, digest)| digest)
                        == policy.management_policy_digest,
                    "Telemetry membership requires renewal after management changes"
                );
                let _ = store
                    .management_policy(&device.manifest().owner_invitation_key, unix_time()?)?;
                let telemetry = TelemetryStore::open(&root)?;
                let placement = if scope == "device" {
                    None
                } else {
                    Some(scope.as_str())
                };
                let data = telemetry.latest_metrics(placement)?;
                if data["records"]
                    .as_array()
                    .is_none_or(|records| records.is_empty())
                {
                    return Ok(());
                }
                let mut audience = Audience::open_with_guard(
                    &root,
                    &device,
                    &scope,
                    Some(roster_guard(device.manifest(), &compact, true)),
                )?;
                let (sequence, _) = audience.state.publication_position()?;
                let request_id = uuid::Uuid::new_v4().to_string();
                audience.state.publish(
                    &request_id,
                    sequence.checked_add(1).context("MLS sequence exhausted")?,
                    &serde_json::to_vec(&json!({"type":"metrics","scope":scope,"sample":data}))?,
                    unix_time()?,
                )?;
                audience.persist_witness()
            })();
            if result.is_err() {
                tracing::debug!(
                    "MLS audience publication paused pending policy or outbox recovery"
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let mut audience = Audience::open_with_guard(
            &root,
            &device,
            "device",
            Some(roster_guard(device.manifest(), &compact, true)),
        )?;
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
