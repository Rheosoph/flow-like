use crate::{enrollment::unix_time, state::StateStore, vault};
use anyhow::{Context, Result, ensure};
use chacha20poly1305::{
    KeyInit, XChaCha20Poly1305, XNonce,
    aead::{Aead, Payload},
};
use rand_core::{OsRng, RngCore};
use rusqlite::{OptionalExtension, ToSql, params};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{LazyLock, Mutex, PoisonError},
    time::{Duration, Instant},
};
use tokio::io::{AsyncRead, AsyncReadExt};
use tokio_util::sync::CancellationToken;
use zeroize::Zeroizing;

pub(crate) mod resources;

const LOG_RECORD_BYTES: usize = 4096;
const METRIC_RECORD_BYTES: usize = 8 * 1024;
const LOG_LINE_BYTES: usize = 2048;
const STREAM_LINES_PER_SECOND: u32 = 100;
const PLACEMENT_LINES_PER_SECOND: u32 = 200;
const CAPTURE_FLUSH_INTERVAL: Duration = Duration::from_millis(250);
const CAPTURE_BATCH_LINES: usize = 128;
const SAMPLE_INTERVAL: Duration = Duration::from_secs(5);
pub(crate) const MESSAGE_RETENTION: i64 = 2_000;
const TELEMETRY_BUDGET: i64 = 50_000;
const TELEMETRY_BUDGET_BYTES: i64 = 64 * 1024 * 1024;
static BUDGET_CHECKED: Mutex<Option<Instant>> = Mutex::new(None);

/// Log capture also enforces the device budget, throttled to the sampling
/// cadence, so the bound holds while the sampler backs off.
fn budget_check_due() -> bool {
    let mut checked = BUDGET_CHECKED
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    let now = Instant::now();
    if checked.is_some_and(|at| now.duration_since(at) < SAMPLE_INTERVAL) {
        return false;
    }
    *checked = Some(now);
    true
}

/// Each placement and kind keeps its own newest records, so one noisy stream
/// cannot evict another scope's logs, metrics, messages or usage checkpoints.
fn partition_retention(kind: &str) -> i64 {
    match kind {
        "log" => 5_000,
        "message" => MESSAGE_RETENTION,
        "metrics" => 1_000,
        _ => 16,
    }
}

fn record_bound(kind: &str) -> usize {
    if kind == "metrics" {
        METRIC_RECORD_BYTES
    } else {
        LOG_RECORD_BYTES
    }
}

pub struct TelemetryStore {
    pub(crate) store: StateStore,
    key: Zeroizing<[u8; 32]>,
}

impl TelemetryStore {
    pub fn open(state_dir: &Path) -> Result<Self> {
        Self::open_with_busy_timeout(state_dir, Duration::from_secs(5))
    }

    pub(crate) fn open_with_busy_timeout(state_dir: &Path, busy_timeout: Duration) -> Result<Self> {
        let path = state_dir.join("telemetry.key");
        if !path.try_exists()? {
            let mut key = Zeroizing::new([0; 32]);
            OsRng.fill_bytes(key.as_mut());
            if let Err(error) = vault::write_new_private(&path, key.as_ref()) {
                if !path.try_exists()? {
                    return Err(error);
                }
            }
        }
        let bytes = vault::read_private(&path)?;
        ensure!(bytes.len() == 32, "Invalid telemetry storage key");
        let telemetry = Self {
            store: StateStore::open_with_busy_timeout(
                &state_dir.join("management.sqlite"),
                busy_timeout,
            )?,
            key: Zeroizing::new(bytes.as_slice().try_into()?),
        };
        if telemetry.store.seal_repair_pending {
            if let Err(error) = telemetry.repair_legacy_seals() {
                tracing::warn!("Sealed telemetry repair after enrollment will retry: {error:#}");
            }
        }
        Ok(telemetry)
    }

    pub(crate) fn transaction<T>(&self, work: impl FnOnce() -> Result<T>) -> Result<T> {
        self.store.connection.execute_batch("BEGIN IMMEDIATE")?;
        let result = work().and_then(|value| {
            self.store.connection.execute_batch("COMMIT")?;
            Ok(value)
        });
        if result.is_err() {
            let _ = self.store.connection.execute_batch("ROLLBACK");
        }
        result
    }

    pub fn append(&self, placement: Option<&str>, kind: &str, value: &Value) -> Result<u64> {
        ensure!(
            matches!(kind, "log" | "metrics" | "message")
                || kind
                    .strip_prefix("usage-")
                    .and_then(|slot| slot.parse::<u8>().ok())
                    .is_some_and(|slot| slot < 32 && kind == format!("usage-{slot}")),
            "Invalid telemetry kind: {kind}"
        );
        if let Some(id) = placement {
            flow_like_device_protocol::validate_management_id(id)?;
        }
        let now = unix_time()?;
        self.transaction(|| self.append_in_transaction(placement, kind, value, now))
    }

    /// Captured service output is committed in batches to bound fsyncs and write-lock hold time.
    pub(crate) fn append_logs(&self, placement: &str, values: &[Value]) -> Result<()> {
        flow_like_device_protocol::validate_management_id(placement)?;
        let now = unix_time()?;
        self.transaction(|| {
            for value in values {
                self.insert_sealed(Some(placement), "log", value, now)?;
            }
            self.trim_partition(Some(placement), "log")?;
            if budget_check_due() {
                self.evict_over_budget(TELEMETRY_BUDGET, TELEMETRY_BUDGET_BYTES)?;
            }
            Ok(())
        })
    }

    pub(crate) fn append_in_transaction(
        &self,
        placement: Option<&str>,
        kind: &str,
        value: &Value,
        now: i64,
    ) -> Result<u64> {
        let sequence = self.insert_sealed(placement, kind, value, now)?;
        self.trim_partition(placement, kind)?;
        Ok(sequence)
    }

    fn insert_sealed(
        &self,
        placement: Option<&str>,
        kind: &str,
        value: &Value,
        now: i64,
    ) -> Result<u64> {
        let plaintext = Zeroizing::new(serde_json::to_vec(value)?);
        ensure!(
            plaintext.len() <= record_bound(kind),
            "Telemetry {kind} record of {} bytes exceeds its {} byte bound",
            plaintext.len(),
            record_bound(kind)
        );
        self.store.connection.execute("INSERT INTO telemetry_records(placement_id,kind,created_at,ciphertext) VALUES(?1,?2,?3,x'')",params![placement,kind,now])?;
        let sequence = self.store.connection.last_insert_rowid();
        let sealed = self.seal(&plaintext, &self.aad(sequence, placement, kind, now)?)?;
        self.store.connection.execute(
            "UPDATE telemetry_records SET ciphertext=?2 WHERE sequence=?1",
            params![sequence, sealed],
        )?;
        Ok(sequence.try_into()?)
    }

    /// Device-scope messages are retained per project, so one project's
    /// commands cannot evict another project's history.
    pub(crate) fn insert_message(
        &self,
        placement: Option<&str>,
        project: Option<&str>,
        value: &Value,
        now: i64,
    ) -> Result<u64> {
        let sequence = self.insert_sealed(placement, "message", value, now)?;
        self.store.connection.execute(
            "INSERT INTO operational_message_scopes(sequence,project_id) VALUES(?1,?2)",
            params![sequence, project],
        )?;
        Ok(sequence)
    }

    pub(crate) fn trim_messages(
        &self,
        placement: Option<&str>,
        project: Option<&str>,
    ) -> Result<()> {
        match placement {
            Some(_) => self.trim_partition(placement, "message"),
            None => self.trim_device_messages(project),
        }
    }

    fn trim_device_messages(&self, project: Option<&str>) -> Result<()> {
        let cutoff: Option<i64> = self
            .store
            .connection
            .query_row(
                "SELECT t.sequence FROM telemetry_records t LEFT JOIN operational_message_scopes s ON s.sequence=t.sequence WHERE t.placement_id IS NULL AND t.kind='message' AND s.project_id IS ?1 ORDER BY t.sequence DESC LIMIT 1 OFFSET ?2",
                params![project, MESSAGE_RETENTION],
                |row| row.get(0),
            )
            .optional()?;
        if let Some(cutoff) = cutoff {
            self.evict(
                "placement_id IS NULL AND kind='message' AND sequence<=?2 AND (SELECT s.project_id FROM operational_message_scopes s WHERE s.sequence=telemetry_records.sequence) IS ?1",
                params![project, cutoff],
            )?;
        }
        Ok(())
    }

    // Telemetry eviction never touches command or cryptographic replay journals.
    fn trim_partition(&self, placement: Option<&str>, kind: &str) -> Result<()> {
        if placement.is_none() && kind == "message" {
            return self.trim_device_messages(None);
        }
        let cutoff: Option<i64> = self
            .store
            .connection
            .query_row(
                "SELECT sequence FROM telemetry_records WHERE placement_id IS ?1 AND kind=?2 ORDER BY sequence DESC LIMIT 1 OFFSET ?3",
                params![placement, kind, partition_retention(kind)],
                |row| row.get(0),
            )
            .optional()?;
        if let Some(cutoff) = cutoff {
            self.evict(
                "placement_id IS ?1 AND kind=?2 AND sequence<=?3",
                params![placement, kind, cutoff],
            )?;
        }
        Ok(())
    }

    /// Records eviction watermarks for readers and counts records an archive
    /// roster had not sealed yet, so retained history reports the gap.
    /// Device-scope messages of a project keep their watermark under
    /// `device/<project>`; `/` never occurs in a placement id.
    fn evict(&self, condition: &str, parameters: &[&dyn ToSql]) -> Result<usize> {
        let connection = &self.store.connection;
        connection.execute(
            &format!("INSERT INTO telemetry_evictions(scope,kind,evicted_through,evicted) SELECT CASE WHEN placement_id IS NULL AND kind='message' THEN COALESCE('device/'||(SELECT s.project_id FROM operational_message_scopes s WHERE s.sequence=telemetry_records.sequence),'device') ELSE COALESCE(placement_id,'device') END AS eviction_scope,kind,MAX(sequence),COUNT(*) FROM telemetry_records WHERE {condition} GROUP BY eviction_scope,kind ON CONFLICT(scope,kind) DO UPDATE SET evicted_through=MAX(evicted_through,excluded.evicted_through),evicted=evicted+excluded.evicted"),
            parameters,
        )?;
        connection.execute(
            &format!("UPDATE archive_rosters SET dropped=dropped+(SELECT COUNT(*) FROM telemetry_records WHERE {condition} AND COALESCE(placement_id,'device')=archive_rosters.scope AND (kind=archive_rosters.kind OR (archive_rosters.kind='log' AND kind='message')) AND sequence>archive_rosters.telemetry_cursor) WHERE scope IN (SELECT COALESCE(placement_id,'device') FROM telemetry_records WHERE {condition})"),
            parameters,
        )?;
        Ok(connection.execute(
            &format!("DELETE FROM telemetry_records WHERE {condition}"),
            parameters,
        )?)
    }

    /// Bounds total storage when many scopes are active. Retired placements go
    /// first, then the oldest service logs; structured records are evicted last.
    pub(crate) fn enforce_retention_budget(&self) -> Result<usize> {
        self.transaction(|| self.evict_over_budget(TELEMETRY_BUDGET, TELEMETRY_BUDGET_BYTES))
    }

    fn evict_over_budget(&self, rows: i64, bytes: i64) -> Result<usize> {
        let connection = &self.store.connection;
        let total: i64 =
            connection.query_row("SELECT COUNT(*) FROM telemetry_records", [], |row| {
                row.get(0)
            })?;
        // The used file size bounds the sealed bytes, so the full scan only runs near the budget.
        let used: i64 = connection.query_row(
            "SELECT (c.page_count-f.freelist_count)*s.page_size FROM pragma_page_count() c,pragma_freelist_count() f,pragma_page_size() s",
            [],
            |row| row.get(0),
        )?;
        let stored: i64 = if used > bytes {
            connection.query_row(
                "SELECT COALESCE(SUM(length(ciphertext)),0) FROM telemetry_records",
                [],
                |row| row.get(0),
            )?
        } else {
            0
        };
        if total <= rows && stored <= bytes {
            return Ok(0);
        }
        self.evict(
            "sequence IN (SELECT sequence FROM (SELECT r.sequence,ROW_NUMBER() OVER w AS position,SUM(length(r.ciphertext)) OVER w-length(r.ciphertext) AS preceding FROM telemetry_records r LEFT JOIN placement_identities i ON i.id=r.placement_id COLLATE BINARY WINDOW w AS (ORDER BY COALESCE(i.retired,0) DESC,r.kind='log' DESC,r.sequence)) WHERE position<=?1 OR preceding<?2)",
            params![total - rows, stored - bytes],
        )
    }

    /// Databases enrolled before sealed rows bound to the storage identity can
    /// hold records sealed under the replaced local identity. They can never
    /// authenticate again, so remove them once instead of failing every read.
    fn repair_legacy_seals(&self) -> Result<()> {
        self.transaction(|| {
            let pending: bool = self.store.connection.query_row(
                "SELECT seal_repair_pending FROM device_identity WHERE singleton=1",
                [],
                |row| row.get(0),
            )?;
            if !pending {
                return Ok(());
            }
            let cipher = self.cipher()?;
            let mut unreadable = Vec::new();
            {
                let mut statement = self.store.connection.prepare(
                    "SELECT sequence,created_at,ciphertext,placement_id,kind FROM telemetry_records",
                )?;
                let mut rows = statement.query([])?;
                while let Some(row) = rows.next()? {
                    let sequence: i64 = row.get(0)?;
                    let placement: Option<String> = row.get(3)?;
                    let kind: String = row.get(4)?;
                    if self
                        .open_record(
                            &cipher,
                            sequence,
                            placement.as_deref(),
                            &kind,
                            row.get(1)?,
                            &row.get::<_, Vec<u8>>(2)?,
                        )
                        .is_err()
                    {
                        unreadable.push(sequence);
                    }
                }
            }
            if !unreadable.is_empty() {
                self.evict(
                    "sequence IN (SELECT value FROM json_each(?1))",
                    params![serde_json::to_string(&unreadable)?],
                )?;
            }
            let usage = self.quarantine_unreadable_usage()?;
            self.store.connection.execute(
                "UPDATE device_identity SET seal_repair_pending=0 WHERE singleton=1",
                [],
            )?;
            if !unreadable.is_empty() || usage > 0 {
                tracing::warn!(
                    telemetry_records = unreadable.len(),
                    usage_checkpoints = usage,
                    "Removed protected records sealed under the replaced pre-enrollment device identity"
                );
            }
            Ok(())
        })
    }

    fn cipher(&self) -> Result<XChaCha20Poly1305> {
        XChaCha20Poly1305::new_from_slice(self.key.as_ref())
            .map_err(|_| anyhow::anyhow!("Telemetry key initialization failed"))
    }

    fn seal(&self, plaintext: &[u8], aad: &[u8]) -> Result<Vec<u8>> {
        let mut nonce = [0; 24];
        OsRng.fill_bytes(&mut nonce);
        let mut sealed = nonce.to_vec();
        sealed.extend(
            self.cipher()?
                .encrypt(
                    XNonce::from_slice(&nonce),
                    Payload {
                        msg: plaintext,
                        aad,
                    },
                )
                .map_err(|_| anyhow::anyhow!("Telemetry encryption failed"))?,
        );
        Ok(sealed)
    }

    fn open_record(
        &self,
        cipher: &XChaCha20Poly1305,
        sequence: i64,
        placement: Option<&str>,
        kind: &str,
        created_at: i64,
        bytes: &[u8],
    ) -> Result<Zeroizing<Vec<u8>>> {
        ensure!(bytes.len() >= 40, "Invalid protected telemetry record");
        cipher
            .decrypt(
                XNonce::from_slice(&bytes[..24]),
                Payload {
                    msg: &bytes[24..],
                    aad: &self.aad(sequence, placement, kind, created_at)?,
                },
            )
            .map(Zeroizing::new)
            .map_err(|_| anyhow::anyhow!("Telemetry authentication failed"))
    }

    fn operational_aad(&self, domain: &str, binding: &Value) -> Result<Vec<u8>> {
        Ok(serde_json::to_vec(&json!([
            "flow-like/standalone/operational/v1",
            self.store.storage_id(),
            domain,
            binding
        ]))?)
    }

    pub(crate) fn seal_payload<T: serde::Serialize>(
        &self,
        domain: &str,
        binding: &Value,
        value: &T,
    ) -> Result<Vec<u8>> {
        let plaintext = Zeroizing::new(serde_json::to_vec(value)?);
        ensure!(
            plaintext.len() <= 16384,
            "Protected operational payload exceeds its bound"
        );
        self.seal(&plaintext, &self.operational_aad(domain, binding)?)
    }

    pub(crate) fn open_payload<T: serde::de::DeserializeOwned>(
        &self,
        domain: &str,
        binding: &Value,
        bytes: &[u8],
    ) -> Result<T> {
        ensure!(
            (40..=16424).contains(&bytes.len()),
            "Invalid protected operational record"
        );
        let plaintext = Zeroizing::new(
            self.cipher()?
                .decrypt(
                    XNonce::from_slice(&bytes[..24]),
                    Payload {
                        msg: &bytes[24..],
                        aad: &self.operational_aad(domain, binding)?,
                    },
                )
                .map_err(|_| anyhow::anyhow!("Operational authentication failed"))?,
        );
        Ok(serde_json::from_slice(&plaintext)?)
    }

    fn aad(
        &self,
        sequence: i64,
        placement: Option<&str>,
        kind: &str,
        created_at: i64,
    ) -> Result<Vec<u8>> {
        Ok(serde_json::to_vec(&json!([
            "flow-like/standalone/telemetry/v1",
            self.store.storage_id(),
            sequence,
            placement,
            kind,
            created_at
        ]))?)
    }

    pub fn read(
        &self,
        placement: Option<&str>,
        kind: &str,
        after: u64,
        limit: u32,
    ) -> Result<Value> {
        self.read_filtered(placement, kind, after, limit, true, None)
    }

    /// `evicted_through`, when present, means at least one matching record after
    /// `after` and at or below it was evicted before being read. It is a gap
    /// signal, not a resume cursor: other scopes or kinds in the same read can
    /// still retain lower sequences, so callers keep paging from `next`.
    pub(crate) fn read_filtered(
        &self,
        placement: Option<&str>,
        kind: &str,
        after: u64,
        limit: u32,
        exact_placement: bool,
        project: Option<&str>,
    ) -> Result<Value> {
        let after = i64::try_from(after)?;
        let limit = limit.clamp(1, 100);
        let mut statement=self.store.connection.prepare("SELECT sequence,created_at,ciphertext,placement_id,kind FROM telemetry_records t WHERE (?5=0 AND ?1 IS NULL OR placement_id IS ?1) AND (kind=?2 OR (?2='log' AND kind='message')) AND sequence>?3 AND (?6 IS NULL OR EXISTS(SELECT 1 FROM operational_message_scopes s WHERE s.sequence=t.sequence AND s.project_id=?6 COLLATE BINARY)) ORDER BY sequence LIMIT ?4")?;
        let mut rows = statement.query(params![
            placement,
            kind,
            after,
            limit,
            exact_placement,
            project
        ])?;
        let mut records = Vec::new();
        let mut size = 0;
        let mut next = after;
        let cipher = self.cipher()?;
        while let Some(row) = rows.next()? {
            let sequence: i64 = row.get(0)?;
            let created_at: i64 = row.get(1)?;
            let record_placement: Option<String> = row.get(3)?;
            let record_kind: String = row.get(4)?;
            let plaintext = self.open_record(
                &cipher,
                sequence,
                record_placement.as_deref(),
                &record_kind,
                created_at,
                &row.get::<_, Vec<u8>>(2)?,
            )?;
            if size + plaintext.len() > 10 * 1024 {
                break;
            }
            let data: Value = serde_json::from_slice(&plaintext)?;
            if let Some(project) = project {
                ensure!(
                    data["project_id"].as_str() == Some(project),
                    "Protected message project scope differs"
                );
            }
            size += plaintext.len();
            next = sequence;
            records.push(
                json!({"sequence":sequence,"timestamp":created_at,"kind":record_kind,"data":data}),
            );
        }
        let mut result = json!({"records":records,"next":next});
        // Project readers only see watermarks of their own placements and device-scope messages.
        let evicted_through: Option<i64> = self.store.connection.query_row(
            "SELECT MAX(evicted_through) FROM telemetry_evictions e WHERE (?4=0 AND ?1 IS NULL OR e.scope=COALESCE(?1,'device') OR ?1 IS NULL AND substr(e.scope,1,7)='device/') AND (e.kind=?2 OR (?2='log' AND e.kind='message')) AND (?3 IS NULL OR e.scope='device/'||?3 COLLATE BINARY OR EXISTS(SELECT 1 FROM placement_identities i WHERE i.id=e.scope COLLATE BINARY AND i.project_id=?3 COLLATE BINARY))",
            params![placement, kind, project, exact_placement],
            |row| row.get(0),
        )?;
        if let Some(evicted_through) = evicted_through.filter(|through| *through > after) {
            result["evicted_through"] = json!(evicted_through);
        }
        Ok(result)
    }

    pub fn latest_metrics(&self, placement: Option<&str>) -> Result<Value> {
        self.latest(placement, "metrics")
    }

    fn latest(&self, placement: Option<&str>, kind: &str) -> Result<Value> {
        let sequence: Option<i64> = self.store.connection.query_row(
            "SELECT MAX(sequence) FROM telemetry_records WHERE placement_id IS ?1 AND kind=?2",
            params![placement, kind],
            |r| r.get(0),
        )?;
        match sequence {
            Some(sequence) => self.read(placement, kind, (sequence - 1).try_into()?, 1),
            None => Ok(json!({"records":[],"next":0})),
        }
    }

    pub(crate) fn usage(
        &self,
        placement: &crate::state::PlacementRecord,
    ) -> Result<Vec<crate::usage::RuntimeUsageSnapshot>> {
        let mut snapshots = Vec::new();
        let now = unix_time()?;
        for replica in &placement.replicas {
            if replica.observed_state != crate::state::ObservedState::Running {
                continue;
            }
            let Some(pid) = replica.process_id else {
                continue;
            };
            let record = self.latest(Some(&placement.id), &format!("usage-{}", replica.slot))?;
            let value = &record["records"][0];
            let data = &value["data"];
            if value["timestamp"]
                .as_i64()
                .is_none_or(|time| time < now - 15 || time > now + 30)
                || data["process_id"].as_u64() != Some(u64::from(pid))
                || data["config_revision"].as_u64() != Some(placement.config_revision)
                || data["intent_revision"].as_u64() != Some(placement.intent_revision)
                || !self.usage_is_active(data)?
            {
                continue;
            }
            let snapshot: crate::usage::RuntimeUsageSnapshot =
                serde_json::from_value(data["counters"].clone())?;
            if snapshot.validate_after(None) {
                snapshots.push(snapshot);
            }
        }
        Ok(snapshots)
    }
}

pub(crate) fn usage_summary(
    snapshots: &[crate::usage::RuntimeUsageSnapshot],
    expected: u64,
) -> Value {
    let mut summary = json!({"scope":"supervised_services", "window":"current_process_lifetimes",
        "reported_replicas":snapshots.len(), "expected_replicas":expected, "freshness_seconds":15});
    let values: Vec<_> = snapshots
        .iter()
        .map(|snapshot| serde_json::to_value(snapshot).expect("usage serializes"))
        .collect();
    for key in [
        "invocations_started",
        "invocations_succeeded",
        "invocations_failed",
        "invocations_cancelled",
        "in_flight",
        "runtime_messages",
        "request_payload_bytes",
        "response_payload_bytes",
        "concurrency_rejections",
    ] {
        let sum = if values.is_empty() {
            None
        } else {
            values
                .iter()
                .try_fold(0u64, |sum, value| sum.checked_add(value[key].as_u64()?))
        };
        summary[key] = json!(sum);
    }
    summary
}

/// Waits before retrying a failed background pass; false once cancelled.
pub(crate) async fn back_off(cancel: &CancellationToken, base: Duration, failures: u32) -> bool {
    let delay = base
        .saturating_mul(1 << failures.clamp(1, 6))
        .min(Duration::from_secs(300));
    tokio::select! {
        _ = cancel.cancelled() => false,
        _ = tokio::time::sleep(delay) => true,
    }
}

pub async fn sample(state_dir: PathBuf, cancel: CancellationToken) -> Result<()> {
    let mut sampler = Sampler::new(state_dir);
    let mut tick = tokio::time::interval(SAMPLE_INTERVAL);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut failures = 0u32;
    loop {
        tokio::select! {_=cancel.cancelled()=>return Ok(()),_=tick.tick()=>()}
        match sampler.pass() {
            Ok(()) => failures = 0,
            Err(error) => {
                failures = failures.saturating_add(1);
                sampler.store = None;
                tracing::warn!(
                    failures,
                    "Telemetry sampling pass failed; retrying with backoff: {error:#}"
                );
                if !back_off(&cancel, SAMPLE_INTERVAL, failures).await {
                    return Ok(());
                }
            }
        }
    }
}

type IsolationSample = (u64, u64, Option<(u64, u64)>, Instant);

struct Sampler {
    state_dir: PathBuf,
    store: Option<TelemetryStore>,
    system: sysinfo::System,
    networks: sysinfo::Networks,
    network_window: resources::NetworkWindow,
    sample_time: Instant,
    first: bool,
    previous_processes: HashMap<sysinfo::Pid, u64>,
    previous_isolation: HashMap<(String, u8), IsolationSample>,
}

impl Sampler {
    fn new(state_dir: PathBuf) -> Self {
        Self {
            state_dir,
            store: None,
            system: sysinfo::System::new(),
            networks: sysinfo::Networks::new_with_refreshed_list(),
            network_window: resources::NetworkWindow::default(),
            sample_time: Instant::now(),
            first: true,
            previous_processes: HashMap::new(),
            previous_isolation: HashMap::new(),
        }
    }

    fn pass(&mut self) -> Result<()> {
        use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate};
        if self.store.is_none() {
            self.store = Some(
                TelemetryStore::open(&self.state_dir)
                    .context("Open telemetry store for sampling")?,
            );
        }
        let store = self.store.as_ref().context("Telemetry store unavailable")?;
        let state_dir = &self.state_dir;
        let system = &mut self.system;
        store
            .reconcile_usage()
            .context("Reconcile usage checkpoints")?;
        store
            .project_messages()
            .context("Project operational messages")?;
        store
            .enforce_retention_budget()
            .context("Enforce telemetry retention budget")?;
        let placements = store.store.list_placements()?;
        let metric_cohorts = placements
            .iter()
            .map(|placement| Ok((placement.id.clone(), store.metric_cohort(placement)?)))
            .collect::<Result<HashMap<_, _>>>()?;
        let mut pids: Vec<Pid> = placements
            .iter()
            .flat_map(|p| {
                p.replicas
                    .iter()
                    .filter_map(|r| r.process_id.map(Pid::from_u32))
            })
            .collect();
        let agent_pid = Pid::from_u32(std::process::id());
        pids.push(agent_pid);
        pids.sort_unstable();
        pids.dedup();
        system.refresh_cpu_usage();
        system.refresh_memory();
        system.refresh_processes_specifics(
            ProcessesToUpdate::Some(&pids),
            true,
            ProcessRefreshKind::nothing()
                .with_cpu()
                .with_memory()
                .with_disk_usage(),
        );
        let current_processes: HashMap<_, _> = pids
            .iter()
            .filter_map(|pid| system.process(*pid).map(|p| (*pid, p.start_time())))
            .collect();
        self.networks.refresh(true);
        let network = self.network_window.observe(
            self.networks
                .iter()
                .map(|(name, value)| {
                    (
                        name.clone(),
                        (value.total_received(), value.total_transmitted()),
                    )
                })
                .collect(),
        );
        let now = Instant::now();
        let sample_seconds = now.duration_since(self.sample_time).as_secs_f64();
        self.sample_time = now;
        // CPU deltas need two samples of the same process, including after a replica restart.
        if self.first {
            self.first = false;
            self.previous_processes = current_processes;
            return Ok(());
        }
        let previous_processes = &self.previous_processes;
        let previous_isolation = &self.previous_isolation;
        let agent = system.process(agent_pid);
        let mut device_usage = Vec::new();
        let mut expected_usage = 0u64;
        let mut placement_usage = HashMap::new();
        for placement in &placements {
            expected_usage += u64::from(placement.running_replicas);
            let usage = store.usage(placement)?;
            placement_usage.insert(
                placement.id.clone(),
                usage_summary(&usage, u64::from(placement.running_replicas)),
            );
            device_usage.extend(usage);
        }
        store.append(None, "metrics", &json!({
            "cpu_percent": system.global_cpu_usage(),
            "memory_used_bytes": system.used_memory(),
            "memory_total_bytes": system.total_memory(),
            "agent_cpu_percent_of_one_core": agent.map(|p| p.cpu_usage()),
            "agent_memory_bytes": agent.map(|p| p.memory()),
            "logical_cpus": system.cpus().len(),
            "network": network,
            "storage_volume": resources::volume_space(state_dir),
            "placements": placements.len(),
            "desired_replicas": placements.iter().map(|p| u64::from(p.desired_replicas)).sum::<u64>(),
            "ready_replicas": placements.iter().map(|p| u64::from(p.ready_replicas)).sum::<u64>(),
            "sample_seconds": sample_seconds,
            "usage": usage_summary(&device_usage, expected_usage),
            "usage_retained": store.retained_usage(None, None)?
        }))?;
        let mut current_isolation = HashMap::new();
        for placement in placements {
            let mut replicas = Vec::new();
            let mut cpu_sum = 0.0f64;
            let mut memory_sum = 0u64;
            let mut observed = 0u8;
            let mut cpu_observed = 0u8;
            let mut io_rate_sum = (0.0f64, 0.0f64);
            let mut io_observed = 0u8;
            let isolated = placement
                .config
                .pointer("/resources/profile")
                .and_then(Value::as_str)
                == Some("linux_sandbox");
            for replica in &placement.replicas {
                let process = replica
                    .process_id
                    .and_then(|pid| system.process(Pid::from_u32(pid)));
                let mut cpu = process
                    .filter(|p| previous_processes.get(&p.pid()) == Some(&p.start_time()))
                    .map(|p| p.cpu_usage());
                let mut memory = process.map(|p| p.memory());
                let mut processes_and_threads = None;
                let mut io_seconds = sample_seconds;
                let mut io = process
                    .filter(|p| previous_processes.get(&p.pid()) == Some(&p.start_time()))
                    .map(|p| {
                        let value = p.disk_usage();
                        (value.read_bytes, value.written_bytes)
                    });
                if isolated {
                    // Never substitute the monitor's usage when isolation
                    // accounting is unavailable or the cgroup has changed.
                    cpu = None;
                    memory = None;
                    io = None;
                    if let Some(sample) = replica.process_id.and_then(|pid| {
                        crate::isolation::sample(pid, &placement.id, replica.slot, state_dir).ok()
                    }) {
                        let key = (placement.id.clone(), replica.slot);
                        let now = Instant::now();
                        if let Some((generation, usage, previous_io, time)) =
                            previous_isolation.get(&key)
                            && *generation == sample.generation
                            && let Some(delta) = sample.cpu_microseconds.checked_sub(*usage)
                            && now.duration_since(*time).as_secs_f64() > 0.0
                        {
                            io_seconds = now.duration_since(*time).as_secs_f64();
                            io = sample.io_bytes.zip(*previous_io).and_then(
                                |(current, previous)| {
                                    Some((
                                        current.0.checked_sub(previous.0)?,
                                        current.1.checked_sub(previous.1)?,
                                    ))
                                },
                            );
                            cpu = Some(
                                (delta as f64 / now.duration_since(*time).as_secs_f64() / 10_000.0)
                                    as f32,
                            );
                        }
                        current_isolation.insert(
                            key,
                            (
                                sample.generation,
                                sample.cpu_microseconds,
                                sample.io_bytes,
                                now,
                            ),
                        );
                        memory = Some(sample.memory_bytes);
                        processes_and_threads = Some(sample.processes_and_threads);
                    }
                }
                if let Some(memory) = memory {
                    observed += 1;
                    memory_sum = memory_sum.saturating_add(memory);
                }
                if let Some(cpu) = cpu {
                    cpu_observed += 1;
                    cpu_sum += f64::from(cpu);
                }
                if let Some((read, written)) = io
                    && io_seconds > 0.0
                {
                    io_observed += 1;
                    io_rate_sum.0 += read as f64 / io_seconds;
                    io_rate_sum.1 += written as f64 / io_seconds;
                }
                replicas.push(json!({
                    "slot": replica.slot,
                    "cpu_percent": cpu,
                    "memory_bytes": memory,
                    "processes_and_threads": processes_and_threads
                }));
            }
            // Reject a cohort that changed while the OS counters were sampled.
            let current_cohort = store.metric_cohort(&placement)?;
            let process_cohort = current_cohort.filter(|cohort| {
                metric_cohorts.get(&placement.id).and_then(Option::as_ref) == Some(cohort)
            });
            // Process CPU sums use 100% per logical CPU. Missing samples remain null.
            store.append(
                Some(&placement.id),
                "metrics",
                &json!({
                    "project_id": placement.config.get("project_id"),
                    "process_cohort": process_cohort,
                    "config_revision":placement.config_revision,
                    "intent_revision":placement.intent_revision,
                    "cpu_percent": (observed > 0 && cpu_observed == observed).then_some(cpu_sum),
                    "cpu_basis": "one_logical_cpu",
                    "memory_bytes": (observed > 0).then_some(memory_sum),
                    "io": {"basis": if isolated {"cgroup_block_io"} else {"process_io"},
                        "read_bytes_per_second": (io_observed > 0 && io_observed == placement.running_replicas && io_rate_sum.0.is_finite()).then_some(io_rate_sum.0),
                        "written_bytes_per_second": (io_observed > 0 && io_observed == placement.running_replicas && io_rate_sum.1.is_finite()).then_some(io_rate_sum.1),
                        "replicas_observed":io_observed},
                    "disk_quota": if isolated { crate::isolation::disk_sample(state_dir, &placement.id).ok() } else { None },
                    "memory_basis": if isolated { "cgroup_current" } else { "process_rss" },
                    "processes_observed": observed,
                    "desired_replicas": placement.desired_replicas,
                    "running_replicas": placement.running_replicas,
                    "ready_replicas": placement.ready_replicas,
                    "replicas": replicas,
                    "sample_seconds": sample_seconds,
                    "usage": placement_usage.get(&placement.id),
                    "usage_retained": store.retained_usage(Some(&placement.id), None)?
                }),
            )?;
        }
        self.previous_processes = current_processes;
        self.previous_isolation = current_isolation;
        Ok(())
    }
}

static PLACEMENT_LOG_BUDGETS: LazyLock<Mutex<HashMap<String, (i64, u32)>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// Shared by every replica and stream of one placement.
fn admit_placement_line(placement: &str, now: i64) -> bool {
    let mut budgets = PLACEMENT_LOG_BUDGETS
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    if budgets.len() > 4096 {
        budgets.retain(|_, (window, _)| *window == now);
    }
    let (window, admitted) = budgets.entry(placement.to_owned()).or_insert((now, 0));
    if *window != now {
        *window = now;
        *admitted = 0;
    }
    if *admitted >= PLACEMENT_LINES_PER_SECOND {
        return false;
    }
    *admitted += 1;
    true
}

fn json_escaped_len(character: char) -> usize {
    match character {
        '"' | '\\' | '\n' | '\r' | '\t' | '\u{08}' | '\u{0c}' => 2,
        character if u32::from(character) < 0x20 => 6,
        character => character.len_utf8(),
    }
}

fn record_fits(value: &Value) -> bool {
    serde_json::to_vec(value).is_ok_and(|bytes| bytes.len() <= LOG_RECORD_BYTES)
}

/// Lossy decoding and JSON escaping can grow a line past the record bound, so
/// the message is cut on its serialized size instead of dropping the line.
fn log_record(stream: &str, line: &[u8], truncated: bool) -> Value {
    let text = String::from_utf8_lossy(line);
    let record = |message: &str, truncated: bool| json!({"stream":stream,"message":message,"truncated":truncated});
    let budget = serde_json::to_vec(&record("", false))
        .map_or(0, |empty| LOG_RECORD_BYTES.saturating_sub(empty.len()));
    let mut used = 0;
    let end = text
        .char_indices()
        .find(|(_, character)| {
            used += json_escaped_len(*character);
            used > budget
        })
        .map_or(text.len(), |(index, _)| index);
    let mut message = &text[..end];
    let mut value = record(message, truncated || end < text.len());
    while !record_fits(&value) && !message.is_empty() {
        let half = message.chars().count() / 2;
        message = &message[..message
            .char_indices()
            .nth(half)
            .map_or(0, |(index, _)| index)];
        value = record(message, true);
    }
    value
}

struct LogSink {
    state_dir: PathBuf,
    placement: String,
    stream: &'static str,
    store: Option<TelemetryStore>,
    pending: Vec<Value>,
    window: i64,
    admitted: u32,
    dropped: u64,
    failing: bool,
}

impl LogSink {
    fn new(state_dir: PathBuf, placement: String, stream: &'static str) -> Self {
        Self {
            state_dir,
            placement,
            stream,
            store: None,
            pending: Vec::new(),
            window: 0,
            admitted: 0,
            dropped: 0,
            failing: false,
        }
    }

    fn close_window(&mut self, now: i64) {
        if now == self.window {
            return;
        }
        if self.dropped > 0 {
            self.pending.push(json!({"stream":self.stream,
                "message":format!("[flow-like] {} log lines dropped by the capture rate limit", self.dropped),
                "truncated":false,"dropped_lines":self.dropped}));
            self.dropped = 0;
        }
        self.window = now;
        self.admitted = 0;
    }

    fn push(&mut self, line: &[u8], truncated: bool, now: i64) {
        self.close_window(now);
        if self.admitted < STREAM_LINES_PER_SECOND && admit_placement_line(&self.placement, now) {
            self.admitted += 1;
            self.pending.push(log_record(self.stream, line, truncated));
        } else {
            self.dropped = self.dropped.saturating_add(1);
        }
    }

    fn persist(&mut self, batch: &[Value]) -> Result<()> {
        if self.store.is_none() {
            self.store = Some(TelemetryStore::open(&self.state_dir)?);
        }
        self.store
            .as_ref()
            .context("Telemetry store unavailable")?
            .append_logs(&self.placement, batch)
    }

    /// Persistence failures drop the batch but never stop draining the child's pipe.
    fn flush(&mut self, now: i64) {
        self.close_window(now);
        if self.pending.is_empty() {
            return;
        }
        let batch = std::mem::take(&mut self.pending);
        match self.persist(&batch) {
            Ok(()) => {
                if self.failing {
                    tracing::info!(placement = %self.placement, stream = self.stream, "Service log capture resumed");
                }
                self.failing = false;
            }
            Err(error) => {
                if !self.failing {
                    tracing::warn!(placement = %self.placement, stream = self.stream, lines = batch.len(), "Service log capture could not persist lines: {error:#}");
                }
                self.failing = true;
                self.store = None;
            }
        }
    }
}

pub async fn capture<R: AsyncRead + Unpin>(
    mut reader: R,
    state_dir: PathBuf,
    placement: String,
    stream: &'static str,
) -> Result<()> {
    let mut sink = LogSink::new(state_dir, placement, stream);
    let mut buffer = [0; 1024];
    let mut line = Vec::with_capacity(LOG_LINE_BYTES);
    let mut truncated = false;
    let mut flush = tokio::time::interval(CAPTURE_FLUSH_INTERVAL);
    flush.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        let read = tokio::select! {
            read = reader.read(&mut buffer) => read,
            _ = flush.tick() => {
                sink.flush(unix_time().unwrap_or_default());
                continue;
            }
        };
        let now = unix_time().unwrap_or_default();
        let size = match read {
            Ok(size) => size,
            Err(error) => {
                sink.flush(now);
                return Err(error.into());
            }
        };
        if size == 0 {
            if !line.is_empty() {
                sink.push(&line, truncated, now);
            }
            sink.flush(now.saturating_add(1));
            return Ok(());
        }
        for byte in &buffer[..size] {
            if *byte == b'\n' {
                sink.push(&line, truncated, now);
                line.clear();
                truncated = false;
            } else if line.len() < LOG_LINE_BYTES {
                line.push(*byte);
            } else {
                truncated = true;
            }
        }
        if sink.pending.len() >= CAPTURE_BATCH_LINES {
            sink.flush(now);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn a_full_replica_metric_fits_storage_and_the_management_frame() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let store = TelemetryStore::open(&dir.path().canonicalize()?)?;
        let sample = json!({
            "project_id": "p".repeat(128),
            "cpu_percent": 1234.5678901234567_f64,
            "cpu_basis": "one_logical_cpu",
            "memory_bytes": u64::MAX,
            "memory_basis":"cgroup_current",
            "io":{"basis":"cgroup_block_io","read_bytes_per_second":1.2345678901234567e20_f64,
                "written_bytes_per_second":1.2345678901234567e20_f64,"replicas_observed":32},
            "disk_quota":{"used_bytes":u64::MAX,"limit_bytes":u64::MAX,"used_inodes":u64::MAX,"limit_inodes":u64::MAX},
            "processes_observed": 32,
            "desired_replicas": 32,
            "running_replicas": 32,
            "ready_replicas": 32,
            "replicas": (0..32).map(|slot| json!({
                "slot":slot, "cpu_percent":1234.5678901234567_f64,
                "memory_bytes":u64::MAX,"processes_and_threads":u64::MAX
            })).collect::<Vec<_>>(),
            "sample_seconds":5,
            "usage":usage_summary(&[crate::usage::RuntimeUsageSnapshot {
                version:1, sequence:u64::MAX, invocations_started:u64::MAX,
                invocations_succeeded:u64::MAX, runtime_messages:u64::MAX,
                request_payload_bytes:u64::MAX, response_payload_bytes:u64::MAX,
                ..Default::default()
            }], 32),
            "config_revision":u64::MAX,
            "intent_revision":u64::MAX,
            "usage_retained":{
                "version":1,"scope":"supervised_services","window":"retained_observed_process_checkpoints",
                "since":i64::MAX,"through":i64::MAX,"billing":false,"tail_loss_possible":true,
                "counters":{
                    "invocations_started":u64::MAX,"invocations_succeeded":u64::MAX,
                    "invocations_failed":u64::MAX,"invocations_cancelled":u64::MAX,
                    "runtime_messages":u64::MAX,"request_payload_bytes":u64::MAX,
                    "response_payload_bytes":u64::MAX,"concurrency_rejections":u64::MAX
                },
                "coverage":{
                    "registered_runs":u64::MAX,"reported_runs":u64::MAX,"finalized_runs":u64::MAX,
                    "incomplete_runs":u64::MAX,"unreported_runs":u64::MAX,
                    "fresh_active_runs":u64::MAX,"stale_active_runs":u64::MAX,
                    "awaiting_first_report_runs":u64::MAX,"ended_pending_runs":u64::MAX
                }
            }
        });
        assert!(serde_json::to_vec(&sample)?.len() <= 8192);
        store.append(Some("placement"), "metrics", &sample)?;
        let records = store.latest_metrics(Some("placement"))?;
        let saved = &records["records"][0]["data"];
        assert_eq!(saved["replicas"].as_array().unwrap().len(), 32);
        assert_eq!(saved["usage"], sample["usage"]);
        assert_eq!(saved["usage_retained"], sample["usage_retained"]);
        assert_eq!(saved["project_id"], sample["project_id"]);
        for replica in saved["replicas"].as_array().unwrap() {
            assert_eq!(replica["memory_bytes"], u64::MAX);
            assert!((replica["cpu_percent"].as_f64().unwrap() - 1234.5678901234567).abs() < 1e-9);
        }
        assert!(serde_json::to_vec(&json!({"result":records}))?.len() < 16 * 1024);
        assert!(
            store
                .append(None, "metrics", &json!({"data":"x".repeat(8192)}))
                .is_err()
        );
        assert!(
            store
                .append(None, "log", &json!({"data":"x".repeat(4096)}))
                .is_err()
        );
        Ok(())
    }
    #[test]
    fn usage_summary_exposes_partial_coverage_and_process_lifetime_totals() {
        assert_eq!(usage_summary(&[], 2)["runtime_messages"], Value::Null);
        let snapshots = [
            crate::usage::RuntimeUsageSnapshot {
                version: 1,
                sequence: 1,
                invocations_started: 2,
                invocations_succeeded: 1,
                in_flight: 1,
                runtime_messages: 7,
                ..Default::default()
            },
            crate::usage::RuntimeUsageSnapshot {
                version: 1,
                sequence: 2,
                invocations_started: 3,
                invocations_failed: 3,
                runtime_messages: 5,
                ..Default::default()
            },
        ];
        let summary = usage_summary(&snapshots, 3);
        assert_eq!(summary["reported_replicas"], 2);
        assert_eq!(summary["expected_replicas"], 3);
        assert_eq!(summary["runtime_messages"], 12);
        assert_eq!(summary["invocations_started"], 5);
        assert_eq!(summary["window"], "current_process_lifetimes");
    }
    fn logs(count: usize, label: &str) -> Vec<Value> {
        (0..count)
            .map(|index| json!({"stream":"stdout","message":format!("{label}-{index}"),"truncated":false}))
            .collect()
    }

    fn partition_count(store: &TelemetryStore, placement: &str, kind: &str) -> Result<i64> {
        Ok(store.store.connection.query_row(
            "SELECT COUNT(*) FROM telemetry_records WHERE placement_id=?1 AND kind=?2",
            params![placement, kind],
            |row| row.get(0),
        )?)
    }

    #[test]
    fn a_chatty_placement_only_evicts_its_own_records_and_readers_see_the_gap() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let store = TelemetryStore::open(&dir.path().canonicalize()?)?;
        store.append(Some("quiet"), "log", &json!({"message":"quiet-log"}))?;
        store.append(Some("quiet"), "metrics", &json!({"cpu_percent":1.0}))?;
        store.append(None, "message", &json!({"kind":"operation"}))?;
        let first_noisy = store.append(Some("noisy"), "metrics", &json!({"cpu_percent":2.0}))?;
        let retained = partition_retention("log") as usize;
        for batch in 0..6 {
            store.append_logs("noisy", &logs(1000, &format!("batch-{batch}")))?;
        }
        assert_eq!(partition_count(&store, "noisy", "log")?, retained as i64);
        assert_eq!(partition_count(&store, "noisy", "metrics")?, 1);
        let quiet = store.read(Some("quiet"), "log", 0, 10)?;
        assert_eq!(quiet["records"][0]["data"]["message"], "quiet-log");
        assert!(quiet.get("evicted_through").is_none());
        assert_eq!(
            store.latest_metrics(Some("quiet"))?["records"][0]["data"]["cpu_percent"],
            1.0
        );
        assert_eq!(
            store.read(None, "message", 0, 10)?["records"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        let noisy = store.read(Some("noisy"), "log", 0, 10)?;
        let evicted_through = noisy["evicted_through"].as_u64().unwrap();
        assert!(evicted_through > first_noisy);
        assert!(noisy["records"][0]["sequence"].as_u64().unwrap() > evicted_through);
        assert_eq!(noisy["records"][0]["data"]["message"], "batch-1-0");
        let caught_up = noisy["records"][0]["sequence"].as_u64().unwrap();
        assert!(
            store
                .read(Some("noisy"), "log", caught_up, 10)?
                .get("evicted_through")
                .is_none()
        );
        let evicted: i64 = store.store.connection.query_row(
            "SELECT evicted FROM telemetry_evictions WHERE scope='noisy' AND kind='log'",
            [],
            |row| row.get(0),
        )?;
        assert_eq!(evicted, 1000);
        Ok(())
    }

    #[test]
    fn device_budget_evicts_retired_placements_before_live_scopes() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let root = dir.path().canonicalize()?;
        let store = TelemetryStore::open(&root)?;
        store.store.connection.execute_batch(
            "INSERT INTO placement_identities(id,project_id,deployment_id,retired) VALUES('removed','p','d',1),('live','p','d',0);",
        )?;
        store.append(Some("live"), "metrics", &json!({"cpu_percent":3.0}))?;
        store.append_logs("removed", &logs(20, "removed"))?;
        store.append_logs("live", &logs(20, "live"))?;
        let over_budget = || store.transaction(|| store.evict_over_budget(15, i64::MAX));
        assert_eq!(over_budget()?, 26);
        assert_eq!(partition_count(&store, "removed", "log")?, 0);
        assert_eq!(partition_count(&store, "live", "log")?, 14);
        assert_eq!(partition_count(&store, "live", "metrics")?, 1);
        assert_eq!(over_budget()?, 0);
        Ok(())
    }

    #[test]
    fn device_budget_also_bounds_sealed_bytes() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let store = TelemetryStore::open(&dir.path().canonicalize()?)?;
        store.append(Some("live"), "metrics", &json!({"cpu_percent":3.0}))?;
        store.append_logs("live", &logs(40, "live"))?;
        let sealed_bytes = || -> Result<i64> {
            Ok(store.store.connection.query_row(
                "SELECT SUM(length(ciphertext)) FROM telemetry_records",
                [],
                |row| row.get(0),
            )?)
        };
        let budget = sealed_bytes()? / 2;
        let over_budget = || store.transaction(|| store.evict_over_budget(i64::MAX, budget));
        assert!(over_budget()? > 0);
        assert!(sealed_bytes()? <= budget);
        assert_eq!(partition_count(&store, "live", "metrics")?, 1);
        assert_eq!(over_budget()?, 0);
        Ok(())
    }

    #[test]
    fn device_scope_messages_are_retained_per_project() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let store = TelemetryStore::open(&dir.path().canonicalize()?)?;
        let message = |project: &str, index: i64| json!({"kind":"operation","project_id":project,"source_id":format!("{project}-{index}")});
        store.transaction(|| {
            store.insert_message(None, Some("quiet"), &message("quiet", 0), 0)?;
            for index in 0..MESSAGE_RETENTION + 10 {
                store.insert_message(None, Some("chatty"), &message("chatty", index), 0)?;
            }
            store.trim_messages(None, Some("quiet"))?;
            store.trim_messages(None, Some("chatty"))
        })?;
        let quiet = store.read_filtered(None, "message", 0, 100, false, Some("quiet"))?;
        assert_eq!(quiet["records"][0]["data"]["source_id"], "quiet-0");
        assert!(quiet.get("evicted_through").is_none());
        let chatty = store.read_filtered(None, "message", 0, 100, false, Some("chatty"))?;
        assert_eq!(chatty["records"][0]["data"]["source_id"], "chatty-10");
        assert!(chatty["evicted_through"].as_u64().is_some());
        assert!(
            store.read(None, "message", 0, 100)?["evicted_through"]
                .as_u64()
                .is_some()
        );
        Ok(())
    }

    #[tokio::test]
    async fn a_failing_sampling_pass_backs_off_instead_of_ending_the_sampler() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let cancel = CancellationToken::new();
        let sampler = tokio::spawn(sample(dir.path().join("missing"), cancel.clone()));
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert!(!sampler.is_finished());
        cancel.cancel();
        sampler.await??;
        Ok(())
    }

    #[test]
    fn unsealed_evictions_are_counted_for_retained_history() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let store = TelemetryStore::open(&dir.path().canonicalize()?)?;
        store.append_logs("noisy", &logs(100, "early"))?;
        let sealed_through: i64 = store.store.connection.query_row(
            "SELECT sequence FROM telemetry_records WHERE placement_id='noisy' ORDER BY sequence LIMIT 1 OFFSET 39",
            [],
            |row| row.get(0),
        )?;
        store.store.connection.execute(
            "INSERT INTO archive_rosters(scope,kind,policy_jws,telemetry_cursor) VALUES('noisy','log','roster',?1),('noisy','metrics','roster',0)",
            [sealed_through],
        )?;
        store.append_logs("noisy", &logs(partition_retention("log") as usize, "later"))?;
        let dropped: i64 = store.store.connection.query_row(
            "SELECT dropped FROM archive_rosters WHERE scope='noisy' AND kind='log'",
            [],
            |row| row.get(0),
        )?;
        assert_eq!(dropped, 60);
        let metrics: i64 = store.store.connection.query_row(
            "SELECT dropped FROM archive_rosters WHERE scope='noisy' AND kind='metrics'",
            [],
            |row| row.get(0),
        )?;
        assert_eq!(metrics, 0);
        Ok(())
    }

    #[test]
    fn escaped_log_lines_are_truncated_to_fit_instead_of_dropped() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let store = TelemetryStore::open(&dir.path().canonicalize()?)?;
        let short = log_record("stdout", b"plain \"quoted\" line", false);
        assert_eq!(short["message"], "plain \"quoted\" line");
        assert_eq!(short["truncated"], false);
        let mut records = vec![short];
        for line in [
            vec![0x01; LOG_LINE_BYTES],
            vec![0xff; LOG_LINE_BYTES],
            vec![b'"'; LOG_LINE_BYTES],
            [b"\x1b[31m".repeat(409), b"xyz".to_vec()].concat(),
        ] {
            let record = log_record("stderr", &line, false);
            assert!(serde_json::to_vec(&record)?.len() <= LOG_RECORD_BYTES);
            assert_eq!(record["truncated"], true);
            assert!(!record["message"].as_str().unwrap().is_empty());
            records.push(record);
        }
        let exact = log_record("stdout", &[b'a'; LOG_LINE_BYTES], true);
        assert_eq!(exact["message"].as_str().unwrap().len(), LOG_LINE_BYTES);
        assert_eq!(exact["truncated"], true);
        records.push(exact);
        store.append_logs("placement", &records)?;
        assert_eq!(partition_count(&store, "placement", "log")?, 6);
        Ok(())
    }

    #[tokio::test]
    async fn capture_batches_lines_and_reports_rate_limited_drops() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let root = dir.path().canonicalize()?;
        let placement = format!("capture-{}", uuid::Uuid::new_v4().simple());
        let input: Vec<u8> = (0..300)
            .flat_map(|index| format!("line-{index}\n").into_bytes())
            .collect();
        capture(input.as_slice(), root.clone(), placement.clone(), "stdout").await?;
        let store = TelemetryStore::open(&root)?;
        let mut after = 0;
        let mut admitted = 0u64;
        let mut dropped = 0u64;
        loop {
            let page = store.read(Some(&placement), "log", after, 100)?;
            let records = page["records"].as_array().unwrap();
            if records.is_empty() {
                break;
            }
            for record in records {
                match record["data"]["dropped_lines"].as_u64() {
                    Some(count) => dropped += count,
                    None => admitted += 1,
                }
            }
            after = page["next"].as_u64().unwrap();
        }
        assert!((100..=200).contains(&admitted));
        assert!(dropped > 0);
        assert_eq!(admitted + dropped, 300);
        Ok(())
    }

    #[test]
    fn protected_records_are_scoped_and_detect_database_tampering() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let root = dir.path().canonicalize()?;
        let store = TelemetryStore::open(&root)?;
        store.append(
            Some("a"),
            "log",
            &json!({"secret":"private-workflow-output"}),
        )?;
        store.append(Some("b"), "log", &json!({"message":"other-project"}))?;
        assert_eq!(
            store.read(Some("a"), "log", 0, 20)?["records"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        assert!(
            store.read(None, "log", 0, 20)?["records"]
                .as_array()
                .unwrap()
                .is_empty()
        );
        let ciphertext: Vec<u8> = store.store.connection.query_row(
            "SELECT ciphertext FROM telemetry_records WHERE sequence=1",
            [],
            |r| r.get(0),
        )?;
        assert!(
            !ciphertext
                .windows(b"private-workflow-output".len())
                .any(|w| w == b"private-workflow-output")
        );
        store.store.connection.execute(
            "UPDATE telemetry_records SET placement_id='b' WHERE sequence=1",
            [],
        )?;
        assert!(store.read(Some("b"), "log", 0, 20).is_err());
        Ok(())
    }
}
