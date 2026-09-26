use crate::{
    enrollment::{DeviceSession, unix_time},
    state::StateStore,
    telemetry::TelemetryStore,
};
use anyhow::{Context, Result, ensure};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use flow_like_device_crypto::archive::{ArchivePins, ArchivePosition, seal_archive};
use flow_like_device_protocol::*;
use rusqlite::{OptionalExtension, params};
use serde_json::{Value, json};
use std::{
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
        "Archive recipients require the current management policy"
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
            "Archive roster outlives management policy"
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
            .context("Archive recipient has no current grant")?;
        ensure!(
            roster.expires_at <= policy.expires_at,
            "Archive roster outlives management policy"
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
            "Archive recipient lacks access to this telemetry scope"
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

fn seal_one(root: &Path, device: &DeviceSession, scope: &str, kind: &str) -> Result<()> {
    let store = StateStore::open(&root.join("management.sqlite"))?;
    let telemetry = TelemetryStore::open(root)?;
    store.connection.execute_batch("BEGIN IMMEDIATE")?;
    let result = (|| -> Result<()> {
        let (compact,cursor,sequence,previous):(String,u64,u64,Option<String>)=store.connection.query_row("SELECT policy_jws,telemetry_cursor,sequence,manifest_digest FROM archive_rosters WHERE scope=?1 AND kind=?2",params![scope,kind],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?)))?;
        let now = unix_time()?;
        let roster = verify_archive_roster(&compact, &device.manifest().owner_invitation_key, now)?;
        validate_current(&store, device.manifest(), &roster, now)?;
        let pending: u64 = store.connection.query_row(
            "SELECT COUNT(*) FROM archive_outbox WHERE uploaded=0",
            [],
            |r| r.get(0),
        )?;
        ensure!(pending < 256, "Archive outbox is full");
        let sample = telemetry.read(
            if scope == "device" { None } else { Some(scope) },
            kind,
            cursor,
            100,
        )?;
        if sample["records"].as_array().is_none_or(|r| r.is_empty()) {
            return Ok(());
        }
        let next = sample["next"]
            .as_u64()
            .context("Missing telemetry cursor")?;
        let sequence = sequence
            .checked_add(1)
            .context("Archive sequence exhausted")?;
        let archive_id = uuid::Uuid::new_v4().to_string();
        let pins = ArchivePins {
            device_id: device.manifest().device_id.clone(),
            scope: scope.into(),
            kind: roster.kind.clone(),
            owner_invitation_key: device.manifest().owner_invitation_key.clone(),
            device_signing_key: device.telemetry_signer().public_key(),
        };
        let bundle = seal_archive(
            &pins,
            &compact,
            &device.telemetry_signer(),
            ArchivePosition {
                archive_id: archive_id.clone(),
                sequence,
                previous_manifest_digest: previous,
            },
            &serde_json::to_vec(&sample)?,
            now,
        )?;
        let encoded = serde_json::to_string(&bundle)?;
        ensure!(
            encoded.len() <= 90 * 1024,
            "Archive segment exceeds cloud transfer bound"
        );
        store.connection.execute("INSERT INTO archive_outbox(archive_id,scope,kind,sequence,bundle_json,created_at) VALUES(?1,?2,?3,?4,?5,?6)",params![archive_id,scope,kind,sequence,encoded,now])?;
        store.connection.execute("UPDATE archive_rosters SET telemetry_cursor=?3,sequence=?4,manifest_digest=?5 WHERE scope=?1 AND kind=?2",params![scope,kind,next,sequence,compact_digest(&bundle.manifest_jws)])?;
        store.connection.execute("DELETE FROM archive_outbox WHERE archive_id IN (SELECT archive_id FROM archive_outbox WHERE uploaded=1 ORDER BY created_at DESC LIMIT -1 OFFSET 256)",[])?;
        Ok(())
    })();
    match result {
        Ok(()) => {
            store.connection.execute_batch("COMMIT")?;
            Ok(())
        }
        Err(error) => {
            let _ = store.connection.execute_batch("ROLLBACK");
            Err(error)
        }
    }
}

pub async fn publish(
    root: PathBuf,
    device: Arc<DeviceSession>,
    cancel: CancellationToken,
) -> Result<()> {
    let mut tick = tokio::time::interval(Duration::from_secs(30));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        tokio::select! {_=cancel.cancelled()=>return Ok(()),_=tick.tick()=>()}
        let store = StateStore::open(&root.join("management.sqlite"))?;
        let scopes: Vec<(String, String)> = store
            .connection
            .prepare("SELECT scope,kind FROM archive_rosters")?
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<Result<_, _>>()?;
        for (scope, kind) in scopes {
            if seal_one(&root, &device, &scope, &kind).is_err() {
                tracing::debug!("Archive publication paused pending policy or outbox capacity");
            }
        }
        let pending:Vec<(String,String)>=store.connection.prepare("SELECT archive_id,bundle_json FROM archive_outbox WHERE uploaded=0 ORDER BY created_at,sequence LIMIT 8")?.query_map([],|r|Ok((r.get(0)?,r.get(1)?)))?.collect::<Result<_,_>>()?;
        for (id, encoded) in pending {
            let bundle: EncryptedArchive = serde_json::from_str(&encoded)?;
            let result = tokio::select! {_=cancel.cancelled()=>return Ok(()),result=device.upload_archive(&bundle)=>result};
            if result.is_err() {
                break;
            }
            store.connection.execute(
                "UPDATE archive_outbox SET uploaded=1 WHERE archive_id=?1",
                [id],
            )?;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use flow_like_device_crypto::archive::open_archive;

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
        seal_one(&root, &device, "device", "log")?;
        seal_one(&root, &device, "device", "log")?;
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
        assert!(seal_one(&root, &device, "device", "log").is_err());
        roster.policy_version = 2;
        roster.previous_policy_digest = Some(compact_digest(&compact));
        roster.management_policy_digest = Some(compact_digest(&policy));
        apply_policy(
            &store,
            device.manifest(),
            &sign_archive_roster(&roster, &owner)?,
            now,
        )?;
        seal_one(&root, &device, "device", "log")?;
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
