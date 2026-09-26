use crate::{enrollment::unix_time, telemetry::TelemetryStore, usage::RuntimeUsageSnapshot};
use anyhow::{Context, Result, ensure};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// Build a full authorized snapshot; caller encrypts it before leaving the device.
pub(crate) fn fleet_snapshot(
    state_dir: &std::path::Path,
    scope: &flow_like_device_protocol::ManagementScope,
    kind: flow_like_device_protocol::FleetKind,
    boot_id: &str,
    now: i64,
) -> Result<Value> {
    use flow_like_device_protocol::{FleetKind, ManagementScope};
    let telemetry = TelemetryStore::open(state_dir)?;
    let placements = telemetry.store.scoped_placements(scope)?;
    let value = match kind {
        FleetKind::Status => json!({"device_id": telemetry.store.device_id(),
            "boot_id":boot_id,"observed_at":now.checked_mul(1000).context("Fleet timestamp overflow")?,
            "placements":placements.into_iter().map(crate::management::inspection_placement).collect::<Vec<_>>()}),
        FleetKind::Metrics => match scope {
            ManagementScope::Device => {
                let mut sample = telemetry.latest_metrics(None)?;
                let mut projects =
                    std::collections::BTreeMap::<&str, Vec<&crate::state::PlacementRecord>>::new();
                for placement in &placements {
                    if let Some(project) = placement.config["project_id"].as_str() {
                        projects.entry(project).or_default().push(placement);
                    }
                }
                let mut project_samples = Vec::new();
                let mut bytes = serde_json::to_vec(&sample)?.len();
                for (project, placements) in projects {
                    let item = json!({"project_id":project,"sample":telemetry.project_metrics_from(project, &placements)?});
                    bytes = bytes
                        .checked_add(serde_json::to_vec(&item)?.len() + 1)
                        .context("Fleet snapshot size overflow")?;
                    ensure!(
                        bytes <= flow_like_device_protocol::MAX_FLEET_PLAINTEXT - 1024,
                        "Fleet snapshot exceeds its complete payload limit"
                    );
                    project_samples.push(item);
                }
                sample["projects"] = json!(project_samples);
                sample
            }
            ManagementScope::Project { project_id } => telemetry
                .project_metrics_from(project_id, &placements.iter().collect::<Vec<_>>())?,
            ManagementScope::Placement { placement_id, .. } => {
                if placements.is_empty() {
                    json!({"records":[],"next":0})
                } else {
                    telemetry.latest_metrics(Some(placement_id))?
                }
            }
        },
    };
    ensure!(
        serde_json::to_vec(&value)?.len() <= flow_like_device_protocol::MAX_FLEET_PLAINTEXT - 1024,
        "Fleet snapshot exceeds its complete payload limit"
    );
    Ok(value)
}

pub(crate) fn migrate(db: &Connection) -> Result<()> {
    db.execute_batch(r#"
        CREATE TABLE usage_processes (
            run_id TEXT PRIMARY KEY NOT NULL, placement_id TEXT NOT NULL,
            slot INTEGER NOT NULL, process_id INTEGER NOT NULL,
            config_revision INTEGER NOT NULL, intent_revision INTEGER NOT NULL,
            state TEXT NOT NULL, updated_at INTEGER NOT NULL, ciphertext BLOB NOT NULL
        );
        CREATE INDEX usage_process_placement ON usage_processes(placement_id,state);
        CREATE TABLE usage_totals (
            placement_id TEXT PRIMARY KEY NOT NULL, project_id TEXT NOT NULL,
            updated_at INTEGER NOT NULL, ciphertext BLOB NOT NULL
        );
        CREATE INDEX usage_project ON usage_totals(project_id);
        CREATE TABLE operational_outbox (
            sequence INTEGER PRIMARY KEY AUTOINCREMENT, kind TEXT NOT NULL,
            source_id TEXT NOT NULL, placement_id TEXT, project_id TEXT,
            state TEXT NOT NULL, config_revision INTEGER, intent_revision INTEGER,
            slot INTEGER, process_id INTEGER, created_at INTEGER NOT NULL
        );
        CREATE TABLE operational_message_scopes (
            sequence INTEGER PRIMARY KEY REFERENCES telemetry_records(sequence) ON DELETE CASCADE,
            project_id TEXT
        );
        CREATE TABLE operational_coverage (singleton INTEGER PRIMARY KEY CHECK(singleton=1),dropped INTEGER NOT NULL);
        INSERT INTO operational_coverage VALUES(1,0);
        CREATE TRIGGER operational_outbox_bound AFTER INSERT ON operational_outbox BEGIN
            DELETE FROM operational_outbox WHERE sequence IN (SELECT sequence FROM operational_outbox ORDER BY sequence DESC LIMIT -1 OFFSET 10000);
            UPDATE operational_coverage SET dropped=dropped+changes() WHERE singleton=1;
        END;
        CREATE TRIGGER operational_command_insert AFTER INSERT ON management_operations BEGIN
            INSERT INTO operational_outbox(kind,source_id,placement_id,project_id,state,created_at)
            VALUES('operation',NEW.operation_id,NEW.placement_id,NEW.project_id,
                CASE json_extract(NEW.result_json,'$.state') WHEN 'accepted' THEN 'accepted' WHEN 'completed' THEN 'completed' WHEN 'failed' THEN 'failed' ELSE 'unknown' END,
                unixepoch());
        END;
        CREATE TRIGGER operational_command_update AFTER UPDATE OF result_json ON management_operations
        WHEN OLD.result_json != NEW.result_json BEGIN
            INSERT INTO operational_outbox(kind,source_id,placement_id,project_id,state,created_at)
            VALUES('operation',NEW.operation_id,NEW.placement_id,NEW.project_id,
                CASE json_extract(NEW.result_json,'$.state') WHEN 'accepted' THEN 'accepted' WHEN 'completed' THEN 'completed' WHEN 'failed' THEN 'failed' ELSE 'unknown' END,
                unixepoch());
        END;
        CREATE TRIGGER operational_replica_insert AFTER INSERT ON placement_replicas BEGIN
            INSERT INTO operational_outbox(kind,source_id,placement_id,project_id,state,config_revision,intent_revision,slot,process_id,created_at)
            SELECT 'replica',NEW.placement_id,NEW.placement_id,i.project_id,NEW.observed_state,NEW.config_revision,NEW.intent_revision,NEW.slot,NEW.process_id,unixepoch()
            FROM placement_identities i WHERE i.id=NEW.placement_id COLLATE BINARY;
        END;
        CREATE TRIGGER operational_replica_update AFTER UPDATE ON placement_replicas
        WHEN OLD.observed_state != NEW.observed_state OR OLD.process_id IS NOT NEW.process_id
            OR OLD.config_revision != NEW.config_revision OR OLD.intent_revision != NEW.intent_revision BEGIN
            INSERT INTO operational_outbox(kind,source_id,placement_id,project_id,state,config_revision,intent_revision,slot,process_id,created_at)
            SELECT 'replica',NEW.placement_id,NEW.placement_id,i.project_id,NEW.observed_state,NEW.config_revision,NEW.intent_revision,NEW.slot,NEW.process_id,unixepoch()
            FROM placement_identities i WHERE i.id=NEW.placement_id COLLATE BINARY;
        END;
        CREATE TRIGGER operational_replica_delete AFTER DELETE ON placement_replicas BEGIN
            INSERT INTO operational_outbox(kind,source_id,placement_id,project_id,state,config_revision,intent_revision,slot,process_id,created_at)
            SELECT 'replica',OLD.placement_id,OLD.placement_id,i.project_id,'removed',OLD.config_revision,OLD.intent_revision,OLD.slot,OLD.process_id,unixepoch()
            FROM placement_identities i WHERE i.id=OLD.placement_id COLLATE BINARY;
        END;
    "#)?;
    Ok(())
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProcessBinding {
    pub run_id: String,
    pub placement_id: String,
    pub slot: u8,
    pub process_id: u32,
    pub config_revision: u64,
    pub intent_revision: u64,
}

impl ProcessBinding {
    pub(crate) fn is_physically_bound(&self, store: &crate::state::StateStore) -> Result<bool> {
        Ok(store.connection.query_row("SELECT EXISTS(SELECT 1 FROM placement_replicas WHERE placement_id=?1 COLLATE BINARY AND slot=?2 AND process_id=?3 AND config_revision=?4 AND intent_revision=?5 AND observed_state IN ('starting','running','stopping'))",params![self.placement_id,self.slot,self.process_id,self.config_revision,self.intent_revision],|row|row.get(0))?)
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct Counters {
    invocations_started: u64,
    invocations_succeeded: u64,
    invocations_failed: u64,
    invocations_cancelled: u64,
    runtime_messages: u64,
    request_payload_bytes: u64,
    response_payload_bytes: u64,
    concurrency_rejections: u64,
}
impl Counters {
    fn from(snapshot: &RuntimeUsageSnapshot) -> Self {
        Self {
            invocations_started: snapshot.invocations_started,
            invocations_succeeded: snapshot.invocations_succeeded,
            invocations_failed: snapshot.invocations_failed,
            invocations_cancelled: snapshot.invocations_cancelled,
            runtime_messages: snapshot.runtime_messages,
            request_payload_bytes: snapshot.request_payload_bytes,
            response_payload_bytes: snapshot.response_payload_bytes,
            concurrency_rejections: snapshot.concurrency_rejections,
        }
    }
    fn add_delta(&mut self, current: &Self, previous: &Self) -> Result<()> {
        macro_rules! add { ($($field:ident),*) => { $(self.$field = self.$field.checked_add(current.$field.checked_sub(previous.$field).context("Usage counter regressed")?).context("Retained usage overflow")?;)* } }
        add!(
            invocations_started,
            invocations_succeeded,
            invocations_failed,
            invocations_cancelled,
            runtime_messages,
            request_payload_bytes,
            response_payload_bytes,
            concurrency_rejections
        );
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Process {
    binding: ProcessBinding,
    project_id: String,
    snapshot: Option<RuntimeUsageSnapshot>,
    state: String,
    started_at: i64,
    updated_at: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Totals {
    version: u32,
    placement_id: String,
    project_id: String,
    since: i64,
    through: i64,
    counters: Counters,
    registered_runs: u64,
    reported_runs: u64,
    finalized_runs: u64,
    incomplete_runs: u64,
    unreported_runs: u64,
}

impl TelemetryStore {
    fn transaction<T>(&self, work: impl FnOnce() -> Result<T>) -> Result<T> {
        self.store.connection.execute_batch("BEGIN IMMEDIATE")?;
        match work() {
            Ok(value) => {
                self.store.connection.execute_batch("COMMIT")?;
                Ok(value)
            }
            Err(error) => {
                let _ = self.store.connection.execute_batch("ROLLBACK");
                Err(error)
            }
        }
    }

    fn process(&self, run: &str) -> Result<Option<Process>> {
        let row: Option<(String,String,i64,Vec<u8>)> = self.store.connection.query_row("SELECT placement_id,state,updated_at,ciphertext FROM usage_processes WHERE run_id=?1", [run], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?))).optional()?;
        row.map(|(placement, state, updated, bytes)| {
            let process: Process = self.open_payload(
                "usage-process",
                &json!([run, placement, state, updated]),
                &bytes,
            )?;
            ensure!(
                process.binding.run_id == run
                    && process.binding.placement_id == placement
                    && process.state == state
                    && process.updated_at == updated,
                "Usage checkpoint binding differs"
            );
            Ok(process)
        })
        .transpose()
    }

    fn totals(&self, placement: &str, project: &str, now: i64) -> Result<Totals> {
        let row: Option<(String, i64, Vec<u8>)> = self
            .store
            .connection
            .query_row(
                "SELECT project_id,updated_at,ciphertext FROM usage_totals WHERE placement_id=?1",
                [placement],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()?;
        if let Some((stored_project, updated, bytes)) = row {
            ensure!(stored_project == project, "Retained usage project differs");
            let value: Totals = self.open_payload(
                "usage-totals",
                &json!([placement, project, updated]),
                &bytes,
            )?;
            ensure!(
                value.version == 1
                    && value.placement_id == placement
                    && value.project_id == project
                    && value.through == updated,
                "Retained usage binding differs"
            );
            Ok(value)
        } else {
            Ok(Totals {
                version: 1,
                placement_id: placement.into(),
                project_id: project.into(),
                since: now,
                through: now,
                counters: Default::default(),
                registered_runs: 0,
                reported_runs: 0,
                finalized_runs: 0,
                incomplete_runs: 0,
                unreported_runs: 0,
            })
        }
    }

    fn save_usage(&self, process: &Process, totals: &Totals) -> Result<()> {
        let binding = &process.binding;
        let bytes = self.seal_payload(
            "usage-process",
            &json!([
                binding.run_id,
                binding.placement_id,
                process.state,
                process.updated_at
            ]),
            process,
        )?;
        self.store.connection.execute("INSERT INTO usage_processes(run_id,placement_id,slot,process_id,config_revision,intent_revision,state,updated_at,ciphertext) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9) ON CONFLICT(run_id) DO UPDATE SET state=excluded.state,updated_at=excluded.updated_at,ciphertext=excluded.ciphertext", params![binding.run_id,binding.placement_id,binding.slot,binding.process_id,binding.config_revision,binding.intent_revision,process.state,process.updated_at,bytes])?;
        let bytes = self.seal_payload(
            "usage-totals",
            &json!([totals.placement_id, totals.project_id, totals.through]),
            totals,
        )?;
        self.store.connection.execute("INSERT INTO usage_totals(placement_id,project_id,updated_at,ciphertext) VALUES(?1,?2,?3,?4) ON CONFLICT(placement_id) DO UPDATE SET updated_at=excluded.updated_at,ciphertext=excluded.ciphertext", params![totals.placement_id,totals.project_id,totals.through,bytes])?;
        Ok(())
    }

    fn current_project(&self, binding: &ProcessBinding) -> Result<String> {
        ensure!(
            uuid::Uuid::parse_str(&binding.run_id)?.to_string() == binding.run_id
                && binding.slot < 32,
            "Invalid usage process identity"
        );
        ensure!(
            self.store.replica_is_current(
                &binding.placement_id,
                binding.slot,
                binding.config_revision,
                binding.intent_revision,
                binding.process_id
            )?,
            "Usage process is no longer current"
        );
        let project: String = self.store.connection.query_row(
            "SELECT project_id FROM placement_identities WHERE id=?1 COLLATE BINARY AND retired=0",
            [&binding.placement_id],
            |row| row.get(0),
        )?;
        Ok(project)
    }

    pub(crate) fn begin_usage(&self, binding: &ProcessBinding) -> Result<()> {
        self.transaction(|| {
            let project = self.current_project(binding)?;
            if let Some(process) = self.process(&binding.run_id)? {
                ensure!(
                    process.binding == *binding
                        && process.project_id == project
                        && process.state == "active",
                    "Usage process binding is already closed or differs"
                );
                return Ok(());
            }
            let already_active: bool = self.store.connection.query_row("SELECT EXISTS(SELECT 1 FROM usage_processes WHERE placement_id=?1 AND slot=?2 AND process_id=?3 AND config_revision=?4 AND intent_revision=?5 AND state='active')",params![binding.placement_id,binding.slot,binding.process_id,binding.config_revision,binding.intent_revision],|r|r.get(0))?;
            ensure!(!already_active,"Usage process already has an active reporting incarnation");
            let now = unix_time()?;
            let mut totals = self.totals(&binding.placement_id, &project, now)?;
            totals.registered_runs = totals
                .registered_runs
                .checked_add(1)
                .context("Usage process count overflow")?;
            totals.through = totals.through.max(now);
            let process = Process {
                binding: binding.clone(),
                project_id: project,
                snapshot: None,
                state: "active".into(),
                started_at: now,
                updated_at: now,
            };
            self.save_usage(&process, &totals)
        })
    }

    pub(crate) fn record_usage(
        &self,
        binding: &ProcessBinding,
        snapshot: &RuntimeUsageSnapshot,
        finalized: bool,
    ) -> Result<()> {
        self.transaction(|| {
            ensure!(self.process_is_bound(binding)?,"Usage process slot binding ended");
            let project: String = self.store.connection.query_row("SELECT project_id FROM placement_identities WHERE id=?1 COLLATE BINARY AND retired=0",[&binding.placement_id],|row|row.get(0))?;
            let mut process = self.process(&binding.run_id)?.context("Usage process was not registered")?;
            ensure!(process.binding == *binding && process.project_id == project, "Usage process binding differs");
            if process.snapshot.as_ref() == Some(snapshot) {
                ensure!(process.state == if finalized {"finalized"} else {"active"}, "Usage retry changed finality");
                return Ok(());
            }
            ensure!(process.state == "active" && snapshot.validate_after(process.snapshot.as_ref()), "Usage checkpoint regressed or process ended");
            let now = unix_time()?;
            let mut totals = self.totals(&binding.placement_id,&project,now)?;
            totals.counters.add_delta(&Counters::from(snapshot), &process.snapshot.as_ref().map(Counters::from).unwrap_or_default())?;
            if process.snapshot.is_none() { totals.reported_runs = totals.reported_runs.checked_add(1).context("Reported usage count overflow")?; }
            if finalized {
                ensure!(snapshot.in_flight == 0, "Final usage still has active invocations");
                process.state = "finalized".into();
                totals.finalized_runs = totals.finalized_runs.checked_add(1).context("Finalized usage count overflow")?;
            }
            process.snapshot = Some(snapshot.clone());
            process.updated_at = process.updated_at.max(now);
            totals.through = totals.through.max(now);
            self.save_usage(&process,&totals)?;
            self.append_in_transaction(Some(&binding.placement_id), &format!("usage-{}",binding.slot), &json!({"project_id":project,"replica_slot":binding.slot,"process_id":binding.process_id,"process_run_id":binding.run_id,"config_revision":binding.config_revision,"intent_revision":binding.intent_revision,"counters":snapshot}), now)?;
            Ok(())
        })
    }

    pub(crate) fn finish_usage(&self, run_id: &str) -> Result<()> {
        self.transaction(|| {
            let Some(mut process) = self.process(run_id)? else {
                return Ok(());
            };
            if process.state != "active" {
                return Ok(());
            }
            let now = unix_time()?;
            let mut totals =
                self.totals(&process.binding.placement_id, &process.project_id, now)?;
            totals.incomplete_runs = totals
                .incomplete_runs
                .checked_add(1)
                .context("Incomplete usage count overflow")?;
            if process.snapshot.is_none() {
                totals.unreported_runs = totals
                    .unreported_runs
                    .checked_add(1)
                    .context("Unreported usage count overflow")?;
            }
            process.state = "incomplete".into();
            process.updated_at = process.updated_at.max(now);
            totals.through = totals.through.max(now);
            self.save_usage(&process, &totals)
        })
    }
}

impl TelemetryStore {
    pub(crate) fn usage_is_active(&self, value: &Value) -> Result<bool> {
        let Some(run) = value["process_run_id"].as_str() else {
            return Ok(false);
        };
        let Some(process) = self.process(run)? else {
            return Ok(false);
        };
        Ok(process.state == "active"
            && value["process_id"].as_u64() == Some(u64::from(process.binding.process_id))
            && value["config_revision"].as_u64() == Some(process.binding.config_revision)
            && value["intent_revision"].as_u64() == Some(process.binding.intent_revision)
            && value["replica_slot"].as_u64() == Some(u64::from(process.binding.slot)))
    }

    fn process_is_bound(&self, binding: &ProcessBinding) -> Result<bool> {
        // Intent revokes resource admission immediately. Checkpoints remain bound to
        // their original physical child until it exits, including graceful drain.
        binding.is_physically_bound(&self.store)
    }

    pub(crate) fn reconcile_usage(&self) -> Result<()> {
        let runs: Vec<String> = self
            .store
            .connection
            .prepare("SELECT run_id FROM usage_processes WHERE state='active'")?
            .query_map([], |row| row.get(0))?
            .collect::<std::result::Result<_, _>>()?;
        for run in runs {
            if let Some(process) = self.process(&run)? {
                if !self.process_is_bound(&process.binding)? {
                    self.finish_usage(&run)?;
                }
            }
        }
        self.prune_usage_checkpoints(unix_time()?)
    }

    fn prune_usage_checkpoints(&self, now: i64) -> Result<()> {
        self.transaction(|| {
            self.store.connection.execute("DELETE FROM usage_processes WHERE state!='active' AND (updated_at<?1 OR run_id IN (SELECT run_id FROM usage_processes WHERE state!='active' ORDER BY updated_at DESC,run_id DESC LIMIT -1 OFFSET 10000))",[now.saturating_sub(86400)])?;
            Ok(())
        })
    }

    pub(crate) fn retained_usage(
        &self,
        placement: Option<&str>,
        project: Option<&str>,
    ) -> Result<Value> {
        let now = unix_time()?;
        let ids: Vec<(String,String)> = self.store.connection.prepare("SELECT t.placement_id,t.project_id FROM usage_totals t JOIN placement_identities i ON i.id=t.placement_id COLLATE BINARY AND i.project_id=t.project_id COLLATE BINARY WHERE (?1 IS NULL OR t.placement_id=?1 COLLATE BINARY) AND (?2 IS NULL OR t.project_id=?2 COLLATE BINARY)")?.query_map(params![placement,project], |row| Ok((row.get(0)?,row.get(1)?)))?.collect::<std::result::Result<_,_>>()?;
        let mut counters = Counters::default();
        let mut since = None::<i64>;
        let mut through = None::<i64>;
        let mut registered = 0u64;
        let mut reported = 0u64;
        let mut finalized = 0u64;
        let mut incomplete = 0u64;
        let mut unreported = 0u64;
        for (id, project) in &ids {
            let totals = self.totals(id, project, now)?;
            counters.add_delta(&totals.counters, &Counters::default())?;
            since = Some(since.map_or(totals.since, |v| v.min(totals.since)));
            through = Some(through.map_or(totals.through, |v| v.max(totals.through)));
            macro_rules! sum {
                ($target:ident,$field:ident) => {
                    $target = $target
                        .checked_add(totals.$field)
                        .context("Retained usage aggregation overflow")?;
                };
            }
            sum!(registered, registered_runs);
            sum!(reported, reported_runs);
            sum!(finalized, finalized_runs);
            sum!(incomplete, incomplete_runs);
            sum!(unreported, unreported_runs);
        }
        let mut fresh = 0u64;
        let mut stale = 0u64;
        let mut awaiting = 0u64;
        let mut ended_pending = 0u64;
        let runs: Vec<String> = self.store.connection.prepare("SELECT p.run_id FROM usage_processes p JOIN placement_identities i ON i.id=p.placement_id COLLATE BINARY WHERE p.state='active' AND (?1 IS NULL OR p.placement_id=?1 COLLATE BINARY) AND (?2 IS NULL OR i.project_id=?2 COLLATE BINARY)")?.query_map(params![placement,project], |r|r.get(0))?.collect::<std::result::Result<_,_>>()?;
        for run in runs {
            let p = self.process(&run)?.context("Usage process disappeared")?;
            if !self.process_is_bound(&p.binding)? {
                ended_pending += 1;
            } else if p.snapshot.is_none() {
                awaiting += 1;
            } else if p.updated_at < now - 15 || p.updated_at > now + 30 {
                stale += 1;
            } else {
                fresh += 1;
            }
        }
        Ok(
            json!({"version":1,"scope":"supervised_services","window":"retained_observed_process_checkpoints","since":since,"through":through,
            "counters": if reported==0{Value::Null}else{serde_json::to_value(counters)?},
            "coverage":{"registered_runs":registered,"reported_runs":reported,"finalized_runs":finalized,"incomplete_runs":incomplete,"unreported_runs":unreported,
                "fresh_active_runs":fresh,"stale_active_runs":stale,"awaiting_first_report_runs":awaiting,"ended_pending_runs":ended_pending},
            "tail_loss_possible":incomplete>0 || unreported>0 || stale>0 || awaiting>0 || ended_pending>0,
            "billing":false}),
        )
    }

    /// A process ID can be reused after a crash. Bind resource observations to
    /// the authenticated reporting incarnation for every current replica.
    pub(crate) fn metric_cohort(
        &self,
        placement: &crate::state::PlacementRecord,
    ) -> Result<Option<String>> {
        if placement.running_replicas == 0 || placement.replicas.len() > 32 {
            return Ok(None);
        }
        let runs: Vec<String> = self.store.connection.prepare(
            "SELECT u.run_id FROM usage_processes u JOIN placement_replicas r ON r.placement_id=u.placement_id AND r.slot=u.slot AND r.process_id=u.process_id AND r.config_revision=u.config_revision AND r.intent_revision=u.intent_revision WHERE u.placement_id=?1 COLLATE BINARY AND u.state='active' AND r.observed_state='running' ORDER BY u.slot LIMIT 33"
        )?.query_map([&placement.id], |row| row.get(0))?.collect::<std::result::Result<_, _>>()?;
        if runs.len() != usize::from(placement.running_replicas) {
            return Ok(None);
        }
        let mut identities = std::collections::BTreeMap::new();
        for run in runs {
            let Some(process) = self.process(&run)? else {
                return Ok(None);
            };
            let binding = &process.binding;
            if process.state != "active"
                || process.project_id.as_str()
                    != placement.config["project_id"].as_str().unwrap_or("")
                || binding.placement_id != placement.id
                || binding.config_revision != placement.config_revision
                || binding.intent_revision != placement.intent_revision
                || !placement.replicas.iter().any(|replica| {
                    replica.slot == binding.slot
                        && replica.process_id == Some(binding.process_id)
                        && replica.observed_state == crate::state::ObservedState::Running
                        && replica.config_revision == binding.config_revision
                        && replica.intent_revision == binding.intent_revision
                })
                || identities
                    .insert(binding.slot, (binding.process_id, run))
                    .is_some()
            {
                return Ok(None);
            }
        }
        Ok(Some(
            blake3::hash(&serde_json::to_vec(&(
                "flow-like/metric-cohort/v1",
                &placement.id,
                identities,
            ))?)
            .to_hex()
            .to_string(),
        ))
    }

    pub(crate) fn project_metrics(&self, project: &str) -> Result<Value> {
        let placements =
            self.store
                .scoped_placements(&flow_like_device_protocol::ManagementScope::Project {
                    project_id: project.into(),
                })?;
        self.project_metrics_from(project, &placements.iter().collect::<Vec<_>>())
    }

    fn project_metrics_from(
        &self,
        project: &str,
        placements: &[&crate::state::PlacementRecord],
    ) -> Result<Value> {
        let now = unix_time()?;
        let mut snapshots = Vec::new();
        let mut expected = 0u64;
        let mut desired = 0u64;
        let mut ready = 0u64;
        let mut running = 0u64;
        let mut observed = 0u64;
        let mut sampled = 0u64;
        let mut awaiting = 0u64;
        let mut cpu = Some(0.0f64);
        let mut memory = Some(0u64);
        let mut read_rate = Some(0.0f64);
        let mut write_rate = Some(0.0f64);
        let mut oldest_sample = None::<i64>;
        for placement in placements
            .iter()
            .filter(|p| p.config["project_id"].as_str() == Some(project))
        {
            {
                snapshots.extend(self.usage(placement)?);
                expected += u64::from(placement.running_replicas);
            }
            desired += u64::from(placement.desired_replicas);
            ready += u64::from(placement.ready_replicas);
            running += u64::from(placement.running_replicas);
            if placement.running_replicas == 0 {
                continue;
            }
            let sample = self.latest_metrics(Some(&placement.id))?;
            let record = &sample["records"][0];
            let data = &record["data"];
            let timestamp = record["timestamp"].as_i64();
            let fresh = timestamp.is_some_and(|t| t >= now - 15 && t <= now + 30)
                && data["project_id"].as_str() == Some(project)
                && data["config_revision"].as_u64() == Some(placement.config_revision)
                && data["intent_revision"].as_u64() == Some(placement.intent_revision)
                && data["running_replicas"].as_u64() == Some(u64::from(placement.running_replicas))
                && self
                    .metric_cohort(placement)?
                    .is_some_and(|cohort| data["process_cohort"].as_str() == Some(cohort.as_str()));
            if !fresh {
                awaiting += 1;
                cpu = None;
                memory = None;
                read_rate = None;
                write_rate = None;
                continue;
            }
            sampled += 1;
            let t = timestamp.expect("fresh sample timestamp");
            oldest_sample = Some(oldest_sample.map_or(t, |v| v.min(t)));
            let measured = data["processes_observed"]
                .as_u64()
                .filter(|n| *n <= u64::from(placement.running_replicas))
                .unwrap_or(0);
            observed += measured;
            if measured != u64::from(placement.running_replicas) {
                cpu = None;
                memory = None;
                read_rate = None;
                write_rate = None;
                continue;
            }
            cpu = cpu
                .zip(
                    data["cpu_percent"]
                        .as_f64()
                        .filter(|v| v.is_finite() && *v >= 0.0),
                )
                .map(|(a, b)| a + b)
                .filter(|v| v.is_finite());
            for (total, field) in [
                (&mut read_rate, "read_bytes_per_second"),
                (&mut write_rate, "written_bytes_per_second"),
            ] {
                *total = total
                    .zip(
                        data["io"][field]
                            .as_f64()
                            .filter(|v| v.is_finite() && *v >= 0.0),
                    )
                    .map(|(a, b)| a + b)
                    .filter(|v| v.is_finite());
            }
            memory = memory
                .zip(data["memory_bytes"].as_u64())
                .and_then(|(a, b)| a.checked_add(b));
        }
        if observed == 0 {
            cpu = None;
            memory = None;
            read_rate = None;
            write_rate = None;
        }
        Ok(
            json!({"records":[{"timestamp":now,"data":{"project_id":project,"desired_replicas":desired,"ready_replicas":ready,
            "cpu_percent":cpu,"cpu_basis":"one_logical_cpu","memory_bytes":memory,"running_replicas":running,"processes_observed":observed,
            "io":{"basis":"observed_placement_workers","read_bytes_per_second":read_rate,"written_bytes_per_second":write_rate},
            "metric_coverage":{"sampled_placements":sampled,"missing_or_stale_placements":awaiting,"oldest_sample_at":oldest_sample,"freshness_seconds":15},
            "usage":crate::telemetry::usage_summary(&snapshots,expected),"usage_retained":self.retained_usage(None,Some(project))?}}],"next":0}),
        )
    }

    pub(crate) fn project_messages(&self) -> Result<()> {
        self.transaction(|| {
            type Row = (i64,String,String,Option<String>,Option<String>,String,Option<i64>,Option<i64>,Option<i64>,Option<i64>,i64);
            let rows: Vec<Row> = self.store.connection.prepare("SELECT sequence,kind,source_id,placement_id,project_id,state,config_revision,intent_revision,slot,process_id,created_at FROM operational_outbox ORDER BY sequence LIMIT 256")?.query_map([], |r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?,r.get(7)?,r.get(8)?,r.get(9)?,r.get(10)?)))?.collect::<std::result::Result<_,_>>()?;
            for (id,kind,source,placement,project,state,config,intent,slot,pid,time) in rows {
                let value=json!({"version":1,"transition_id":id,"kind":kind,"source_id":source,"placement_id":placement,"project_id":project,"state":state,"config_revision":config,"intent_revision":intent,"replica_slot":slot,"process_id":pid});
                let sequence=self.append_in_transaction(placement.as_deref(),"message",&value,time)?;
                self.store.connection.execute("INSERT INTO operational_message_scopes(sequence,project_id) VALUES(?1,?2)",params![sequence,project])?;
                self.store.connection.execute("DELETE FROM operational_outbox WHERE sequence=?1",[id])?;
            }
            Ok(())
        })
    }

    pub(crate) fn messages(
        &self,
        placement: Option<&str>,
        project: Option<&str>,
        after: u64,
        limit: u32,
    ) -> Result<Value> {
        let mut result = self.read_filtered(placement, "message", after, limit, false, project)?;
        // Global loss counts are device-only; scoped readers must not learn other project activity.
        if placement.is_none() && project.is_none() {
            let dropped: i64 = self.store.connection.query_row(
                "SELECT dropped FROM operational_coverage WHERE singleton=1",
                [],
                |r| r.get(0),
            )?;
            result["outbox_dropped"] = json!(dropped);
        }
        result["retention_limit"] = json!(10000);
        Ok(result)
    }
}

pub(crate) struct UsageProcessGuard {
    root: std::path::PathBuf,
    run: String,
    armed: bool,
}
impl UsageProcessGuard {
    pub(crate) fn new(root: &std::path::Path, run: &str) -> Self {
        Self {
            root: root.to_owned(),
            run: run.to_owned(),
            armed: false,
        }
    }
    pub(crate) fn arm(&mut self) {
        self.armed = true;
    }
}
impl Drop for UsageProcessGuard {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        if let Ok(store) =
            TelemetryStore::open_with_busy_timeout(&self.root, std::time::Duration::ZERO)
        {
            let _ = store.finish_usage(&self.run);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{DesiredState, ObservedState, StateStore};

    #[test]
    fn unattended_fleet_projection_filters_scope_and_never_exports_configuration() -> Result<()> {
        use flow_like_device_protocol::{FleetKind, ManagementScope};
        let (dir, _, _) = fixture()?;
        let scope = ManagementScope::Project {
            project_id: "project".into(),
        };
        let status = fleet_snapshot(dir.path(), &scope, FleetKind::Status, "boot", 100)?;
        assert_eq!(status["observed_at"], 100_000);
        assert_eq!(status["placements"].as_array().unwrap().len(), 1);
        assert_eq!(status["placements"][0]["id"], "service");
        for private in ["auth_secret", "project_path", "other-project", "variables"] {
            assert!(!status.to_string().contains(private));
        }
        let metrics = fleet_snapshot(dir.path(), &scope, FleetKind::Metrics, "boot", 100)?;
        assert_eq!(metrics["records"][0]["data"]["project_id"], "project");
        assert!(!metrics.to_string().contains("other-project"));
        let missing = ManagementScope::Placement {
            project_id: "other-project".into(),
            placement_id: "service".into(),
        };
        assert_eq!(
            fleet_snapshot(dir.path(), &missing, FleetKind::Status, "boot", 100)?["placements"],
            json!([])
        );
        assert_eq!(
            fleet_snapshot(dir.path(), &missing, FleetKind::Metrics, "boot", 100)?["records"],
            json!([])
        );
        Ok(())
    }

    fn fixture() -> Result<(tempfile::TempDir, StateStore, TelemetryStore)> {
        let dir = tempfile::tempdir()?;
        let mut state = StateStore::open(&dir.path().join("management.sqlite"))?;
        for (id, project) in [("service", "project"), ("other", "other-project")] {
            state.upsert_placement(id,&json!({"id":id,"project_id":project,"deployment_id":"deployment","revision":"one","source":"offline","project_path":dir.path(),"max_replicas":2,"hosting":{"host":"127.0.0.1","port":8080,"max_in_flight":4,"request_timeout_secs":30,"auth_secret":"service"},"events":[{"event_id":"http","event_version":[1,0,0],"board_version":[1,0,0]}]}),DesiredState::Running)?;
            state.set_replica_count(id, 1, 2)?;
            for slot in 0..2 {
                assert!(state.claim_replica(id, slot, 1, 1)?);
                state.record_replica(
                    id,
                    slot,
                    1,
                    1,
                    ObservedState::Starting,
                    Some(100 + u32::from(slot)),
                    None,
                )?;
                assert!(state.record_replica_prepared(id, slot, 1, 1, 100 + u32::from(slot))?);
            }
        }
        let telemetry = TelemetryStore::open(dir.path())?;
        Ok((dir, state, telemetry))
    }
    fn binding(placement: &str, slot: u8) -> ProcessBinding {
        ProcessBinding {
            run_id: uuid::Uuid::new_v4().to_string(),
            placement_id: placement.into(),
            slot,
            process_id: 100 + u32::from(slot),
            config_revision: 1,
            intent_revision: 1,
        }
    }
    fn snapshot(sequence: u64, requests: u64) -> RuntimeUsageSnapshot {
        RuntimeUsageSnapshot {
            version: 1,
            sequence,
            process_uptime_ms: sequence * 1000,
            invocations_started: requests,
            invocations_succeeded: requests,
            runtime_messages: requests * 3,
            ..Default::default()
        }
    }

    #[test]
    fn retained_checkpoints_deduplicate_replicas_and_survive_restart() -> Result<()> {
        let (dir, state, telemetry) = fixture()?;
        let a = binding("service", 0);
        let b = binding("service", 1);
        let other = binding("other", 0);
        for run in [&a, &b, &other] {
            telemetry.begin_usage(run)?;
        }
        telemetry.record_usage(&a, &snapshot(1, 10), false)?;
        telemetry.record_usage(&a, &snapshot(1, 10), false)?;
        telemetry.record_usage(&b, &snapshot(1, 7), false)?;
        telemetry.record_usage(&other, &snapshot(1, 100), false)?;
        telemetry.record_usage(&a, &snapshot(2, 12), true)?;
        telemetry.record_usage(&a, &snapshot(2, 12), true)?;
        assert!(telemetry.record_usage(&a, &snapshot(3, 13), false).is_err());
        assert_eq!(
            telemetry.retained_usage(Some("service"), None)?["counters"]["invocations_started"],
            19
        );
        assert_eq!(
            telemetry.retained_usage(None, Some("project"))?["counters"]["runtime_messages"],
            57
        );
        assert_eq!(
            telemetry.retained_usage(None, None)?["counters"]["invocations_started"],
            119
        );
        let ciphertext: Vec<u8> = state.connection.query_row(
            "SELECT ciphertext FROM usage_totals WHERE placement_id='service'",
            [],
            |r| r.get(0),
        )?;
        assert!(
            !ciphertext
                .windows(b"invocations_started".len())
                .any(|v| v == b"invocations_started")
        );
        drop(telemetry);
        let reopened = TelemetryStore::open(dir.path())?;
        reopened.record_usage(&b, &snapshot(1, 7), false)?;
        reopened.record_usage(&b, &snapshot(2, 8), false)?;
        let retained = reopened.retained_usage(None, Some("project"))?;
        assert_eq!(retained["counters"]["invocations_started"], 20);
        assert_eq!(retained["coverage"]["registered_runs"], 2);
        assert_eq!(retained["coverage"]["finalized_runs"], 1);
        assert!(retained["counters"].get("in_flight").is_none());
        Ok(())
    }

    #[test]
    fn regression_finality_and_binding_fail_without_partial_totals() -> Result<()> {
        let (_dir, _state, telemetry) = fixture()?;
        let run = binding("service", 0);
        telemetry.begin_usage(&run)?;
        telemetry.record_usage(&run, &snapshot(1, 3), false)?;
        let before = telemetry.retained_usage(Some("service"), None)?;
        let mut regressed = snapshot(2, 4);
        regressed.runtime_messages = 0;
        assert!(telemetry.record_usage(&run, &regressed, false).is_err());
        let mut busy = snapshot(2, 4);
        busy.in_flight = 1;
        busy.invocations_succeeded = 3;
        assert!(telemetry.record_usage(&run, &busy, true).is_err());
        let mut other = run.clone();
        other.placement_id = "other".into();
        assert!(
            telemetry
                .record_usage(&other, &snapshot(2, 4), false)
                .is_err()
        );
        assert_eq!(telemetry.retained_usage(Some("service"), None)?, before);
        telemetry.finish_usage(&run.run_id)?;
        telemetry.finish_usage(&run.run_id)?;
        assert!(
            telemetry
                .record_usage(&run, &snapshot(2, 4), false)
                .is_err()
        );
        assert_eq!(
            telemetry.retained_usage(Some("service"), None)?["coverage"]["incomplete_runs"],
            1
        );
        Ok(())
    }

    #[test]
    fn missing_process_reports_remain_visible_after_retirement() -> Result<()> {
        let (_dir, mut state, telemetry) = fixture()?;
        let run = binding("service", 0);
        let missing = binding("service", 1);
        telemetry.begin_usage(&run)?;
        telemetry.record_usage(&run, &snapshot(1, 5), false)?;
        telemetry.begin_usage(&missing)?;
        assert_eq!(
            telemetry.retained_usage(None, Some("project"))?["coverage"]["awaiting_first_report_runs"],
            1
        );
        state.reset_observed()?;
        telemetry.reconcile_usage()?;
        telemetry.reconcile_usage()?;
        state.connection.execute("UPDATE placements SET desired_state='stopped',observed_state='stopped' WHERE id='service'",[])?;
        state.remove_placement("service")?;
        let retained = telemetry.retained_usage(None, Some("project"))?;
        assert_eq!(retained["counters"]["invocations_started"], 5);
        assert_eq!(retained["coverage"]["incomplete_runs"], 2);
        assert_eq!(retained["coverage"]["unreported_runs"], 1);
        assert_eq!(retained["tail_loss_possible"], true);
        assert!(
            telemetry
                .record_usage(&run, &snapshot(2, 8), false)
                .is_err()
        );
        assert_eq!(
            telemetry.retained_usage(None, Some("unknown"))?["counters"],
            Value::Null
        );
        Ok(())
    }

    #[test]
    fn transition_projection_is_atomic_encrypted_scoped_and_restart_safe() -> Result<()> {
        let (dir, state, telemetry) = fixture()?;
        state.connection.execute("INSERT INTO management_operations(operation_id,request_digest,principal,project_id,placement_id,accepted_at,result_json) VALUES('op','digest','owner','project','service',1,?1)",[json!({"state":"accepted","result":{"secret":"do-not-copy-sensitive-value"}}).to_string()])?;
        state.connection.execute(
            "UPDATE management_operations SET result_json=?1 WHERE operation_id='op'",
            [
                json!({"state":"completed","result":{"secret":"do-not-copy-sensitive-value"}})
                    .to_string(),
            ],
        )?;
        let queued: i64 = state.connection.query_row(
            "SELECT COUNT(*) FROM operational_outbox WHERE source_id='op'",
            [],
            |r| r.get(0),
        )?;
        assert_eq!(queued, 2);
        telemetry.project_messages()?;
        telemetry.append(Some("service"), "log", &json!({"message":"service output"}))?;
        let logs = telemetry.read(Some("service"), "log", 0, 100)?;
        let records = logs["records"].as_array().unwrap();
        assert!(
            records
                .iter()
                .any(|r| r["kind"] == "log" && r["data"]["message"] == "service output")
        );
        assert_eq!(
            records
                .iter()
                .filter(|r| r["kind"] == "message" && r["data"]["source_id"] == "op")
                .count(),
            2
        );
        let cursor = logs["next"].as_u64().unwrap();
        assert!(
            telemetry.read(Some("service"), "log", cursor, 100)?["records"]
                .as_array()
                .unwrap()
                .is_empty()
        );
        let first = telemetry.messages(None, Some("project"), 0, 100)?;
        let messages = first["records"].as_array().unwrap();
        assert_eq!(
            messages
                .iter()
                .filter(|r| r["data"]["source_id"] == "op")
                .count(),
            2
        );
        assert!(
            messages
                .iter()
                .all(|r| r["data"]["project_id"] == "project")
        );
        assert!(!serde_json::to_string(&first)?.contains("do-not-copy-sensitive-value"));
        assert!(first.get("outbox_dropped").is_none());
        drop(telemetry);
        let reopened = TelemetryStore::open(dir.path())?;
        reopened.project_messages()?;
        assert_eq!(reopened.messages(None, Some("project"), 0, 100)?, first);
        let after = first["next"].as_u64().unwrap();
        assert!(
            reopened.messages(None, Some("project"), after, 100)?["records"]
                .as_array()
                .unwrap()
                .is_empty()
        );
        assert!(
            reopened.messages(Some("other"), None, 0, 100)?["records"]
                .as_array()
                .unwrap()
                .iter()
                .all(|r| r["data"]["placement_id"] == "other")
        );
        let cipher: Vec<u8> = state.connection.query_row(
            "SELECT ciphertext FROM telemetry_records WHERE kind='message' LIMIT 1",
            [],
            |r| r.get(0),
        )?;
        assert!(
            !cipher
                .windows(b"transition_id".len())
                .any(|v| v == b"transition_id")
        );
        Ok(())
    }
    #[test]
    fn finished_checkpoint_retention_preserves_totals_and_active_runs() -> Result<()> {
        let (_dir, _state, telemetry) = fixture()?;
        let closed = binding("service", 0);
        let active = binding("service", 1);
        telemetry.begin_usage(&closed)?;
        telemetry.record_usage(&closed, &snapshot(1, 7), true)?;
        telemetry.begin_usage(&active)?;
        telemetry.record_usage(&active, &snapshot(1, 3), false)?;
        telemetry.prune_usage_checkpoints(unix_time()? + 86401)?;
        assert!(telemetry.process(&closed.run_id)?.is_none());
        assert!(telemetry.process(&active.run_id)?.is_some());
        assert_eq!(
            telemetry.retained_usage(Some("service"), None)?["counters"]["invocations_started"],
            10
        );
        assert_eq!(
            telemetry.retained_usage(Some("service"), None)?["coverage"]["finalized_runs"],
            1
        );
        assert!(
            telemetry
                .record_usage(&closed, &snapshot(1, 7), true)
                .is_err()
        );
        Ok(())
    }

    #[test]
    fn project_metrics_require_fresh_exact_revision_samples() -> Result<()> {
        let (_dir, state, telemetry) = fixture()?;
        for slot in 0..2 {
            telemetry.begin_usage(&binding("service", slot))?;
        }
        let cohort = telemetry
            .metric_cohort(&state.get_placement("service")?.unwrap())?
            .unwrap();
        telemetry.append(Some("service"),"metrics",&json!({"project_id":"project","config_revision":1,"intent_revision":1,"running_replicas":2,"processes_observed":2,"cpu_percent":175.5,"memory_bytes":1024,"process_cohort":cohort}))?;
        telemetry.append(Some("other"),"metrics",&json!({"project_id":"other-project","config_revision":1,"intent_revision":1,"running_replicas":2,"processes_observed":2,"cpu_percent":999.0,"memory_bytes":999999}))?;
        let sample = telemetry.project_metrics("project")?;
        assert_eq!(sample["records"][0]["data"]["cpu_percent"], 175.5);
        assert_eq!(sample["records"][0]["data"]["memory_bytes"], 1024);
        assert_eq!(sample["records"][0]["data"]["processes_observed"], 2);
        state.connection.execute(
            "UPDATE placements SET config_revision=2 WHERE id='service'",
            [],
        )?;
        let sample = telemetry.project_metrics("project")?;
        assert_eq!(sample["records"][0]["data"]["cpu_percent"], Value::Null);
        assert_eq!(
            sample["records"][0]["data"]["metric_coverage"]["missing_or_stale_placements"],
            1
        );
        Ok(())
    }
    #[test]
    fn metric_cohort_rejects_old_samples_after_same_pid_and_revision_restart() -> Result<()> {
        let (_dir, state, telemetry) = fixture()?;
        let placement = state.get_placement("service")?.unwrap();
        assert!(telemetry.metric_cohort(&placement)?.is_none());
        let old = binding("service", 0);
        telemetry.begin_usage(&old)?;
        assert!(
            telemetry.metric_cohort(&placement)?.is_none(),
            "partial reporting cohort cannot authenticate a full resource sample"
        );
        telemetry.begin_usage(&binding("service", 1))?;
        let cohort = telemetry.metric_cohort(&placement)?.unwrap();
        let sample = json!({"project_id":"project","config_revision":1,"intent_revision":1,"running_replicas":2,"processes_observed":2,"cpu_percent":42.0,"memory_bytes":1024,"process_cohort":cohort});
        telemetry.append(Some("service"), "metrics", &sample)?;
        assert_eq!(
            telemetry.project_metrics("project")?["records"][0]["data"]["cpu_percent"],
            42.0
        );
        telemetry.finish_usage(&old.run_id)?;
        telemetry.begin_usage(&binding("service", 0))?;
        let replacement = telemetry.metric_cohort(&placement)?.unwrap();
        assert_ne!(replacement, cohort);
        let stale = telemetry.project_metrics("project")?;
        assert!(stale["records"][0]["data"]["cpu_percent"].is_null());
        assert_eq!(
            stale["records"][0]["data"]["metric_coverage"]["missing_or_stale_placements"],
            1
        );
        let mut fresh = sample;
        fresh["process_cohort"] = replacement.into();
        telemetry.append(Some("service"), "metrics", &fresh)?;
        assert_eq!(
            telemetry.project_metrics("project")?["records"][0]["data"]["memory_bytes"],
            1024
        );
        Ok(())
    }

    #[test]
    fn transient_scaling_intent_does_not_close_a_live_process_checkpoint() -> Result<()> {
        let (_dir, state, telemetry) = fixture()?;
        let run = binding("service", 1);
        telemetry.begin_usage(&run)?;
        telemetry.record_usage(&run, &snapshot(1, 2), false)?;
        state.set_replica_count("service", 1, 1)?;
        telemetry.reconcile_usage()?;
        assert_eq!(telemetry.process(&run.run_id)?.unwrap().state, "active");
        telemetry.record_usage(&run, &snapshot(2, 3), false)?;
        state.set_replica_count("service", 1, 2)?;
        telemetry.record_usage(&run, &snapshot(2, 3), false)?;
        assert_eq!(
            telemetry.retained_usage(Some("service"), None)?["counters"]["invocations_started"],
            3
        );
        state.record_replica("service", 1, 1, 1, ObservedState::Stopped, None, None)?;
        telemetry.reconcile_usage()?;
        assert_eq!(telemetry.process(&run.run_id)?.unwrap().state, "incomplete");
        Ok(())
    }

    #[test]
    fn altered_plaintext_message_scope_cannot_expand_project_visibility() -> Result<()> {
        let (_dir, state, telemetry) = fixture()?;
        telemetry.project_messages()?;
        state.connection.execute("UPDATE operational_message_scopes SET project_id='project' WHERE project_id='other-project'",[])?;
        assert!(telemetry.messages(None, Some("project"), 0, 100).is_err());
        Ok(())
    }
    #[test]
    fn dropping_usage_under_database_contention_is_nonblocking_and_recoverable() -> Result<()> {
        let (dir, state, telemetry) = fixture()?;
        let run = binding("service", 0);
        telemetry.begin_usage(&run)?;
        telemetry.record_usage(&run, &snapshot(1, 2), false)?;
        let mut guard = UsageProcessGuard::new(dir.path(), &run.run_id);
        guard.arm();
        state.connection.execute_batch("BEGIN IMMEDIATE")?;
        let started = std::time::Instant::now();
        drop(guard);
        assert!(
            started.elapsed() < std::time::Duration::from_secs(1),
            "Drop waited for the normal SQLite busy timeout"
        );
        state.connection.execute_batch("ROLLBACK")?;
        assert_eq!(telemetry.process(&run.run_id)?.unwrap().state, "active");
        state.record_replica("service", 0, 1, 1, ObservedState::Stopped, None, None)?;
        telemetry.reconcile_usage()?;
        assert_eq!(telemetry.process(&run.run_id)?.unwrap().state, "incomplete");
        assert_eq!(
            telemetry.retained_usage(Some("service"), None)?["counters"]["invocations_started"],
            2
        );
        Ok(())
    }
}
