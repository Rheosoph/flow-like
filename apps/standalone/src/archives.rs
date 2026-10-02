use crate::{
    diagnostics::{HistoryPause, RulesDiffer, Sealing, TaskFailure},
    enrollment::{DeviceSession, api_error_code, api_error_message, api_status, unix_time},
    state::StateStore,
    telemetry::TelemetryStore,
};
use anyhow::{Context, Result, anyhow, ensure};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use flow_like_device_crypto::archive::{ArchivePins, ArchivePosition, seal_archive};
use flow_like_device_protocol::*;
use reqwest::StatusCode;
use rusqlite::{OptionalExtension, params};
use serde_json::{Value, json};
use std::{
    future::Future,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};
use tokio_util::sync::CancellationToken;

fn kind_name(kind: &ArchiveKind) -> &'static str {
    match kind {
        ArchiveKind::Logs => "log",
        ArchiveKind::Metrics => "metrics",
    }
}

/// The roster must still be covered by the access rules the device accepted; when they
/// changed, the owner signs a new roster.
fn validate_current(
    store: &StateStore,
    device: &OnboardingManifest,
    roster: &ArchiveRoster,
    now: i64,
) -> Result<()> {
    ensure!(
        roster.device_id == device.device_id,
        "Archive device mismatch"
    );
    ensure!(
        store.management_policy_head()?.map(|(_, digest)| digest)
            == roster.management_policy_digest,
        RulesDiffer("Archive recipients require the current management policy")
    );
    let project = if roster.scope == "device" {
        None
    } else {
        Some(
            store
                .get_placement(&roster.scope)?
                .context("Unknown archive placement")?
                .config["project_id"]
                .as_str()
                .context("Missing placement project")?
                .to_string(),
        )
    };
    let policy = store.management_policy(&device.owner_invitation_key, now)?;
    ensure!(
        roster.project_id == project,
        "Archive project does not match the placement"
    );
    if let Some(policy) = &policy {
        ensure!(
            roster.expires_at <= policy.expires_at,
            RulesDiffer("Archive roster outlives management policy")
        );
    }
    let capability = match roster.kind {
        ArchiveKind::Logs => ManagementCapability::Logs,
        ArchiveKind::Metrics => ManagementCapability::Metrics,
    };
    for recipient in &roster.recipients {
        if recipient.user_id == device.owner_id {
            continue;
        }
        let policy = policy
            .as_ref()
            .ok_or(RulesDiffer("Archive recipient has no current grant"))?;
        ensure!(
            roster.expires_at <= policy.expires_at,
            RulesDiffer("Archive roster outlives management policy")
        );
        ensure!(
            policy
                .grants
                .iter()
                .any(|grant| grant.user_id == recipient.user_id
                    && grant.expires_at >= roster.expires_at
                    && grant.capabilities.contains(&capability)
                    && match &grant.scope {
                        ManagementScope::Device => true,
                        ManagementScope::Project { project_id } =>
                            project.as_ref() == Some(project_id),
                        ManagementScope::Placement {
                            project_id,
                            placement_id,
                        } => project.as_ref() == Some(project_id) && placement_id == &roster.scope,
                    }),
            RulesDiffer("Archive recipient lacks access to this telemetry scope")
        );
    }
    Ok(())
}

/// The owner signs the recipient roster. New history pauses whenever management membership changes.
pub fn apply_policy(
    store: &StateStore,
    device: &OnboardingManifest,
    compact: &str,
    now: i64,
) -> Result<Value> {
    let roster = verify_archive_roster(compact, &device.owner_invitation_key, now)?;
    validate_current(store, device, &roster, now)?;
    let kind = kind_name(&roster.kind);
    let old: Option<String> = store
        .connection
        .query_row(
            "SELECT policy_jws FROM archive_rosters WHERE scope=?1 AND kind=?2",
            params![roster.scope, kind],
            |r| r.get(0),
        )
        .optional()?;
    if let Some(old) = old {
        if old == compact {
            return Ok(json!({"policy_digest":compact_digest(compact)}));
        }
        let previous = verify_archive_roster_head(&old, &device.owner_invitation_key)?;
        ensure!(
            roster.policy_version == previous.policy_version + 1
                && roster.previous_policy_digest == Some(compact_digest(&old)),
            "Archive policy chain changed"
        );
    } else {
        ensure!(
            roster.policy_version == 1 && roster.previous_policy_digest.is_none(),
            "Archive policy genesis required"
        );
    }
    store.connection.execute("INSERT INTO archive_rosters(scope,kind,policy_jws) VALUES(?1,?2,?3) ON CONFLICT(scope,kind) DO UPDATE SET policy_jws=excluded.policy_jws",params![roster.scope,kind,compact])?;
    Ok(json!({"policy_digest":compact_digest(compact),"policy_version":roster.policy_version}))
}

fn chunk(bytes: &str, sequence: u64, offset: u32, limit: u32) -> Result<Value> {
    ensure!(
        limit > 0 && limit <= 4096 && offset as usize <= bytes.len(),
        "Invalid archive chunk bounds"
    );
    let end = (offset as usize + limit as usize).min(bytes.len());
    Ok(
        json!({"available":true,"sequence":sequence,"offset":offset,"total":bytes.len(),"digest":compact_digest(bytes),"chunk":URL_SAFE_NO_PAD.encode(&bytes.as_bytes()[offset as usize..end])}),
    )
}
pub fn read_roster(
    root: &Path,
    scope: &str,
    kind: &ArchiveKind,
    offset: u32,
    limit: u32,
) -> Result<Value> {
    let store = StateStore::open(&root.join("management.sqlite"))?;
    read_roster_from_store(&store, scope, kind, offset, limit)
}

pub(crate) fn read_roster_from_store(
    store: &StateStore,
    scope: &str,
    kind: &ArchiveKind,
    offset: u32,
    limit: u32,
) -> Result<Value> {
    let compact: Option<String> = store
        .connection
        .query_row(
            "SELECT policy_jws FROM archive_rosters WHERE scope=?1 AND kind=?2",
            params![scope, kind_name(kind)],
            |r| r.get(0),
        )
        .optional()?;
    compact
        .map(|value| chunk(&value, 0, offset, limit))
        .unwrap_or_else(|| Ok(json!({"available":false})))
}
pub fn read(
    root: &Path,
    scope: &str,
    kind: &ArchiveKind,
    sequence: u64,
    offset: u32,
    limit: u32,
) -> Result<Value> {
    let store = StateStore::open(&root.join("management.sqlite"))?;
    read_from_store(&store, scope, kind, sequence, offset, limit)
}

pub(crate) fn read_from_store(
    store: &StateStore,
    scope: &str,
    kind: &ArchiveKind,
    sequence: u64,
    offset: u32,
    limit: u32,
) -> Result<Value> {
    let bundle:Option<(u64,String)>=store.connection.query_row("SELECT sequence,bundle_json FROM archive_outbox WHERE scope=?1 AND kind=?2 AND (?3=0 OR sequence=?3) ORDER BY sequence LIMIT 1",params![scope,kind_name(kind),sequence],|r|Ok((r.get(0)?,r.get(1)?))).optional()?;
    bundle
        .map(|(sequence, value)| chunk(&value, sequence, offset, limit))
        .unwrap_or_else(|| Ok(json!({"available":false})))
}

/// A segment's encoded bundle must stay below the hub's transfer bound even with
/// the maximum recipient roster, so the plaintext budget leaves room for it.
const SEGMENT_PLAINTEXT_BYTES: usize = 48 * 1024;
const MAX_SEGMENT_BYTES: usize = 90 * 1024;
const SEGMENTS_PER_STREAM_PASS: usize = 16;
const UPLOADS_PER_STREAM_PASS: usize = 16;
const PENDING_PER_STREAM: u64 = 32;
const PENDING_SEGMENTS: u64 = 512;
const RETAINED_UPLOADED_SEGMENTS: i64 = 256;
const PUBLISH_INTERVAL: Duration = Duration::from_secs(30);

/// Waiting for a re-signed roster, a current policy or outbox capacity is
/// expected; any other sealing failure is a fault that stalls the stream.
#[derive(Debug)]
struct PublicationPaused {
    error: anyhow::Error,
    reason: Option<HistoryPause>,
}

impl std::fmt::Display for PublicationPaused {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{:#}", self.error)
    }
}

impl std::error::Error for PublicationPaused {}

fn paused(error: anyhow::Error, reason: Option<HistoryPause>) -> anyhow::Error {
    if error.downcast_ref::<rusqlite::Error>().is_some() {
        error
    } else {
        PublicationPaused { error, reason }.into()
    }
}

/// A roster that no longer verifies: past its lifetime, or damaged.
fn roster_paused(error: ProtocolError) -> anyhow::Error {
    let expired = matches!(error, ProtocolError::InvalidTime);
    paused(error.into(), expired.then_some(HistoryPause::RosterExpired))
}

/// The access rules stop a roster that is itself still valid.
fn rules_paused(error: anyhow::Error) -> anyhow::Error {
    let reason = if error.is::<RulesDiffer>() {
        Some(HistoryPause::RulesChanged)
    } else {
        matches!(
            error.downcast_ref::<ProtocolError>(),
            Some(ProtocolError::InvalidTime)
        )
        .then_some(HistoryPause::RulesExpired)
    };
    paused(error, reason)
}

/// The hub's answer that the account stores no more history: its tier keeps none, or its
/// quota is used up. Hubs that do not send the code yet are recognised by their sentence.
fn storage_refusal(error: &anyhow::Error) -> Option<HistoryPause> {
    (api_status(error) == Some(StatusCode::PAYMENT_REQUIRED)).then(|| {
        if api_error_code(error) == Some("ARCHIVE_TIER_WITHOUT_HISTORY")
            || api_error_message(error) == Some("This account tier does not store device telemetry")
        {
            HistoryPause::TierWithoutHistory
        } else {
            HistoryPause::QuotaReached
        }
    })
}

/// Whether one stream records, once the publisher has looked at it.
pub(crate) fn status(root: &Path, scope: &str, kind: &ArchiveKind) -> Option<Value> {
    crate::diagnostics::global().history_status(root, scope, kind_name(kind))
}

/// Reads whole records after `cursor` until the serialized budget is reached.
/// The first record is always included so a small budget still progresses.
fn collect_records(
    telemetry: &TelemetryStore,
    placement: Option<&str>,
    kind: &str,
    cursor: u64,
    budget: usize,
) -> Result<(Vec<Value>, u64)> {
    let mut records = Vec::new();
    let mut size = 0usize;
    let mut next = cursor;
    loop {
        let page = telemetry.read(placement, kind, next, 100)?;
        let Some(page) = page["records"].as_array().filter(|page| !page.is_empty()) else {
            return Ok((records, next));
        };
        for record in page {
            let bytes = serde_json::to_vec(record)?.len() + 1;
            if !records.is_empty() && size + bytes > budget {
                return Ok((records, next));
            }
            size += bytes;
            next = record["sequence"]
                .as_u64()
                .context("Telemetry record has no sequence")?;
            records.push(record.clone());
        }
    }
}

/// Seals the next segment of one stream; false when it is caught up.
fn seal_one(
    telemetry: &TelemetryStore,
    device: &DeviceSession,
    scope: &str,
    kind: &str,
) -> Result<bool> {
    let store = &telemetry.store;
    telemetry.transaction(|| {
        let (compact,cursor,sequence,previous,dropped):(String,u64,u64,Option<String>,u64)=store.connection.query_row("SELECT policy_jws,telemetry_cursor,sequence,manifest_digest,dropped FROM archive_rosters WHERE scope=?1 AND kind=?2",params![scope,kind],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?)))?;
        let now = unix_time()?;
        let roster = verify_archive_roster(&compact, &device.manifest().owner_invitation_key, now)
            .map_err(roster_paused)?;
        validate_current(store, device.manifest(), &roster, now).map_err(rules_paused)?;
        let (stream_pending, pending): (u64, u64) = store.connection.query_row(
            "SELECT COALESCE(SUM(scope=?1 AND kind=?2),0),COUNT(*) FROM archive_outbox WHERE uploaded=0",
            params![scope, kind],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        if stream_pending >= PENDING_PER_STREAM || pending >= PENDING_SEGMENTS {
            let full = anyhow!("Archive outbox is full for {scope}/{kind}: {stream_pending} stream and {pending} device segments await upload");
            return Err(paused(full, Some(HistoryPause::OutboxFull)));
        }
        let placement = (scope != "device").then_some(scope);
        let sequence = sequence
            .checked_add(1)
            .context("Archive sequence exhausted")?;
        let pins = ArchivePins {
            device_id: device.manifest().device_id.clone(),
            scope: scope.into(),
            kind: roster.kind.clone(),
            owner_invitation_key: device.manifest().owner_invitation_key.clone(),
            device_signing_key: device.telemetry_signer().public_key(),
        };
        let mut budget = SEGMENT_PLAINTEXT_BYTES;
        loop {
            let (records, next) = collect_records(telemetry, placement, kind, cursor, budget)?;
            if records.is_empty() && dropped == 0 {
                return Ok(false);
            }
            let count = records.len();
            let mut segment = json!({"records":records,"next":next});
            // The gap is part of the signed plaintext, so readers can tell missing
            // history from a quiet period.
            if dropped > 0 {
                segment["gap"] = json!({"after":cursor,"dropped":dropped});
            }
            let archive_id = uuid::Uuid::new_v4().to_string();
            let bundle = seal_archive(
                &pins,
                &compact,
                &device.telemetry_signer(),
                ArchivePosition {
                    archive_id: archive_id.clone(),
                    sequence,
                    previous_manifest_digest: previous.clone(),
                },
                &serde_json::to_vec(&segment)?,
                now,
            )?;
            let encoded = serde_json::to_string(&bundle)?;
            if encoded.len() > MAX_SEGMENT_BYTES {
                ensure!(
                    count > 1,
                    "Archive segment for {scope}/{kind} is {} bytes, above the {MAX_SEGMENT_BYTES} byte cloud transfer bound",
                    encoded.len()
                );
                budget /= 2;
                continue;
            }
            store.connection.execute("INSERT INTO archive_outbox(archive_id,scope,kind,sequence,bundle_json,created_at) VALUES(?1,?2,?3,?4,?5,?6)",params![archive_id,scope,kind,sequence,encoded,now])?;
            store.connection.execute("UPDATE archive_rosters SET telemetry_cursor=?3,sequence=?4,manifest_digest=?5,dropped=0 WHERE scope=?1 AND kind=?2",params![scope,kind,next,sequence,compact_digest(&bundle.manifest_jws)])?;
            store.connection.execute("DELETE FROM archive_outbox WHERE rowid IN (SELECT rowid FROM archive_outbox WHERE uploaded=1 ORDER BY rowid DESC LIMIT -1 OFFSET ?1)",[RETAINED_UPLOADED_SEGMENTS])?;
            return Ok(true);
        }
    })
}

/// Returns why the first failing stream could not seal; other streams still seal. What each
/// stream's roster allows is kept for roster reads.
fn seal_streams(
    root: &Path,
    telemetry: &TelemetryStore,
    device: &DeviceSession,
) -> Result<Option<TaskFailure>> {
    let streams: Vec<(String, String)> = telemetry
        .store
        .connection
        .prepare("SELECT scope,kind FROM archive_rosters ORDER BY scope,kind")?
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<Result<_, _>>()?;
    let mut failure = None;
    let mut outcomes = Vec::with_capacity(streams.len());
    for (scope, kind) in streams {
        let (sealing, fault) = seal_stream(telemetry, device, &scope, &kind);
        failure = failure.or(fault);
        outcomes.push((scope, kind, sealing));
    }
    crate::diagnostics::global().set_history_sealing(root, outcomes);
    Ok(failure)
}

/// Whether the stream can seal, and the fault to report when nothing expected stops it.
fn seal_stream(
    telemetry: &TelemetryStore,
    device: &DeviceSession,
    scope: &str,
    kind: &str,
) -> (Sealing, Option<TaskFailure>) {
    for _ in 0..SEGMENTS_PER_STREAM_PASS {
        match seal_one(telemetry, device, scope, kind) {
            Ok(true) => (),
            Ok(false) => break,
            Err(error) => {
                if let Some(pause) = error.downcast_ref::<PublicationPaused>() {
                    tracing::debug!(scope = %scope, kind = %kind, "Archive publication paused pending policy or outbox capacity: {error:#}");
                    return (Err(pause.reason), None);
                }
                tracing::warn!(scope = %scope, kind = %kind, "Archive sealing failed; this stream retries next pass: {error:#}");
                return (Err(None), Some(TaskFailure::classify(&error)));
            }
        }
    }
    (Ok(()), None)
}

type PendingStream = ((String, String), Vec<(String, u64, String)>);

/// The hub accepts only the next sequence of each stream, so every stream
/// uploads in its own sequence order and a stuck stream never blocks others.
fn pending_segments(store: &StateStore) -> Result<Vec<PendingStream>> {
    let streams: Vec<(String, String)> = store
        .connection
        .prepare(
            "SELECT DISTINCT scope,kind FROM archive_outbox WHERE uploaded=0 ORDER BY scope,kind",
        )?
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<Result<_, _>>()?;
    let mut statement = store.connection.prepare("SELECT archive_id,sequence,bundle_json FROM archive_outbox WHERE uploaded=0 AND scope=?1 AND kind=?2 ORDER BY sequence LIMIT ?3")?;
    streams
        .into_iter()
        .map(|stream| -> Result<PendingStream> {
            let segments = statement
                .query_map(params![stream.0, stream.1, UPLOADS_PER_STREAM_PASS], |r| {
                    Ok((r.get(0)?, r.get(1)?, r.get(2)?))
                })?
                .collect::<Result<_, _>>()?;
            Ok((stream, segments))
        })
        .collect()
}

/// A rejected segment only holds back its own stream. Any other failure means the
/// hub is unreachable or refuses the device, so the remaining streams wait for the
/// next pass instead of each waiting out a request timeout.
fn ends_upload_phase(error: &anyhow::Error) -> bool {
    !matches!(
        api_status(error),
        Some(
            StatusCode::BAD_REQUEST
                | StatusCode::CONFLICT
                | StatusCode::PAYLOAD_TOO_LARGE
                | StatusCode::UNPROCESSABLE_ENTITY
        )
    )
}

struct Uploads {
    /// Why the hub did not take every pending segment.
    failure: Option<TaskFailure>,
    /// Whether the account still stores history; `None` when no upload was answered.
    storage: Option<std::result::Result<(), HistoryPause>>,
}

impl Uploads {
    /// An answer that ends the pass. A full or absent history store is the account's
    /// state, not a fault of the publisher, so it is no task failure.
    fn ended(mut self, stream: (&str, &str, u64), error: &anyhow::Error) -> Self {
        let (scope, kind, sequence) = stream;
        if let Some(refusal) = storage_refusal(error) {
            tracing::debug!(scope = %scope, kind = %kind, sequence, "The hub stores no more history for this account; uploads wait: {error:#}");
            self.storage = Some(Err(refusal));
        } else {
            tracing::warn!(scope = %scope, kind = %kind, sequence, "Archive upload failed; every stream retries next pass: {error:#}");
            self.failure = Some(TaskFailure::classify(error));
        }
        self
    }
}

/// Returns why the hub did not take every pending segment, and what the answered uploads
/// showed about the account's history store.
async fn upload_pending<F, Fut>(
    store: &mut StateStore,
    cancel: &CancellationToken,
    mut upload: F,
) -> Result<Uploads>
where
    F: FnMut(EncryptedArchive) -> Fut,
    Fut: Future<Output = Result<()>>,
{
    let streams = pending_segments(store)?;
    let mut uploads = Uploads {
        failure: None,
        storage: streams.is_empty().then_some(Ok(())),
    };
    for ((scope, kind), segments) in streams {
        for (id, sequence, encoded) in segments {
            let bundle: EncryptedArchive = match serde_json::from_str(&encoded) {
                Ok(bundle) => bundle,
                Err(error) => {
                    tracing::warn!(scope = %scope, kind = %kind, sequence, archive_id = %id, "Pending archive segment cannot be decoded; this stream stops uploading: {error}");
                    break;
                }
            };
            let result = tokio::select! {_=cancel.cancelled()=>return Ok(uploads),result=upload(bundle)=>result};
            match result {
                Ok(()) => {
                    store.connection.execute(
                        "UPDATE archive_outbox SET uploaded=1 WHERE archive_id=?1",
                        [id],
                    )?;
                    uploads.storage = Some(Ok(()));
                }
                Err(error) if ends_upload_phase(&error) => {
                    return Ok(uploads.ended((&scope, &kind, sequence), &error));
                }
                Err(error) => {
                    tracing::warn!(scope = %scope, kind = %kind, sequence, "Archive segment rejected; this stream retries next pass: {error:#}");
                    uploads.failure.get_or_insert(TaskFailure::HubRefused);
                    break;
                }
            }
        }
    }
    Ok(uploads)
}

async fn publish_pass(
    root: &Path,
    device: &DeviceSession,
    cancel: &CancellationToken,
) -> Result<Option<TaskFailure>> {
    let mut telemetry = TelemetryStore::open(root)?;
    let sealing = seal_streams(root, &telemetry, device)?;
    let uploads = upload_pending(&mut telemetry.store, cancel, |bundle| async move {
        device.upload_archive(&bundle).await
    })
    .await?;
    if let Some(storage) = uploads.storage {
        crate::diagnostics::global().set_history_storage(root, storage.err());
    }
    Ok(uploads.failure.or(sealing))
}

pub async fn publish(
    root: PathBuf,
    device: Arc<DeviceSession>,
    cancel: CancellationToken,
) -> Result<()> {
    let mut tick = tokio::time::interval(PUBLISH_INTERVAL);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut failures = 0u32;
    loop {
        tokio::select! {_=cancel.cancelled()=>return Ok(()),_=tick.tick()=>()}
        let result = publish_pass(&root, &device, &cancel).await;
        crate::diagnostics::global().report(
            crate::diagnostics::ARCHIVE_PUBLISHER,
            match &result {
                Ok(None) => Ok(()),
                Ok(Some(failure)) => Err(*failure),
                Err(error) => Err(TaskFailure::classify(error)),
            },
        );
        match result {
            Ok(_) => failures = 0,
            Err(error) => {
                failures = failures.saturating_add(1);
                tracing::warn!(
                    failures,
                    "Archive publication pass failed; retrying with backoff: {error:#}"
                );
                if !crate::telemetry::back_off(&cancel, PUBLISH_INTERVAL, failures).await {
                    return Ok(());
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use flow_like_device_crypto::archive::open_archive;

    fn device_logs_roster(root: &Path) -> Result<(DeviceSession, ArchivePins, [u8; 32])> {
        let (device, pins, seed, _) = owned_device_logs_roster(root)?;
        Ok((device, pins, seed))
    }

    fn owned_device_logs_roster(
        root: &Path,
    ) -> Result<(DeviceSession, ArchivePins, [u8; 32], SigningKey)> {
        let owner = SigningKey::generate();
        let device = DeviceSession::test_session(
            "https://example.test/api/v1".into(),
            "device".into(),
            SigningKey::generate(),
        )
        .test_with_invitation_key(owner.public_key());
        let store = StateStore::open(&root.join("management.sqlite"))?;
        let now = unix_time()?;
        let seed = [78; 32];
        let roster = ArchiveRoster {
            version: 1,
            device_id: "device".into(),
            scope: "device".into(),
            project_id: None,
            kind: ArchiveKind::Logs,
            policy_version: 1,
            previous_policy_digest: None,
            management_policy_digest: None,
            recipients: vec![ArchiveRecipient {
                recipient_id: "owner-key".into(),
                user_id: "owner".into(),
                public_key: x25519_dalek::PublicKey::from(&x25519_dalek::StaticSecret::from(seed))
                    .to_bytes(),
            }],
            issued_at: now,
            expires_at: now + 300,
        };
        apply_policy(
            &store,
            device.manifest(),
            &sign_archive_roster(&roster, &owner)?,
            now,
        )?;
        let pins = ArchivePins {
            device_id: "device".into(),
            scope: "device".into(),
            kind: ArchiveKind::Logs,
            owner_invitation_key: owner.public_key(),
            device_signing_key: device.telemetry_signer().public_key(),
        };
        Ok((device, pins, seed, owner))
    }

    /// Why sealing paused, when it paused for an expected reason.
    fn pause(sealed: Result<bool>) -> Option<Option<HistoryPause>> {
        let error = sealed.err()?;
        Some(error.downcast_ref::<PublicationPaused>()?.reason)
    }

    /// The device's log stream, with the roster its owner signed.
    struct Stream {
        _directory: tempfile::TempDir,
        root: PathBuf,
        device: DeviceSession,
        owner: SigningKey,
        telemetry: TelemetryStore,
    }

    impl Stream {
        fn new() -> Result<Self> {
            let directory = tempfile::tempdir()?;
            let root = directory.path().canonicalize()?;
            assert!(status(&root, "device", &ArchiveKind::Logs).is_none());
            let (device, _, _, owner) = owned_device_logs_roster(&root)?;
            Ok(Self {
                telemetry: TelemetryStore::open(&root)?,
                _directory: directory,
                root,
                device,
                owner,
            })
        }

        fn status(&self) -> Option<Value> {
            status(&self.root, "device", &ArchiveKind::Logs)
        }

        /// Seals once; a roster read then reports this state and reason.
        fn observed(&self, state: &str, reason: Option<&str>) {
            seal_streams(&self.root, &self.telemetry, &self.device).unwrap();
            let found = self.status().unwrap();
            assert_eq!(
                (&found["state"], &found["reason"]),
                (&json!(state), &json!(reason))
            );
            assert!(found["since"].as_i64().is_some_and(|since| since > 0));
            assert!(status(&self.root, "device", &ArchiveKind::Metrics).is_none());
        }

        fn execute(&self, sql: &str, values: impl rusqlite::Params) {
            let connection = &self.telemetry.store.connection;
            connection.execute(sql, values).unwrap();
        }

        /// Access rules the owner signed at `issued_at`, valid for five minutes.
        fn policy(&self, issued_at: i64) -> String {
            let policy = ManagementPolicy {
                version: 1,
                device_id: "device".into(),
                policy_version: 1,
                previous_policy_digest: None,
                grants: vec![],
                issued_at,
                expires_at: issued_at + 300,
            };
            sign_management_policy(&policy, &self.owner).unwrap()
        }

        fn roster(&self, now: i64) -> ArchiveRoster {
            let connection = &self.telemetry.store.connection;
            let compact: String = connection
                .query_row("SELECT policy_jws FROM archive_rosters", [], |row| {
                    row.get(0)
                })
                .unwrap();
            verify_archive_roster(&compact, &self.owner.public_key(), now).unwrap()
        }

        /// Replaces the stored roster without the checks that accept one.
        fn install(&self, roster: &ArchiveRoster) {
            let signed = sign_archive_roster(roster, &self.owner).unwrap();
            self.execute("UPDATE archive_rosters SET policy_jws=?1", [signed]);
        }
    }

    #[test]
    fn changed_rules_outrank_a_refusing_hub_which_outranks_the_outbox_it_fills() -> Result<()> {
        let stream = Stream::new()?;
        stream.observed("recording", None);
        for sequence in 1..=PENDING_PER_STREAM {
            stream.execute(
                "INSERT INTO archive_outbox(archive_id,scope,kind,sequence,bundle_json,created_at) VALUES(?1,'device','log',?1,'{}',0)",
                [sequence],
            );
        }
        stream.observed("paused", Some("outbox_full"));
        let registry = crate::diagnostics::global();
        registry.set_history_storage(&stream.root, Some(HistoryPause::QuotaReached));
        stream.observed("paused", Some("quota_reached"));
        let now = unix_time()?;
        let (rules, owner) = (stream.policy(now), stream.owner.public_key());
        let store = &stream.telemetry.store;
        store.accept_management_policy(&rules, &owner, "device", now)?;
        stream.observed("paused", Some("rules_changed"));
        registry.set_history_storage(&stream.root, None);
        stream.execute("DELETE FROM archive_outbox", []);
        stream.observed("paused", Some("rules_changed"));
        Ok(())
    }

    #[test]
    fn ended_rules_and_rosters_pause_a_stream_until_its_roster_is_gone() -> Result<()> {
        let stream = Stream::new()?;
        let now = unix_time()?;
        let roster = stream.roster(now);
        let ended = stream.policy(now - 600);
        let digest = compact_digest(&ended);
        stream.execute(
            "INSERT INTO management_policy(singleton,policy_version,policy_digest,policy_jws) VALUES(1,1,?1,?2)",
            [&digest, &ended],
        );
        stream.install(&ArchiveRoster {
            management_policy_digest: Some(digest),
            ..roster.clone()
        });
        stream.observed("paused", Some("rules_expired"));
        stream.install(&ArchiveRoster {
            issued_at: now - 600,
            expires_at: now - 300,
            ..roster
        });
        stream.observed("paused", Some("roster_expired"));
        stream.execute("UPDATE archive_rosters SET policy_jws='damaged'", []);
        stream.observed("paused", None);
        stream.execute("DELETE FROM archive_rosters", []);
        seal_streams(&stream.root, &stream.telemetry, &stream.device)?;
        assert!(stream.status().is_none());
        Ok(())
    }

    #[test]
    fn sealing_catches_up_in_bounded_segments_and_signs_eviction_gaps() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let root = directory.path().canonicalize()?;
        let (device, pins, seed) = device_logs_roster(&root)?;
        let telemetry = TelemetryStore::open(&root)?;
        for index in 0..400 {
            telemetry.append(
                None,
                "log",
                &json!({"message":format!("{index}-{}", "x".repeat(300))}),
            )?;
        }
        seal_streams(&root, &telemetry, &device)?;
        let open = |encoded: &str| -> Result<Value> {
            Ok(serde_json::from_slice(&open_archive(
                &pins,
                &serde_json::from_str(encoded)?,
                "owner-key",
                &seed,
            )?)?)
        };
        let segments: Vec<String> = telemetry
            .store
            .connection
            .prepare("SELECT bundle_json FROM archive_outbox ORDER BY sequence")?
            .query_map([], |r| r.get(0))?
            .collect::<Result<_, _>>()?;
        assert!(segments.len() > 1);
        let mut records = 0;
        for encoded in &segments {
            assert!(encoded.len() <= MAX_SEGMENT_BYTES);
            let segment = open(encoded)?;
            assert!(segment.get("gap").is_none());
            records += segment["records"].as_array().unwrap().len();
        }
        assert_eq!(records, 400);
        assert!(!seal_one(&telemetry, &device, "device", "log")?);
        let cursor: u64 = telemetry.store.connection.query_row(
            "SELECT telemetry_cursor FROM archive_rosters WHERE scope='device' AND kind='log'",
            [],
            |r| r.get(0),
        )?;
        telemetry.store.connection.execute(
            "UPDATE archive_rosters SET dropped=7 WHERE scope='device' AND kind='log'",
            [],
        )?;
        assert!(seal_one(&telemetry, &device, "device", "log")?);
        let latest: String = telemetry.store.connection.query_row(
            "SELECT bundle_json FROM archive_outbox ORDER BY sequence DESC LIMIT 1",
            [],
            |r| r.get(0),
        )?;
        let gap = open(&latest)?;
        assert_eq!(gap["gap"], json!({"after":cursor,"dropped":7}));
        assert!(gap["records"].as_array().unwrap().is_empty());
        let dropped: u64 = telemetry.store.connection.query_row(
            "SELECT dropped FROM archive_rosters WHERE scope='device' AND kind='log'",
            [],
            |r| r.get(0),
        )?;
        assert_eq!(dropped, 0);
        assert!(!seal_one(&telemetry, &device, "device", "log")?);
        let tampered = telemetry.append(None, "log", &json!({"message":"tampered"}))?;
        telemetry.store.connection.execute(
            "UPDATE telemetry_records SET ciphertext=zeroblob(64) WHERE sequence=?1",
            [tampered],
        )?;
        let fault = seal_one(&telemetry, &device, "device", "log").unwrap_err();
        assert!(!fault.is::<PublicationPaused>());
        Ok(())
    }

    #[test]
    fn uploads_follow_each_stream_sequence_after_the_clock_steps_back() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let store = StateStore::open(&directory.path().join("management.sqlite"))?;
        for (id, scope, sequence, created_at, uploaded) in [
            ("a2", "placement", 2, 100, 0),
            ("a1", "placement", 1, 200, 0),
            ("b3", "device", 3, 50, 0),
            ("b2", "device", 2, 40, 1),
        ] {
            store.connection.execute(
                "INSERT INTO archive_outbox(archive_id,scope,kind,sequence,bundle_json,created_at,uploaded) VALUES(?1,?2,'log',?3,'{}',?4,?5)",
                params![id, scope, sequence, created_at, uploaded],
            )?;
        }
        let order: Vec<(String, Vec<String>)> = pending_segments(&store)?
            .into_iter()
            .map(|((scope, _), segments)| {
                (scope, segments.into_iter().map(|(id, _, _)| id).collect())
            })
            .collect();
        assert_eq!(
            order,
            vec![
                ("device".to_owned(), vec!["b3".to_owned()]),
                (
                    "placement".to_owned(),
                    vec!["a1".to_owned(), "a2".to_owned()]
                ),
            ]
        );
        Ok(())
    }

    async fn api_failure(status: u16) -> anyhow::Error {
        api_answer(status, "").await
    }

    async fn api_answer(status: u16, body: &str) -> anyhow::Error {
        let response = reqwest::Response::from(
            axum::http::Response::builder()
                .status(status)
                .body(body.to_owned())
                .unwrap(),
        );
        crate::enrollment::response_json::<Value>(response)
            .await
            .unwrap_err()
    }

    fn pending(store: &StateStore, id: &str, scope: &str, sequence: u64) {
        let bundle = serde_json::to_string(&EncryptedArchive {
            roster_jws: "roster".into(),
            manifest_jws: id.into(),
            ciphertext: "ciphertext".into(),
            recipient_keys: Vec::new(),
        })
        .unwrap();
        store.connection.execute(
            "INSERT INTO archive_outbox(archive_id,scope,kind,sequence,bundle_json,created_at) VALUES(?1,?2,'log',?3,?4,0)",
            params![id, scope, sequence, bundle],
        ).unwrap();
    }

    const QUOTA_REACHED: &str = r#"{"error":{"code":"PAYMENT_REQUIRED","message":"Device telemetry storage quota reached"}}"#;

    #[tokio::test]
    async fn a_refusal_to_store_history_is_told_from_other_hub_answers() {
        let tier = r#"{"error":{"code":"PAYMENT_REQUIRED","message":"This account tier does not store device telemetry"}}"#;
        let coded = r#"{"error":{"code":"ARCHIVE_TIER_WITHOUT_HISTORY","message":"No history on this plan"}}"#;
        for (status, body, refusal) in [
            (402, QUOTA_REACHED, Some(HistoryPause::QuotaReached)),
            (402, "", Some(HistoryPause::QuotaReached)),
            (402, tier, Some(HistoryPause::TierWithoutHistory)),
            (402, coded, Some(HistoryPause::TierWithoutHistory)),
            (403, tier, None),
            (503, QUOTA_REACHED, None),
        ] {
            assert_eq!(
                storage_refusal(&api_answer(status, body).await),
                refusal,
                "{status} {body}"
            );
        }
    }

    type Storage = Option<std::result::Result<(), HistoryPause>>;

    /// One upload pass over the pending segments; `answer` decides per manifest.
    async fn pass<F, Fut>(store: &mut StateStore, answer: F) -> (Option<TaskFailure>, Storage)
    where
        F: Fn(String) -> Fut,
        Fut: Future<Output = Result<()>>,
    {
        let cancel = CancellationToken::new();
        let uploads = upload_pending(store, &cancel, |bundle| answer(bundle.manifest_jws));
        let uploads = uploads.await.unwrap();
        (uploads.failure, uploads.storage)
    }

    #[tokio::test]
    async fn a_hub_without_room_for_history_pauses_it_without_failing_the_publisher() -> Result<()>
    {
        let directory = tempfile::tempdir()?;
        let mut store = StateStore::open(&directory.path().join("management.sqlite"))?;
        let idle = pass(&mut store, |_| async { Ok(()) }).await;
        assert_eq!(idle, (None, Some(Ok(()))));
        for (id, scope, sequence) in [("a1", "a", 1), ("a2", "a", 2), ("b1", "b", 1)] {
            pending(&store, id, scope, sequence);
        }
        let full = pass(&mut store, |manifest| async move {
            match manifest.as_str() {
                "a1" => Ok(()),
                "a2" => Err(api_answer(402, QUOTA_REACHED).await),
                _ => panic!("{manifest} was uploaded after the hub refused history"),
            }
        });
        assert_eq!(full.await, (None, Some(Err(HistoryPause::QuotaReached))));
        let down = pass(&mut store, |_| async {
            Err(anyhow!("error sending request: connection refused"))
        });
        assert_eq!(down.await, (Some(TaskFailure::Internal), None));
        let room = pass(&mut store, |_| async { Ok(()) }).await;
        assert_eq!(room, (None, Some(Ok(()))));
        Ok(())
    }

    #[tokio::test]
    async fn a_rejected_stream_never_blocks_others_but_an_unreachable_hub_ends_the_pass()
    -> Result<()> {
        for status in [400, 409, 413, 422] {
            assert!(!ends_upload_phase(&api_failure(status).await));
        }
        for status in [401, 402, 403, 429, 500, 503] {
            assert!(ends_upload_phase(&api_failure(status).await));
        }
        assert!(ends_upload_phase(&anyhow!("error sending request")));
        let directory = tempfile::tempdir()?;
        let mut store = StateStore::open(&directory.path().join("management.sqlite"))?;
        for (id, scope, sequence) in [
            ("a1", "a", 1),
            ("a2", "a", 2),
            ("b1", "b", 1),
            ("c1", "c", 1),
            ("c2", "c", 2),
            ("d1", "d", 1),
            ("e1", "e", 1),
        ] {
            let bundle = if id == "b1" {
                "{corrupt".to_owned()
            } else {
                serde_json::to_string(&EncryptedArchive {
                    roster_jws: "roster".into(),
                    manifest_jws: id.into(),
                    ciphertext: "ciphertext".into(),
                    recipient_keys: Vec::new(),
                })?
            };
            store.connection.execute(
                "INSERT INTO archive_outbox(archive_id,scope,kind,sequence,bundle_json,created_at) VALUES(?1,?2,'log',?3,?4,0)",
                params![id, scope, sequence, bundle],
            )?;
        }
        let mut attempts = Vec::new();
        upload_pending(&mut store, &CancellationToken::new(), |bundle| {
            attempts.push(bundle.manifest_jws.clone());
            async move {
                match bundle.manifest_jws.as_str() {
                    "a1" => Err(api_failure(409).await),
                    "d1" => Err(anyhow!("error sending request: connection refused")),
                    _ => Ok(()),
                }
            }
        })
        .await?;
        assert_eq!(attempts, ["a1", "c1", "c2", "d1"]);
        let uploaded: Vec<String> = store
            .connection
            .prepare("SELECT archive_id FROM archive_outbox WHERE uploaded=1 ORDER BY archive_id")?
            .query_map([], |r| r.get(0))?
            .collect::<Result<_, _>>()?;
        assert_eq!(uploaded, ["c1", "c2"]);
        Ok(())
    }

    #[tokio::test]
    async fn a_failing_publication_pass_backs_off_instead_of_ending_the_publisher() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let device = Arc::new(DeviceSession::test_session(
            "https://example.test/api/v1".into(),
            "device".into(),
            SigningKey::generate(),
        ));
        let cancel = CancellationToken::new();
        let publisher = tokio::spawn(publish(
            directory.path().join("missing"),
            device,
            cancel.clone(),
        ));
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert!(!publisher.is_finished());
        cancel.cancel();
        publisher.await??;
        Ok(())
    }

    #[test]
    fn retained_segments_are_encrypted_and_pause_after_membership_change() -> Result<()> {
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
        let seed = [76; 32];
        let key = x25519_dalek::PublicKey::from(&x25519_dalek::StaticSecret::from(seed)).to_bytes();
        let mut roster = ArchiveRoster {
            version: 1,
            device_id: "device".into(),
            scope: "device".into(),
            project_id: None,
            kind: ArchiveKind::Logs,
            policy_version: 1,
            previous_policy_digest: None,
            management_policy_digest: None,
            recipients: vec![ArchiveRecipient {
                recipient_id: "owner-key".into(),
                user_id: "owner".into(),
                public_key: key,
            }],
            issued_at: now,
            expires_at: now + 300,
        };
        let compact = sign_archive_roster(&roster, &owner)?;
        apply_policy(&store, device.manifest(), &compact, now)?;
        TelemetryStore::open(&root)?.append(
            None,
            "log",
            &json!({"message":"retained-private-message"}),
        )?;
        TelemetryStore::open(&root)?.append(None,"message",&json!({"version":1,"kind":"operation","source_id":"structured-operation-fixture","state":"completed"}))?;
        seal_one(&TelemetryStore::open(&root)?, &device, "device", "log")?;
        seal_one(&TelemetryStore::open(&root)?, &device, "device", "log")?;
        let bundles: i64 =
            store
                .connection
                .query_row("SELECT COUNT(*) FROM archive_outbox", [], |r| r.get(0))?;
        assert_eq!(bundles, 1);
        let encoded: String =
            store
                .connection
                .query_row("SELECT bundle_json FROM archive_outbox", [], |r| r.get(0))?;
        assert!(!encoded.contains("retained-private-message"));
        let bundle: EncryptedArchive = serde_json::from_str(&encoded)?;
        let pins = ArchivePins {
            device_id: "device".into(),
            scope: "device".into(),
            kind: ArchiveKind::Logs,
            owner_invitation_key: owner.public_key(),
            device_signing_key: device.telemetry_signer().public_key(),
        };
        assert!(
            std::str::from_utf8(&open_archive(&pins, &bundle, "owner-key", &seed)?)?
                .contains("retained-private-message")
        );
        assert!(!encoded.contains("structured-operation-fixture"));
        let plaintext = open_archive(&pins, &bundle, "owner-key", &seed)?;
        let records: Value = serde_json::from_slice(&plaintext)?;
        assert_eq!(
            records["records"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|record| record["kind"] == "message"
                    && record["data"]["source_id"] == "structured-operation-fixture")
                .count(),
            1
        );
        let policy = ManagementPolicy {
            version: 1,
            device_id: "device".into(),
            policy_version: 1,
            previous_policy_digest: None,
            grants: vec![],
            issued_at: now,
            expires_at: now + 600,
        };
        let policy = sign_management_policy(&policy, &owner)?;
        store.accept_management_policy(&policy, &owner.public_key(), "device", now)?;
        TelemetryStore::open(&root)?.append(None, "log", &json!({"message":"later"}))?;
        let stopped = seal_one(&TelemetryStore::open(&root)?, &device, "device", "log");
        assert_eq!(pause(stopped), Some(Some(HistoryPause::RulesChanged)));
        roster.policy_version = 2;
        roster.previous_policy_digest = Some(compact_digest(&compact));
        roster.management_policy_digest = Some(compact_digest(&policy));
        apply_policy(
            &store,
            device.manifest(),
            &sign_archive_roster(&roster, &owner)?,
            now,
        )?;
        seal_one(&TelemetryStore::open(&root)?, &device, "device", "log")?;
        assert_eq!(
            store
                .connection
                .query_row("SELECT COUNT(*) FROM archive_outbox", [], |r| r
                    .get::<_, u64>(0))?,
            2
        );
        roster.policy_version = 3;
        roster.previous_policy_digest = Some(compact_digest(&sign_archive_roster(
            &ArchiveRoster {
                policy_version: 2,
                ..roster.clone()
            },
            &owner,
        )?));
        roster.recipients.push(ArchiveRecipient {
            recipient_id: "unapproved".into(),
            user_id: "other".into(),
            public_key: x25519_dalek::PublicKey::from(&x25519_dalek::StaticSecret::from([77; 32]))
                .to_bytes(),
        });
        assert!(
            apply_policy(
                &store,
                device.manifest(),
                &sign_archive_roster(&roster, &owner)?,
                now
            )
            .is_err()
        );
        Ok(())
    }
}
