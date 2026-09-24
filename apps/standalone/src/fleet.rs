use crate::{
    enrollment::{DeviceSession, unix_time},
    state::StateStore,
};
use anyhow::{Context, Result, ensure};
use flow_like_device_crypto::fleet::seal_fleet;
use flow_like_device_protocol::*;
use rand_core::RngCore;
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};
use tokio_util::sync::CancellationToken;

pub(crate) fn migrate(db: &Connection) -> Result<()> {
    db.execute_batch("CREATE TABLE fleet_publication (singleton INTEGER PRIMARY KEY CHECK(singleton=1),sequence INTEGER NOT NULL,digest TEXT,pending TEXT); INSERT INTO fleet_publication VALUES(1,0,NULL,NULL); CREATE TABLE fleet_published_streams (stream TEXT PRIMARY KEY,content_digest TEXT NOT NULL,published_at INTEGER NOT NULL);")?;
    Ok(())
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Pending {
    bundle: EncryptedFleetSnapshot,
    stream: String,
    content_digest: String,
    observed_at: i64,
}
fn stream(reader: &FleetReader, audience: &FleetAudience) -> Result<String> {
    Ok(fleet_digest(&serde_json::to_vec(&(
        &reader.user_id,
        &reader.controller_key,
        &audience.grant_id,
        &audience.scope,
        audience.kind,
    ))?))
}
fn current_audiences(
    store: &StateStore,
    device: &OnboardingManifest,
    recipients: &FleetRecipients,
    now: i64,
) -> Result<Vec<(FleetReader, String, FleetAudience)>> {
    ensure!(
        recipients.readers.len() <= MAX_FLEET_READERS,
        "Fleet recipients exceed device limit"
    );
    let policy = recipients
        .policy_jws
        .as_ref()
        .map(|jws| {
            verify_management_policy(jws, &device.owner_invitation_key, now).map(|p| (jws, p))
        })
        .transpose()?;
    if let Some((compact, _)) = &policy {
        ensure!(
            store.management_policy_head()?.map(|(_, digest)| digest)
                == Some(compact_digest(compact)),
            "Fleet publication waits for current locally accepted management policy"
        );
    } else {
        ensure!(
            store
                .management_policy(&device.owner_invitation_key, now)
                .ok()
                .flatten()
                .is_none(),
            "Fleet server omitted current management policy"
        );
    }
    let mut values = Vec::new();
    for compact in &recipients.readers {
        let reader = inspect_fleet_reader(compact, now)?;
        for audience in fleet_audiences(
            &reader,
            compact,
            device,
            policy.as_ref().map(|(j, p)| (j.as_str(), p)),
            now,
        ) {
            values.push((reader.clone(), compact.clone(), audience));
        }
    }
    ensure!(
        values.len() <= MAX_FLEET_STREAMS,
        "Fleet audience limit reached; reduce overlapping reader grants"
    );
    Ok(values)
}
fn head(store: &StateStore) -> Result<FleetHead> {
    Ok(store.connection.query_row(
        "SELECT sequence,digest FROM fleet_publication WHERE singleton=1",
        [],
        |r| {
            Ok(FleetHead {
                sequence: r.get(0)?,
                manifest_digest: r.get(1)?,
            })
        },
    )?)
}
fn pending(store: &StateStore) -> Result<Option<Pending>> {
    let encoded: Option<String> = store.connection.query_row(
        "SELECT pending FROM fleet_publication WHERE singleton=1",
        [],
        |r| r.get(0),
    )?;
    encoded
        .map(|v| serde_json::from_str(&v).map_err(Into::into))
        .transpose()
}
fn acknowledge(
    store: &StateStore,
    pending: &Pending,
    receipt: &FleetHead,
    device_key: &Ed25519PublicKey,
) -> Result<()> {
    let manifest = verify_fleet_snapshot(&pending.bundle, device_key)?;
    ensure!(
        receipt.sequence == manifest.sequence
            && receipt.manifest_digest == Some(compact_digest(&pending.bundle.manifest_jws)),
        "Fleet acknowledgement does not match durable ciphertext"
    );
    let tx = store.connection.unchecked_transaction()?;
    ensure!(tx.execute("UPDATE fleet_publication SET sequence=?1,digest=?2,pending=NULL WHERE singleton=1 AND sequence=?3 AND pending=?4",params![receipt.sequence,receipt.manifest_digest,manifest.sequence-1,serde_json::to_string(pending)?])?==1,"Fleet publication changed concurrently");
    tx.execute("INSERT INTO fleet_published_streams(stream,content_digest,published_at) VALUES(?1,?2,?3) ON CONFLICT(stream) DO UPDATE SET content_digest=excluded.content_digest,published_at=excluded.published_at",params![pending.stream,pending.content_digest,pending.observed_at])?;
    tx.commit()?;
    Ok(())
}
fn queue(
    store: &StateStore,
    device: &DeviceSession,
    boot: &str,
    reader: &FleetReader,
    audience: FleetAudience,
    value: serde_json::Value,
    now: i64,
) -> Result<Pending> {
    let head = head(store)?;
    let stream = stream(reader, &audience)?;
    let content_digest = content_digest(&value)?;
    let snapshot = FleetSnapshot {
        version: 1,
        observed_at: now,
        boot_id: boot.into(),
        inspection: (audience.kind == FleetKind::Status).then(|| value.clone()),
        metrics: (audience.kind == FleetKind::Metrics).then_some(value),
    };
    let manifest = FleetManifest {
        version: 1,
        device_id: device.manifest().device_id.clone(),
        api_base_url: device.manifest().api_base_url.clone(),
        user_id: reader.user_id.clone(),
        controller_key: reader.controller_key.clone(),
        audience,
        sequence: head.sequence + 1,
        previous_digest: head.manifest_digest,
        boot_id: boot.into(),
        observed_at: now,
        encapsulation: [0; 32],
        ciphertext_digest: String::new(),
        ciphertext_size: 16,
    };
    let pending = Pending {
        bundle: seal_fleet(manifest, reader, &device.telemetry_signer(), &snapshot)?,
        stream,
        content_digest,
        observed_at: now,
    };
    ensure!(store.connection.execute("UPDATE fleet_publication SET pending=?1 WHERE singleton=1 AND pending IS NULL AND sequence=?2",params![serde_json::to_string(&pending)?,head.sequence])?==1,"Fleet publication already pending");
    Ok(pending)
}
fn content_digest(value: &serde_json::Value) -> Result<String> {
    let mut value = value.clone();
    if let Some(object) = value.as_object_mut() {
        object.remove("observed_at");
    }
    Ok(fleet_digest(&serde_json::to_vec(&value)?))
}
fn due(store: &StateStore, stream: &str, kind: FleetKind, digest: &str, now: i64) -> Result<bool> {
    let old: Option<(String, i64)> = store
        .connection
        .query_row(
            "SELECT content_digest,published_at FROM fleet_published_streams WHERE stream=?1",
            [stream],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    Ok(old.is_none_or(|(old, at)| {
        at > now
            || now - at >= if kind == FleetKind::Metrics { 30 } else { 60 }
            || kind == FleetKind::Status && old != digest && now - at >= 5
    }))
}

/// One durable ciphertext bounds publication during outages. New samples replace
/// unsent observations only after the previous publication has a definite outcome.
pub async fn publish(
    root: PathBuf,
    device: Arc<DeviceSession>,
    boot_id: String,
    cancel: CancellationToken,
) -> Result<()> {
    let mut recipients = None::<(FleetRecipients, i64)>;
    let mut failures = 0u32;
    loop {
        let delay = if failures == 0 {
            5
        } else {
            (5u64.saturating_mul(1u64 << failures.min(6))).min(300)
        };
        let jitter = u64::from(rand_core::OsRng.next_u32()) % 3;
        tokio::select! {_=cancel.cancelled()=>return Ok(()),_=tokio::time::sleep(Duration::from_secs(delay+jitter))=>()}
        let now = unix_time()?;
        let store = StateStore::open(&root.join("management.sqlite"))?;
        if recipients.as_ref().is_none_or(|(_, at)| now - *at >= 30) || pending(&store)?.is_some() {
            match tokio::select! {_=cancel.cancelled()=>return Ok(()),result=device.fleet_recipients()=>result}
            {
                Ok(current) => recipients = Some((current, now)),
                Err(_) => {
                    failures = failures.saturating_add(1);
                    tracing::debug!("Fleet snapshot publication is waiting for the hub");
                    continue;
                }
            }
        }
        let current = &mut recipients
            .as_mut()
            .context("Fleet recipients unavailable")?
            .0;
        let audiences = match current_audiences(&store, device.manifest(), current, now) {
            Ok(a) => a,
            Err(_) => {
                failures = failures.saturating_add(1);
                recipients = None;
                tracing::warn!(
                    "Fleet snapshot publication awaits current signed readers and policy"
                );
                continue;
            }
        };
        let result = publish_pass(
            &root, store, &device, &boot_id, current, &audiences, now, &cancel,
        )
        .await;
        match result {
            Ok(()) => failures = 0,
            Err(_) => {
                failures = failures.saturating_add(1);
                recipients = None;
                tracing::warn!(
                    "Fleet snapshot publication paused; durable pending snapshot retained"
                );
            }
        }
    }
}
#[allow(clippy::too_many_arguments)]
async fn publish_pass(
    root: &Path,
    store: StateStore,
    device: &DeviceSession,
    boot: &str,
    current: &mut FleetRecipients,
    audiences: &[(FleetReader, String, FleetAudience)],
    now: i64,
    cancel: &CancellationToken,
) -> Result<()> {
    let key = device.telemetry_signer().public_key();
    if let Some(pending) = pending(&store)? {
        let manifest = verify_fleet_snapshot(&pending.bundle, &key)?;
        if current.head.sequence == manifest.sequence
            && current.head.manifest_digest == Some(compact_digest(&pending.bundle.manifest_jws))
        {
            acknowledge(&store, &pending, &current.head, &key)?;
        } else {
            let local = head(&store)?;
            ensure!(
                current.head.sequence == local.sequence
                    && current.head.manifest_digest == local.manifest_digest,
                "Fleet server publication head differs from the device's durable head"
            );
            if audiences.iter().any(|(r, _, a)| {
                r.user_id == manifest.user_id
                    && r.controller_key == manifest.controller_key
                    && a == &manifest.audience
            }) {
                let ack = tokio::select! {_=cancel.cancelled()=>return Ok(()),result=device.upload_fleet(&pending.bundle)=>result}?;
                acknowledge(&store, &pending, &ack, &key)?;
                current.head = ack;
            } else {
                // The server attests the old head and this reader no longer has
                // authority. Never send that ciphertext after revocation.
                store.connection.execute(
                    "UPDATE fleet_publication SET pending=NULL WHERE singleton=1 AND pending=?1",
                    [serde_json::to_string(&pending)?],
                )?;
            }
        }
    }
    let local = head(&store)?;
    ensure!(
        current.head.sequence == local.sequence
            && current.head.manifest_digest == local.manifest_digest,
        "Fleet publication head requires the original device database"
    );
    let allowed: Vec<String> = audiences
        .iter()
        .map(|(r, _, a)| stream(r, a))
        .collect::<Result<_>>()?;
    let published_at: BTreeMap<String, i64> = store
        .connection
        .prepare("SELECT stream,published_at FROM fleet_published_streams")?
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<Result<_, _>>()?;
    for stream in published_at.keys() {
        if !allowed.contains(stream) {
            store.connection.execute(
                "DELETE FROM fleet_published_streams WHERE stream=?1",
                [stream],
            )?;
        }
    }
    // Serve unpublished and longest-waiting streams first. The durable ACK
    // timestamp preserves fairness after restart, even when early scopes flap.
    let mut ordered: Vec<_> = audiences
        .iter()
        .zip(allowed)
        .map(|(audience, id)| {
            let at = published_at.get(&id).copied().filter(|at| *at <= now);
            (at, id, audience)
        })
        .collect();
    ordered.sort_unstable_by(|left, right| (&left.0, &left.1).cmp(&(&right.0, &right.1)));
    let mut projections = BTreeMap::new();
    let mut published = 0;
    for (_, stream_id, (reader, _, audience)) in ordered {
        if published >= 32 {
            break;
        }
        // Metrics have a fixed interval, so no database projection is needed
        // until this recipient's next sample is due.
        if audience.kind == FleetKind::Metrics
            && !due(&store, &stream_id, FleetKind::Metrics, "", now)?
        {
            continue;
        }
        // Do not regenerate a scope's database snapshot for each recipient.
        let projection_key = format!(
            "{}:{:?}",
            inventory_scope_key(&audience.scope)?,
            audience.kind
        );
        if !projections.contains_key(&projection_key) {
            let projection =
                crate::operational::fleet_snapshot(root, &audience.scope, audience.kind, boot, now);
            if projection.is_err() {
                tracing::warn!(
                    "Fleet snapshot scope is unavailable or exceeds its complete payload limit"
                );
            }
            projections.insert(projection_key.clone(), projection.ok());
        }
        let Some(value) = &projections[&projection_key] else {
            // No ciphertext was queued for this scope. Its previous full
            // snapshot remains stale while other authorized streams progress.
            continue;
        };
        if !due(
            &store,
            &stream_id,
            audience.kind,
            &content_digest(value)?,
            now,
        )? {
            continue;
        }
        let pending = queue(
            &store,
            device,
            boot,
            reader,
            audience.clone(),
            value.clone(),
            now,
        )?;
        let ack = tokio::select! {_=cancel.cancelled()=>return Ok(()),result=device.upload_fleet(&pending.bundle)=>result}?;
        acknowledge(&store, &pending, &ack, &key)?;
        current.head = ack;
        published += 1;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> Result<(tempfile::TempDir, DeviceSession, FleetReader, FleetAudience)> {
        let root = tempfile::tempdir()?;
        let controller = SigningKey::generate();
        let device = DeviceSession::test_management_session(
            "https://hub.example/api/v1".into(),
            "device".into(),
            SigningKey::generate(),
            controller.public_key(),
        );
        let now = unix_time()?;
        let reader = FleetReader {
            version: 1,
            device_id: "device".into(),
            api_base_url: device.manifest().api_base_url.clone(),
            user_id: "owner".into(),
            controller_key: controller.public_key(),
            archive_key: [42; 32],
            revision: 1,
            issued_at: now,
            expires_at: now + 600,
        };
        let compact = sign_fleet_reader(&reader, &controller)?;
        let audience = fleet_audiences(&reader, &compact, device.manifest(), None, now).remove(0);
        Ok((root, device, reader, audience))
    }
    #[tokio::test]
    async fn lost_ack_reconciles_exact_durable_ciphertext_after_restart_without_network()
    -> Result<()> {
        let (root, device, reader, audience) = fixture()?;
        let now = unix_time()?;
        let store = StateStore::open(&root.path().join("management.sqlite"))?;
        let queued = queue(
            &store,
            &device,
            "boot",
            &reader,
            audience,
            serde_json::json!({"placements":[]}),
            now,
        )?;
        assert!(
            queue(
                &store,
                &device,
                "boot",
                &reader,
                verify_fleet_snapshot(&queued.bundle, &device.telemetry_signer().public_key())?
                    .audience,
                serde_json::json!({"placements":[]}),
                now
            )
            .is_err()
        );
        let serialized = serde_json::to_string(&queued)?;
        assert!(!serialized.contains("placements"));
        drop(store);
        let store = StateStore::open(&root.path().join("management.sqlite"))?;
        assert_eq!(
            serde_json::to_string(&pending(&store)?.unwrap())?,
            serialized
        );
        let mut recipients = FleetRecipients {
            readers: vec![],
            policy_jws: None,
            head: FleetHead {
                sequence: 1,
                manifest_digest: Some(compact_digest(&queued.bundle.manifest_jws)),
            },
        };
        publish_pass(
            root.path(),
            store,
            &device,
            "boot-2",
            &mut recipients,
            &[],
            now,
            &CancellationToken::new(),
        )
        .await?;
        let store = StateStore::open(&root.path().join("management.sqlite"))?;
        assert!(pending(&store)?.is_none());
        assert_eq!(head(&store)?.sequence, 1);
        Ok(())
    }
    #[tokio::test]
    async fn revoked_reader_drops_only_definitely_unsent_snapshot_and_keeps_chain_head()
    -> Result<()> {
        let (root, device, reader, audience) = fixture()?;
        let now = unix_time()?;
        let store = StateStore::open(&root.path().join("management.sqlite"))?;
        queue(
            &store,
            &device,
            "boot",
            &reader,
            audience,
            serde_json::json!({"placements":[]}),
            now,
        )?;
        let mut recipients = FleetRecipients {
            readers: vec![],
            policy_jws: None,
            head: FleetHead::default(),
        };
        publish_pass(
            root.path(),
            store,
            &device,
            "boot",
            &mut recipients,
            &[],
            now,
            &CancellationToken::new(),
        )
        .await?;
        let store = StateStore::open(&root.path().join("management.sqlite"))?;
        assert!(pending(&store)?.is_none());
        assert_eq!(head(&store)?.sequence, 0);
        Ok(())
    }
    #[test]
    fn status_change_detection_excludes_observation_clock_and_metrics_have_fixed_cadence()
    -> Result<()> {
        let (root, _, _, _) = fixture()?;
        let store = StateStore::open(&root.path().join("management.sqlite"))?;
        let first = serde_json::json!({"observed_at":1000,"placements":[]});
        let second = serde_json::json!({"observed_at":2000,"placements":[]});
        let digest = content_digest(&first)?;
        assert_eq!(digest, content_digest(&second)?);
        store.connection.execute(
            "INSERT INTO fleet_published_streams VALUES('stream',?1,100)",
            [&digest],
        )?;
        assert!(!due(&store, "stream", FleetKind::Status, &digest, 159)?);
        assert!(due(&store, "stream", FleetKind::Status, &digest, 160)?);
        assert!(!due(&store, "stream", FleetKind::Metrics, "changed", 129)?);
        assert!(due(&store, "stream", FleetKind::Metrics, "changed", 130)?);
        Ok(())
    }
    #[tokio::test]
    async fn publication_budget_is_fair_after_restart_and_isolates_oversized_scopes() -> Result<()>
    {
        use axum::{Json, Router, extract::State, routing::post};
        use std::sync::Mutex;
        #[derive(Clone)]
        struct Hub {
            telemetry: Ed25519PublicKey,
            head: Arc<Mutex<FleetHead>>,
            observed: Arc<Mutex<BTreeMap<String, Vec<i64>>>>,
        }
        async fn upload(
            State(hub): State<Hub>,
            Json(body): Json<serde_json::Value>,
        ) -> Json<FleetHead> {
            let bundle: EncryptedFleetSnapshot =
                serde_json::from_value(body["bundle"].clone()).unwrap();
            let manifest = verify_fleet_snapshot(&bundle, &hub.telemetry).unwrap();
            let mut head = hub.head.lock().unwrap();
            assert_eq!(manifest.sequence, head.sequence + 1);
            assert_eq!(manifest.previous_digest, head.manifest_digest);
            *head = FleetHead {
                sequence: manifest.sequence,
                manifest_digest: Some(compact_digest(&bundle.manifest_jws)),
            };
            hub.observed
                .lock()
                .unwrap()
                .entry(manifest.audience.grant_id)
                .or_default()
                .push(manifest.observed_at);
            Json(head.clone())
        }
        let (root, _, mut reader, template) = fixture()?;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        reader.api_base_url = format!("http://{}/api/v1", listener.local_addr()?);
        let device = DeviceSession::test_management_session(
            reader.api_base_url.clone(),
            reader.device_id.clone(),
            SigningKey::generate(),
            reader.controller_key.clone(),
        );
        let observed = Arc::new(Mutex::new(BTreeMap::<String, Vec<i64>>::new()));
        let hub = Hub {
            telemetry: device.telemetry_signer().public_key(),
            head: Arc::new(Mutex::new(FleetHead::default())),
            observed: observed.clone(),
        };
        let router = Router::new()
            .route("/api/v1/devices/{id}/fleet/snapshots", post(upload))
            .with_state(hub);
        let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        let database = root.path().join("management.sqlite");
        let mut store = StateStore::open(&database)?;
        let mut audiences = Vec::new();
        for index in 0..40 {
            let project_id = format!("project-{index:02}");
            store.upsert_placement(
                &project_id,
                &serde_json::json!({"id":project_id,"project_id":project_id,"revision":"initial"}),
                crate::state::DesiredState::Stopped,
            )?;
            let mut audience = template.clone();
            audience.grant_id = project_id.clone();
            audience.scope = ManagementScope::Project { project_id };
            audiences.push((reader.clone(), String::new(), audience));
        }
        // This full scope exceeds the projection bound. It must neither publish
        // a truncated inventory nor prevent another scope from being published.
        store.connection.execute_batch(
            "WITH RECURSIVE n(value) AS (SELECT 0 UNION ALL SELECT value+1 FROM n WHERE value<1024)
             INSERT INTO placements(id,config_json,desired_state,intent_revision,observed_state,config_revision)
             SELECT 'oversized-'||value,json_object('id','oversized-'||value,'project_id','oversized'),'stopped',1,'unknown',1 FROM n",
        )?;
        let mut oversized = template;
        oversized.grant_id = "oversized".into();
        oversized.scope = ManagementScope::Project {
            project_id: "oversized".into(),
        };
        audiences.insert(0, (reader, String::new(), oversized));
        drop(store);
        let mut recipients = FleetRecipients {
            readers: vec![],
            policy_jws: None,
            head: FleetHead::default(),
        };
        let now = unix_time()?;
        for pass in 0..=15 {
            // Reopen the database on every pass, preserving only durable ACKs.
            let store = StateStore::open(&database)?;
            store.connection.execute(
                "UPDATE placements SET intent_revision=intent_revision+1 WHERE id >= 'project-00' AND id <= 'project-31'",
                [],
            )?;
            let before = recipients.head.sequence;
            publish_pass(
                root.path(),
                store,
                &device,
                "boot",
                &mut recipients,
                &audiences,
                now + pass * 5,
                &CancellationToken::new(),
            )
            .await?;
            assert!(recipients.head.sequence - before <= 32);
            if pass == 1 {
                assert_eq!(observed.lock().unwrap().len(), 40);
            }
        }
        server.abort();
        let observed = observed.lock().unwrap();
        assert_eq!(observed.len(), 40);
        assert!(!observed.contains_key("oversized"));
        for index in 0..40 {
            let samples = &observed[&format!("project-{index:02}")];
            assert!(samples.len() >= 2, "reader {index} missed its heartbeat");
            assert!(samples.windows(2).all(|pair| pair[1] - pair[0] <= 65));
            assert!(now + 75 - samples.last().unwrap() <= 65);
        }
        Ok(())
    }
    #[tokio::test]
    async fn unattended_publisher_http_assertions_encrypt_public_inventory_without_a_controller_session()
    -> Result<()> {
        use axum::{Json, Router, extract::State, routing::post};
        use std::sync::Mutex;
        #[derive(Clone)]
        struct Hub {
            device: String,
            base: String,
            auth: Ed25519PublicKey,
            telemetry: Ed25519PublicKey,
            reader: String,
            head: Arc<Mutex<FleetHead>>,
            seen: Arc<Mutex<std::collections::HashSet<String>>>,
            delivered: tokio::sync::mpsc::UnboundedSender<EncryptedFleetSnapshot>,
        }
        impl Hub {
            fn admit(&self, body: &serde_json::Value, path: &str) {
                let proof = verify_client_assertion(
                    body["client_assertion"].as_str().unwrap(),
                    &self.auth,
                    &self.device,
                    &format!("{}/devices/{}/fleet/{path}", self.base, self.device),
                    unix_time().unwrap(),
                )
                .unwrap();
                assert!(self.seen.lock().unwrap().insert(proof.jti));
            }
        }
        async fn readers(
            State(hub): State<Hub>,
            Json(body): Json<serde_json::Value>,
        ) -> Json<FleetRecipients> {
            hub.admit(&body, "recipients");
            Json(FleetRecipients {
                readers: vec![hub.reader],
                policy_jws: None,
                head: hub.head.lock().unwrap().clone(),
            })
        }
        async fn upload(
            State(hub): State<Hub>,
            Json(body): Json<serde_json::Value>,
        ) -> Json<FleetHead> {
            hub.admit(&body, "snapshots");
            let bundle: EncryptedFleetSnapshot =
                serde_json::from_value(body["bundle"].clone()).unwrap();
            let manifest = verify_fleet_snapshot(&bundle, &hub.telemetry).unwrap();
            assert_eq!(manifest.device_id, hub.device);
            let mut head = hub.head.lock().unwrap();
            assert_eq!(manifest.sequence, head.sequence + 1);
            assert_eq!(manifest.previous_digest, head.manifest_digest);
            *head = FleetHead {
                sequence: manifest.sequence,
                manifest_digest: Some(compact_digest(&bundle.manifest_jws)),
            };
            hub.delivered.send(bundle).unwrap();
            Json(head.clone())
        }
        let directory = tempfile::tempdir()?;
        let mut store = StateStore::open(&directory.path().join("management.sqlite"))?;
        let device_id = store.device_id().to_owned();
        store.upsert_placement("rest-service",&serde_json::json!({"id":"rest-service","project_id":"project","deployment_id":"deployment","revision":"revision","variables":{"secret":"must-stay-private"}}),crate::state::DesiredState::Stopped)?;
        drop(store);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let base = format!("http://{}/api/v1", listener.local_addr()?);
        let auth = SigningKey::generate();
        let controller = SigningKey::generate();
        let archive_seed = [19; 32];
        let device = Arc::new(DeviceSession::test_management_session(
            base.clone(),
            device_id.clone(),
            SigningKey::from_bytes(&auth.to_bytes()),
            controller.public_key(),
        ));
        let now = unix_time()?;
        let declaration = FleetReader {
            version: 1,
            device_id: device_id.clone(),
            api_base_url: base.clone(),
            user_id: "owner".into(),
            controller_key: controller.public_key(),
            archive_key: x25519_dalek::x25519(archive_seed, x25519_dalek::X25519_BASEPOINT_BYTES),
            revision: 1,
            issued_at: now,
            expires_at: now + 600,
        };
        let reader_jws = sign_fleet_reader(&declaration, &controller)?;
        let (delivered, mut received) = tokio::sync::mpsc::unbounded_channel();
        let hub = Hub {
            device: device_id.clone(),
            base,
            auth: auth.public_key(),
            telemetry: device.telemetry_signer().public_key(),
            reader: reader_jws.clone(),
            head: Arc::new(Mutex::new(FleetHead::default())),
            seen: Arc::new(Mutex::new(std::collections::HashSet::new())),
            delivered,
        };
        let router = Router::new()
            .route("/api/v1/devices/{id}/fleet/recipients", post(readers))
            .route("/api/v1/devices/{id}/fleet/snapshots", post(upload))
            .with_state(hub);
        let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        let cancel = CancellationToken::new();
        let publisher = tokio::spawn(publish(
            directory.path().to_path_buf(),
            device.clone(),
            "unattended-boot".into(),
            cancel.clone(),
        ));
        // Fair publication orders streams by their hashed reader identity, so
        // metrics can precede status when the test generates a different key.
        let received = tokio::time::timeout(Duration::from_secs(15), async {
            loop {
                let bundle = received
                    .recv()
                    .await
                    .context("Publisher closed before encrypted inventory")?;
                let header =
                    verify_fleet_snapshot(&bundle, &device.telemetry_signer().public_key())?;
                if header.audience.kind == FleetKind::Status {
                    return Ok::<_, anyhow::Error>(bundle);
                }
            }
        })
        .await;
        cancel.cancel();
        let publication = publisher.await;
        server.abort();
        let bundle = received??;
        publication??;
        let header = verify_fleet_snapshot(&bundle, &device.telemetry_signer().public_key())?;
        assert_eq!(header.audience.kind, FleetKind::Status);
        let bootstrap = SigningKey::generate();
        let mut onboarding = device.manifest().clone();
        onboarding.bootstrap_key = bootstrap.public_key();
        let onboarding_jws = sign_manifest(&onboarding, &controller)?;
        let identity = DeviceIdentity {
            auth_key: auth.public_key(),
            management_key: [13; 32],
            telemetry_key: device.telemetry_signer().public_key(),
        };
        let binding = EnrollmentBinding {
            version: 1,
            enrollment_id: onboarding.enrollment_id.clone(),
            device_id: device_id.clone(),
            identity: identity.clone(),
            manifest_digest: compact_digest(&onboarding_jws),
            challenge_id: "challenge".into(),
            challenge_nonce: "fleet-http-test-random-nonce".into(),
            issued_at: now,
            expires_at: now + 60,
        };
        let receipt = DeviceReceipt {
            enrollment_id: onboarding.enrollment_id,
            device_id: device_id.clone(),
            owner_id: "owner".into(),
            name: onboarding.name,
            identity,
            manifest_jws: onboarding_jws.clone(),
            binding_jws: sign_binding(&binding, &bootstrap)?,
            registered_at: now,
            auth_epoch: 1,
        };
        let trusted = flow_like_device_crypto::fleet::FleetTrustedContext {
            api_base_url: onboarding.api_base_url,
            user_id: "owner".into(),
            onboarding_manifest_jws: onboarding_jws,
            owner_controller_key: controller.public_key(),
        };
        let view = FleetView {
            snapshots: vec![bundle.clone()],
            reader_jws,
            policy_jws: None,
        };
        let plaintext = flow_like_device_crypto::fleet::open_fleet(
            &trusted,
            &receipt,
            &view,
            &bundle,
            &controller.public_key(),
            &archive_seed,
            unix_time()?,
        )?;
        let snapshot: serde_json::Value = serde_json::from_slice(&plaintext)?;
        assert_eq!(snapshot["inspection"]["device_id"], device_id);
        assert_eq!(
            snapshot["inspection"]["placements"][0]["id"],
            "rest-service"
        );
        assert!(!std::str::from_utf8(&plaintext)?.contains("must-stay-private"));
        assert!(!serde_json::to_string(&bundle)?.contains("rest-service"));
        assert_eq!(
            StateStore::open(&directory.path().join("management.sqlite"))?.device_id(),
            device_id
        );
        Ok(())
    }
}
