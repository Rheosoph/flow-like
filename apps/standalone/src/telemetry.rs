use crate::{enrollment::unix_time, state::StateStore, vault};
use anyhow::{Context, Result, ensure};
use chacha20poly1305::{
    KeyInit, XChaCha20Poly1305, XNonce,
    aead::{Aead, Payload},
};
use rand_core::{OsRng, RngCore};
use rusqlite::params;
use serde_json::{Value, json};
use std::{
    path::{Path, PathBuf},
    time::Duration,
};
use tokio::io::{AsyncRead, AsyncReadExt};
use tokio_util::sync::CancellationToken;
use zeroize::Zeroizing;

pub(crate) mod resources;

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
        Ok(Self {
            store: StateStore::open_with_busy_timeout(
                &state_dir.join("management.sqlite"),
                busy_timeout,
            )?,
            key: Zeroizing::new(bytes.as_slice().try_into()?),
        })
    }

    pub fn append(&self, placement: Option<&str>, kind: &str, value: &Value) -> Result<u64> {
        ensure!(
            matches!(kind, "log" | "metrics" | "message")
                || kind
                    .strip_prefix("usage-")
                    .and_then(|slot| slot.parse::<u8>().ok())
                    .is_some_and(|slot| slot < 32 && kind == format!("usage-{slot}")),
            "Invalid telemetry kind"
        );
        if let Some(id) = placement {
            flow_like_device_protocol::validate_management_id(id)?;
        }
        let plaintext = Zeroizing::new(serde_json::to_vec(value)?);
        // A placement metric includes up to 32 replica samples. Keep one complete sample
        // below the read budget and the encrypted management frame limit.
        let max_bytes = if kind == "metrics" { 8 * 1024 } else { 4096 };
        ensure!(
            plaintext.len() <= max_bytes,
            "Telemetry record exceeds its bound"
        );
        let now = unix_time()?;
        self.store.connection.execute_batch("BEGIN IMMEDIATE")?;
        let result = self.append_in_transaction(placement, kind, value, now);
        match result {
            Ok(sequence) => {
                self.store.connection.execute_batch("COMMIT")?;
                Ok(sequence)
            }
            Err(error) => {
                let _ = self.store.connection.execute_batch("ROLLBACK");
                Err(error)
            }
        }
    }

    pub(crate) fn append_in_transaction(
        &self,
        placement: Option<&str>,
        kind: &str,
        value: &Value,
        now: i64,
    ) -> Result<u64> {
        let plaintext = Zeroizing::new(serde_json::to_vec(value)?);
        ensure!(
            plaintext.len() <= if kind == "metrics" { 8192 } else { 4096 },
            "Telemetry record exceeds its bound"
        );
        self.store.connection.execute("INSERT INTO telemetry_records(placement_id,kind,created_at,ciphertext) VALUES(?1,?2,?3,x'')",params![placement,kind,now])?;
        let sequence = self.store.connection.last_insert_rowid();
        let aad = self.aad(sequence, placement, kind, now)?;
        let mut nonce = [0; 24];
        OsRng.fill_bytes(&mut nonce);
        let cipher = XChaCha20Poly1305::new_from_slice(self.key.as_ref())
            .map_err(|_| anyhow::anyhow!("Telemetry key initialization failed"))?;
        let mut sealed = nonce.to_vec();
        sealed.extend(
            cipher
                .encrypt(
                    XNonce::from_slice(&nonce),
                    Payload {
                        msg: &plaintext,
                        aad: &aad,
                    },
                )
                .map_err(|_| anyhow::anyhow!("Telemetry encryption failed"))?,
        );
        self.store.connection.execute(
            "UPDATE telemetry_records SET ciphertext=?2 WHERE sequence=?1",
            params![sequence, sealed],
        )?;
        // Telemetry eviction never touches command or cryptographic replay journals.
        self.store.connection.execute("DELETE FROM telemetry_records WHERE sequence IN (SELECT sequence FROM telemetry_records ORDER BY sequence DESC LIMIT -1 OFFSET 10000)",[])?;
        Ok(sequence.try_into()?)
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
        let aad = serde_json::to_vec(&json!([
            "flow-like/standalone/operational/v1",
            self.store.device_id(),
            domain,
            binding
        ]))?;
        let mut nonce = [0; 24];
        OsRng.fill_bytes(&mut nonce);
        let cipher = XChaCha20Poly1305::new_from_slice(self.key.as_ref())
            .map_err(|_| anyhow::anyhow!("Telemetry key initialization failed"))?;
        let mut sealed = nonce.to_vec();
        sealed.extend(
            cipher
                .encrypt(
                    XNonce::from_slice(&nonce),
                    Payload {
                        msg: &plaintext,
                        aad: &aad,
                    },
                )
                .map_err(|_| anyhow::anyhow!("Operational encryption failed"))?,
        );
        Ok(sealed)
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
        let aad = serde_json::to_vec(&json!([
            "flow-like/standalone/operational/v1",
            self.store.device_id(),
            domain,
            binding
        ]))?;
        let cipher = XChaCha20Poly1305::new_from_slice(self.key.as_ref())
            .map_err(|_| anyhow::anyhow!("Telemetry key initialization failed"))?;
        let plaintext = Zeroizing::new(
            cipher
                .decrypt(
                    XNonce::from_slice(&bytes[..24]),
                    Payload {
                        msg: &bytes[24..],
                        aad: &aad,
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
            self.store.device_id(),
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
        let cipher = XChaCha20Poly1305::new_from_slice(self.key.as_ref())
            .map_err(|_| anyhow::anyhow!("Telemetry key initialization failed"))?;
        while let Some(row) = rows.next()? {
            let sequence: i64 = row.get(0)?;
            let created_at: i64 = row.get(1)?;
            let bytes: Vec<u8> = row.get(2)?;
            let record_placement: Option<String> = row.get(3)?;
            let record_kind: String = row.get(4)?;
            ensure!(bytes.len() >= 40, "Invalid protected telemetry record");
            let plaintext = Zeroizing::new(
                cipher
                    .decrypt(
                        XNonce::from_slice(&bytes[..24]),
                        Payload {
                            msg: &bytes[24..],
                            aad: &self.aad(
                                sequence,
                                record_placement.as_deref(),
                                &record_kind,
                                created_at,
                            )?,
                        },
                    )
                    .map_err(|_| anyhow::anyhow!("Telemetry authentication failed"))?,
            );
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
        Ok(json!({"records":records,"next":next}))
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

pub async fn sample(state_dir: PathBuf, cancel: CancellationToken) -> Result<()> {
    use sysinfo::{Networks, Pid, ProcessRefreshKind, ProcessesToUpdate, System};
    let store = TelemetryStore::open(&state_dir)?;
    let mut system = System::new();
    let mut tick = tokio::time::interval(Duration::from_secs(5));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut first = true;
    let mut networks = Networks::new_with_refreshed_list();
    let mut network_window = resources::NetworkWindow::default();
    let mut sample_time = std::time::Instant::now();
    let mut previous_processes = std::collections::HashMap::new();
    let mut previous_isolation: std::collections::HashMap<
        (String, u8),
        (u64, u64, Option<(u64, u64)>, std::time::Instant),
    > = std::collections::HashMap::new();
    loop {
        tokio::select! {_=cancel.cancelled()=>return Ok(()),_=tick.tick()=>()}
        store.reconcile_usage()?;
        store.project_messages()?;
        let placements = store.store.list_placements()?;
        let metric_cohorts = placements
            .iter()
            .map(|placement| Ok((placement.id.clone(), store.metric_cohort(placement)?)))
            .collect::<Result<std::collections::HashMap<_, _>>>()?;
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
        let current_processes: std::collections::HashMap<_, _> = pids
            .iter()
            .filter_map(|pid| system.process(*pid).map(|p| (*pid, p.start_time())))
            .collect();
        networks.refresh(true);
        let network = network_window.observe(
            networks
                .iter()
                .map(|(name, value)| {
                    (
                        name.clone(),
                        (value.total_received(), value.total_transmitted()),
                    )
                })
                .collect(),
        );
        let now = std::time::Instant::now();
        let sample_seconds = now.duration_since(sample_time).as_secs_f64();
        sample_time = now;
        // CPU deltas need two samples of the same process, including after a replica restart.
        if first {
            first = false;
            previous_processes = current_processes;
            continue;
        }
        let agent = system.process(agent_pid);
        let mut device_usage = Vec::new();
        let mut expected_usage = 0u64;
        let mut placement_usage = std::collections::HashMap::new();
        for placement in &placements {
            {
                expected_usage += u64::from(placement.running_replicas);
                let usage = store.usage(placement)?;
                placement_usage.insert(
                    placement.id.clone(),
                    usage_summary(&usage, u64::from(placement.running_replicas)),
                );
                device_usage.extend(usage);
            }
        }
        store.append(None, "metrics", &json!({
            "cpu_percent": system.global_cpu_usage(),
            "memory_used_bytes": system.used_memory(),
            "memory_total_bytes": system.total_memory(),
            "agent_cpu_percent_of_one_core": agent.map(|p| p.cpu_usage()),
            "agent_memory_bytes": agent.map(|p| p.memory()),
            "logical_cpus": system.cpus().len(),
            "network": network,
            "storage_volume": resources::volume_space(&state_dir),
            "placements": placements.len(),
            "desired_replicas": placements.iter().map(|p| u64::from(p.desired_replicas)).sum::<u64>(),
            "ready_replicas": placements.iter().map(|p| u64::from(p.ready_replicas)).sum::<u64>(),
            "sample_seconds": sample_seconds,
            "usage": usage_summary(&device_usage, expected_usage),
            "usage_retained": store.retained_usage(None, None)?
        }))?;
        let mut current_isolation = std::collections::HashMap::new();
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
                        crate::isolation::sample(pid, &placement.id, replica.slot, &state_dir).ok()
                    }) {
                        let key = (placement.id.clone(), replica.slot);
                        let now = std::time::Instant::now();
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
                    "disk_quota": if isolated { crate::isolation::disk_sample(&state_dir, &placement.id).ok() } else { None },
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
        previous_processes = current_processes;
        previous_isolation = current_isolation;
    }
}

pub async fn capture<R: AsyncRead + Unpin>(
    mut reader: R,
    state_dir: PathBuf,
    placement: String,
    stream: &'static str,
) -> Result<()> {
    let store = TelemetryStore::open(&state_dir)?;
    let mut buffer = [0; 1024];
    let mut line = Vec::with_capacity(2048);
    let mut truncated = false;
    let mut window = unix_time()?;
    let mut count = 0u32;
    loop {
        let size = reader.read(&mut buffer).await?;
        if size == 0 {
            if !line.is_empty() {
                let _ = append_line(&store, &placement, stream, &line, truncated);
            }
            return Ok(());
        }
        for byte in &buffer[..size] {
            if *byte == b'\n' {
                let now = unix_time()?;
                if now != window {
                    window = now;
                    count = 0;
                }
                if count < 100 {
                    let _ = append_line(&store, &placement, stream, &line, truncated);
                    count += 1;
                }
                line.clear();
                truncated = false;
            } else if line.len() < 2048 {
                line.push(*byte);
            } else {
                truncated = true;
            }
        }
    }
}

fn append_line(
    store: &TelemetryStore,
    placement: &str,
    stream: &str,
    line: &[u8],
    truncated: bool,
) -> Result<()> {
    let text = String::from_utf8_lossy(line);
    store
        .append(
            Some(placement),
            "log",
            &json!({"stream":stream,"message":text,"truncated":truncated}),
        )
        .context("Persist protected service log")?;
    Ok(())
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
