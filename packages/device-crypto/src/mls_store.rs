//! Encrypted MLS snapshots with one transaction covering ratchets, policy,
//! replay position and outbound ciphertext. Storage keys and rollback witnesses
//! belong outside ordinary database backups.

use super::mls::{Identity, MemberIdentity, Received, TelemetryGroup};
use anyhow::{Context, Result, bail, ensure};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use chacha20poly1305::{
    XChaCha20Poly1305, XNonce,
    aead::{Aead, KeyInit, Payload},
};
use flow_like_device_protocol::{
    Ed25519PublicKey, SigningKey, TelemetryDeliveryReceipt, TelemetryEnvelope,
    TelemetryEnvelopeKind, TelemetryMember, TelemetryRoster, compact_digest,
    sign_telemetry_delivery_receipt, sign_telemetry_envelope, verify_telemetry_delivery_receipt,
    verify_telemetry_envelope, verify_telemetry_roster,
};
use openmls_rust_crypto::{OpenMlsRustCrypto, RustCrypto};
use openmls_traits::{OpenMlsProvider, crypto::OpenMlsCrypto, types::HashType};
use rand_core::{OsRng, RngCore};
#[cfg(feature = "sqlite")]
use rusqlite::{Connection, OpenFlags, OptionalExtension, TransactionBehavior, params};
use serde::{Deserialize, Serialize};
#[cfg(feature = "sqlite")]
use std::{path::Path, time::Duration};
use zeroize::{Zeroize, Zeroizing};

const MAX_SNAPSHOT: usize = 32 * 1024 * 1024;
const MAX_OUTBOX: usize = 128;
const MAX_REPLAY: usize = 256;
const MAX_PACKAGES: usize = 4096;
const MAX_WIRE: usize = 1024 * 1024;
const FORMAT: u32 = 1;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MlsPins {
    pub device_id: String,
    pub scope: String,
    pub owner_invitation_key: Ed25519PublicKey,
    pub publisher: MemberIdentity,
    pub local: MemberIdentity,
}

impl MlsPins {
    fn validate(&self) -> Result<()> {
        ensure!(
            !self.device_id.is_empty() && self.device_id.len() <= 256,
            "Invalid MLS device identity"
        );
        ensure!(
            !self.scope.is_empty() && self.scope.len() <= 256,
            "Invalid MLS audience scope"
        );
        self.owner_invitation_key.validate()?;
        for member in [&self.publisher, &self.local] {
            MemberIdentity::new(member.id().to_vec(), *member.signature_key())?;
            std::str::from_utf8(member.id()).context("MLS endpoint identity is not UTF-8")?;
            ensure!(
                self.owner_invitation_key.to_bytes()? != *member.signature_key(),
                "MLS endpoint keys must be separate from invitation authority"
            );
        }
        ensure!(
            self.owner_invitation_key.to_bytes()? != *self.publisher.signature_key(),
            "MLS publisher must be separate from invitation authority"
        );
        Ok(())
    }

    fn namespace(&self) -> Result<String> {
        Ok(digest(&serde_json::to_vec(&(
            &self.device_id,
            &self.scope,
            &self.local,
        ))?)?)
    }

    fn group_scope(&self) -> Result<String> {
        // The group identifier includes both device and audience boundaries.
        digest(&serde_json::to_vec(&(&self.device_id, &self.scope))?)
    }
}

/// Retain this witness independently of a database backup. A lower revision or
/// a different digest at the same revision is a known rollback and fails closed.
/// Restoring both database and witness is outside this software-only guarantee.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MlsCheckpoint {
    pub store_id: [u8; 32],
    pub revision: u64,
    pub digest: String,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProtectedSnapshot {
    store_id: [u8; 32],
    revision: u64,
    #[serde(with = "encoded_bytes")]
    ciphertext: Vec<u8>,
}

impl ProtectedSnapshot {
    pub fn checkpoint(&self) -> Result<MlsCheckpoint> {
        Ok(MlsCheckpoint {
            store_id: self.store_id,
            revision: self.revision,
            digest: digest(&serde_json::to_vec(self)?)?,
        })
    }
}

/// A backend must serialize writers, roll back on callback failure, and return
/// success only after committing the replacement. This keeps the snapshot engine
/// independent of SQLite for a future browser persistence adapter.
pub trait SnapshotBackend {
    fn transact<R>(
        &mut self,
        operation: impl FnOnce(Option<ProtectedSnapshot>) -> Result<(ProtectedSnapshot, R)>,
    ) -> Result<R>;
}

#[cfg(feature = "sqlite")]
pub type SqliteTransactionGuard = Box<dyn Fn(&Connection) -> Result<()> + Send>;

#[cfg(feature = "sqlite")]
pub struct SqliteSnapshotBackend {
    connection: Connection,
    namespace: String,
    guard: Option<SqliteTransactionGuard>,
}

#[cfg(feature = "sqlite")]
impl SqliteSnapshotBackend {
    /// The parent directory and management database must already be initialized
    /// by the agent. This adapter does not own the management schema version.
    pub fn open(path: &Path, pins: &MlsPins) -> Result<Self> {
        pins.validate()?;
        let connection = Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NOFOLLOW,
        )?;
        connection.busy_timeout(Duration::from_secs(5))?;
        let application_id: i64 =
            connection.pragma_query_value(None, "application_id", |row| row.get(0))?;
        ensure!(
            application_id == 0x464c5341,
            "MLS storage requires the agent management database"
        );
        connection.pragma_update(None, "synchronous", "FULL")?;
        connection.pragma_update(None, "secure_delete", true)?;
        connection.execute_batch(
            "CREATE TABLE IF NOT EXISTS mls_protected_snapshots (
            namespace TEXT PRIMARY KEY NOT NULL,
            snapshot BLOB NOT NULL
        )",
        )?;
        Ok(Self {
            connection,
            namespace: pins.namespace()?,
            guard: None,
        })
    }

    /// Check application authorization while the snapshot's writer lock is held.
    /// The callback must only read this connection and must not open another writer.
    pub fn with_transaction_guard(mut self, guard: SqliteTransactionGuard) -> Self {
        self.guard = Some(guard);
        self
    }
}

#[cfg(feature = "sqlite")]
impl SnapshotBackend for SqliteSnapshotBackend {
    fn transact<R>(
        &mut self,
        operation: impl FnOnce(Option<ProtectedSnapshot>) -> Result<(ProtectedSnapshot, R)>,
    ) -> Result<R> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        if let Some(guard) = &self.guard {
            guard(&transaction)?;
        }
        let encoded: Option<Vec<u8>> = transaction
            .query_row(
                "SELECT snapshot FROM mls_protected_snapshots WHERE namespace=?1",
                [&self.namespace],
                |row| row.get(0),
            )
            .optional()?;
        let current = encoded
            .map(|bytes| {
                ensure!(
                    bytes.len() <= MAX_SNAPSHOT * 2,
                    "MLS database snapshot exceeds its bound"
                );
                serde_json::from_slice(&bytes).context("Invalid encrypted MLS snapshot")
            })
            .transpose()?;
        let (next, result) = operation(current)?;
        let encoded = serde_json::to_vec(&next)?;
        ensure!(
            encoded.len() <= MAX_SNAPSHOT * 2,
            "MLS database snapshot exceeds its bound"
        );
        transaction.execute(
            "INSERT INTO mls_protected_snapshots(namespace,snapshot) VALUES (?1,?2)
            ON CONFLICT(namespace) DO UPDATE SET snapshot=excluded.snapshot",
            params![self.namespace, encoded],
        )?;
        if let Some(guard) = &self.guard {
            guard(&transaction)?;
        }
        transaction.commit()?;
        Ok(result)
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MlsDelivery {
    pub envelope_jws: String,
    pub policy_jws: String,
    #[serde(with = "encoded_bytes")]
    pub wire: Vec<u8>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MlsPublication {
    pub sequence: u64,
    pub message: MlsDelivery,
    pub welcome: Option<MlsDelivery>,
}

pub enum MlsReceipt {
    Application(Zeroizing<Vec<u8>>),
    EpochChanged(u64),
    Removed,
    Duplicate,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredValue {
    key: String,
    value: String,
}
impl Drop for StoredValue {
    fn drop(&mut self) {
        self.key.zeroize();
        self.value.zeroize();
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Outbound {
    request_id: String,
    request_digest: String,
    publication: MlsPublication,
    #[serde(default)]
    required_members: Option<Vec<TelemetryMember>>,
    #[serde(default)]
    acknowledged_members: Vec<TelemetryMember>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct EvictedPublication {
    sequence: u64,
    policy_digest: String,
    envelope_digest: String,
    welcome_digest: Option<String>,
    members: Vec<TelemetryMember>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Replay {
    sequence: u64,
    digest: String,
    #[serde(default)]
    policy_digest: String,
    #[serde(default)]
    receipt_pending: bool,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Snapshot {
    format: u32,
    pins: MlsPins,
    values: Vec<StoredValue>,
    signing_seed: [u8; 32],
    policy_jws: Option<String>,
    group_exists: bool,
    retired: bool,
    sequence: u64,
    acknowledged: u64,
    received: u64,
    replay: Vec<Replay>,
    outbox: Vec<Outbound>,
    #[serde(default)]
    evicted: Vec<EvictedPublication>,
    used_packages: Vec<String>,
    key_package: Option<Vec<u8>>,
}

impl Drop for Snapshot {
    fn drop(&mut self) {
        self.signing_seed.zeroize();
    }
}

struct SessionProvider(OpenMlsRustCrypto);
impl Drop for SessionProvider {
    fn drop(&mut self) {
        if let Ok(mut values) = self.0.storage().values.write() {
            for (mut key, mut value) in values.drain() {
                key.zeroize();
                value.zeroize();
            }
        }
    }
}

impl Snapshot {
    fn provider(&self) -> Result<SessionProvider> {
        let provider = SessionProvider(OpenMlsRustCrypto::default());
        {
            let mut values = provider
                .0
                .storage()
                .values
                .write()
                .map_err(|_| anyhow::anyhow!("MLS storage lock failed"))?;
            ensure!(
                self.values.len() <= 16_384,
                "MLS storage entry limit reached"
            );
            for entry in &self.values {
                let key = URL_SAFE_NO_PAD.decode(&entry.key)?;
                let value = URL_SAFE_NO_PAD.decode(&entry.value)?;
                ensure!(
                    values.insert(key, value).is_none(),
                    "Duplicate MLS storage entry"
                );
            }
        }
        Ok(provider)
    }

    fn snapshot_provider(&mut self, provider: &OpenMlsRustCrypto) -> Result<()> {
        let values = provider
            .storage()
            .values
            .read()
            .map_err(|_| anyhow::anyhow!("MLS storage lock failed"))?;
        self.values = values
            .iter()
            .map(|(key, value)| StoredValue {
                key: URL_SAFE_NO_PAD.encode(key),
                value: URL_SAFE_NO_PAD.encode(value),
            })
            .collect();
        self.values.sort_by(|a, b| a.key.cmp(&b.key));
        Ok(())
    }

    fn policy(&self, compact: &str, now: i64) -> Result<TelemetryRoster> {
        let policy = verify_telemetry_roster(compact, &self.pins.owner_invitation_key, now)?;
        ensure!(
            policy.device_id == self.pins.device_id && policy.scope == self.pins.scope,
            "MLS policy belongs to another audience"
        );
        ensure!(
            member(&policy.publisher)? == self.pins.publisher,
            "MLS policy changed the pinned publisher"
        );
        ensure!(
            policy
                .members
                .iter()
                .all(|member| member.signing_key != self.pins.owner_invitation_key),
            "MLS members cannot use the invitation authority key"
        );
        Ok(policy)
    }

    fn current_policy(&self, now: i64) -> Result<TelemetryRoster> {
        self.policy(
            self.policy_jws
                .as_deref()
                .context("MLS policy is not initialized")?,
            now,
        )
    }

    fn next_policy(&self, compact: &str, now: i64) -> Result<TelemetryRoster> {
        let next = self.policy(compact, now)?;
        match &self.policy_jws {
            Some(current) => {
                // A policy may be renewed after its expiry, but it cannot skip
                // the accepted version or fork its signed predecessor.
                let previous = self.policy_at_issue(current)?;
                ensure!(
                    next.policy_version
                        == previous
                            .policy_version
                            .checked_add(1)
                            .context("MLS policy version exhausted")?
                        && next.previous_policy_digest.as_deref()
                            == Some(compact_digest(current).as_str()),
                    "MLS policy is stale or forked"
                );
            }
            None => ensure!(
                next.policy_version == 1 && next.previous_policy_digest.is_none(),
                "MLS genesis policy is invalid"
            ),
        }
        Ok(next)
    }

    fn policy_at_issue(
        &self,
        compact: &str,
    ) -> Result<TelemetryRoster, flow_like_device_protocol::ProtocolError> {
        // Only an already authenticated, encrypted local policy reaches this
        // path. Its issue time permits signature verification after expiry.
        let payload =
            compact
                .split('.')
                .nth(1)
                .ok_or(flow_like_device_protocol::ProtocolError::Invalid(
                    "policy encoding",
                ))?;
        let bytes = URL_SAFE_NO_PAD
            .decode(payload)
            .map_err(|_| flow_like_device_protocol::ProtocolError::Invalid("policy encoding"))?;
        let policy: TelemetryRoster = serde_json::from_slice(&bytes)
            .map_err(|_| flow_like_device_protocol::ProtocolError::Invalid("policy encoding"))?;
        verify_telemetry_roster(compact, &self.pins.owner_invitation_key, policy.issued_at)
    }

    fn group(&self, provider: &OpenMlsRustCrypto) -> Result<TelemetryGroup> {
        ensure!(
            self.group_exists && !self.retired,
            "MLS audience requires a fresh leaf admission"
        );
        let policy = self.policy_at_issue(
            self.policy_jws
                .as_deref()
                .context("MLS policy is missing")?,
        )?;
        Ok(TelemetryGroup::load(
            provider,
            &self.pins.group_scope()?,
            self.pins.publisher.clone(),
            &members(&policy)?,
        )?)
    }

    fn next_sequence(&self) -> Result<u64> {
        self.sequence
            .checked_add(1)
            .context("MLS sequence exhausted")
    }
}

/// The only public operations return data after the backend transaction commits.
/// No mutable OpenMLS provider or active group leaves the transaction boundary.
pub struct ProtectedMlsStore<B: SnapshotBackend> {
    backend: B,
    pins: MlsPins,
    key: Zeroizing<[u8; 32]>,
    checkpoint: Option<MlsCheckpoint>,
}

impl<B: SnapshotBackend> ProtectedMlsStore<B> {
    pub fn create(
        mut backend: B,
        pins: MlsPins,
        key: [u8; 32],
        local_key: &SigningKey,
    ) -> Result<Self> {
        pins.validate()?;
        ensure!(
            *pins.local.signature_key() == local_key.public_key().to_bytes()?,
            "MLS local key differs from its identity"
        );
        let key = Zeroizing::new(key);
        let checkpoint = backend.transact(|current| {
            ensure!(
                current.is_none(),
                "MLS state already exists; refusing to replace live ratchets"
            );
            let provider = SessionProvider(OpenMlsRustCrypto::default());
            Identity::from_signing_key(&provider.0, pins.local.id().to_vec(), local_key)?;
            let mut snapshot = Snapshot {
                format: FORMAT,
                pins: pins.clone(),
                values: Vec::new(),
                signing_seed: local_key.to_bytes(),
                policy_jws: None,
                group_exists: false,
                retired: false,
                sequence: 0,
                acknowledged: 0,
                received: 0,
                replay: Vec::new(),
                outbox: Vec::new(),
                evicted: Vec::new(),
                used_packages: Vec::new(),
                key_package: None,
            };
            snapshot.snapshot_provider(&provider.0)?;
            let mut store_id = [0; 32];
            OsRng.fill_bytes(&mut store_id);
            let protected = seal_snapshot(&pins, &key, store_id, 1, &snapshot)?;
            let checkpoint = protected.checkpoint()?;
            Ok((protected, checkpoint))
        })?;
        Ok(Self {
            backend,
            pins,
            key,
            checkpoint: Some(checkpoint),
        })
    }

    /// A supplied checkpoint must come from an independently trusted high-water
    /// witness, not an adjacent database row restored from the same backup.
    pub fn open(backend: B, pins: MlsPins, key: [u8; 32], minimum: MlsCheckpoint) -> Result<Self> {
        pins.validate()?;
        ensure!(minimum.revision > 0, "Invalid MLS rollback witness");
        let mut store = Self {
            backend,
            pins,
            key: Zeroizing::new(key),
            checkpoint: Some(minimum),
        };
        store.inspect(|_| Ok(()))?;
        Ok(store)
    }

    pub fn checkpoint(&self) -> MlsCheckpoint {
        self.checkpoint.clone().expect("initialized MLS checkpoint")
    }

    pub fn into_backend(self) -> B {
        self.backend
    }

    fn inspect<R>(&mut self, operation: impl FnOnce(&Snapshot) -> Result<R>) -> Result<R> {
        let pins = &self.pins;
        let key = &self.key;
        let minimum = self.checkpoint.as_ref();
        let (result, checkpoint) = self.backend.transact(|current| {
            let current = current.context("MLS state is missing; fresh admission required")?;
            verify_checkpoint(&current, minimum)?;
            let snapshot = open_snapshot(pins, key, &current)?;
            let result = operation(&snapshot)?;
            let checkpoint = current.checkpoint()?;
            Ok((current, (result, checkpoint)))
        })?;
        self.checkpoint = Some(checkpoint);
        Ok(result)
    }

    fn mutate<R>(
        &mut self,
        operation: impl FnOnce(&mut Snapshot, &OpenMlsRustCrypto) -> Result<R>,
    ) -> Result<R> {
        let pins = &self.pins;
        let key = &self.key;
        let minimum = self.checkpoint.as_ref();
        let (result, checkpoint) = self.backend.transact(|current| {
            let current = current.context("MLS state is missing; fresh admission required")?;
            verify_checkpoint(&current, minimum)?;
            let mut snapshot = open_snapshot(pins, key, &current)?;
            let provider = snapshot.provider()?;
            let result = operation(&mut snapshot, &provider.0)?;
            snapshot.snapshot_provider(&provider.0)?;
            let next = seal_snapshot(
                pins,
                key,
                current.store_id,
                current
                    .revision
                    .checked_add(1)
                    .context("MLS revision exhausted")?,
                &snapshot,
            )?;
            let checkpoint = next.checkpoint()?;
            Ok((next, (result, checkpoint)))
        })?;
        self.checkpoint = Some(checkpoint);
        Ok(result)
    }

    /// Repeated publication returns the same one-use public package until it is
    /// consumed by Welcome. A fresh leaf requires a new protected store identity.
    pub fn key_package(&mut self) -> Result<Vec<u8>> {
        self.mutate(|snapshot, provider| {
            ensure!(
                !snapshot.group_exists && !snapshot.retired,
                "MLS leaf is already admitted"
            );
            if let Some(package) = &snapshot.key_package {
                return Ok(package.clone());
            }
            let identity = Identity::load(provider, snapshot.pins.local.clone())?;
            let package = identity.key_package(provider)?;
            snapshot.key_package = Some(package.clone());
            Ok(package)
        })
    }

    pub fn pending_outbox(&mut self) -> Result<Vec<MlsPublication>> {
        self.inspect(|snapshot| {
            Ok(snapshot
                .outbox
                .iter()
                .map(|item| item.publication.clone())
                .collect())
        })
    }

    pub fn publication_position(&mut self) -> Result<(u64, u64)> {
        self.inspect(|snapshot| Ok((snapshot.sequence, snapshot.acknowledged)))
    }

    pub fn reader_position(&mut self) -> Result<(bool, bool, u64)> {
        self.inspect(|snapshot| Ok((snapshot.group_exists, snapshot.retired, snapshot.received)))
    }

    /// The explicit next sequence fences retries after an acknowledgement has
    /// removed the old outbox record. Unacknowledged retries return exact bytes.
    pub fn apply_policy(
        &mut self,
        request_id: &str,
        sequence: u64,
        policy_jws: &str,
        packages: &[(MemberIdentity, Vec<u8>)],
        now: i64,
    ) -> Result<MlsPublication> {
        request_identifier(request_id)?;
        let request_digest = digest(&serde_json::to_vec(&(
            "membership",
            sequence,
            policy_jws,
            packages,
        ))?)?;
        self.mutate(|snapshot, provider| {
            ensure!(
                snapshot.pins.local == snapshot.pins.publisher && !snapshot.retired,
                "Only the device can commit MLS membership"
            );
            if let Some(existing) =
                existing_publication(snapshot, request_id, &request_digest, sequence, true)?
            {
                return Ok(existing);
            }
            let policy = snapshot.next_policy(policy_jws, now)?;
            let approved = members(&policy)?;
            if waive_removed_readers(snapshot, &policy.members)? {
                evict_confirmed(snapshot)?;
            }
            ensure!(
                snapshot.outbox.len() < MAX_OUTBOX,
                "MLS outbox is full; current readers must receive outstanding messages"
            );
            let mut package_digests = Vec::new();
            for (_, package) in packages {
                ensure!(
                    !package.is_empty() && package.len() <= MAX_WIRE,
                    "Invalid MLS key package size"
                );
                let hash = digest(package)?;
                ensure!(
                    !snapshot.used_packages.contains(&hash) && !package_digests.contains(&hash),
                    "MLS key package was already used"
                );
                package_digests.push(hash);
            }
            ensure!(
                snapshot.used_packages.len() + package_digests.len() <= MAX_PACKAGES,
                "MLS package history is full; trust rotation is required"
            );
            let identity = Identity::load(provider, snapshot.pins.local.clone())?;
            let mut group = if snapshot.group_exists {
                snapshot.group(provider)?
            } else {
                TelemetryGroup::create(provider, &identity, &snapshot.pins.group_scope()?)?
            };
            let messages = group.prepare_membership(provider, &identity, &approved, packages)?;
            let message = signed_delivery(
                snapshot,
                sequence,
                policy_jws,
                TelemetryEnvelopeKind::Commit,
                messages.commit,
            )?;
            let welcome = messages
                .welcome
                .map(|wire| {
                    signed_delivery(
                        snapshot,
                        sequence,
                        policy_jws,
                        TelemetryEnvelopeKind::Welcome,
                        wire,
                    )
                })
                .transpose()?;
            group.merge_pending(provider)?;
            exact_roster(&group, &approved)?;
            snapshot.group_exists = true;
            snapshot.policy_jws = Some(policy_jws.into());
            snapshot.used_packages.extend(package_digests);
            let publication = MlsPublication {
                sequence,
                message,
                welcome,
            };
            record_publication(snapshot, request_id, &request_digest, publication.clone())?;
            Ok(publication)
        })
    }

    pub fn publish(
        &mut self,
        request_id: &str,
        sequence: u64,
        plaintext: &[u8],
        now: i64,
    ) -> Result<MlsPublication> {
        request_identifier(request_id)?;
        ensure!(
            plaintext.len() <= 64 * 1024,
            "MLS application exceeds its bound"
        );
        let request_digest = digest(&serde_json::to_vec(&("application", sequence, plaintext))?)?;
        self.mutate(|snapshot, provider| {
            ensure!(
                snapshot.pins.local == snapshot.pins.publisher,
                "Only the device can publish telemetry"
            );
            if let Some(existing) =
                existing_publication(snapshot, request_id, &request_digest, sequence, false)?
            {
                return Ok(existing);
            }
            let policy = snapshot.current_policy(now)?;
            let approved = members(&policy)?;
            let mut group = snapshot.group(provider)?;
            exact_roster(&group, &approved)?;
            let identity = Identity::load(provider, snapshot.pins.local.clone())?;
            let wire = group.encrypt(provider, &identity, plaintext, &approved)?;
            let message = signed_delivery(
                snapshot,
                sequence,
                snapshot
                    .policy_jws
                    .as_deref()
                    .context("Missing MLS policy")?,
                TelemetryEnvelopeKind::Application,
                wire,
            )?;
            let publication = MlsPublication {
                sequence,
                message,
                welcome: None,
            };
            record_publication(snapshot, request_id, &request_digest, publication.clone())?;
            if policy.members.len() == 1 {
                evict_confirmed(snapshot)?;
            }
            Ok(publication)
        })
    }

    pub fn join(&mut self, delivery: &MlsDelivery, now: i64) -> Result<()> {
        self.mutate(|snapshot, provider| {
            ensure!(
                !snapshot.group_exists && !snapshot.retired && snapshot.key_package.is_some(),
                "MLS leaf cannot accept another Welcome"
            );
            let (envelope, policy) = verified_delivery(snapshot, delivery, now)?;
            ensure!(
                envelope.kind == TelemetryEnvelopeKind::Welcome,
                "Expected an MLS Welcome envelope"
            );
            let approved = members(&policy)?;
            ensure!(
                approved.contains(&snapshot.pins.local),
                "MLS policy does not admit this endpoint"
            );
            let group = TelemetryGroup::join(
                provider,
                &snapshot.pins.group_scope()?,
                &delivery.wire,
                snapshot.pins.publisher.clone(),
                &approved,
            )?;
            exact_roster(&group, &approved)?;
            snapshot.group_exists = true;
            snapshot.key_package = None;
            snapshot.policy_jws = Some(delivery.policy_jws.clone());
            snapshot.received = envelope.sequence;
            snapshot.replay.push(Replay {
                sequence: envelope.sequence,
                digest: compact_digest(&delivery.envelope_jws),
                policy_digest: envelope.policy_digest,
                receipt_pending: !snapshot.retired,
            });
            Ok(())
        })
    }

    /// A verified frame whose policy is unavailable, whose sequence has a gap,
    /// or whose transaction fails consumes no durable receive ratchet.
    pub fn receive(&mut self, delivery: &MlsDelivery, now: i64) -> Result<MlsReceipt> {
        self.mutate(|snapshot, provider| {
            let (envelope, policy) = verified_delivery(snapshot, delivery, now)?;
            let wire_digest = compact_digest(&delivery.envelope_jws);
            if let Some(previous) = snapshot
                .replay
                .iter()
                .find(|item| item.sequence == envelope.sequence)
            {
                ensure!(
                    previous.digest == wire_digest,
                    "MLS message sequence was reused with different content"
                );
                return Ok(MlsReceipt::Duplicate);
            }
            ensure!(
                envelope.sequence
                    == snapshot
                        .received
                        .checked_add(1)
                        .context("MLS receive sequence exhausted")?,
                "MLS history is missing or outside the replay window; ordered recovery is required"
            );
            let mut group = snapshot.group(provider)?;
            let approved = members(&policy)?;
            match envelope.kind {
                TelemetryEnvelopeKind::Application => ensure!(
                    snapshot.policy_jws.as_deref() == Some(delivery.policy_jws.as_str()),
                    "MLS application requires its accepted membership policy"
                ),
                TelemetryEnvelopeKind::Commit => {
                    snapshot.next_policy(&delivery.policy_jws, now)?;
                }
                TelemetryEnvelopeKind::Welcome => {
                    bail!("An existing MLS leaf cannot receive another Welcome")
                }
            }
            let received = group.process(provider, &delivery.wire, &approved)?;
            let result = match received {
                Received::Application(plaintext) => {
                    ensure!(
                        envelope.kind == TelemetryEnvelopeKind::Application,
                        "MLS envelope content differs from its signature"
                    );
                    exact_roster(&group, &approved)?;
                    MlsReceipt::Application(Zeroizing::new(plaintext))
                }
                Received::EpochChanged(epoch) => {
                    ensure!(
                        envelope.kind == TelemetryEnvelopeKind::Commit,
                        "MLS envelope content differs from its signature"
                    );
                    exact_roster(&group, &approved)?;
                    snapshot.policy_jws = Some(delivery.policy_jws.clone());
                    if !group.is_active() {
                        snapshot.retired = true;
                        drop(group);
                        TelemetryGroup::delete_retired(provider, &snapshot.pins.group_scope()?)?;
                        MlsReceipt::Removed
                    } else {
                        MlsReceipt::EpochChanged(epoch)
                    }
                }
            };
            snapshot.received = envelope.sequence;
            snapshot.replay.push(Replay {
                sequence: envelope.sequence,
                digest: wire_digest,
                policy_digest: envelope.policy_digest,
                receipt_pending: !snapshot.retired,
            });
            if snapshot.replay.len() > MAX_REPLAY {
                ensure!(
                    !snapshot.replay[0].receipt_pending,
                    "Confirm pending delivery receipts before reading more MLS messages"
                );
                snapshot.replay.remove(0);
            }
            Ok(result)
        })
    }

    /// Acknowledge only the oldest durable record. A relay acknowledgement means
    /// durable transport acceptance; it is not proof every reader applied it.
    pub fn acknowledge(&mut self, sequence: u64, envelope_digest: &str) -> Result<()> {
        self.mutate(|snapshot, _| {
            if sequence <= snapshot.acknowledged {
                return Ok(());
            }
            let first = snapshot.outbox.first().context("MLS outbox is empty")?;
            ensure!(
                first.publication.sequence == sequence
                    && compact_digest(&first.publication.message.envelope_jws) == envelope_digest,
                "MLS acknowledgement does not match the oldest message"
            );
            evict_first(snapshot)?;
            Ok(())
        })
    }

    /// Receipts are derived only from messages already committed in this store.
    pub fn pending_delivery_receipts(&mut self) -> Result<Vec<TelemetryDeliveryReceipt>> {
        self.inspect(|snapshot| {
            let endpoint_id = String::from_utf8(snapshot.pins.local.id().to_vec())
                .context("Invalid endpoint identifier")?;
            Ok(snapshot
                .replay
                .iter()
                .filter(|entry| entry.receipt_pending)
                .map(|entry| TelemetryDeliveryReceipt {
                    version: 1,
                    device_id: snapshot.pins.device_id.clone(),
                    scope: snapshot.pins.scope.clone(),
                    endpoint_id: endpoint_id.clone(),
                    sequence: entry.sequence,
                    policy_digest: entry.policy_digest.clone(),
                    envelope_digest: entry.digest.clone(),
                })
                .collect())
        })
    }
    pub fn signed_delivery_receipt(&mut self, sequence: u64) -> Result<String> {
        self.inspect(|snapshot| {
            let replay = snapshot
                .replay
                .iter()
                .find(|entry| entry.sequence == sequence)
                .context("MLS delivery no longer retained")?;
            ensure!(
                !replay.policy_digest.is_empty(),
                "Legacy MLS delivery needs explicit owner acknowledgement"
            );
            let receipt = TelemetryDeliveryReceipt {
                version: 1,
                device_id: snapshot.pins.device_id.clone(),
                scope: snapshot.pins.scope.clone(),
                endpoint_id: String::from_utf8(snapshot.pins.local.id().to_vec())
                    .context("Invalid endpoint identifier")?,
                sequence,
                policy_digest: replay.policy_digest.clone(),
                envelope_digest: replay.digest.clone(),
            };
            Ok(sign_telemetry_delivery_receipt(
                &receipt,
                &SigningKey::from_bytes(&snapshot.signing_seed),
            )?)
        })
    }
    pub fn confirm_delivery_receipt(&mut self, sequence: u64, envelope_digest: &str) -> Result<()> {
        self.mutate(|snapshot, _| {
            let replay = snapshot
                .replay
                .iter_mut()
                .find(|entry| entry.sequence == sequence)
                .context("MLS receipt confirmation is stale")?;
            ensure!(
                replay.digest == envelope_digest,
                "MLS receipt confirmation changed"
            );
            replay.receipt_pending = false;
            Ok(())
        })
    }
    /// Verify the endpoint key against the immutable publication roster, record
    /// its acknowledgement, then evict only a contiguous fully delivered prefix.
    pub fn accept_delivery_receipt(
        &mut self,
        endpoint_id: &str,
        sequence: u64,
        compact: &str,
    ) -> Result<u64> {
        self.mutate(|snapshot, _| {
            ensure!(
                snapshot.pins.local == snapshot.pins.publisher,
                "Only the publisher accepts delivery receipts"
            );
            let publication = if let Some(entry) = snapshot
                .outbox
                .iter()
                .find(|entry| entry.publication.sequence == sequence)
            {
                receipt_metadata(snapshot, &entry.publication)?
            } else {
                let previous = snapshot
                    .evicted
                    .iter()
                    .find(|entry| entry.sequence == sequence)
                    .context(
                        "MLS delivery receipt is older than the retained acknowledgement window",
                    )?;
                verify_delivery_receipt(snapshot, previous, endpoint_id, compact)?;
                return Ok(snapshot.acknowledged);
            };
            let member = verify_delivery_receipt(snapshot, &publication, endpoint_id, compact)?;
            let entry = snapshot
                .outbox
                .iter_mut()
                .find(|entry| entry.publication.sequence == sequence)
                .context("Missing MLS publication")?;
            if !entry.acknowledged_members.contains(&member) {
                entry.acknowledged_members.push(member);
            }
            evict_confirmed(snapshot)?;
            Ok(snapshot.acknowledged)
        })
    }
}

fn request_identifier(id: &str) -> Result<()> {
    ensure!(
        !id.is_empty() && id.len() <= 128 && id.bytes().all(|b| b.is_ascii_graphic()),
        "Invalid MLS publication request identity"
    );
    Ok(())
}

fn receipt_metadata(
    snapshot: &Snapshot,
    publication: &MlsPublication,
) -> Result<EvictedPublication> {
    let roster = snapshot.policy_at_issue(&publication.message.policy_jws)?;
    Ok(EvictedPublication {
        sequence: publication.sequence,
        policy_digest: compact_digest(&publication.message.policy_jws),
        envelope_digest: compact_digest(&publication.message.envelope_jws),
        welcome_digest: publication
            .welcome
            .as_ref()
            .map(|welcome| compact_digest(&welcome.envelope_jws)),
        members: roster
            .members
            .into_iter()
            .filter(|member| member != &roster.publisher)
            .collect(),
    })
}
fn verify_delivery_receipt(
    snapshot: &Snapshot,
    publication: &EvictedPublication,
    endpoint_id: &str,
    compact: &str,
) -> Result<TelemetryMember> {
    let member = publication
        .members
        .iter()
        .find(|member| member.endpoint_id == endpoint_id)
        .context("Endpoint was not a recipient of this MLS publication")?;
    let receipt = verify_telemetry_delivery_receipt(compact, &member.signing_key)?;
    ensure!(
        receipt.device_id == snapshot.pins.device_id
            && receipt.scope == snapshot.pins.scope
            && receipt.endpoint_id == endpoint_id
            && receipt.sequence == publication.sequence
            && receipt.policy_digest == publication.policy_digest
            && (receipt.envelope_digest == publication.envelope_digest
                || publication.welcome_digest.as_deref() == Some(receipt.envelope_digest.as_str())),
        "MLS delivery receipt binding changed"
    );
    Ok(member.clone())
}
fn waive_removed_readers(snapshot: &mut Snapshot, current: &[TelemetryMember]) -> Result<bool> {
    let mut removed = false;
    for index in 0..snapshot.outbox.len() {
        if snapshot.outbox[index].required_members.is_none() {
            let previous = receipt_metadata(snapshot, &snapshot.outbox[index].publication)?;
            snapshot.outbox[index].required_members = Some(previous.members);
        }
        let members = snapshot.outbox[index]
            .required_members
            .as_mut()
            .expect("initialized above");
        let previous = members.len();
        members.retain(|member| current.contains(member));
        removed |= previous != members.len();
    }
    Ok(removed)
}
fn evict_first(snapshot: &mut Snapshot) -> Result<()> {
    let first = snapshot.outbox.first().context("MLS outbox is empty")?;
    let metadata = receipt_metadata(snapshot, &first.publication)?;
    snapshot.acknowledged = metadata.sequence;
    snapshot.evicted.push(metadata);
    if snapshot.evicted.len() > MAX_REPLAY {
        snapshot.evicted.remove(0);
    }
    snapshot.outbox.remove(0);
    Ok(())
}
fn evict_confirmed(snapshot: &mut Snapshot) -> Result<()> {
    while let Some(first) = snapshot.outbox.first() {
        let required = match &first.required_members {
            Some(required) => required.clone(),
            None => receipt_metadata(snapshot, &first.publication)?.members,
        };
        if required
            .iter()
            .any(|member| !first.acknowledged_members.contains(member))
        {
            break;
        }
        evict_first(snapshot)?;
    }
    Ok(())
}

fn existing_publication(
    snapshot: &Snapshot,
    id: &str,
    request_digest: &str,
    sequence: u64,
    membership: bool,
) -> Result<Option<MlsPublication>> {
    if let Some(existing) = snapshot.outbox.iter().find(|entry| entry.request_id == id) {
        ensure!(
            existing.request_digest == request_digest && existing.publication.sequence == sequence,
            "MLS publication identity changed"
        );
        return Ok(Some(existing.publication.clone()));
    }
    ensure!(
        sequence == snapshot.next_sequence()?,
        "MLS publication sequence is stale or skipped"
    );
    ensure!(
        membership || snapshot.outbox.len() < MAX_OUTBOX,
        "MLS outbox is full; publication is suspended"
    );
    Ok(None)
}

fn record_publication(
    snapshot: &mut Snapshot,
    id: &str,
    request_digest: &str,
    publication: MlsPublication,
) -> Result<()> {
    ensure!(
        publication.sequence == snapshot.next_sequence()?,
        "MLS publication order changed"
    );
    snapshot.sequence = publication.sequence;
    let policy = snapshot.policy_at_issue(&publication.message.policy_jws)?;
    let required_members = policy
        .members
        .into_iter()
        .filter(|member| member != &policy.publisher)
        .collect();
    snapshot.outbox.push(Outbound {
        request_id: id.into(),
        request_digest: request_digest.into(),
        publication,
        required_members: Some(required_members),
        acknowledged_members: Vec::new(),
    });
    Ok(())
}

fn exact_roster(group: &TelemetryGroup, approved: &[MemberIdentity]) -> Result<()> {
    let actual = group.roster()?;
    ensure!(
        actual.len() == approved.len() && actual.iter().all(|member| approved.contains(member)),
        "MLS group does not match the owner-approved roster"
    );
    Ok(())
}

fn signed_delivery(
    snapshot: &Snapshot,
    sequence: u64,
    policy_jws: &str,
    kind: TelemetryEnvelopeKind,
    wire: Vec<u8>,
) -> Result<MlsDelivery> {
    ensure!(
        !wire.is_empty() && wire.len() <= MAX_WIRE,
        "Invalid MLS message size"
    );
    let envelope = TelemetryEnvelope {
        device_id: snapshot.pins.device_id.clone(),
        scope: snapshot.pins.scope.clone(),
        sequence,
        policy_digest: compact_digest(policy_jws),
        kind,
        wire_digest: digest(&wire)?,
    };
    Ok(MlsDelivery {
        envelope_jws: sign_telemetry_envelope(
            &envelope,
            &SigningKey::from_bytes(&snapshot.signing_seed),
        )?,
        policy_jws: policy_jws.into(),
        wire,
    })
}

fn verified_delivery(
    snapshot: &Snapshot,
    delivery: &MlsDelivery,
    now: i64,
) -> Result<(TelemetryEnvelope, TelemetryRoster)> {
    ensure!(
        !delivery.wire.is_empty() && delivery.wire.len() <= MAX_WIRE,
        "Invalid MLS message size"
    );
    let envelope = verify_telemetry_envelope(
        &delivery.envelope_jws,
        &Ed25519PublicKey::from_bytes(*snapshot.pins.publisher.signature_key())?,
    )?;
    ensure!(
        envelope.device_id == snapshot.pins.device_id && envelope.scope == snapshot.pins.scope,
        "MLS message belongs to another audience"
    );
    ensure!(
        envelope.wire_digest == digest(&delivery.wire)?
            && envelope.policy_digest == compact_digest(&delivery.policy_jws),
        "MLS message envelope binding failed"
    );
    let policy = snapshot.policy(&delivery.policy_jws, now)?;
    Ok((envelope, policy))
}

fn member(value: &flow_like_device_protocol::TelemetryMember) -> Result<MemberIdentity> {
    Ok(MemberIdentity::new(
        value.endpoint_id.as_bytes().to_vec(),
        value.signing_key.to_bytes()?,
    )?)
}
fn members(policy: &TelemetryRoster) -> Result<Vec<MemberIdentity>> {
    policy.members.iter().map(member).collect()
}

fn digest(bytes: &[u8]) -> Result<String> {
    Ok(URL_SAFE_NO_PAD.encode(
        RustCrypto::default()
            .hash(HashType::Sha2_256, bytes)
            .map_err(|_| anyhow::anyhow!("MLS digest failed"))?,
    ))
}

fn aad(pins: &MlsPins, store_id: &[u8; 32], revision: u64) -> Result<Vec<u8>> {
    Ok(serde_json::to_vec(&(
        "flow-like/mls/protected-snapshot/v1",
        pins,
        store_id,
        revision,
    ))?)
}

fn seal_snapshot(
    pins: &MlsPins,
    key: &[u8; 32],
    store_id: [u8; 32],
    revision: u64,
    snapshot: &Snapshot,
) -> Result<ProtectedSnapshot> {
    let plaintext = Zeroizing::new(serde_json::to_vec(snapshot)?);
    ensure!(
        plaintext.len() <= MAX_SNAPSHOT,
        "MLS protected state is full; delivery or fresh admission is required"
    );
    let mut nonce = [0; 24];
    OsRng.fill_bytes(&mut nonce);
    let encrypted = XChaCha20Poly1305::new(key.into())
        .encrypt(
            XNonce::from_slice(&nonce),
            Payload {
                msg: &plaintext,
                aad: &aad(pins, &store_id, revision)?,
            },
        )
        .map_err(|_| anyhow::anyhow!("MLS snapshot encryption failed"))?;
    let mut ciphertext = nonce.to_vec();
    ciphertext.extend_from_slice(&encrypted);
    Ok(ProtectedSnapshot {
        store_id,
        revision,
        ciphertext,
    })
}

fn open_snapshot(
    pins: &MlsPins,
    key: &[u8; 32],
    protected: &ProtectedSnapshot,
) -> Result<Snapshot> {
    ensure!(
        protected.revision > 0 && (40..=MAX_SNAPSHOT + 40).contains(&protected.ciphertext.len()),
        "Invalid MLS protected state"
    );
    let plaintext = Zeroizing::new(
        XChaCha20Poly1305::new(key.into())
            .decrypt(
                XNonce::from_slice(&protected.ciphertext[..24]),
                Payload {
                    msg: &protected.ciphertext[24..],
                    aad: &aad(pins, &protected.store_id, protected.revision)?,
                },
            )
            .map_err(|_| anyhow::anyhow!("MLS snapshot authentication failed"))?,
    );
    let snapshot: Snapshot =
        serde_json::from_slice(&plaintext).context("Invalid MLS protected state")?;
    ensure!(
        snapshot.format == FORMAT && &snapshot.pins == pins,
        "MLS snapshot binding changed"
    );
    ensure!(
        snapshot.outbox.len() <= MAX_OUTBOX
            && snapshot.replay.len() <= MAX_REPLAY
            && snapshot.evicted.len() <= MAX_REPLAY
            && snapshot.evicted.iter().all(
                |entry| entry.members.len() <= flow_like_device_protocol::MAX_TELEMETRY_MEMBERS
            )
            && snapshot
                .outbox
                .iter()
                .all(|entry| entry.required_members.as_ref().is_none_or(
                    |members| members.len() <= flow_like_device_protocol::MAX_TELEMETRY_MEMBERS
                ) && entry.acknowledged_members.len()
                    <= flow_like_device_protocol::MAX_TELEMETRY_MEMBERS)
            && snapshot.used_packages.len() <= MAX_PACKAGES,
        "Invalid MLS state limits"
    );
    Ok(snapshot)
}

fn verify_checkpoint(protected: &ProtectedSnapshot, minimum: Option<&MlsCheckpoint>) -> Result<()> {
    if let Some(minimum) = minimum {
        ensure!(
            protected.store_id == minimum.store_id && protected.revision >= minimum.revision,
            "MLS state rollback detected; fresh admission required"
        );
        if protected.revision == minimum.revision {
            ensure!(
                protected.checkpoint()?.digest == minimum.digest,
                "MLS state fork detected; fresh admission required"
            );
        }
    }
    Ok(())
}

mod encoded_bytes {
    use super::*;
    pub fn serialize<S: serde::Serializer>(bytes: &[u8], serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&URL_SAFE_NO_PAD.encode(bytes))
    }
    pub fn deserialize<'de, D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Vec<u8>, D::Error> {
        let value = String::deserialize(deserializer)?;
        if value.len() > MAX_SNAPSHOT * 2 {
            return Err(serde::de::Error::custom("MLS encoded bytes exceed bound"));
        }
        URL_SAFE_NO_PAD
            .decode(value)
            .map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use flow_like_device_protocol::{TelemetryMember, sign_telemetry_roster};
    use std::sync::{Arc, Mutex};

    #[derive(Default)]
    struct MemoryState {
        snapshot: Option<ProtectedSnapshot>,
        fail_commit: bool,
    }

    #[derive(Clone, Default)]
    struct MemoryBackend(Arc<Mutex<MemoryState>>);

    impl SnapshotBackend for MemoryBackend {
        fn transact<R>(
            &mut self,
            operation: impl FnOnce(Option<ProtectedSnapshot>) -> Result<(ProtectedSnapshot, R)>,
        ) -> Result<R> {
            let mut state = self.0.lock().unwrap();
            let (snapshot, result) = operation(state.snapshot.clone())?;
            if std::mem::take(&mut state.fail_commit) {
                bail!("injected commit failure");
            }
            state.snapshot = Some(snapshot);
            Ok(result)
        }
    }

    struct Fixture {
        owner: SigningKey,
        publisher_key: SigningKey,
        reader_key: SigningKey,
        publisher: TelemetryMember,
        reader: TelemetryMember,
        now: i64,
    }

    impl Fixture {
        fn new() -> Self {
            let publisher_key = SigningKey::generate();
            let reader_key = SigningKey::generate();
            Self {
                owner: SigningKey::generate(),
                publisher: TelemetryMember {
                    endpoint_id: "device-publisher".into(),
                    signing_key: publisher_key.public_key(),
                },
                reader: TelemetryMember {
                    endpoint_id: "browser-reader".into(),
                    signing_key: reader_key.public_key(),
                },
                publisher_key,
                reader_key,
                now: std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_secs() as i64,
            }
        }

        fn pins(&self, local: &TelemetryMember) -> MlsPins {
            MlsPins {
                device_id: "device-test".into(),
                scope: "placement-test".into(),
                owner_invitation_key: self.owner.public_key(),
                publisher: member(&self.publisher).unwrap(),
                local: member(local).unwrap(),
            }
        }

        fn policy(
            &self,
            version: u64,
            previous: Option<&str>,
            readers: &[TelemetryMember],
        ) -> String {
            let mut members = vec![self.publisher.clone()];
            members.extend_from_slice(readers);
            sign_telemetry_roster(
                &TelemetryRoster {
                    version: 1,
                    device_id: "device-test".into(),
                    scope: "placement-test".into(),
                    policy_version: version,
                    previous_policy_digest: previous.map(compact_digest),
                    management_policy_digest: None,
                    publisher: self.publisher.clone(),
                    members,
                    issued_at: self.now - 1,
                    expires_at: self.now + 600,
                },
                &self.owner,
            )
            .unwrap()
        }

        fn pair(
            &self,
        ) -> (
            ProtectedMlsStore<MemoryBackend>,
            ProtectedMlsStore<MemoryBackend>,
            String,
        ) {
            let mut publisher = ProtectedMlsStore::create(
                MemoryBackend::default(),
                self.pins(&self.publisher),
                [41; 32],
                &self.publisher_key,
            )
            .unwrap();
            let mut reader = ProtectedMlsStore::create(
                MemoryBackend::default(),
                self.pins(&self.reader),
                [42; 32],
                &self.reader_key,
            )
            .unwrap();
            let genesis = self.policy(1, None, &[]);
            publisher
                .apply_policy("genesis", 1, &genesis, &[], self.now)
                .unwrap();
            let policy = self.policy(2, Some(&genesis), std::slice::from_ref(&self.reader));
            let package = reader.key_package().unwrap();
            assert_eq!(package, reader.key_package().unwrap());
            let admission = publisher
                .apply_policy(
                    "admit-reader",
                    2,
                    &policy,
                    &[(member(&self.reader).unwrap(), package)],
                    self.now,
                )
                .unwrap();
            reader
                .join(admission.welcome.as_ref().unwrap(), self.now)
                .unwrap();
            (publisher, reader, policy)
        }
    }

    fn assert_application(receipt: MlsReceipt, expected: &[u8]) {
        match receipt {
            MlsReceipt::Application(bytes) => assert_eq!(&bytes[..], expected),
            _ => panic!("expected application receipt"),
        }
    }

    fn restart(
        store: ProtectedMlsStore<MemoryBackend>,
        key: [u8; 32],
    ) -> ProtectedMlsStore<MemoryBackend> {
        let checkpoint = store.checkpoint();
        ProtectedMlsStore::open(store.backend, store.pins, key, checkpoint).unwrap()
    }

    #[test]
    fn publisher_only_application_has_no_recipient_delivery_obligations() {
        let fixture = Fixture::new();
        let mut publisher = ProtectedMlsStore::create(
            MemoryBackend::default(),
            fixture.pins(&fixture.publisher),
            [41; 32],
            &fixture.publisher_key,
        )
        .unwrap();
        let policy = fixture.policy(1, None, &[]);
        publisher
            .apply_policy("genesis", 1, &policy, &[], fixture.now)
            .unwrap();
        publisher
            .publish("no-readers", 2, b"sample", fixture.now)
            .unwrap();
        assert!(publisher.pending_outbox().unwrap().is_empty());
        assert_eq!(publisher.publication_position().unwrap(), (2, 2));
        assert!(
            publisher
                .publish("no-readers", 2, b"sample", fixture.now)
                .is_err()
        );
        publisher
            .publish("next", 3, b"sample", fixture.now)
            .unwrap();
        assert_eq!(publisher.publication_position().unwrap(), (3, 3));
    }

    #[test]
    fn signed_delivery_receipts_require_every_endpoint_and_survive_restart() {
        let fixture = Fixture::new();
        let (mut publisher, mut alice, previous) = fixture.pair();
        let alice_welcome = alice.signed_delivery_receipt(2).unwrap();
        assert_eq!(
            publisher
                .accept_delivery_receipt(&fixture.reader.endpoint_id, 2, &alice_welcome)
                .unwrap(),
            2
        );
        let bob_key = SigningKey::generate();
        let bob = TelemetryMember {
            endpoint_id: "bob".into(),
            signing_key: bob_key.public_key(),
        };
        let mut bob_store = ProtectedMlsStore::create(
            MemoryBackend::default(),
            fixture.pins(&bob),
            [43; 32],
            &bob_key,
        )
        .unwrap();
        let policy = fixture.policy(3, Some(&previous), &[fixture.reader.clone(), bob.clone()]);
        let commit = publisher
            .apply_policy(
                "bob-admission",
                3,
                &policy,
                &[(member(&bob).unwrap(), bob_store.key_package().unwrap())],
                fixture.now,
            )
            .unwrap();
        alice.receive(&commit.message, fixture.now).unwrap();
        bob_store
            .join(commit.welcome.as_ref().unwrap(), fixture.now)
            .unwrap();
        let alice_ack = alice.signed_delivery_receipt(3).unwrap();
        let bob_ack = bob_store.signed_delivery_receipt(3).unwrap();
        assert_eq!(
            publisher
                .accept_delivery_receipt(&fixture.reader.endpoint_id, 3, &alice_ack)
                .unwrap(),
            2
        );
        publisher = restart(publisher, [41; 32]);
        assert_eq!(publisher.pending_outbox().unwrap().len(), 1);
        assert_eq!(
            publisher
                .accept_delivery_receipt(&bob.endpoint_id, 3, &bob_ack)
                .unwrap(),
            3
        );
        assert!(publisher.pending_outbox().unwrap().is_empty());
        assert_eq!(
            publisher
                .accept_delivery_receipt(&fixture.reader.endpoint_id, 3, &alice_ack)
                .unwrap(),
            3
        );
        assert!(
            publisher
                .accept_delivery_receipt(&bob.endpoint_id, 3, &alice_ack)
                .is_err()
        );
        let mut altered =
            verify_telemetry_delivery_receipt(&alice_ack, &fixture.reader.signing_key).unwrap();
        altered.envelope_digest = compact_digest("different delivery");
        let altered = sign_telemetry_delivery_receipt(&altered, &fixture.reader_key).unwrap();
        assert!(
            publisher
                .accept_delivery_receipt(&fixture.reader.endpoint_id, 3, &altered)
                .is_err()
        );
        assert!(
            publisher
                .accept_delivery_receipt(&fixture.reader.endpoint_id, 4, &alice_ack)
                .is_err()
        );
        let sample = publisher
            .publish("sample", 4, b"metrics", fixture.now)
            .unwrap();
        alice.receive(&sample.message, fixture.now).unwrap();
        bob_store.receive(&sample.message, fixture.now).unwrap();
        let ack = alice.signed_delivery_receipt(4).unwrap();
        let before = publisher.checkpoint();
        publisher.backend.0.lock().unwrap().fail_commit = true;
        assert!(
            publisher
                .accept_delivery_receipt(&fixture.reader.endpoint_id, 4, &ack)
                .is_err()
        );
        assert_eq!(publisher.checkpoint(), before);
        assert_eq!(
            publisher
                .accept_delivery_receipt(
                    &bob.endpoint_id,
                    4,
                    &bob_store.signed_delivery_receipt(4).unwrap()
                )
                .unwrap(),
            3
        );
        assert_eq!(
            publisher
                .accept_delivery_receipt(&fixture.reader.endpoint_id, 4, &ack)
                .unwrap(),
            4
        );
    }

    #[test]
    fn removal_unblocks_full_outbox_without_charging_new_readers_for_old_messages() {
        let fixture = Fixture::new();
        let (mut publisher, _reader, previous) = fixture.pair();
        for sequence in 3..=MAX_OUTBOX as u64 {
            publisher
                .publish(
                    &format!("sample-{sequence}"),
                    sequence,
                    b"metrics",
                    fixture.now,
                )
                .unwrap();
        }
        assert!(
            publisher
                .publish("full", MAX_OUTBOX as u64 + 1, b"metrics", fixture.now)
                .is_err()
        );
        let new_key = SigningKey::generate();
        let new_member = TelemetryMember {
            endpoint_id: "new".into(),
            signing_key: new_key.public_key(),
        };
        let mut new_reader = ProtectedMlsStore::create(
            MemoryBackend::default(),
            fixture.pins(&new_member),
            [43; 32],
            &new_key,
        )
        .unwrap();
        let policy = fixture.policy(3, Some(&previous), std::slice::from_ref(&new_member));
        let next = MAX_OUTBOX as u64 + 1;
        let publication = publisher
            .apply_policy(
                "replace",
                next,
                &policy,
                &[(
                    member(&new_member).unwrap(),
                    new_reader.key_package().unwrap(),
                )],
                fixture.now,
            )
            .unwrap();
        assert_eq!(publisher.publication_position().unwrap(), (next, next - 1));
        assert_eq!(publisher.pending_outbox().unwrap().len(), 1);
        new_reader
            .join(publication.welcome.as_ref().unwrap(), fixture.now)
            .unwrap();
        assert_eq!(
            publisher
                .accept_delivery_receipt(
                    &new_member.endpoint_id,
                    next,
                    &new_reader.signed_delivery_receipt(next).unwrap()
                )
                .unwrap(),
            next
        );
    }

    #[test]
    fn reader_receipts_exist_only_after_receive_commit_and_clear_after_confirmation() {
        let fixture = Fixture::new();
        let (mut publisher, mut reader, _) = fixture.pair();
        let sample = publisher
            .publish("sample", 3, b"metrics", fixture.now)
            .unwrap();
        reader.backend.0.lock().unwrap().fail_commit = true;
        assert!(reader.receive(&sample.message, fixture.now).is_err());
        assert!(reader.signed_delivery_receipt(3).is_err());
        reader.receive(&sample.message, fixture.now).unwrap();
        reader = restart(reader, [42; 32]);
        let receipts = reader.pending_delivery_receipts().unwrap();
        assert_eq!(
            receipts.iter().map(|r| r.sequence).collect::<Vec<_>>(),
            vec![2, 3]
        );
        let before = reader.checkpoint();
        assert!(
            reader
                .confirm_delivery_receipt(3, &compact_digest("wrong"))
                .is_err()
        );
        assert_eq!(reader.checkpoint(), before);
        reader
            .confirm_delivery_receipt(3, &compact_digest(&sample.message.envelope_jws))
            .unwrap();
        assert_eq!(reader.pending_delivery_receipts().unwrap().len(), 1);
    }

    #[test]
    fn restart_preserves_outbox_ratchets_replay_and_acknowledged_sequence() {
        let fixture = Fixture::new();
        let (mut publisher, mut reader, _) = fixture.pair();
        let sent = publisher
            .publish("sample", 3, b"telemetry", fixture.now)
            .unwrap();
        publisher = restart(publisher, [41; 32]);
        let retry = publisher
            .publish("sample", 3, b"telemetry", fixture.now)
            .unwrap();
        assert_eq!(
            serde_json::to_vec(&sent).unwrap(),
            serde_json::to_vec(&retry).unwrap()
        );
        assert!(
            publisher
                .publish("sample", 3, b"changed", fixture.now)
                .is_err()
        );
        assert_application(
            reader.receive(&retry.message, fixture.now).unwrap(),
            b"telemetry",
        );
        reader = restart(reader, [42; 32]);
        assert!(matches!(
            reader.receive(&sent.message, fixture.now).unwrap(),
            MlsReceipt::Duplicate
        ));
        let next = publisher
            .publish("next", 4, b"next sample", fixture.now)
            .unwrap();
        assert_application(
            reader.receive(&next.message, fixture.now).unwrap(),
            b"next sample",
        );

        let pending = publisher.pending_outbox().unwrap();
        let checkpoint = publisher.checkpoint();
        assert!(publisher.acknowledge(1, "wrong").is_err());
        assert_eq!(publisher.checkpoint(), checkpoint);
        assert!(
            publisher
                .acknowledge(2, &compact_digest(&pending[1].message.envelope_jws))
                .is_err()
        );
        for publication in pending {
            publisher
                .acknowledge(
                    publication.sequence,
                    &compact_digest(&publication.message.envelope_jws),
                )
                .unwrap();
        }
        assert_eq!(publisher.publication_position().unwrap(), (4, 4));
        assert!(publisher.pending_outbox().unwrap().is_empty());
        assert!(
            publisher
                .publish("sample", 3, b"telemetry", fixture.now)
                .is_err()
        );
    }

    #[test]
    fn commit_failures_return_no_ciphertext_or_plaintext_and_allow_safe_retry() {
        let fixture = Fixture::new();
        let (mut publisher, mut reader, _) = fixture.pair();
        let before = publisher.checkpoint();
        publisher.backend.0.lock().unwrap().fail_commit = true;
        assert!(
            publisher
                .publish("sample", 3, b"commit marker", fixture.now)
                .is_err()
        );
        assert_eq!(publisher.checkpoint(), before);
        assert_eq!(publisher.publication_position().unwrap(), (2, 0));
        let sent = publisher
            .publish("sample", 3, b"commit marker", fixture.now)
            .unwrap();

        let before = reader.checkpoint();
        reader.backend.0.lock().unwrap().fail_commit = true;
        assert!(reader.receive(&sent.message, fixture.now).is_err());
        assert_eq!(reader.checkpoint(), before);
        assert_application(
            reader.receive(&sent.message, fixture.now).unwrap(),
            b"commit marker",
        );
        assert!(matches!(
            reader.receive(&sent.message, fixture.now).unwrap(),
            MlsReceipt::Duplicate
        ));
    }

    #[test]
    fn batched_membership_removes_old_leaf_before_next_publication() {
        let fixture = Fixture::new();
        let (mut publisher, mut reader, previous) = fixture.pair();
        let replacement_key = SigningKey::generate();
        let replacement = TelemetryMember {
            endpoint_id: "second-browser".into(),
            signing_key: replacement_key.public_key(),
        };
        let mut fresh = ProtectedMlsStore::create(
            MemoryBackend::default(),
            fixture.pins(&replacement),
            [43; 32],
            &replacement_key,
        )
        .unwrap();
        let policy = fixture.policy(3, Some(&previous), std::slice::from_ref(&replacement));
        let change = publisher
            .apply_policy(
                "replace-reader",
                3,
                &policy,
                &[(member(&replacement).unwrap(), fresh.key_package().unwrap())],
                fixture.now,
            )
            .unwrap();
        assert!(matches!(
            reader.receive(&change.message, fixture.now).unwrap(),
            MlsReceipt::Removed
        ));
        fresh
            .join(change.welcome.as_ref().unwrap(), fixture.now)
            .unwrap();
        reader = restart(reader, [42; 32]);
        assert!(reader.key_package().is_err());
        assert!(
            reader
                .join(change.welcome.as_ref().unwrap(), fixture.now)
                .is_err()
        );
        let sent = publisher
            .publish("after-removal", 4, b"only the new reader", fixture.now)
            .unwrap();
        assert!(reader.receive(&sent.message, fixture.now).is_err());
        assert_application(
            fresh.receive(&sent.message, fixture.now).unwrap(),
            b"only the new reader",
        );
    }

    #[test]
    fn unauthorized_forked_and_expired_policies_do_not_advance_membership() {
        let fixture = Fixture::new();
        let (mut publisher, _, previous) = fixture.pair();
        let next = fixture.policy(3, Some(&previous), std::slice::from_ref(&fixture.reader));
        let parsed =
            verify_telemetry_roster(&next, &fixture.owner.public_key(), fixture.now).unwrap();
        let forged = sign_telemetry_roster(&parsed, &SigningKey::generate()).unwrap();
        let wrong_predecessor = fixture.policy(
            3,
            Some("different predecessor"),
            std::slice::from_ref(&fixture.reader),
        );
        let before = publisher.checkpoint();
        for policy in [&forged, &wrong_predecessor, &previous] {
            assert!(
                publisher
                    .apply_policy("rejected", 3, policy, &[], fixture.now)
                    .is_err()
            );
            assert_eq!(publisher.checkpoint(), before);
        }
        assert!(
            publisher
                .publish("expired", 3, b"expired", fixture.now + 601)
                .is_err()
        );
        assert_eq!(publisher.checkpoint(), before);
        let renewal = publisher
            .apply_policy("renewal", 3, &next, &[], fixture.now)
            .unwrap();
        assert_eq!(renewal.sequence, 3);
    }

    #[test]
    fn envelope_forgery_gaps_and_wrong_policy_leave_receive_ratchets_intact() {
        let fixture = Fixture::new();
        let (mut publisher, mut reader, previous) = fixture.pair();
        let sent = publisher
            .publish("sample", 3, b"bound telemetry", fixture.now)
            .unwrap();
        let future = publisher
            .publish("future", 4, b"future", fixture.now)
            .unwrap();
        let mut changed_wire = sent.message.clone();
        changed_wire.wire[0] ^= 1;
        let mut changed_policy = sent.message.clone();
        changed_policy.policy_jws =
            fixture.policy(3, Some(&previous), std::slice::from_ref(&fixture.reader));
        let mut changed_scope = sent.message.clone();
        let mut envelope = verify_telemetry_envelope(
            &sent.message.envelope_jws,
            &fixture.publisher_key.public_key(),
        )
        .unwrap();
        envelope.scope = "another-placement".into();
        changed_scope.envelope_jws =
            sign_telemetry_envelope(&envelope, &fixture.publisher_key).unwrap();
        let mut changed_signer = sent.message.clone();
        envelope.scope = "placement-test".into();
        changed_signer.envelope_jws =
            sign_telemetry_envelope(&envelope, &fixture.reader_key).unwrap();
        let before = reader.checkpoint();
        for delivery in [
            &changed_wire,
            &changed_policy,
            &changed_scope,
            &changed_signer,
            &future.message,
        ] {
            assert!(reader.receive(delivery, fixture.now).is_err());
            assert_eq!(reader.checkpoint(), before);
        }
        assert_application(
            reader.receive(&sent.message, fixture.now).unwrap(),
            b"bound telemetry",
        );
        assert_application(
            reader.receive(&future.message, fixture.now).unwrap(),
            b"future",
        );
    }

    #[test]
    fn rollback_wrong_key_and_snapshot_substitution_fail_closed() {
        let fixture = Fixture::new();
        let (mut publisher, _, _) = fixture.pair();
        let backend = publisher.backend.clone();
        let old = backend.0.lock().unwrap().snapshot.clone();
        publisher
            .publish("sample", 3, b"new epoch state", fixture.now)
            .unwrap();
        let witness = publisher.checkpoint();
        let current = backend.0.lock().unwrap().snapshot.clone();
        backend.0.lock().unwrap().snapshot = old;
        assert!(
            ProtectedMlsStore::open(
                backend.clone(),
                fixture.pins(&fixture.publisher),
                [41; 32],
                witness.clone()
            )
            .is_err()
        );
        backend.0.lock().unwrap().snapshot = current;
        assert!(
            ProtectedMlsStore::open(
                backend.clone(),
                fixture.pins(&fixture.publisher),
                [99; 32],
                witness.clone()
            )
            .is_err()
        );
        let mut wrong_scope = fixture.pins(&fixture.publisher);
        wrong_scope.scope = "other-placement".into();
        assert!(
            ProtectedMlsStore::open(backend.clone(), wrong_scope, [41; 32], witness.clone())
                .is_err()
        );
        backend
            .0
            .lock()
            .unwrap()
            .snapshot
            .as_mut()
            .unwrap()
            .ciphertext[25] ^= 1;
        assert!(
            ProtectedMlsStore::open(
                backend.clone(),
                fixture.pins(&fixture.publisher),
                [41; 32],
                witness
            )
            .is_err()
        );
        assert!(
            ProtectedMlsStore::create(
                backend,
                fixture.pins(&fixture.publisher),
                [41; 32],
                &fixture.publisher_key
            )
            .is_err()
        );
    }

    #[cfg(feature = "sqlite")]
    #[test]
    fn sqlite_guard_failure_before_commit_preserves_ratchets_and_outbox() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let fixture = Fixture::new();
        let directory = tempfile::tempdir().unwrap();
        let path = directory
            .path()
            .canonicalize()
            .unwrap()
            .join("management.sqlite");
        let management = Connection::open(&path).unwrap();
        management
            .pragma_update(None, "application_id", 0x464c5341)
            .unwrap();
        let calls = Arc::new(AtomicUsize::new(0));
        let fail_at = Arc::new(AtomicUsize::new(usize::MAX));
        let guard_calls = calls.clone();
        let guard_fail_at = fail_at.clone();
        let pins = fixture.pins(&fixture.publisher);
        let backend = SqliteSnapshotBackend::open(&path, &pins)
            .unwrap()
            .with_transaction_guard(Box::new(move |connection| {
                ensure!(
                    !connection.is_autocommit(),
                    "Authorization ran outside the snapshot transaction"
                );
                let count = guard_calls.fetch_add(1, Ordering::SeqCst) + 1;
                ensure!(
                    count != guard_fail_at.load(Ordering::SeqCst),
                    "Authorization expired before commit"
                );
                Ok(())
            }));
        let mut publisher =
            ProtectedMlsStore::create(backend, pins, [41; 32], &fixture.publisher_key).unwrap();
        publisher
            .apply_policy(
                "genesis",
                1,
                &fixture.policy(1, None, &[]),
                &[],
                fixture.now,
            )
            .unwrap();
        let checkpoint = publisher.checkpoint();
        let stored: Vec<u8> = management
            .query_row("SELECT snapshot FROM mls_protected_snapshots", [], |row| {
                row.get(0)
            })
            .unwrap();
        fail_at.store(calls.load(Ordering::SeqCst) + 2, Ordering::SeqCst);
        assert!(
            publisher
                .publish("sample", 2, b"must not escape before commit", fixture.now)
                .is_err()
        );
        assert_eq!(publisher.checkpoint(), checkpoint);
        assert_eq!(
            management
                .query_row("SELECT snapshot FROM mls_protected_snapshots", [], |row| {
                    row.get::<_, Vec<u8>>(0)
                })
                .unwrap(),
            stored
        );
        fail_at.store(usize::MAX, Ordering::SeqCst);
        assert_eq!(
            publisher
                .publish("sample", 2, b"must not escape before commit", fixture.now)
                .unwrap()
                .sequence,
            2
        );
        assert_eq!(publisher.publication_position().unwrap().0, 2);
    }

    #[cfg(feature = "sqlite")]
    #[test]
    fn sqlite_shares_management_database_without_plaintext_and_serializes_writers() {
        let fixture = Fixture::new();
        let directory = tempfile::tempdir().unwrap();
        let path = directory
            .path()
            .canonicalize()
            .unwrap()
            .join("management.sqlite");
        let management = Connection::open(&path).unwrap();
        management
            .pragma_update(None, "application_id", 0x464c5341)
            .unwrap();
        management.pragma_update(None, "user_version", 77).unwrap();
        let before_version: i64 = Connection::open(&path)
            .unwrap()
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .unwrap();
        let pins = fixture.pins(&fixture.publisher);
        let mut publisher = ProtectedMlsStore::create(
            SqliteSnapshotBackend::open(&path, &pins).unwrap(),
            pins.clone(),
            [41; 32],
            &fixture.publisher_key,
        )
        .unwrap();
        let initial = publisher.checkpoint();
        let mut second = ProtectedMlsStore::open(
            SqliteSnapshotBackend::open(&path, &pins).unwrap(),
            pins,
            [41; 32],
            initial,
        )
        .unwrap();
        let mut reader = ProtectedMlsStore::create(
            MemoryBackend::default(),
            fixture.pins(&fixture.reader),
            [42; 32],
            &fixture.reader_key,
        )
        .unwrap();
        let package = reader.key_package().unwrap();
        let packages = [(member(&fixture.reader).unwrap(), package)];
        let policy = fixture.policy(1, None, std::slice::from_ref(&fixture.reader));
        publisher
            .apply_policy("genesis", 1, &policy, &packages, fixture.now)
            .unwrap();
        assert!(
            second
                .apply_policy("different-writer", 1, &policy, &packages, fixture.now)
                .is_err()
        );
        let marker = b"private-telemetry-database-marker-78143";
        let sent = second.publish("telemetry", 2, marker, fixture.now).unwrap();
        let retry = publisher
            .publish("telemetry", 2, marker, fixture.now)
            .unwrap();
        assert_eq!(sent.message.wire, retry.message.wire);
        let connection = Connection::open(&path).unwrap();
        let after_version: i64 = connection
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .unwrap();
        assert_eq!(before_version, after_version);
        let raw: Vec<u8> = connection
            .query_row("SELECT snapshot FROM mls_protected_snapshots", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert!(!raw.windows(marker.len()).any(|part| part == marker));
        assert!(
            !raw.windows(policy.len())
                .any(|part| part == policy.as_bytes())
        );
        assert!(
            !raw.windows(32)
                .any(|part| part == fixture.publisher_key.to_bytes())
        );
        let encoded_seed = URL_SAFE_NO_PAD.encode(fixture.publisher_key.to_bytes());
        assert!(
            !raw.windows(encoded_seed.len())
                .any(|part| part == encoded_seed.as_bytes())
        );
    }
}
