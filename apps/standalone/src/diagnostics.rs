//! In-memory agent facts for live inspection and status snapshots. Nothing here is
//! persisted, so a same-schema rollback never meets data that a newer agent wrote.

use crate::{enrollment::unix_time, state::PlacementRecord};
use anyhow::Result;
use flow_like_device_protocol::DeviceIdentity;
use serde_json::{Map, Value, json};
use std::{
    collections::{BTreeMap, HashMap},
    future::Future,
    net::IpAddr,
    path::{Path, PathBuf},
    sync::{Arc, LazyLock, Mutex, MutexGuard, PoisonError},
    time::{Duration, Instant},
};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

/// Live capabilities a client may rely on; a missing flag means an older agent.
pub const FEATURES: &[&str] = &[
    "placement_diagnostics",
    "task_health",
    "placement_events",
    "offline_summary",
    "host_operation",
    "network_interfaces",
    "rollout_history",
    "operations",
    "metrics_history",
    "offline_lookup",
    "reader_bindings",
    "acme_failure_detail",
    "archive_status",
    "artifact_capacity",
    "scheduled_events",
    #[cfg(feature = "runtime")]
    "model_store",
    #[cfg(feature = "runtime")]
    "model_host",
    #[cfg(feature = "runtime")]
    "model_runtime_llamacpp",
    #[cfg(feature = "runtime")]
    "model_runtime_onnx",
    #[cfg(feature = "runtime")]
    "model_runtime_manifest",
    #[cfg(feature = "runtime")]
    "model_runtime_updates",
    #[cfg(all(feature = "runtime", target_os = "macos", target_arch = "aarch64"))]
    "model_runtime_mlx",
];

pub const DEVICE_PRESENCE: &str = "device_presence";
pub const MANAGEMENT_TRANSPORT: &str = "management_transport";
pub const REBOOT_WATCHER: &str = "reboot_watcher";
pub const UPDATE_WATCHER: &str = "update_watcher";
pub const FLEET_PUBLISHER: &str = "fleet_publisher";
pub const ARCHIVE_PUBLISHER: &str = "archive_publisher";
pub const ACME_RENEWAL: &str = "acme_renewal";
pub const SECRET_PUBLISHER: &str = "secret_publisher";
pub const TELEMETRY_SAMPLER: &str = "telemetry_sampler";
pub const LIVE_TELEMETRY_PUBLISHER: &str = "live_telemetry_publisher";
pub const CERTIFICATE_INVENTORY_PUBLISHER: &str = "certificate_inventory_publisher";
pub const CERTIFICATE_RENEWAL: &str = "certificate_renewal";
pub const MODEL_HOST: &str = "model_host";

const MAX_ERROR_TEXT: usize = 1024;
const MAX_EVENTS: usize = 64;
const MAX_INTERFACES: usize = 16;
const MAX_INTERFACE_ADDRESSES: usize = 4;
const MAX_INTERFACE_NAME: usize = 32;
/// A device has 32 certificate slots; beyond this the causes are forgotten and set again.
const MAX_RENEWAL_FAILURES: usize = 256;
/// Facts that cost file or OS reads are reused for this long.
const FACT_CACHE: Duration = Duration::from_secs(15);
const STATE_PLACEHOLDER: &str = "<state>";
/// 2001-09-09: anything earlier is an uptime or an unset clock, not a boot time.
const EARLIEST_CREDIBLE_BOOT: u64 = 1_000_000_000;
/// Where a placement process keeps the state of its schedules, bots and person-started runs
/// below its data root. The modules that write them are built only with the runtime, so
/// these readers share no type with them.
const SCHEDULE_DIRECTORY: &str = ".standalone-schedule";
const BOT_DIRECTORY: &str = ".standalone-bots";
const RUN_DIRECTORY: &str = ".standalone-run";
const STATE_FILE: &str = "state.json";
const MAX_STATE_FILE_BYTES: u64 = 64 * 1024;
/// The directories of a placement's data root that lie on the way to its state files. The
/// agent creates them as private directories and never as links.
const PLACEMENT_DATA_ROOT: [&str; 3] = ["placement-data", "current", "store"];
const MAX_SCHEDULES: usize = 16;
const MAX_BOTS: usize = 8;
const MAX_ACTIONS: usize = 16;
/// The row keys of the lists of schedules, bots and person-started events, and of their
/// `*_truncated` flags.
const TRIGGER_LISTS: [(&str, &str); 3] = [
    ("schedules", "schedules_truncated"),
    ("bots", "bots_truncated"),
    ("actions", "actions_truncated"),
];
/// What the three lists of one row add to it together, keys and flags included.
const TRIGGER_FACTS_BUDGET: usize = 4096;
/// Beyond this many unusable files, they are forgotten and logged again.
const MAX_FILE_FAULTS: usize = 256;
/// The end of the year 9999: anything later in a state file is not a time.
const LATEST_TIME: i64 = 253_402_300_799;
/// A one-time schedule's instant lies in 2000-01-01 … 2100-01-01 UTC.
const ONCE_INSTANTS: std::ops::RangeInclusive<i64> = 946_684_800..=4_102_444_800;
/// Why a schedule or a bot does not run here: the codes of the hub claim's gate.
const HOLDS: [&str; 5] = [
    "other_service",
    "not_released",
    "runs_elsewhere",
    "hub_unreachable",
    "hub_too_old",
];
/// How a scheduled, bot or person-started run ended.
const OUTCOMES: [&str; 4] = ["succeeded", "failed", "cancelled", "timed_out"];
const SCHEDULE_SKIPS: [&str; 3] = ["overlap", "missed", "busy"];
const ONCE_STATES: [&str; 5] = ["pending", "started", "ran", "missed", "passed"];
/// A one-time schedule in one of these states never runs again.
const ONCE_FINISHED: [&str; 3] = ["ran", "missed", "passed"];
const BOT_PROVIDERS: [&str; 2] = ["telegram", "discord"];
const BOT_STATES: [&str; 8] = [
    "waiting",
    "connecting",
    "connected",
    "reconnecting",
    "token_refused",
    "intents_refused",
    "conflict",
    "webhook_set",
];
/// Bot states that change with every network blip; a snapshot sends them as `ok`.
const BOT_CONNECTION_STATES: [&str; 3] = ["connecting", "connected", "reconnecting"];
const ACTION_KINDS: [&str; 2] = ["action", "form"];

pub fn model_host_available() -> bool {
    #[cfg(feature = "runtime")]
    {
        crate::models::host::ModelHost::current().is_some()
    }
    #[cfg(not(feature = "runtime"))]
    false
}

pub fn model_host_failed() -> bool {
    global()
        .lock()
        .tasks
        .get(MODEL_HOST)
        .is_some_and(|health| health.state == TaskState::Stopped)
}

/// Model capabilities follow the running host; event capabilities follow the build.
pub fn features() -> Value {
    features_with_models(model_host_available())
}

fn features_with_models(available: bool) -> Value {
    Value::Object(
        FEATURES
            .iter()
            .copied()
            .filter(|flag| available || !flag.starts_with("model_"))
            .chain(crate::event_kind::flags())
            .map(|flag| (flag.to_owned(), json!(1)))
            .collect(),
    )
}

/// The access rules a task needs are not the ones the device accepted: they changed, or
/// the hub and the device hold different ones. The owner clears it; it is not a fault.
#[derive(Debug)]
pub struct RulesDiffer(pub &'static str);

impl std::fmt::Display for RulesDiffer {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.0)
    }
}

impl std::error::Error for RulesDiffer {}

/// Fixed causes only: free text could carry hub responses or customer identifiers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TaskFailure {
    HubUnreachable,
    HubRefused,
    Storage,
    Policy,
    Internal,
}

impl TaskFailure {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::HubUnreachable => "hub_unreachable",
            Self::HubRefused => "hub_refused",
            Self::Storage => "storage",
            Self::Policy => "policy",
            Self::Internal => "internal",
        }
    }

    /// A 5xx answer is an outage of the hub or its gateway, not a decision about this device.
    fn hub_answered(status: reqwest::StatusCode) -> Self {
        if status.is_server_error() {
            Self::HubUnreachable
        } else {
            Self::HubRefused
        }
    }

    pub fn classify(error: &anyhow::Error) -> Self {
        if let Some(status) = crate::enrollment::api_status(error) {
            return Self::hub_answered(status);
        }
        error
            .chain()
            .find_map(|cause| {
                if let Some(request) = cause.downcast_ref::<reqwest::Error>() {
                    return Some(
                        request
                            .status()
                            .map_or(Self::HubUnreachable, Self::hub_answered),
                    );
                }
                if cause.is::<rusqlite::Error>() || cause.is::<std::io::Error>() {
                    return Some(Self::Storage);
                }
                (cause.is::<flow_like_device_protocol::ProtocolError>()
                    || cause.is::<RulesDiffer>())
                .then_some(Self::Policy)
            })
            .unwrap_or(Self::Internal)
    }
}

/// Why a certificate renewal attempt failed. Fixed causes only, like `TaskFailure`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RenewalFailure {
    Dns,
    PortBind,
    RateLimited,
    CaRejected,
    AuthorityExpired,
    Network,
    Internal,
}

impl RenewalFailure {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Dns => "dns",
            Self::PortBind => "port_bind",
            Self::RateLimited => "rate_limited",
            Self::CaRejected => "ca_rejected",
            Self::AuthorityExpired => "authority_expired",
            Self::Network => "network",
            Self::Internal => "internal",
        }
    }
}

impl std::fmt::Display for RenewalFailure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// Which renewal of a certificate failed: one certificate can have had both kinds.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Renewal {
    Acme,
    Issuer,
}

/// Why a retained-history stream is not recording. Fixed causes only.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HistoryPause {
    RulesExpired,
    RulesChanged,
    RosterExpired,
    OutboxFull,
    QuotaReached,
    TierWithoutHistory,
}

impl HistoryPause {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::RulesExpired => "rules_expired",
            Self::RulesChanged => "rules_changed",
            Self::RosterExpired => "roster_expired",
            Self::OutboxFull => "outbox_full",
            Self::QuotaReached => "quota_reached",
            Self::TierWithoutHistory => "tier_without_history",
        }
    }
}

/// Whether a stream can seal new history; `Err(None)` is a fault without a fixed cause.
pub type Sealing = std::result::Result<(), Option<HistoryPause>>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TaskState {
    Ok,
    Failing,
    Stopped,
}

impl TaskState {
    fn as_str(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Failing => "failing",
            Self::Stopped => "stopped",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TaskHealth {
    pub state: TaskState,
    pub since: i64,
    pub consecutive_failures: u32,
    pub category: Option<TaskFailure>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReplicaRestarts {
    pub failures: u32,
    pub max_restarts: u32,
    pub crash_looping: bool,
    /// Set only while the supervisor waits to restart the replica.
    pub retry_at: Option<Instant>,
    pub last_started_at: Option<i64>,
}

/// One schedule as its placement process last wrote it, after validation.
#[derive(Clone, Debug, PartialEq)]
struct ScheduleFact {
    event_id: String,
    timezone: String,
    hold: Option<&'static str>,
    running: bool,
    last: Option<(i64, &'static str)>,
    runs: u64,
    failed: u64,
    plan: SchedulePlan,
}

/// When a schedule runs: by its expression, or once at an instant.
#[derive(Clone, Debug, PartialEq)]
enum SchedulePlan {
    Repeating {
        expression: String,
        next_at: Option<i64>,
        skipped: u64,
        last_skip: Option<(i64, &'static str)>,
        watermark: Option<i64>,
    },
    Once {
        at: i64,
        state: &'static str,
        /// This service had it armed before its time.
        armed: bool,
        /// The pinned event version the entry was written for.
        event_version: Option<[u64; 3]>,
    },
}

/// One bot as its placement process last wrote it, after validation.
#[derive(Clone, Debug, PartialEq)]
struct BotFact {
    event_id: String,
    provider: &'static str,
    state: &'static str,
    hold: Option<&'static str>,
    bot_name: Option<String>,
    connected_at: Option<i64>,
    last_message_at: Option<i64>,
    last_outcome: Option<&'static str>,
    running: u64,
    runs: u64,
    failed: u64,
    /// The UTC date (`YYYY-MM-DD`) whose runs the process counted, and that count.
    day: Option<(String, u64)>,
    /// Messages left unanswered since the service started: a flood, too busy, too old.
    dropped: u64,
}

/// A person-started event of one instance, or of every instance that runs, added up.
#[derive(Clone, Debug, PartialEq)]
struct ActionFact {
    event_id: String,
    kind: &'static str,
    fields: u64,
    file_fields: u64,
    running: u64,
    runs: u64,
    failed: u64,
    /// The last run that ended: when it started and ended, and how.
    last: Option<(i64, i64, &'static str)>,
}

/// A placement state file as its process wrote it, with the entries that are usable.
#[derive(Clone, Debug, PartialEq)]
struct StateFile<T> {
    config_revision: u64,
    intent_revision: u64,
    /// The process's first pass gave every entry its state. Run state files have no pass.
    decided: bool,
    entries: Vec<T>,
    /// Entries that were not usable: an event outside the placement, or values out of bounds.
    dropped: usize,
}

type ScheduleFile = StateFile<ScheduleFact>;
type BotFile = StateFile<BotFact>;

impl<T> StateFile<T> {
    /// Written by a process of the placement's current revisions.
    fn describes(&self, record: &PlacementRecord) -> bool {
        self.config_revision == record.config_revision
            && self.intent_revision == record.intent_revision
    }
}

/// A replica that runs the placement's current revisions: its slot and process ID.
type Runner = (u8, Option<u32>);

/// A read of one kind of a placement's state files: when, while which replicas ran, and
/// what was found. `file` is `None` when there is none or it is not usable.
struct FileRead<T> {
    at: Instant,
    runners: Vec<Runner>,
    file: Option<Arc<T>>,
}

/// Per state directory and placement.
type FileReads<T> = HashMap<(PathBuf, String), FileRead<T>>;

struct Inner {
    agent_started_at: i64,
    replicas: HashMap<(String, u8), ReplicaRestarts>,
    tasks: BTreeMap<String, TaskHealth>,
    /// Per state directory and placement; `None` while the queues could not be read.
    offline_writes: HashMap<(PathBuf, String), (Instant, Option<Value>)>,
    schedules: FileReads<ScheduleFile>,
    bots: FileReads<BotFile>,
    /// The person-started events of every instance that runs, added up.
    actions: FileReads<Vec<ActionFact>>,
    /// State files, by placement and directory, whose unusable content was already logged.
    file_faults: std::collections::HashSet<(PathBuf, String, &'static str)>,
    network: Option<(Instant, Value)>,
    /// Per state directory, renewal kind and certificate: the cause of the last failed attempt.
    renewal_failures: HashMap<(PathBuf, Renewal, String), RenewalFailure>,
    /// Per state directory, scope and kind: what the sealer last found, and since when.
    history: HashMap<(PathBuf, String, String), (Sealing, i64)>,
    /// Per state directory: why the hub stores no more history for the account, and since when.
    history_storage: HashMap<PathBuf, (HistoryPause, i64)>,
}

#[derive(Clone)]
pub struct Diagnostics(Arc<Mutex<Inner>>);

impl Default for Diagnostics {
    fn default() -> Self {
        Self(Arc::new(Mutex::new(Inner {
            agent_started_at: now(),
            replicas: HashMap::new(),
            tasks: BTreeMap::new(),
            offline_writes: HashMap::new(),
            schedules: HashMap::new(),
            bots: HashMap::new(),
            actions: HashMap::new(),
            file_faults: Default::default(),
            network: None,
            renewal_failures: HashMap::new(),
            history: HashMap::new(),
            history_storage: HashMap::new(),
        })))
    }
}

static GLOBAL: LazyLock<Diagnostics> = LazyLock::new(Diagnostics::default);

/// The agent's registry. `run` creates it first, so its start time is the agent's.
pub fn global() -> &'static Diagnostics {
    &GLOBAL
}

fn now() -> i64 {
    unix_time().unwrap_or_default()
}

pub trait TaskOutcome {
    fn failure(self) -> Option<anyhow::Error>;
}

impl TaskOutcome for () {
    fn failure(self) -> Option<anyhow::Error> {
        None
    }
}

impl TaskOutcome for Result<()> {
    fn failure(self) -> Option<anyhow::Error> {
        self.err()
    }
}

impl Diagnostics {
    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub fn agent_started_at(&self) -> i64 {
        self.lock().agent_started_at
    }

    /// A freshly spawned task starts healthy.
    pub fn track(&self, task: &str) {
        self.lock().tasks.insert(
            task.to_owned(),
            TaskHealth {
                state: TaskState::Ok,
                since: now(),
                consecutive_failures: 0,
                category: None,
            },
        );
    }

    pub fn report(&self, task: &str, result: std::result::Result<(), TaskFailure>) {
        let now = now();
        let mut inner = self.lock();
        let health = inner
            .tasks
            .entry(task.to_owned())
            .or_insert_with(|| TaskHealth {
                state: TaskState::Ok,
                since: now,
                consecutive_failures: 0,
                category: None,
            });
        match result {
            Ok(()) if health.state != TaskState::Ok => {
                *health = TaskHealth {
                    state: TaskState::Ok,
                    since: now,
                    consecutive_failures: 0,
                    category: None,
                };
            }
            Ok(()) => {}
            Err(category) => {
                if health.state != TaskState::Failing {
                    health.state = TaskState::Failing;
                    health.since = now;
                    health.consecutive_failures = 0;
                }
                health.consecutive_failures = health.consecutive_failures.saturating_add(1);
                health.category = Some(category);
            }
        }
    }

    pub fn report_error<T>(&self, task: &str, result: &Result<T>) {
        self.report(
            task,
            result.as_ref().map(|_| ()).map_err(TaskFailure::classify),
        );
    }

    /// A stopped task keeps the cause of its last failure when the stop itself has none.
    pub fn stopped(&self, task: &str, category: Option<TaskFailure>) {
        let now = now();
        let mut inner = self.lock();
        let health = inner
            .tasks
            .entry(task.to_owned())
            .or_insert_with(|| TaskHealth {
                state: TaskState::Stopped,
                since: now,
                consecutive_failures: 0,
                category: None,
            });
        health.state = TaskState::Stopped;
        health.since = now;
        health.category = category.or(health.category);
    }

    pub fn tasks(&self) -> Vec<(String, TaskHealth)> {
        self.lock()
            .tasks
            .iter()
            .map(|(name, health)| (name.clone(), health.clone()))
            .collect()
    }

    /// The supervisor replaces every replica's facts once per tick, which also drops
    /// replicas it no longer runs.
    pub fn set_replicas(&self, replicas: HashMap<(String, u8), ReplicaRestarts>) {
        self.lock().replicas = replicas;
    }

    pub fn replica(&self, placement: &str, slot: u8) -> Option<ReplicaRestarts> {
        self.lock()
            .replicas
            .get(&(placement.to_owned(), slot))
            .cloned()
    }

    /// The placement's queued-write totals. Every inspection and snapshot row asks, and a
    /// placement can hold 64 queue databases, so one read serves all of them for a while.
    pub(crate) fn offline_writes(&self, state_dir: &Path, placement: &str) -> Option<Value> {
        self.offline_writes_at(state_dir, placement, Instant::now())
    }

    fn offline_writes_at(&self, state_dir: &Path, placement: &str, now: Instant) -> Option<Value> {
        let key = (state_dir.to_path_buf(), placement.to_owned());
        let fresh = |read_at: &Instant| now.saturating_duration_since(*read_at) < FACT_CACHE;
        if let Some((read_at, summary)) = self.lock().offline_writes.get(&key)
            && fresh(read_at)
        {
            return summary.clone();
        }
        let summary = crate::outbox::summary(state_dir, placement)
            .inspect_err(|error| {
                tracing::warn!(
                    placement_id = placement,
                    "Queued offline writes could not be counted: {error:#}"
                )
            })
            .ok();
        let mut inner = self.lock();
        inner
            .offline_writes
            .retain(|_, (read_at, _)| fresh(read_at));
        inner.offline_writes.insert(key, (now, summary.clone()));
        summary
    }

    /// One kind of a placement's state files, read again after `FACT_CACHE` and as soon as
    /// other replicas run. `read` answers what it found and whether the file was unusable,
    /// which is logged once until the file is usable again.
    #[allow(clippy::too_many_arguments)]
    fn cached<T>(
        &self,
        reads: fn(&mut Inner) -> &mut FileReads<T>,
        directory: &'static str,
        state_dir: &Path,
        placement: &str,
        runners: &[Runner],
        now: Instant,
        read: impl FnOnce() -> (Option<T>, bool),
    ) -> Option<Arc<T>> {
        let key = (state_dir.to_path_buf(), placement.to_owned());
        let fresh = |read_at: &Instant| now.saturating_duration_since(*read_at) < FACT_CACHE;
        if let Some(file) = reads(&mut self.lock())
            .get(&key)
            .filter(|read| fresh(&read.at) && read.runners == runners)
            .map(|read| read.file.clone())
        {
            return file;
        }
        let (file, faulty) = read();
        let file = file.map(Arc::new);
        let mut inner = self.lock();
        if inner.file_faults.len() >= MAX_FILE_FAULTS {
            inner.file_faults.clear();
        }
        let fault = (key.0.clone(), key.1.clone(), directory);
        if !faulty {
            inner.file_faults.remove(&fault);
        } else if inner.file_faults.insert(fault) {
            tracing::warn!(
                placement_id = placement,
                directory,
                "A state file of the placement is not usable, in whole or in part, and is not reported"
            );
        }
        let reads = reads(&mut inner);
        reads.retain(|_, read| fresh(&read.at));
        reads.insert(
            key,
            FileRead {
                at: now,
                runners: runners.to_vec(),
                file: file.clone(),
            },
        );
        file
    }

    /// The schedules of a placement as its running process reports them: written by the
    /// process of the placement's current revisions, after its first pass, while a replica
    /// of it runs. Otherwise only one-time schedules that are over, for as long as the config
    /// pins the event version they were written for; `None` when there is none.
    fn schedules(&self, state_dir: &Path, record: &PlacementRecord) -> Option<Arc<ScheduleFile>> {
        self.schedules_at(state_dir, record, Instant::now())
    }

    fn schedules_at(
        &self,
        state_dir: &Path,
        record: &PlacementRecord,
        now: Instant,
    ) -> Option<Arc<ScheduleFile>> {
        let runners = runners(record);
        let current = |file: &ScheduleFile| file.decided && file.describes(record);
        let file = self.cached(
            |inner| &mut inner.schedules,
            SCHEDULE_DIRECTORY,
            state_dir,
            &record.id,
            &runners,
            now,
            || {
                let read = read_schedule_file(state_dir, &record.id, &record.config);
                // Until its first pass a process still carries the entries of events its
                // config no longer lists; only a decided file of this config is clean.
                let faulty = read.as_ref().map_or(true, |file| {
                    file.as_ref()
                        .is_some_and(|file| current(file) && file.dropped > 0)
                });
                (read.ok().flatten(), faulty)
            },
        )?;
        if !runners.is_empty() && current(&file) {
            return Some(file);
        }
        let finished: Vec<ScheduleFact> = file
            .entries
            .iter()
            .filter(|schedule| schedule.finished_for(&record.config))
            .cloned()
            .collect();
        (!finished.is_empty()).then(|| {
            Arc::new(StateFile {
                entries: finished,
                dropped: 0,
                ..*file
            })
        })
    }

    /// The bots of a placement as its running process reports them, under the rule of
    /// running schedules.
    fn bots(&self, state_dir: &Path, record: &PlacementRecord) -> Option<Arc<BotFile>> {
        self.bots_at(state_dir, record, Instant::now())
    }

    fn bots_at(
        &self,
        state_dir: &Path,
        record: &PlacementRecord,
        now: Instant,
    ) -> Option<Arc<BotFile>> {
        let runners = runners(record);
        if runners.is_empty() {
            return None;
        }
        let current = |file: &BotFile| file.decided && file.describes(record);
        self.cached(
            |inner| &mut inner.bots,
            BOT_DIRECTORY,
            state_dir,
            &record.id,
            &runners,
            now,
            || {
                let read = read_state_file(
                    state_dir,
                    &record.id,
                    BOT_DIRECTORY,
                    STATE_FILE,
                    &record.config,
                    "bots",
                    bot_fact,
                );
                let faulty = read.as_ref().map_or(true, |file| {
                    file.as_ref()
                        .is_some_and(|file| current(file) && file.dropped > 0)
                });
                (read.ok().flatten(), faulty)
            },
        )
        .filter(|file| current(file))
    }

    /// The person-started events of a placement, added up over the instances that run the
    /// current revisions: each writes its own file. `None` while no instance reports.
    fn actions(&self, state_dir: &Path, record: &PlacementRecord) -> Option<Arc<Vec<ActionFact>>> {
        self.actions_at(state_dir, record, Instant::now())
    }

    fn actions_at(
        &self,
        state_dir: &Path,
        record: &PlacementRecord,
        now: Instant,
    ) -> Option<Arc<Vec<ActionFact>>> {
        let runners = runners(record);
        if runners.is_empty() {
            return None;
        }
        self.cached(
            |inner| &mut inner.actions,
            RUN_DIRECTORY,
            state_dir,
            &record.id,
            &runners,
            now,
            || {
                let mut faulty = false;
                let files: Vec<StateFile<ActionFact>> = runners
                    .iter()
                    .filter_map(|(slot, _)| {
                        let read = read_state_file(
                            state_dir,
                            &record.id,
                            RUN_DIRECTORY,
                            &format!("state.{slot}.json"),
                            &record.config,
                            "events",
                            action_fact,
                        );
                        faulty |= read.as_ref().map_or(true, |file| {
                            file.as_ref()
                                .is_some_and(|file| file.describes(record) && file.dropped > 0)
                        });
                        read.ok().flatten().filter(|file| file.describes(record))
                    })
                    .collect();
                ((!files.is_empty()).then(|| added_up(files)), faulty)
            },
        )
    }

    /// Addresses of this host's network interfaces, for device-scope Status readers only.
    pub(crate) fn network(&self) -> Value {
        let now = Instant::now();
        if let Some((read_at, network)) = &self.lock().network
            && now.saturating_duration_since(*read_at) < FACT_CACHE
        {
            return network.clone();
        }
        let network = network_json(os_interfaces());
        self.lock().network = Some((now, network.clone()));
        network
    }

    /// Kept in memory only: after a restart the cause is unknown until the next attempt.
    pub(crate) fn set_renewal_failure(
        &self,
        state_dir: &Path,
        renewal: Renewal,
        certificate_id: &str,
        failure: RenewalFailure,
    ) {
        let mut inner = self.lock();
        if inner.renewal_failures.len() >= MAX_RENEWAL_FAILURES {
            inner.renewal_failures.clear();
        }
        inner.renewal_failures.insert(
            (state_dir.to_path_buf(), renewal, certificate_id.to_owned()),
            failure,
        );
    }

    pub(crate) fn renewal_failure(
        &self,
        state_dir: &Path,
        renewal: Renewal,
        certificate_id: &str,
    ) -> Option<RenewalFailure> {
        self.lock()
            .renewal_failures
            .get(&(state_dir.to_path_buf(), renewal, certificate_id.to_owned()))
            .copied()
    }

    /// What the sealer found for each stream that has a roster; `since` moves only when a
    /// stream's outcome changes, and streams without a roster are forgotten.
    pub(crate) fn set_history_sealing(
        &self,
        state_dir: &Path,
        streams: Vec<(String, String, Sealing)>,
    ) {
        let now = now();
        let mut inner = self.lock();
        let mut previous = std::mem::take(&mut inner.history);
        for (scope, kind, sealing) in streams {
            let key = (state_dir.to_path_buf(), scope, kind);
            let since = previous
                .remove(&key)
                .filter(|(known, _)| *known == sealing)
                .map_or(now, |(_, since)| since);
            inner.history.insert(key, (sealing, since));
        }
        previous.retain(|(other, _, _), _| other != state_dir);
        inner.history.extend(previous);
    }

    /// Whether the hub still stores history for the account, as an answered upload showed.
    pub(crate) fn set_history_storage(&self, state_dir: &Path, refused: Option<HistoryPause>) {
        let now = now();
        let mut inner = self.lock();
        match refused {
            Some(reason) => {
                let known = inner.history_storage.get(state_dir).copied();
                if known.map(|(known, _)| known) != Some(reason) {
                    inner
                        .history_storage
                        .insert(state_dir.to_path_buf(), (reason, now));
                }
            }
            None => {
                inner.history_storage.remove(state_dir);
            }
        }
    }

    /// `{state, reason, since}` of one retained-history stream once the sealer has looked at
    /// it. Rules and roster come first: the owner renews them whatever the hub stores. A
    /// full outbox comes last, because a refusing hub is what fills it.
    pub(crate) fn history_status(
        &self,
        state_dir: &Path,
        scope: &str,
        kind: &str,
    ) -> Option<Value> {
        let inner = self.lock();
        let (sealing, since) =
            *inner
                .history
                .get(&(state_dir.to_path_buf(), scope.to_owned(), kind.to_owned()))?;
        let paused = |reason: Option<HistoryPause>, since: i64| json!({"state":"paused","reason":reason.map(HistoryPause::as_str),"since":since});
        Some(match (sealing, inner.history_storage.get(state_dir)) {
            (Err(Some(reason)), _) if reason != HistoryPause::OutboxFull => {
                paused(Some(reason), since)
            }
            (_, Some((refusal, refused_at))) => paused(Some(*refusal), *refused_at),
            (Err(reason), None) => paused(reason, since),
            (Ok(()), None) => json!({"state":"recording","reason":null,"since":since}),
        })
    }

    /// Device-scope Status: snapshots carry only unhealthy tasks and no counters, so
    /// their content changes only when a task changes state.
    pub(crate) fn tasks_json(&self, snapshot: bool) -> Value {
        Value::Array(
            self.tasks()
                .into_iter()
                .filter(|(_, health)| !snapshot || health.state != TaskState::Ok)
                .map(|(name, health)| {
                    let mut task = json!({
                        "name": name,
                        "state": health.state.as_str(),
                        "since": health.since,
                        "category": health.category.map(TaskFailure::as_str),
                    });
                    if !snapshot {
                        task["consecutive_failures"] = json!(health.consecutive_failures);
                    }
                    task
                })
                .collect(),
        )
    }

    pub(crate) fn host_json(&self) -> Value {
        json!({
            "booted_at": credible_boot_time(sysinfo::System::boot_time()),
            "agent_started_at": self.agent_started_at(),
        })
    }

    /// Logs a background task as soon as it fails or stops before agent shutdown, and
    /// shows device Status readers the same outcome.
    pub fn spawn_monitored<F>(
        &self,
        name: &'static str,
        shutdown: &CancellationToken,
        task: F,
    ) -> JoinHandle<()>
    where
        F: Future + Send + 'static,
        F::Output: TaskOutcome + Send + 'static,
    {
        let diagnostics = self.clone();
        let shutdown = shutdown.clone();
        diagnostics.track(name);
        let task = tokio::spawn(task);
        tokio::spawn(async move {
            match task.await.map(TaskOutcome::failure) {
                Ok(Some(error)) => {
                    tracing::error!(task = name, "Background task failed: {error:#}");
                    diagnostics.stopped(name, Some(TaskFailure::classify(&error)));
                }
                Ok(None) if !shutdown.is_cancelled() => {
                    tracing::error!(task = name, "Background task stopped before agent shutdown");
                    diagnostics.stopped(name, None);
                }
                Ok(None) => {}
                Err(error) => {
                    tracing::error!(task = name, "Background task panicked: {error}");
                    diagnostics.stopped(name, Some(TaskFailure::Internal));
                }
            }
        })
    }

    /// Facts only a device-scope Status audience receives, live or in a snapshot.
    pub(crate) fn device_facts(&self, state_dir: &Path, snapshot: bool) -> Map<String, Value> {
        let mut facts = Map::new();
        facts.insert("agent".into(), agent_json(state_dir));
        facts.insert("host".into(), self.host_json());
        facts.insert("tasks".into(), self.tasks_json(snapshot));
        if snapshot {
            // A live inspection result carries the flags for every reader. A snapshot has
            // them here, so a locked device can still be judged by its owner.
            facts.insert("features".into(), features());
        }
        facts
    }
}

/// `Some(None)` for a missing value, `None` for one outside `known`.
fn fact_word(value: &Value, known: &'static [&'static str]) -> Option<Option<&'static str>> {
    match value {
        Value::Null => Some(None),
        Value::String(word) => known
            .iter()
            .find(|known| **known == word)
            .map(|known| Some(*known)),
        _ => None,
    }
}

/// `Some(None)` for a missing time, `None` for a value that is not a time.
fn fact_time(value: &Value) -> Option<Option<i64>> {
    match value {
        Value::Null => Some(None),
        _ => value
            .as_i64()
            .filter(|time| (0..=LATEST_TIME).contains(time))
            .map(Some),
    }
}

fn fact_count(value: &Value) -> Option<u64> {
    match value {
        Value::Null => Some(0),
        _ => value.as_u64(),
    }
}

/// The sum of the counts under `keys`, or `None` when one is not a count.
fn counted(entry: &Value, keys: &[&str]) -> Option<u64> {
    keys.iter().try_fold(0u64, |sum, key| {
        Some(sum.saturating_add(fact_count(&entry[*key])?))
    })
}

/// `Some(None)` for a missing stamp, `None` for one whose time or word is not usable.
fn stamped(
    entry: &Value,
    key: &str,
    word: &str,
    known: &'static [&'static str],
) -> Option<Option<(i64, &'static str)>> {
    let value = &entry[key];
    if value.is_null() {
        return Some(None);
    }
    Some(Some((
        fact_time(&value["at"])??,
        fact_word(&value[word], known)??,
    )))
}

/// An event version as a config pins it: three numbers.
fn event_version(value: &Value) -> Option<[u64; 3]> {
    match value.as_array()?.as_slice() {
        [major, minor, patch] => Some([major.as_u64()?, minor.as_u64()?, patch.as_u64()?]),
        _ => None,
    }
}

fn pinned_version(config: &Value, event_id: &str) -> Option<[u64; 3]> {
    config["events"]
        .as_array()?
        .iter()
        .find(|event| event["event_id"] == event_id)
        .and_then(|event| event_version(&event["event_version"]))
}

/// `YYYY-MM-DD` with digits only.
fn is_date(text: &str) -> bool {
    text.len() == 10
        && text.bytes().enumerate().all(|(index, byte)| match index {
            4 | 7 => byte == b'-',
            _ => byte.is_ascii_digit(),
        })
}

/// The UTC date of a Unix time as `YYYY-MM-DD` (proleptic Gregorian calendar).
fn utc_date(unix: i64) -> String {
    let days = unix.div_euclid(86_400) + 719_468;
    let era = days.div_euclid(146_097);
    let day_of_era = days.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_index = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_index + 2) / 5 + 1;
    let month = if month_index < 10 {
        month_index + 3
    } else {
        month_index - 9
    };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02}")
}

/// The replicas that run the placement's current revisions.
fn runners(record: &PlacementRecord) -> Vec<Runner> {
    record
        .replicas
        .iter()
        .filter(|replica| {
            replica.observed_state == crate::state::ObservedState::Running
                && replica.config_revision == record.config_revision
                && replica.intent_revision == record.intent_revision
        })
        .map(|replica| (replica.slot, replica.process_id))
        .collect()
}

/// One entry of a schedule state file, or `None` when any of its values is out of bounds.
/// An entry with an instant (`once_at`) is a one-time schedule; every other one repeats.
fn schedule_fact(event_id: &str, entry: &Value) -> Option<ScheduleFact> {
    let text = |key: &str, max: usize, allowed: fn(u8) -> bool| {
        entry[key]
            .as_str()
            .filter(|text| (1..=max).contains(&text.len()) && text.bytes().all(allowed))
            .map(str::to_owned)
    };
    let plan = if entry["once_at"].is_null() {
        SchedulePlan::Repeating {
            expression: text("expression", 128, |byte| {
                byte.is_ascii_alphanumeric() || b"*/,- ".contains(&byte)
            })?,
            next_at: fact_time(&entry["next_at"])?,
            skipped: counted(
                entry,
                &["skipped_overlap", "skipped_missed", "skipped_busy"],
            )?,
            last_skip: stamped(entry, "last_skip", "reason", &SCHEDULE_SKIPS)?,
            watermark: fact_time(&entry["watermark"])?,
        }
    } else {
        SchedulePlan::Once {
            at: entry["once_at"]
                .as_i64()
                .filter(|at| ONCE_INSTANTS.contains(at))?,
            state: fact_word(&entry["once_state"], &ONCE_STATES)??,
            armed: fact_time(&entry["once_armed_at"])?.is_some(),
            event_version: match &entry["event_version"] {
                Value::Null => None,
                version => Some(event_version(version)?),
            },
        }
    };
    Some(ScheduleFact {
        event_id: event_id.to_owned(),
        timezone: text("timezone", 64, |byte| {
            byte.is_ascii_alphanumeric() || b"_+-/".contains(&byte)
        })?,
        hold: fact_word(&entry["hold"], &HOLDS)?,
        running: fact_time(&entry["running_since"])?.is_some(),
        last: stamped(entry, "last", "outcome", &OUTCOMES)?,
        runs: fact_count(&entry["runs"])?,
        failed: fact_count(&entry["failed"])?,
        plan,
    })
}

impl ScheduleFact {
    /// A one-time schedule that ran, was missed or had passed, written for the event
    /// version that the placement's config still pins.
    fn finished_for(&self, config: &Value) -> bool {
        match &self.plan {
            SchedulePlan::Once {
                state,
                event_version: Some(version),
                ..
            } => {
                ONCE_FINISHED.contains(state)
                    && pinned_version(config, &self.event_id) == Some(*version)
            }
            _ => false,
        }
    }
}

/// One entry of a bot state file, or `None` when any of its values is out of bounds. A name
/// that is not a plain public handle is left out; the entry stays.
fn bot_fact(event_id: &str, entry: &Value) -> Option<BotFact> {
    let bot_name = entry["bot_name"]
        .as_str()
        .filter(|name| {
            (1..=64).contains(&name.len())
                && name
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || b"_.- ".contains(&byte))
        })
        .map(str::to_owned);
    let day = match &entry["day"] {
        Value::Null => None,
        day => Some((
            day["date"]
                .as_str()
                .filter(|date| is_date(date))?
                .to_owned(),
            day["runs"].as_u64()?,
        )),
    };
    Some(BotFact {
        event_id: event_id.to_owned(),
        provider: fact_word(&entry["provider"], &BOT_PROVIDERS)??,
        state: fact_word(&entry["state"], &BOT_STATES)??,
        hold: fact_word(&entry["hold"], &HOLDS)?,
        bot_name,
        connected_at: fact_time(&entry["connected_at"])?,
        last_message_at: fact_time(&entry["last_message_at"])?,
        last_outcome: stamped(entry, "last", "outcome", &OUTCOMES)?.map(|(_, outcome)| outcome),
        running: fact_count(&entry["running"])?,
        runs: fact_count(&entry["runs"])?,
        failed: fact_count(&entry["failed"])?,
        day,
        dropped: counted(entry, &["dropped_flood", "dropped_busy", "skipped_stale"])?,
    })
}

/// One entry of an instance's run state file, or `None` when any of its values is out of
/// bounds.
fn action_fact(event_id: &str, entry: &Value) -> Option<ActionFact> {
    let last = match &entry["last"] {
        Value::Null => None,
        last => {
            let at = fact_time(&last["at"])??;
            Some((
                at,
                fact_time(&last["finished_at"])?.unwrap_or(at),
                fact_word(&last["outcome"], &OUTCOMES)??,
            ))
        }
    };
    Some(ActionFact {
        event_id: event_id.to_owned(),
        kind: fact_word(&entry["kind"], &ACTION_KINDS)??,
        fields: fact_count(&entry["fields"])?,
        file_fields: fact_count(&entry["file_fields"])?,
        running: fact_count(&entry["running"])?,
        runs: fact_count(&entry["runs"])?,
        failed: fact_count(&entry["failed"])?,
        last,
    })
}

/// The instances' facts per event: counts added up, the run that ended last. What the
/// event is comes from the first instance; every instance of one revision has the same.
fn added_up(files: Vec<StateFile<ActionFact>>) -> Vec<ActionFact> {
    let ended = |fact: &ActionFact| fact.last.map(|(at, finished, _)| (finished, at));
    let mut events = BTreeMap::<String, ActionFact>::new();
    for fact in files.into_iter().flat_map(|file| file.entries) {
        let Some(total) = events.get_mut(&fact.event_id) else {
            events.insert(fact.event_id.clone(), fact);
            continue;
        };
        total.running = total.running.saturating_add(fact.running);
        total.runs = total.runs.saturating_add(fact.runs);
        total.failed = total.failed.saturating_add(fact.failed);
        if ended(&fact) > ended(total) {
            total.last = fact.last;
        }
    }
    events.into_values().collect()
}

/// A placement's schedule state file. `Ok(None)` when it has none; an error when the file
/// is not usable as a whole. The file is untrusted input: a flow that runs with the agent's
/// OS account could have written it.
fn read_schedule_file(
    state_dir: &Path,
    placement: &str,
    config: &Value,
) -> Result<Option<ScheduleFile>> {
    read_state_file(
        state_dir,
        placement,
        SCHEDULE_DIRECTORY,
        STATE_FILE,
        config,
        "events",
        schedule_fact,
    )
}

/// A state file of a placement process whose entries are keyed by event ID under `list`.
/// `Ok(None)` when there is none; an error when it is not usable as a whole.
fn read_state_file<T>(
    state_dir: &Path,
    placement: &str,
    directory: &str,
    file: &str,
    config: &Value,
    list: &str,
    entry: fn(&str, &Value) -> Option<T>,
) -> Result<Option<StateFile<T>>> {
    let Some(bytes) =
        read_placement_file(state_dir, placement, directory, file, MAX_STATE_FILE_BYTES)?
    else {
        return Ok(None);
    };
    state_file(&serde_json::from_slice(&bytes)?, config, list, entry).map(Some)
}

/// A file that a placement process keeps at `<data root>/<directory>/<placement>/<file>`,
/// read as untrusted input: the process can put a link or a FIFO anywhere below its data
/// root. No directory on the way may be a link, the file must be a regular file and is
/// opened without following a link or blocking, and at most `max_bytes` plus one byte is
/// read. `Ok(None)` when a directory or the file is missing; an error when the file is
/// larger than `max_bytes` or anything on the way is not what it must be.
pub(crate) fn read_placement_file(
    state_dir: &Path,
    placement_id: &str,
    directory: &str,
    file: &str,
    max_bytes: u64,
) -> Result<Option<Vec<u8>>> {
    for name in [placement_id, directory, file] {
        anyhow::ensure!(
            !name.is_empty() && name != "." && name != ".." && !name.contains(['/', '\0']),
            "{name:?} is not a plain file name"
        );
    }
    let [data, current, store] = PLACEMENT_DATA_ROOT;
    let mut path = state_dir.to_path_buf();
    for name in [data, placement_id, current, store, directory, placement_id] {
        path.push(name);
        match std::fs::symlink_metadata(&path) {
            Ok(metadata) => anyhow::ensure!(
                metadata.is_dir(),
                "Placement state directory {name:?} is not a directory"
            ),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error.into()),
        }
    }
    path.push(file);
    let max = usize::try_from(max_bytes).unwrap_or(usize::MAX);
    let Some(bytes) = small_file(&path, max)? else {
        return Ok(None);
    };
    anyhow::ensure!(
        bytes.len() <= max,
        "Placement state file {file:?} exceeds its {max_bytes} byte limit"
    );
    Ok(Some(bytes))
}

/// What a regular file holds, read without following a link, or `None` when there is no
/// such file. At most one byte more than `max` is read: that many mean the file is larger.
pub(crate) fn small_file(path: &Path, max: usize) -> std::io::Result<Option<Vec<u8>>> {
    use std::io::Read;
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    let file = match options.open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    if !file.metadata()?.is_file() {
        return Err(std::io::Error::other("Not a regular file"));
    }
    let mut bytes = Vec::new();
    file.take((max as u64).saturating_add(1))
        .read_to_end(&mut bytes)?;
    Ok(Some(bytes))
}

/// The usable entries of a parsed state file, for the events `config` lists, by event ID.
fn state_file<T>(
    state: &Value,
    config: &Value,
    list: &str,
    entry: fn(&str, &Value) -> Option<T>,
) -> Result<StateFile<T>> {
    use anyhow::Context;
    anyhow::ensure!(
        state["version"].as_u64() == Some(1),
        "Unknown state file version"
    );
    let all = state[list]
        .as_object()
        .with_context(|| format!("State file has no {list}"))?;
    let listed = |id: &str| {
        config["events"]
            .as_array()
            .is_some_and(|events| events.iter().any(|event| event["event_id"] == id))
    };
    let mut ids: Vec<&String> = all.keys().filter(|id| listed(id)).collect();
    ids.sort();
    let entries: Vec<T> = ids
        .into_iter()
        .filter_map(|id| entry(id, &all[id]))
        .collect();
    Ok(StateFile {
        config_revision: state["config_revision"]
            .as_u64()
            .context("State file names no config revision")?,
        intent_revision: state["intent_revision"]
            .as_u64()
            .context("State file names no intent revision")?,
        decided: state["decided"] == true,
        dropped: all.len() - entries.len(),
        entries,
    })
}

/// Live rows say when a schedule runs next and how its runs went. Snapshot rows carry only
/// what stays the same between runs, so a snapshot is not republished on every tick.
fn schedule_json(schedule: &ScheduleFact, snapshot: bool, now: i64) -> Value {
    let mut row = json!({
        "event_id": schedule.event_id,
        "timezone": schedule.timezone,
        "hold": schedule.hold,
        "last_outcome": schedule.last.map(|(_, outcome)| outcome),
    });
    let live = !snapshot;
    if live {
        row["running"] = json!(schedule.running);
        row["last_at"] = json!(schedule.last.map(|(at, _)| at));
    }
    match &schedule.plan {
        SchedulePlan::Repeating {
            expression,
            next_at,
            skipped,
            last_skip,
            watermark,
        } => {
            row["expression"] = json!(expression);
            if live {
                row["next_at"] = json!(next_at);
                row["runs"] = json!(schedule.runs);
                row["failed"] = json!(schedule.failed);
                row["skipped"] = json!(skipped);
                row["last_skip"] =
                    json!(last_skip.map(|(at, reason)| json!({"at": at, "reason": reason})));
                // The clock was set back behind the last time that ran: nothing runs
                // until it passed that time again.
                row["clock_behind"] = json!(watermark.is_some_and(|watermark| now < watermark));
            }
        }
        SchedulePlan::Once {
            at, state, armed, ..
        } => {
            row["once_at"] = json!(at);
            row["once_state"] = json!(state);
            if live {
                let waits = *state == "pending" && schedule.hold.is_none() && *armed;
                row["next_at"] = json!(waits.then_some(*at));
            }
        }
    }
    row
}

/// Snapshot rows leave out what changes with every message and every network blip.
fn bot_json(bot: &BotFact, snapshot: bool, today: &str) -> Value {
    if snapshot {
        let state = if BOT_CONNECTION_STATES.contains(&bot.state) {
            "ok"
        } else {
            bot.state
        };
        return json!({
            "event_id": bot.event_id,
            "provider": bot.provider,
            "hold": bot.hold,
            "state": state,
        });
    }
    let runs_today = bot
        .day
        .as_ref()
        .filter(|(date, _)| date == today)
        .map_or(0, |(_, runs)| *runs);
    json!({
        "event_id": bot.event_id,
        "provider": bot.provider,
        "state": bot.state,
        "hold": bot.hold,
        "bot_name": bot.bot_name,
        "connected_at": bot.connected_at,
        "last_message_at": bot.last_message_at,
        "last_outcome": bot.last_outcome,
        "running": bot.running,
        "runs": bot.runs,
        "runs_today": runs_today,
        "failed": bot.failed,
        "dropped": bot.dropped,
    })
}

/// Snapshot rows say what can be run, not how often it ran.
fn action_json(action: &ActionFact, snapshot: bool) -> Value {
    let mut row = json!({
        "event_id": action.event_id,
        "kind": action.kind,
        "fields": action.fields,
        "file_fields": action.file_fields,
    });
    if !snapshot {
        row["running"] = json!(action.running);
        row["last_at"] = json!(action.last.map(|(at, _, _)| at));
        row["last_outcome"] = json!(action.last.map(|(_, _, outcome)| outcome));
        row["runs"] = json!(action.runs);
        row["failed"] = json!(action.failed);
    }
    row
}

/// The first `max` entries as row facts, and whether there were more.
fn listed<T>(entries: &[T], max: usize, fact: impl Fn(&T) -> Value) -> (Vec<Value>, bool) {
    (
        entries.iter().take(max).map(fact).collect(),
        entries.len() > max,
    )
}

/// The lists of `TRIGGER_LISTS` that a process reported, within `TRIGGER_FACTS_BUDGET`
/// bytes together with their keys and flags. Over it, entries are dropped from the end of
/// `actions`, then of `schedules`, then of `bots`; a list that lost entries says so.
fn trigger_facts(mut lists: [Option<(Vec<Value>, bool)>; 3]) -> Map<String, Value> {
    let mut sizes = lists.each_ref().map(|list| {
        list.as_ref().map_or_else(Vec::new, |(entries, _)| {
            entries
                .iter()
                .map(|entry| entry.to_string().len())
                .collect()
        })
    });
    // Every flag still `false`: the longer spelling, so the count is never short.
    let skeleton: Map<String, Value> = TRIGGER_LISTS
        .iter()
        .zip(&lists)
        .filter(|(_, list)| list.is_some())
        .flat_map(|((key, flag), _)| {
            [
                (key.to_string(), json!([])),
                (flag.to_string(), json!(false)),
            ]
        })
        .collect();
    let mut total = Value::Object(skeleton).to_string().len()
        + sizes
            .iter()
            .map(|sizes| sizes.iter().sum::<usize>() + sizes.len().saturating_sub(1))
            .sum::<usize>();
    // Indices into `TRIGGER_LISTS`: actions, schedules, bots.
    for index in [2, 0, 1] {
        while total > TRIGGER_FACTS_BUDGET {
            let (Some((entries, truncated)), Some(size)) =
                (lists[index].as_mut(), sizes[index].pop())
            else {
                break;
            };
            entries.pop();
            *truncated = true;
            total -= size + usize::from(!entries.is_empty());
        }
    }
    TRIGGER_LISTS
        .iter()
        .zip(lists)
        .filter_map(|((key, flag), list)| {
            list.map(|(entries, truncated)| {
                [
                    (key.to_string(), Value::Array(entries)),
                    (flag.to_string(), json!(truncated)),
                ]
            })
        })
        .flatten()
        .collect()
}

/// Without a readable `/proc/stat` the OS probe answers with the uptime, which changes every
/// second and would republish every status snapshot.
fn credible_boot_time(reported: u64) -> Option<u64> {
    (reported >= EARLIEST_CREDIBLE_BOOT).then_some(reported)
}

/// `release_*` stay null for development builds, which have no installed release record.
pub(crate) fn agent_json(state_dir: &Path) -> Value {
    let release = crate::release::installed_release(state_dir);
    json!({
        "version": env!("CARGO_PKG_VERSION"),
        "release_version": release.as_ref().map(|release| release.release_version.clone()),
        "release_sequence": release.as_ref().map(|release| release.sequence),
    })
}

fn truncated(text: &str, max: usize) -> &str {
    let mut end = text.len().min(max);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

/// A network interface with its addresses and the bytes it has carried since boot.
type Interface = (String, Vec<IpAddr>, u64);

fn os_interfaces() -> Vec<Interface> {
    sysinfo::Networks::new_with_refreshed_list()
        .iter()
        .map(|(name, data)| {
            let addresses = data.ip_networks().iter().map(|ip| ip.addr).collect();
            let traffic = data
                .total_received()
                .saturating_add(data.total_transmitted());
            (name.clone(), addresses, traffic)
        })
        .collect()
}

/// Routable addresses come first: a link-local address needs a zone to be usable in a link.
fn address_rank(address: &IpAddr) -> u8 {
    match address {
        IpAddr::V4(address) if address.is_link_local() => 2,
        IpAddr::V4(_) => 0,
        IpAddr::V6(address) if address.is_unicast_link_local() => 3,
        IpAddr::V6(_) => 1,
    }
}

/// A bounded list a client can build service links from. Interfaces without an address
/// are left out. A host with many container bridges keeps its busiest interfaces, which
/// include the ones that reach the network; loopback interfaces come last.
fn network_json(mut interfaces: Vec<Interface>) -> Value {
    interfaces.retain(|(_, addresses, _)| !addresses.is_empty());
    interfaces.sort_by(|a, b| b.2.cmp(&a.2).then_with(|| a.0.cmp(&b.0)));
    interfaces.truncate(MAX_INTERFACES);
    let mut interfaces: Vec<_> = interfaces
        .into_iter()
        .map(|(name, mut addresses, _)| {
            let loopback = addresses.iter().all(IpAddr::is_loopback);
            addresses.sort_by_key(|address| (address_rank(address), *address));
            addresses.dedup();
            addresses.truncate(MAX_INTERFACE_ADDRESSES);
            (loopback, name, addresses)
        })
        .collect();
    interfaces.sort();
    json!({
        "interfaces": interfaces
            .into_iter()
            .map(|(loopback, name, addresses)| {
                json!({
                    "name": truncated(&name, MAX_INTERFACE_NAME),
                    "addresses": addresses.iter().map(IpAddr::to_string).collect::<Vec<_>>(),
                    "loopback": loopback,
                })
            })
            .collect::<Vec<_>>(),
    })
}

/// How much of a placement row to send, from everything down to the pre-diagnostics
/// shape. Readers shed detail until the encrypted message fits.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Detail {
    Full,
    NoReplicaText,
    NoEvents,
    Minimal,
}

pub(crate) const DETAILS: [Detail; 4] = [
    Detail::Full,
    Detail::NoReplicaText,
    Detail::NoEvents,
    Detail::Minimal,
];

/// Builds placement rows for one audience. Snapshots never carry error text, process
/// IDs or countdowns, which change without the placement changing.
pub(crate) struct Rows<'a> {
    diagnostics: &'a Diagnostics,
    state_dir: &'a Path,
    state_prefixes: Vec<String>,
    snapshot: bool,
    /// The device clock the rows are judged by, in Unix seconds.
    now: i64,
}

impl<'a> Rows<'a> {
    pub(crate) fn new(diagnostics: &'a Diagnostics, state_dir: &'a Path, snapshot: bool) -> Self {
        let mut state_prefixes = vec![state_dir.to_string_lossy().into_owned()];
        if let Ok(canonical) = state_dir.canonicalize() {
            let canonical = canonical.to_string_lossy().into_owned();
            if !state_prefixes.contains(&canonical) {
                state_prefixes.push(canonical);
            }
        }
        state_prefixes.retain(|prefix| prefix.len() > 1);
        state_prefixes.sort_by_key(|prefix| std::cmp::Reverse(prefix.len()));
        Self {
            diagnostics,
            state_dir,
            state_prefixes,
            snapshot,
            now: now(),
        }
    }

    pub(crate) fn snapshot(diagnostics: &'a Diagnostics, state_dir: &'a Path) -> Self {
        Self::new(diagnostics, state_dir, true)
    }

    #[cfg(test)]
    pub(crate) fn at(self, now: i64) -> Self {
        Self { now, ..self }
    }

    /// `error_text` is true for the owner and for principals holding Logs on the placement;
    /// everyone else learns only that an error exists.
    pub(crate) fn placement(
        &self,
        record: &PlacementRecord,
        error_text: bool,
        detail: Detail,
    ) -> Value {
        let error_text = error_text && !self.snapshot;
        let mut row = legacy_row(record);
        row["has_error"] = json!(record.last_error.is_some());
        row["source"] = record.config["source"].clone();
        if let Some(summary) = self.diagnostics.offline_writes(self.state_dir, &record.id) {
            row["offline_writes"] = summary;
        }
        if detail >= Detail::NoEvents {
            row["events_truncated"] = json!(true);
        }
        // No key at all means that no running process reports that list: the service is
        // stopped, starting or failing, or has none. Schedules that are over are the one
        // exception: they are reported whatever runs.
        let schedules = self.diagnostics.schedules(self.state_dir, record);
        let bots = self.diagnostics.bots(self.state_dir, record);
        let actions = self.diagnostics.actions(self.state_dir, record);
        if detail >= Detail::NoEvents {
            let reported = [schedules.is_some(), bots.is_some(), actions.is_some()];
            for ((_, flag), reported) in TRIGGER_LISTS.iter().zip(reported) {
                if reported {
                    row[*flag] = json!(true);
                }
            }
        }
        if detail == Detail::Minimal {
            return row;
        }
        if !self.snapshot {
            row["process_id"] = json!(record.process_id);
        }
        if error_text {
            row["last_error"] = json!(record.last_error.as_deref().map(|text| self.redact(text)));
        }
        row["online_metadata_sha256"] = record.config["online_metadata_sha256"].clone();
        if detail < Detail::NoEvents {
            let (events, truncated) = events(&record.config);
            row["events"] = events;
            row["events_truncated"] = json!(truncated);
            let (snapshot, now) = (self.snapshot, self.now);
            let lists = [
                schedules.map(|file| {
                    listed(&file.entries, MAX_SCHEDULES, |schedule| {
                        schedule_json(schedule, snapshot, now)
                    })
                }),
                bots.map(|file| {
                    let today = utc_date(now);
                    listed(&file.entries, MAX_BOTS, |bot| {
                        bot_json(bot, snapshot, &today)
                    })
                }),
                actions.map(|actions| {
                    listed(&actions, MAX_ACTIONS, |action| {
                        action_json(action, snapshot)
                    })
                }),
            ];
            if let Value::Object(object) = &mut row {
                object.extend(trigger_facts(lists));
            }
        }
        let replica_text = error_text && detail == Detail::Full;
        row["replicas"] = record
            .replicas
            .iter()
            .map(|replica| {
                let mut row = json!({
                    "slot": replica.slot,
                    "observed_state": replica.observed_state,
                    "applied_revision": replica.applied_revision,
                    "has_error": replica.last_error.is_some(),
                });
                if !self.snapshot {
                    row["process_id"] = json!(replica.process_id);
                }
                if replica_text {
                    row["last_error"] =
                        json!(replica.last_error.as_deref().map(|text| self.redact(text)));
                }
                if let Some(restarts) = self.diagnostics.replica(&record.id, replica.slot) {
                    row["restarts"] = restarts_json(&restarts, self.snapshot);
                }
                row
            })
            .collect();
        row
    }

    fn redact(&self, text: &str) -> String {
        let text = self
            .state_prefixes
            .iter()
            .fold(text.to_owned(), |text, prefix| {
                text.replace(prefix.as_str(), STATE_PLACEHOLDER)
            });
        truncated(&text, MAX_ERROR_TEXT).to_owned()
    }
}

/// The row older agents sent; clients without the feature flags read only these fields.
fn legacy_row(p: &PlacementRecord) -> Value {
    json!({"id":p.id,"project_id":p.config.get("project_id"),"deployment_id":p.config.get("deployment_id"),"revision":p.config.get("revision"),"desired_state":p.desired_state,"observed_state":p.observed_state,"config_revision":p.config_revision,"intent_revision":p.intent_revision,"applied_revision":p.applied_revision,"desired_replicas":p.desired_replicas,"running_replicas":p.running_replicas,"ready_replicas":p.ready_replicas,"max_replicas":p.config.get("max_replicas").cloned().unwrap_or(json!(1)),"replicas":p.replicas.iter().map(|r|json!({"slot":r.slot,"observed_state":r.observed_state,"applied_revision":r.applied_revision})).collect::<Vec<_>>()})
}

/// Event identities and pinned versions only: variables, secrets and hosting stay on the device.
fn events(config: &Value) -> (Value, bool) {
    let all = config["events"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or(&[]);
    let events = all
        .iter()
        .take(MAX_EVENTS)
        .map(|event| {
            json!({
                "event_id": event["event_id"],
                "event_version": event["event_version"],
                "board_version": event["board_version"],
            })
        })
        .collect();
    (Value::Array(events), all.len() > MAX_EVENTS)
}

fn restarts_json(restarts: &ReplicaRestarts, snapshot: bool) -> Value {
    let mut value = json!({
        "failures": restarts.failures,
        "max_restarts": restarts.max_restarts,
        "crash_looping": restarts.crash_looping,
        "last_started_at": restarts.last_started_at,
    });
    if !snapshot {
        value["retry_in_seconds"] = json!(
            restarts
                .retry_at
                .map(|at| at.saturating_duration_since(Instant::now()).as_secs())
        );
    }
    value
}

/// What `status` prints so a person can compare the device with the hub's record.
pub fn identity_summary(identity: &DeviceIdentity) -> Result<Value> {
    Ok(json!({
        "fingerprint": identity.fingerprint()?,
        "auth_key": identity.auth_key.thumbprint()?,
        "telemetry_key": identity.telemetry_key.thumbprint()?,
    }))
}

#[cfg(test)]
pub(crate) mod test_support {
    use super::*;
    use crate::{
        replicas::ReplicaRecord,
        state::{DesiredState, ObservedState, StateStore},
    };
    use rusqlite::params;
    use std::time::Duration;

    impl Diagnostics {
        /// Adds facts next to the supervisor's, which no unit test runs.
        pub(crate) fn insert_replica(&self, placement: &str, slot: u8, restarts: ReplicaRestarts) {
            self.lock()
                .replicas
                .insert((placement.to_owned(), slot), restarts);
        }

        /// Replaces this host's interfaces until `forget_network`.
        pub(crate) fn insert_network(&self, network: Value) {
            self.lock().network = Some((Instant::now() + Duration::from_secs(3600), network));
        }

        /// Totals no test could queue: they stand in for the placement's queues.
        pub(crate) fn insert_offline_writes(
            &self,
            state_dir: &Path,
            placement: &str,
            summary: Value,
        ) {
            self.lock().offline_writes.insert(
                (state_dir.to_path_buf(), placement.to_owned()),
                (Instant::now() + Duration::from_secs(3600), Some(summary)),
            );
        }

        pub(crate) fn forget_network(&self) {
            self.lock().network = None;
        }
    }

    /// As many interfaces and addresses as a reply carries, each as long as it can be.
    pub(crate) fn worst_case_network() -> Value {
        let address = |interface: u16, index: u16| {
            IpAddr::V6(std::net::Ipv6Addr::new(
                0x2001,
                0xdb8,
                0xffff,
                0xffff,
                0xffff,
                0xffff,
                0xff00 + interface,
                0xff00 + index,
            ))
        };
        network_json(
            (0..MAX_INTERFACES as u16 + 8)
                .map(|interface| {
                    let name = format!("{interface:02}{}", "n".repeat(MAX_INTERFACE_NAME + 8));
                    let addresses = (0..MAX_INTERFACE_ADDRESSES as u16 + 4)
                        .map(|index| address(interface, index))
                        .collect();
                    (name, addresses, 0)
                })
                .collect(),
        )
    }

    /// What the agent registry reports for `task`, for tests that drive a real loop.
    pub(crate) fn health(task: &str) -> Option<(TaskState, Option<TaskFailure>)> {
        global()
            .tasks()
            .into_iter()
            .find(|(name, _)| name == task)
            .map(|(_, health)| (health.state, health.category))
    }

    /// Waits for a loop's first reported pass.
    pub(crate) async fn reported(task: &str) -> (TaskState, Option<TaskFailure>) {
        for _ in 0..200 {
            if let Some(health) = health(task) {
                return health;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        panic!("{task} never reported a pass");
    }

    pub(crate) fn worst_case_id(prefix: &str) -> String {
        format!("{prefix}{}", "-".repeat(128 - prefix.len()))
    }

    pub(crate) fn restarts(retry_in: u64) -> ReplicaRestarts {
        ReplicaRestarts {
            failures: u32::MAX,
            max_restarts: u32::MAX,
            crash_looping: true,
            retry_at: Some(Instant::now() + Duration::from_secs(retry_in)),
            last_started_at: Some(i64::MAX),
        }
    }

    /// 64 full queues at the largest configurable limits.
    pub(crate) fn worst_case_offline_writes() -> Value {
        json!({"scopes":64,"pending_count":6_400_000,"pending_bytes":u64::MAX,"oldest_at":i64::MAX,"quarantined_scopes":64,"needs_attention":64,"mirror_error":false})
    }

    /// A reboot or update with maximal fields, issued by a grantee.
    pub(crate) fn worst_case_host_operation(store: &StateStore) -> Result<String> {
        let id = worst_case_id("worst-host-operation");
        store.connection.execute(
            "INSERT INTO host_operations(operation_id,kind,boot_id,state,created_at) VALUES(?1,'update','boot','requesting',?2)",
            params![id, i64::MAX],
        )?;
        store.connection.execute(
            "INSERT INTO management_operations(operation_id,request_digest,principal,accepted_at,result_json) VALUES(?1,'digest',?2,?3,'{\"state\":\"accepted\"}')",
            params![id, format!("{}:{}:thumbprint", "u".repeat(128), "g".repeat(128)), i64::MAX],
        )?;
        Ok(id)
    }

    /// 32 restarting replicas, 64 maximal events, maximal identifiers and revisions,
    /// over-long errors that name the state directory, and full offline queues.
    pub(crate) fn worst_case_placement(
        store: &mut StateStore,
        root: &Path,
        diagnostics: &Diagnostics,
        id: &str,
    ) -> Result<()> {
        diagnostics.insert_offline_writes(root, id, worst_case_offline_writes());
        let pinned = [u32::MAX - 1; 3];
        let events: Vec<Value> = (0..64)
            .map(|index| {
                json!({"event_id":format!("{index:02}{}", "e".repeat(126)),"event_version":pinned,"board_version":pinned})
            })
            .collect();
        let config = json!({"id":id,"project_id":"p".repeat(128),"deployment_id":"d".repeat(128),"revision":"r".repeat(128),"source":"online","online_metadata_sha256":"f".repeat(64),"project_path":root,"max_replicas":32,"events":events,"variables":{"api-key":"variable-value"}});
        store.upsert_placement(id, &config, DesiredState::Running)?;
        let error = format!("{}/projects failed: {}", root.display(), "x".repeat(2048));
        store.connection.execute(
            "UPDATE placements SET desired_replicas=32,observed_state='backoff',config_revision=?2,intent_revision=?2,applied_revision=?2,process_id=?3,last_error=?4 WHERE id=?1",
            params![id, i64::MAX, u32::MAX, error],
        )?;
        for slot in 0..32u8 {
            store.connection.execute(
                "INSERT INTO placement_replicas(placement_id,slot,config_revision,intent_revision,observed_state,applied_revision,process_id,last_error) VALUES(?1,?2,?3,?3,'stopping',?3,?4,?5)",
                params![id, slot, i64::MAX, u32::MAX - u32::from(slot), error],
            )?;
            diagnostics.insert_replica(id, slot, restarts(3600));
        }
        Ok(())
    }

    /// A placement named `api` whose one replica runs at the placement's current revisions.
    pub(crate) fn running_record(events: &[&str]) -> PlacementRecord {
        let mut record = failing_record();
        record.config["events"] = events
            .iter()
            .map(|id| json!({"event_id": id, "event_version": [1, 0, 0], "board_version": [2, 0, 0]}))
            .collect();
        record.observed_state = ObservedState::Running;
        record.last_error = None;
        record.running_replicas = 1;
        record.ready_replicas = 1;
        record.replicas[0].observed_state = ObservedState::Running;
        record.replicas[0].process_id = Some(4211);
        record.replicas[0].last_error = None;
        record
    }

    /// Where a placement process keeps the files of `directory` below the agent's state
    /// directory.
    pub(crate) fn placement_file_dir(
        state_dir: &Path,
        placement: &str,
        directory: &str,
    ) -> PathBuf {
        let [data, current, store] = PLACEMENT_DATA_ROOT;
        [data, placement, current, store, directory, placement]
            .iter()
            .fold(state_dir.to_path_buf(), |path, name| path.join(name))
    }

    pub(crate) fn write_placement_file(
        state_dir: &Path,
        placement: &str,
        directory: &str,
        file: &str,
        bytes: &[u8],
    ) {
        let directory = placement_file_dir(state_dir, placement, directory);
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::write(directory.join(file), bytes).unwrap();
    }

    /// Where a placement process keeps its schedule state below the agent's state directory.
    pub(crate) fn schedule_state_dir(state_dir: &Path, placement: &str) -> PathBuf {
        placement_file_dir(state_dir, placement, SCHEDULE_DIRECTORY)
    }

    pub(crate) fn write_schedule_state(state_dir: &Path, placement: &str, state: &[u8]) {
        write_placement_file(state_dir, placement, SCHEDULE_DIRECTORY, STATE_FILE, state);
    }

    pub(crate) fn write_bot_state(state_dir: &Path, placement: &str, state: &[u8]) {
        write_placement_file(state_dir, placement, BOT_DIRECTORY, STATE_FILE, state);
    }

    pub(crate) fn write_run_state(state_dir: &Path, placement: &str, slot: u8, state: &[u8]) {
        let file = format!("state.{slot}.json");
        write_placement_file(state_dir, placement, RUN_DIRECTORY, &file, state);
    }

    /// The one-time entry `evt_once` of design §1.6a: armed, waiting for its time.
    pub(crate) fn design_once_entry() -> Value {
        json!({"once_at":1790233200,"timezone":"Europe/Berlin","event_version":[0,0,4],
            "once_state":"pending","once_armed_at":1790000000,"hold":null,"confirmed":true,"since":1789990000,
            "running_since":null,"last":null,"runs":0,"failed":0})
    }

    /// The bot `evt_helper` of design §1.6b: connected, 12 runs on 2026-10-02.
    pub(crate) fn design_bot_entry() -> Value {
        json!({"provider":"telegram","state":"connected","hold":null,"bot_id":7123456789u64,"bot_name":"helper_bot",
            "connected_at":1790000003,"webhook_cleared":true,"watermark":412345678,"watermark_at":1790003600,
            "last_message_at":1790003600,"last":{"at":1790003600,"finished_at":1790003612,"outcome":"succeeded"},
            "running":1,"messages":120,"runs":57,"failed":2,"ignored":61,
            "dropped_flood":0,"dropped_busy":0,"skipped_stale":3,"images_left_out":0,"reconnects":1,
            "day":{"date":"2026-10-02","runs":12}})
    }

    /// The form `evt_notes_form` of one instance, design §1.6c.
    pub(crate) fn design_action_entry() -> Value {
        json!({"kind":"form","fields":1,"file_fields":0,"running":0,"runs":12,"failed":1,
            "last":{"at":1790035200,"finished_at":1790035204,"outcome":"succeeded","origin":"management"}})
    }

    /// A schedule state file of design §1.6a, with its top-level keys that the parent ignores.
    pub(crate) fn design_schedules(events: Value, (config, intent): (u64, u64)) -> Vec<u8> {
        serde_json::to_vec(&json!({"version":1,"config_revision":config,"intent_revision":intent,"decided":true,"grant_id":"g-1",
            "events":events,
            "once_done":[{"event_id":"evt_old","once_at":1789023600,"state":"ran","outcome":"succeeded","at":1789023604}],
            "claims":{"evt_helper":{"hold":null,"confirmed":true,"since":1789990000}}}))
        .unwrap()
    }

    /// A bot state file of design §1.6b.
    pub(crate) fn design_bots(bots: Value, (config, intent): (u64, u64)) -> Vec<u8> {
        serde_json::to_vec(&json!({"version":1,"config_revision":config,"intent_revision":intent,"decided":true,"bots":bots}))
            .unwrap()
    }

    /// Every event of `worst_case_placement` as a one-time schedule that is over, with the
    /// longest values a reader keeps: a placement reports these although nothing of it runs.
    pub(crate) fn worst_case_schedules(state_dir: &Path, id: &str) {
        let pinned = [u32::MAX - 1; 3];
        let entry = json!({"once_at":ONCE_INSTANTS.end(),"timezone":"Z".repeat(64),"event_version":pinned,
            "once_state":"missed","once_armed_at":LATEST_TIME,"hold":null,"running_since":null,
            "last":{"at":LATEST_TIME,"finished_at":LATEST_TIME,"outcome":"timed_out"}});
        let events: Map<String, Value> = (0..64)
            .map(|index| (format!("{index:02}{}", "e".repeat(126)), entry.clone()))
            .collect();
        let state = design_schedules(Value::Object(events), (0, 0));
        write_schedule_state(state_dir, id, &state);
    }

    /// An instance's run state file of design §1.6c.
    pub(crate) fn design_runs(events: Value, (config, intent): (u64, u64)) -> Vec<u8> {
        serde_json::to_vec(
            &json!({"version":1,"config_revision":config,"intent_revision":intent,"events":events}),
        )
        .unwrap()
    }

    pub(crate) fn failing_record() -> PlacementRecord {
        let replica = ReplicaRecord {
            slot: 0,
            config_revision: 1,
            intent_revision: 1,
            observed_state: ObservedState::Backoff,
            applied_revision: Some(1),
            process_id: None,
            last_error: Some("Persistent service exited: 1".into()),
        };
        PlacementRecord {
            id: "api".into(),
            config: json!({"id":"api","project_id":"project","deployment_id":"deployment","revision":"one","source":"offline","project_path":"/srv/project","events":[{"event_id":"http","event_version":[1,0,0],"board_version":[2,0,0]}],"variables":{"api-key":"variable-value"},"secret_overrides":{"token":"secret-name"},"hosting":{"host":"127.0.0.1","port":8080,"max_in_flight":4,"request_timeout_secs":30,"auth_secret":"hosting-secret"}}),
            desired_state: DesiredState::Running,
            intent_revision: 1,
            observed_state: ObservedState::Backoff,
            config_revision: 1,
            applied_revision: Some(1),
            process_id: Some(4211),
            last_error: Some("Persistent service exited: 1".into()),
            desired_replicas: 1,
            running_replicas: 0,
            ready_replicas: 0,
            replicas: vec![replica],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{test_support::*, *};
    use crate::state::ObservedState;
    use std::time::Duration;

    #[test]
    fn task_health_counts_failures_resets_on_success_and_keeps_the_stop_cause() {
        let diagnostics = Diagnostics::default();
        diagnostics.track("fleet_publisher");
        assert_eq!(diagnostics.tasks()[0].1.state, TaskState::Ok);
        diagnostics.report("fleet_publisher", Err(TaskFailure::HubUnreachable));
        let since = diagnostics.tasks()[0].1.since;
        diagnostics.report("fleet_publisher", Err(TaskFailure::HubRefused));
        let failing = diagnostics.tasks()[0].1.clone();
        assert_eq!(failing.state, TaskState::Failing);
        assert_eq!(failing.consecutive_failures, 2);
        assert_eq!(failing.since, since);
        assert_eq!(failing.category, Some(TaskFailure::HubRefused));
        diagnostics.report("fleet_publisher", Ok(()));
        let recovered = diagnostics.tasks()[0].1.clone();
        assert_eq!(recovered.state, TaskState::Ok);
        assert_eq!(recovered.consecutive_failures, 0);
        assert_eq!(recovered.category, None);
        diagnostics.report("fleet_publisher", Err(TaskFailure::Policy));
        diagnostics.stopped("fleet_publisher", None);
        let stopped = diagnostics.tasks()[0].1.clone();
        assert_eq!(stopped.state, TaskState::Stopped);
        assert_eq!(stopped.category, Some(TaskFailure::Policy));
    }

    #[test]
    fn snapshots_carry_only_unhealthy_tasks_without_counters() {
        let diagnostics = Diagnostics::default();
        diagnostics.track("healthy");
        diagnostics.report("failing", Err(TaskFailure::Storage));
        let live = diagnostics.tasks_json(false);
        assert_eq!(live.as_array().unwrap().len(), 2);
        assert_eq!(live[0]["name"], "failing");
        assert_eq!(live[0]["consecutive_failures"], 1);
        assert_eq!(live[0]["category"], "storage");
        assert!(live[1]["category"].is_null());
        let snapshot = diagnostics.tasks_json(true);
        assert_eq!(
            snapshot,
            json!([{"name":"failing","state":"failing","since":live[0]["since"],"category":"storage"}])
        );
    }

    #[tokio::test]
    async fn monitored_tasks_record_failures_early_stops_and_panics_but_not_shutdown() {
        assert!(().failure().is_none());
        assert!(Ok::<(), anyhow::Error>(()).failure().is_none());
        let failure = Err::<(), _>(anyhow::anyhow!("database is locked").context("Sample metrics"))
            .failure()
            .map(|error| format!("{error:#}"))
            .unwrap();
        assert!(failure.contains("Sample metrics") && failure.contains("database is locked"));
        let diagnostics = Diagnostics::default();
        let shutdown = CancellationToken::new();
        let state = |name: &str| {
            diagnostics
                .tasks()
                .into_iter()
                .find(|(task, _)| task == name)
                .map(|(_, health)| (health.state, health.category))
        };
        diagnostics
            .spawn_monitored("failed", &shutdown, async {
                Err::<(), _>(anyhow::Error::new(std::io::Error::other("disk")))
            })
            .await
            .unwrap();
        diagnostics
            .spawn_monitored("ended", &shutdown, async {})
            .await
            .unwrap();
        diagnostics
            .spawn_monitored("panicked", &shutdown, async {
                let fail: fn() = || panic!("background task bug");
                fail();
            })
            .await
            .unwrap();
        assert_eq!(
            state("failed"),
            Some((TaskState::Stopped, Some(TaskFailure::Storage)))
        );
        assert_eq!(state("ended"), Some((TaskState::Stopped, None)));
        assert_eq!(
            state("panicked"),
            Some((TaskState::Stopped, Some(TaskFailure::Internal)))
        );
        shutdown.cancel();
        diagnostics
            .spawn_monitored("shutdown", &shutdown, async {})
            .await
            .unwrap();
        assert_eq!(state("shutdown"), Some((TaskState::Ok, None)));
    }

    #[test]
    fn failures_are_classified_into_fixed_categories() {
        let storage = anyhow::Error::new(rusqlite::Error::InvalidQuery).context("read placements");
        assert_eq!(TaskFailure::classify(&storage), TaskFailure::Storage);
        let policy = anyhow::Error::new(flow_like_device_protocol::ProtocolError::InvalidTime);
        assert_eq!(TaskFailure::classify(&policy), TaskFailure::Policy);
        let rules = anyhow::Error::new(RulesDiffer("Fleet server omitted current policy"))
            .context("publish fleet status");
        assert_eq!(TaskFailure::classify(&rules), TaskFailure::Policy);
        assert!(format!("{rules:#}").ends_with("Fleet server omitted current policy"));
        assert_eq!(
            TaskFailure::classify(&anyhow::anyhow!("unexpected")),
            TaskFailure::Internal
        );
        for (status, category) in [
            (
                reqwest::StatusCode::BAD_GATEWAY,
                TaskFailure::HubUnreachable,
            ),
            (
                reqwest::StatusCode::SERVICE_UNAVAILABLE,
                TaskFailure::HubUnreachable,
            ),
            (reqwest::StatusCode::UNAUTHORIZED, TaskFailure::HubRefused),
            (
                reqwest::StatusCode::PAYMENT_REQUIRED,
                TaskFailure::HubRefused,
            ),
        ] {
            assert_eq!(TaskFailure::hub_answered(status), category, "{status}");
        }
    }

    #[test]
    fn history_status_ranks_causes_and_keeps_the_time_a_cause_began() {
        let registry = Diagnostics::default();
        let root = Path::new("/state");
        let status = |scope: &str| registry.history_status(root, scope, "log");
        let streams = |device: Sealing, api: Sealing| {
            registry.set_history_sealing(
                root,
                vec![
                    ("device".into(), "log".into(), device),
                    ("api".into(), "log".into(), api),
                ],
            )
        };
        assert_eq!(status("device"), None);
        streams(Ok(()), Err(Some(HistoryPause::OutboxFull)));
        for key in registry.lock().history.values_mut() {
            key.1 = 5;
        }
        streams(Ok(()), Err(Some(HistoryPause::OutboxFull)));
        assert_eq!(
            status("device"),
            Some(json!({"state":"recording","reason":null,"since":5}))
        );
        assert_eq!(
            status("api"),
            Some(json!({"state":"paused","reason":"outbox_full","since":5}))
        );
        assert_eq!(registry.history_status(root, "device", "metrics"), None);
        assert_eq!(
            registry.history_status(Path::new("/other"), "device", "log"),
            None
        );

        registry.set_history_storage(root, Some(HistoryPause::QuotaReached));
        registry.lock().history_storage.get_mut(root).unwrap().1 = 7;
        registry.set_history_storage(root, Some(HistoryPause::QuotaReached));
        for scope in ["device", "api"] {
            assert_eq!(
                status(scope),
                Some(json!({"state":"paused","reason":"quota_reached","since":7}))
            );
        }
        registry.set_history_storage(root, Some(HistoryPause::TierWithoutHistory));
        let refused = status("device").unwrap();
        assert_eq!(refused["reason"], "tier_without_history");
        assert!(refused["since"].as_i64() > Some(7));

        streams(Err(Some(HistoryPause::RulesExpired)), Err(None));
        let expired = status("device").unwrap();
        assert_eq!(expired["reason"], "rules_expired");
        assert!(expired["since"].as_i64() > Some(5));
        assert_eq!(status("api").unwrap()["reason"], "tier_without_history");
        registry.set_history_storage(root, None);
        let unknown = status("api").unwrap();
        assert!(unknown["state"] == "paused" && unknown["reason"].is_null());

        registry.set_history_sealing(
            Path::new("/other"),
            vec![("device".into(), "log".into(), Ok(()))],
        );
        registry.set_history_sealing(root, vec![("api".into(), "log".into(), Ok(()))]);
        assert_eq!(status("device"), None);
        assert_eq!(status("api").unwrap()["state"], "recording");
        assert!(
            registry
                .history_status(Path::new("/other"), "device", "log")
                .is_some()
        );
        for reason in [
            HistoryPause::RulesChanged,
            HistoryPause::RosterExpired,
            HistoryPause::QuotaReached,
        ] {
            streams(Err(Some(reason)), Ok(()));
            assert_eq!(status("device").unwrap()["reason"], reason.as_str());
        }
    }

    #[test]
    fn renewal_causes_are_kept_per_state_directory_kind_and_certificate() {
        let registry = Diagnostics::default();
        let (root, other) = (Path::new("/state"), Path::new("/other"));
        registry.set_renewal_failure(root, Renewal::Acme, "certificate", RenewalFailure::Dns);
        registry.set_renewal_failure(
            root,
            Renewal::Issuer,
            "certificate",
            RenewalFailure::AuthorityExpired,
        );
        let known = |root, renewal| registry.renewal_failure(root, renewal, "certificate");
        assert_eq!(known(root, Renewal::Acme), Some(RenewalFailure::Dns));
        assert_eq!(
            known(root, Renewal::Issuer),
            Some(RenewalFailure::AuthorityExpired)
        );
        assert_eq!(known(other, Renewal::Acme), None);
        registry.set_renewal_failure(root, Renewal::Acme, "certificate", RenewalFailure::PortBind);
        assert_eq!(known(root, Renewal::Acme), Some(RenewalFailure::PortBind));
        for index in 0..MAX_RENEWAL_FAILURES {
            registry.set_renewal_failure(
                other,
                Renewal::Acme,
                &index.to_string(),
                RenewalFailure::Network,
            );
        }
        assert!(registry.lock().renewal_failures.len() <= MAX_RENEWAL_FAILURES);
        let causes = [
            RenewalFailure::Dns,
            RenewalFailure::PortBind,
            RenewalFailure::RateLimited,
            RenewalFailure::CaRejected,
            RenewalFailure::AuthorityExpired,
            RenewalFailure::Network,
            RenewalFailure::Internal,
        ]
        .map(RenewalFailure::as_str);
        assert_eq!(
            causes,
            [
                "dns",
                "port_bind",
                "rate_limited",
                "ca_rejected",
                "authority_expired",
                "network",
                "internal"
            ]
        );
    }

    #[test]
    fn host_facts_drop_a_boot_time_that_is_an_uptime_or_an_unset_clock() {
        assert_eq!(credible_boot_time(0), None);
        assert_eq!(credible_boot_time(86_400), None);
        assert_eq!(credible_boot_time(1_727_770_000), Some(1_727_770_000));
        let host = Diagnostics::default().host_json();
        assert!(
            host["booted_at"]
                .as_u64()
                .is_none_or(|at| at >= EARLIEST_CREDIBLE_BOOT)
        );
    }

    #[test]
    fn replica_facts_are_replaced_per_tick() {
        let diagnostics = Diagnostics::default();
        diagnostics.set_replicas(HashMap::from([(("api".into(), 0), restarts(40))]));
        assert!(diagnostics.replica("api", 0).is_some());
        diagnostics.set_replicas(HashMap::new());
        assert!(diagnostics.replica("api", 0).is_none());
    }

    #[test]
    fn rows_give_error_text_only_to_logs_readers_and_snapshots_drop_volatile_fields() {
        let root = tempfile::tempdir().unwrap();
        let diagnostics = Diagnostics::default();
        diagnostics.insert_replica("api", 0, restarts(40));
        let mut record = failing_record();
        record.last_error = Some(format!(
            "Cannot open {}/projects/api: {}",
            root.path().display(),
            "x".repeat(2048)
        ));
        let live = Rows::new(&diagnostics, root.path(), false);
        let logs = live.placement(&record, true, Detail::Full);
        let text = logs["last_error"].as_str().unwrap();
        assert!(text.starts_with("Cannot open <state>/projects/api: "));
        assert_eq!(text.len(), MAX_ERROR_TEXT);
        assert_eq!(logs["has_error"], true);
        assert_eq!(logs["process_id"], 4211);
        assert_eq!(
            logs["replicas"][0]["last_error"],
            "Persistent service exited: 1"
        );
        let restarts = &logs["replicas"][0]["restarts"];
        assert_eq!(restarts["crash_looping"], true);
        assert!((39..=40).contains(&restarts["retry_in_seconds"].as_u64().unwrap()));
        assert_eq!(logs["source"], "offline");
        assert_eq!(
            logs["events"],
            json!([{"event_id":"http","event_version":[1,0,0],"board_version":[2,0,0]}])
        );
        assert_eq!(logs["events_truncated"], false);
        assert!(logs["online_metadata_sha256"].is_null());
        for private in [
            "variable-value",
            "secret-name",
            "hosting-secret",
            "/srv/project",
            "127.0.0.1",
        ] {
            assert!(!logs.to_string().contains(private), "{private}");
        }

        let status = live.placement(&record, false, Detail::Full);
        assert!(status.get("last_error").is_none());
        assert_eq!(status["has_error"], true);
        assert!(status["replicas"][0].get("last_error").is_none());
        assert_eq!(status["replicas"][0]["has_error"], true);

        let snapshot =
            Rows::snapshot(&diagnostics, root.path()).placement(&record, true, Detail::Full);
        let encoded = snapshot.to_string();
        for volatile in ["last_error", "retry_in_seconds", "process_id"] {
            assert!(!encoded.contains(volatile), "{volatile}");
        }
        assert_eq!(snapshot["replicas"][0]["restarts"]["failures"], u32::MAX);
    }

    #[test]
    fn detail_levels_shed_replica_text_then_events_then_diagnostics() {
        let diagnostics = Diagnostics::default();
        diagnostics.insert_replica("api", 0, restarts(40));
        let record = failing_record();
        let rows = Rows::new(&diagnostics, Path::new("/var/lib/flow-like"), false);
        let replica_text = rows.placement(&record, true, Detail::NoReplicaText);
        assert!(replica_text["last_error"].is_string());
        assert!(replica_text["replicas"][0].get("last_error").is_none());
        assert!(replica_text["events"].is_array());
        let no_events = rows.placement(&record, true, Detail::NoEvents);
        assert!(no_events.get("events").is_none());
        assert_eq!(no_events["events_truncated"], true);
        assert!(no_events["replicas"][0]["restarts"].is_object());
        let minimal = rows.placement(&record, true, Detail::Minimal);
        let mut legacy = legacy_row(&record);
        for key in ["has_error", "source", "events_truncated", "offline_writes"] {
            legacy[key] = minimal[key].clone();
        }
        assert_eq!(minimal, legacy);
        assert_eq!(minimal["replicas"][0].as_object().unwrap().len(), 3);
        assert_eq!(minimal["offline_writes"]["scopes"], 0);
    }

    #[test]
    fn offline_write_totals_are_reused_for_fifteen_seconds_per_state_directory() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let other = tempfile::tempdir()?;
        let diagnostics = Diagnostics::default();
        let queue = crate::outbox::test_support::queue(temp.path(), "api", &"a".repeat(64))?;
        let pending = |root: &Path, at: Instant| {
            diagnostics
                .offline_writes_at(root, "api", at)
                .map(|summary| summary["pending_count"].clone())
        };
        let start = Instant::now();
        assert_eq!(pending(temp.path(), start), Some(json!(0)));
        queue.enqueue("private-table-name", json!({"row":1}), None, 100)?;
        let almost = start + FACT_CACHE - Duration::from_millis(1);
        assert_eq!(pending(temp.path(), almost), Some(json!(0)));
        assert_eq!(pending(temp.path(), start + FACT_CACHE), Some(json!(1)));
        assert_eq!(pending(other.path(), start + FACT_CACHE), Some(json!(0)));
        let row = Rows::snapshot(&diagnostics, temp.path()).placement(
            &failing_record(),
            false,
            Detail::Full,
        );
        assert_eq!(row["offline_writes"]["scopes"], 1);
        assert!(!row.to_string().contains("private-table-name"));

        std::fs::write(
            temp.path()
                .join("placement-data/api/current/store/.standalone-outbox/api/stray"),
            b"",
        )?;
        let later = start + FACT_CACHE * 2;
        assert_eq!(
            diagnostics.offline_writes_at(temp.path(), "api", later),
            None
        );
        let unreadable = Rows::new(&diagnostics, temp.path(), false).placement(
            &failing_record(),
            true,
            Detail::Full,
        );
        assert!(unreadable.get("offline_writes").is_none());
        assert_eq!(diagnostics.lock().offline_writes.len(), 1);
        Ok(())
    }

    #[test]
    fn network_lists_reachable_addresses_first_and_stays_bounded() {
        let parse = |address: &str| address.parse::<IpAddr>().unwrap();
        let network = network_json(vec![
            ("lo".into(), vec![parse("::1"), parse("127.0.0.1")], 900),
            ("down".into(), vec![], 0),
            (
                "eth0".into(),
                vec![
                    parse("fe80::1"),
                    parse("2001:db8::7"),
                    parse("169.254.10.1"),
                    parse("192.168.1.20"),
                    parse("192.168.1.20"),
                    parse("10.0.0.5"),
                ],
                500,
            ),
            ("docker0".into(), vec![parse("172.17.0.1")], 0),
        ]);
        assert_eq!(
            network,
            json!({"interfaces":[
                {"name":"docker0","addresses":["172.17.0.1"],"loopback":false},
                {"name":"eth0","addresses":["10.0.0.5","192.168.1.20","2001:db8::7","169.254.10.1"],"loopback":false},
                {"name":"lo","addresses":["127.0.0.1","::1"],"loopback":true},
            ]})
        );
        let mut crowded: Vec<Interface> = (0..20u64)
            .map(|index| (format!("br-{index:02}"), vec![parse("172.18.0.1")], index))
            .collect();
        crowded.push(("eth0".into(), vec![parse("192.168.1.20")], 10_000));
        let busiest = network_json(crowded);
        let names: Vec<_> = busiest["interfaces"]
            .as_array()
            .unwrap()
            .iter()
            .map(|interface| interface["name"].as_str().unwrap())
            .collect();
        assert_eq!(names.len(), MAX_INTERFACES);
        assert_eq!((names[0], names[15]), ("br-05", "eth0"));
        let worst = worst_case_network();
        let interfaces = worst["interfaces"].as_array().unwrap();
        assert_eq!(interfaces.len(), MAX_INTERFACES);
        for interface in interfaces {
            assert_eq!(
                interface["addresses"].as_array().unwrap().len(),
                MAX_INTERFACE_ADDRESSES
            );
            assert_eq!(
                interface["name"].as_str().unwrap().len(),
                MAX_INTERFACE_NAME
            );
        }
        assert!(worst.to_string().len() < 4096);
        assert_eq!(truncated("häuser", 2), "h");
        let diagnostics = Diagnostics::default();
        assert!(diagnostics.network()["interfaces"].is_array());
        diagnostics.insert_network(worst.clone());
        assert_eq!(diagnostics.network(), worst);
        diagnostics.forget_network();
        assert_ne!(diagnostics.network(), worst);
    }

    #[test]
    fn restarts_report_no_countdown_after_the_retry_time() {
        let diagnostics = Diagnostics::default();
        let mut due = restarts(0);
        due.retry_at = Some(Instant::now() - Duration::from_secs(5));
        diagnostics.insert_replica("api", 0, due);
        let row = Rows::new(&diagnostics, Path::new("/state"), false).placement(
            &failing_record(),
            false,
            Detail::Full,
        );
        assert_eq!(row["replicas"][0]["restarts"]["retry_in_seconds"], 0);
    }

    #[test]
    fn features_and_identity_summary_match_the_shared_fingerprint_vector() -> Result<()> {
        use crate::event_kind::{
            API_EVENTS, DISCORD_BOTS, ON_DEMAND_EVENTS, SCHEDULED_ONCE, TELEGRAM_BOTS,
        };
        let mut expected = json!({"placement_diagnostics":1,"task_health":1,"placement_events":1,"offline_summary":1,"host_operation":1,"network_interfaces":1,"rollout_history":1,"operations":1,"metrics_history":1,"offline_lookup":1,"reader_bindings":1,"acme_failure_detail":1,"archive_status":1,"artifact_capacity":1,"scheduled_events":1});
        let hosts_models = cfg!(feature = "runtime");
        // A flag of an event part follows the build: it is there exactly when the part is.
        // The model host is built with the runtime; MLX runs on Apple-silicon Macs only.
        let serves_mlx = hosts_models && cfg!(all(target_os = "macos", target_arch = "aarch64"));
        for (flag, built) in [
            ("api_events", API_EVENTS),
            ("scheduled_once", SCHEDULED_ONCE),
            ("on_demand_events", ON_DEMAND_EVENTS),
            ("telegram_bots", TELEGRAM_BOTS),
            ("discord_bots", DISCORD_BOTS),
            ("model_store", hosts_models),
            ("model_host", hosts_models),
            ("model_runtime_llamacpp", hosts_models),
            ("model_runtime_onnx", hosts_models),
            ("model_runtime_manifest", hosts_models),
            ("model_runtime_updates", hosts_models),
            ("model_runtime_mlx", serves_mlx),
        ] {
            if built {
                expected[flag] = json!(1);
            }
        }
        assert_eq!(features_with_models(true), expected);
        let mut without_models = expected.clone();
        without_models
            .as_object_mut()
            .unwrap()
            .retain(|flag, _| !flag.starts_with("model_"));
        assert_eq!(features_with_models(false), without_models);
        let root = tempfile::tempdir()?;
        let diagnostics = Diagnostics::default();
        assert!(diagnostics.device_facts(root.path(), true)["features"].is_object());
        assert!(
            !diagnostics
                .device_facts(root.path(), false)
                .contains_key("features")
        );
        let vector: Value = serde_json::from_str(include_str!(
            "../../../packages/device-protocol/fixtures/identity-fingerprint.json"
        ))?;
        let identity: DeviceIdentity = serde_json::from_value(vector["identity"].clone())?;
        let summary = identity_summary(&identity)?;
        assert_eq!(summary["fingerprint"], vector["fingerprint"]);
        assert_eq!(summary["auth_key"], identity.auth_key.thumbprint()?);
        assert_eq!(
            summary["telemetry_key"],
            identity.telemetry_key.thumbprint()?
        );
        Ok(())
    }

    /// The state file of the placement `api` at the revisions of `running_record`.
    fn schedule_state(events: Value) -> Vec<u8> {
        serde_json::to_vec(&json!({"version":1,"config_revision":1,"intent_revision":1,"decided":true,"grant_id":"g-1","events":events})).unwrap()
    }

    fn schedule_entry() -> Value {
        json!({"expression":"0 0 2 * * *","timezone":"Europe/Berlin","armed_at":1790000000,
            "watermark":1790035200,"next_at":1790121600,"hold":null,"confirmed":true,"since":1789990000,
            "running_since":null,"last":{"at":1790035200,"finished_at":1790035212,"outcome":"succeeded"},
            "runs":12,"failed":1,"skipped_overlap":0,"skipped_missed":2,"skipped_busy":0,
            "last_skip":{"at":1789948800,"reason":"missed"}})
    }

    /// The row a reader gets for `record` when nothing is cached yet.
    fn schedule_row(
        root: &Path,
        record: &PlacementRecord,
        snapshot: bool,
        detail: Detail,
    ) -> Value {
        Rows::new(&Diagnostics::default(), root, snapshot).placement(record, false, detail)
    }

    /// How many bytes the schedules, bots and actions of `row` add to it.
    fn trigger_bytes(row: &Value) -> usize {
        let mut without = row.clone();
        for (key, flag) in TRIGGER_LISTS {
            without.as_object_mut().unwrap().remove(key);
            without.as_object_mut().unwrap().remove(flag);
        }
        row.to_string().len() - without.to_string().len()
    }

    #[test]
    fn schedule_rows_are_live_facts_and_snapshots_keep_only_what_is_stable() {
        let root = tempfile::tempdir().unwrap();
        let record = running_record(&["evt_report", "http"]);
        let state = schedule_state(json!({"evt_report": schedule_entry()}));
        write_schedule_state(root.path(), "api", &state);
        let live = schedule_row(root.path(), &record, false, Detail::Full);
        assert_eq!(
            live["schedules"],
            json!([{"event_id":"evt_report","expression":"0 0 2 * * *","timezone":"Europe/Berlin",
                "hold":null,"next_at":1790121600,"running":false,"last_at":1790035200,"last_outcome":"succeeded",
                "runs":12,"failed":1,"skipped":2,"last_skip":{"at":1789948800,"reason":"missed"},"clock_behind":false}])
        );
        assert_eq!(live["schedules_truncated"], false);
        let snapshot = schedule_row(root.path(), &record, true, Detail::Full);
        assert_eq!(
            snapshot["schedules"],
            json!([{"event_id":"evt_report","expression":"0 0 2 * * *","timezone":"Europe/Berlin","hold":null,"last_outcome":"succeeded"}])
        );
        assert_eq!(snapshot["schedules_truncated"], false);

        // A held schedule that is running its last allowed run, on a clock that was set back.
        let mut held = schedule_entry();
        held["hold"] = json!("not_released");
        held["next_at"] = Value::Null;
        held["running_since"] = json!(1790035200);
        held["watermark"] = json!(now() + 3600);
        held["last"] = Value::Null;
        held["last_skip"] = Value::Null;
        write_schedule_state(
            root.path(),
            "api",
            &schedule_state(json!({"evt_report": held})),
        );
        let live = schedule_row(root.path(), &record, false, Detail::Full);
        assert_eq!(
            live["schedules"][0],
            json!({"event_id":"evt_report","expression":"0 0 2 * * *","timezone":"Europe/Berlin",
                "hold":"not_released","next_at":null,"running":true,"last_at":null,"last_outcome":null,
                "runs":12,"failed":1,"skipped":2,"last_skip":null,"clock_behind":true})
        );
    }

    #[test]
    fn schedule_state_is_untrusted_and_only_the_running_process_is_reported() {
        let root = tempfile::tempdir().unwrap();
        let record = running_record(&["evt_report", "evt_mail"]);
        let reported = |state: &[u8], record: &PlacementRecord| {
            write_schedule_state(root.path(), "api", state);
            let row = schedule_row(root.path(), record, false, Detail::Full);
            row.get("schedules").map(|schedules| {
                schedules
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|schedule| schedule["event_id"].as_str().unwrap().to_owned())
                    .collect::<Vec<_>>()
            })
        };
        let good = json!({"evt_report": schedule_entry(), "evt_mail": schedule_entry()});
        assert_eq!(
            reported(&schedule_state(good.clone()), &record),
            Some(vec!["evt_mail".to_owned(), "evt_report".to_owned()])
        );

        // One entry that is out of bounds is dropped; the others stay.
        let changes: [(&str, Value); 12] = [
            ("expression", json!("0 0 2 * * * ; reboot")),
            ("expression", json!("0 ".repeat(65))),
            ("expression", json!("")),
            ("timezone", json!("Europe/Ber lin")),
            ("timezone", json!(null)),
            ("hold", json!("a_newer_reason")),
            ("last", json!({"at": 1790035200, "outcome": "exploded"})),
            ("last_skip", json!({"at": 1789948800, "reason": "bored"})),
            ("last_skip", json!({"reason": "missed"})),
            ("next_at", json!(-1)),
            ("watermark", json!(i64::MAX)),
            ("runs", json!("many")),
        ];
        for (key, value) in changes {
            let mut events = good.clone();
            events["evt_mail"][key] = value.clone();
            assert_eq!(
                reported(&schedule_state(events), &record),
                Some(vec!["evt_report".to_owned()]),
                "{key} = {value}"
            );
        }
        // An event that is not part of the placement.
        let mut foreign = good.clone();
        foreign["evt_other"] = schedule_entry();
        assert_eq!(
            reported(&schedule_state(foreign), &record).map(|events| events.len()),
            Some(2)
        );

        // A file that is not usable as a whole.
        let mut padded = good.clone();
        padded["evt_other"] = json!({"padding": "x".repeat(MAX_STATE_FILE_BYTES as usize)});
        assert_eq!(reported(&schedule_state(padded), &record), None);
        assert_eq!(reported(b"{ torn", &record), None);
        let state = |change: fn(&mut Value)| {
            let mut state: Value = serde_json::from_slice(&schedule_state(good.clone())).unwrap();
            change(&mut state);
            serde_json::to_vec(&state).unwrap()
        };
        // Written by another process than the one that runs: stale numbers are never shown.
        for stale in [
            state(|state| state["version"] = json!(2)),
            state(|state| state["intent_revision"] = json!(2)),
            state(|state| state["config_revision"] = json!(2)),
            state(|state| state["decided"] = json!(false)),
            state(|state| state["events"] = json!([])),
        ] {
            assert_eq!(reported(&stale, &record), None);
        }
        let mut updated = record.clone();
        updated.intent_revision = 2;
        assert_eq!(reported(&schedule_state(good.clone()), &updated), None);
        for state in [
            ObservedState::Starting,
            ObservedState::Backoff,
            ObservedState::Stopped,
            ObservedState::Failed,
        ] {
            let mut idle = record.clone();
            idle.replicas[0].observed_state = state;
            assert_eq!(
                reported(&schedule_state(good.clone()), &idle),
                None,
                "{state:?}"
            );
        }
        let mut stopped = record.clone();
        stopped.replicas.clear();
        assert_eq!(reported(&schedule_state(good.clone()), &stopped), None);

        // Neither the directory nor the file is followed through a link.
        #[cfg(unix)]
        {
            let directory = schedule_state_dir(root.path(), "api");
            let elsewhere = tempfile::tempdir().unwrap();
            let moved = elsewhere.path().join(STATE_FILE);
            std::fs::rename(directory.join(STATE_FILE), &moved).unwrap();
            std::os::unix::fs::symlink(&moved, directory.join(STATE_FILE)).unwrap();
            let row = schedule_row(root.path(), &record, false, Detail::Full);
            assert!(row.get("schedules").is_none());
            std::fs::remove_dir_all(&directory).unwrap();
            std::os::unix::fs::symlink(elsewhere.path(), &directory).unwrap();
            let row = schedule_row(root.path(), &record, false, Detail::Full);
            assert!(row.get("schedules").is_none());
        }
    }

    #[test]
    fn schedule_rows_are_bounded_and_shed_with_the_events() {
        let root = tempfile::tempdir().unwrap();
        let ids: Vec<String> = (0..20).map(|index| format!("evt-{index:02}")).collect();
        let record = running_record(&ids.iter().map(String::as_str).collect::<Vec<_>>());
        let events: Map<String, Value> = ids
            .iter()
            .map(|id| (id.clone(), schedule_entry()))
            .collect();
        write_schedule_state(root.path(), "api", &schedule_state(Value::Object(events)));
        for snapshot in [false, true] {
            let full = schedule_row(root.path(), &record, snapshot, Detail::Full);
            let listed = full["schedules"].as_array().unwrap();
            // Sixteen snapshot entries fit the row's budget; sixteen live entries do not.
            assert_eq!(listed.len() == MAX_SCHEDULES, snapshot, "{}", listed.len());
            assert!(listed.len() >= 12, "{}", listed.len());
            for (index, schedule) in listed.iter().enumerate() {
                assert_eq!(schedule["event_id"], json!(format!("evt-{index:02}")));
            }
            assert_eq!(full["schedules_truncated"], true);
            assert!(trigger_bytes(&full) <= TRIGGER_FACTS_BUDGET);
            for detail in [Detail::NoEvents, Detail::Minimal] {
                let shed = schedule_row(root.path(), &record, snapshot, detail);
                assert!(shed.get("schedules").is_none());
                assert_eq!(shed["schedules_truncated"], true);
            }
        }
        // A placement without schedule state has neither key, at any detail.
        let none = tempfile::tempdir().unwrap();
        for detail in DETAILS {
            let row = schedule_row(none.path(), &record, false, detail);
            assert!(row.get("schedules").is_none() && row.get("schedules_truncated").is_none());
        }
    }

    #[test]
    fn schedule_state_is_read_again_for_another_process_or_after_fifteen_seconds() {
        let root = tempfile::tempdir().unwrap();
        let diagnostics = Diagnostics::default();
        let mut record = running_record(&["evt_report"]);
        let runs = |record: &PlacementRecord, at: Instant| {
            diagnostics
                .schedules_at(root.path(), record, at)
                .map(|file| file.entries[0].runs)
        };
        let write = |runs: u64| {
            let mut entry = schedule_entry();
            entry["runs"] = json!(runs);
            write_schedule_state(
                root.path(),
                "api",
                &schedule_state(json!({"evt_report": entry})),
            );
        };
        let start = Instant::now();
        write(1);
        assert_eq!(runs(&record, start), Some(1));
        write(2);
        assert_eq!(
            runs(&record, start + FACT_CACHE - Duration::from_millis(1)),
            Some(1)
        );
        assert_eq!(runs(&record, start + FACT_CACHE), Some(2));
        // A restarted process is never shown with the numbers read for the one before it.
        write(3);
        record.replicas[0].process_id = Some(4212);
        assert_eq!(runs(&record, start + FACT_CACHE), Some(3));
        assert_eq!(diagnostics.lock().schedules.len(), 1);
    }

    #[cfg(unix)]
    #[test]
    fn placement_files_are_opened_without_following_links_or_blocking() -> Result<()> {
        use std::os::unix::{ffi::OsStrExt, fs::symlink};
        const DIRECTORY: &str = ".standalone-run";
        let root = tempfile::tempdir()?;
        let elsewhere = tempfile::tempdir()?;
        let read = |file: &str| read_placement_file(root.path(), "api", DIRECTORY, file, 16);
        assert_eq!(read("state.0.json")?, None);
        write_placement_file(root.path(), "api", DIRECTORY, "state.0.json", &[b'x'; 16]);
        assert_eq!(read("state.0.json")?, Some(vec![b'x'; 16]));
        assert_eq!(read("state.1.json")?, None);
        write_placement_file(root.path(), "api", DIRECTORY, "state.1.json", &[b'x'; 17]);
        assert!(read("state.1.json").is_err());

        let directory = placement_file_dir(root.path(), "api", DIRECTORY);
        std::fs::write(elsewhere.path().join("state.json"), b"{}")?;
        symlink(
            elsewhere.path().join("state.json"),
            directory.join("state.2.json"),
        )?;
        assert!(read("state.2.json").is_err());
        // Nothing ever writes into this FIFO; a read that waited for a writer would hang.
        let fifo = std::ffi::CString::new(directory.join("state.3.json").as_os_str().as_bytes())?;
        assert_eq!(unsafe { libc::mkfifo(fifo.as_ptr(), 0o600) }, 0);
        assert!(read("state.3.json").is_err());
        std::fs::create_dir(directory.join("state.4.json"))?;
        assert!(read("state.4.json").is_err());

        let parent = directory.parent().unwrap().to_path_buf();
        for linked in [directory.clone(), parent] {
            let moved = elsewhere.path().join("moved");
            std::fs::rename(&linked, &moved)?;
            symlink(&moved, &linked)?;
            assert!(read("state.0.json").is_err(), "{}", linked.display());
            std::fs::remove_file(&linked)?;
            std::fs::rename(&moved, &linked)?;
            assert_eq!(read("state.0.json")?, Some(vec![b'x'; 16]));
        }
        for name in ["", ".", "..", "a/b"] {
            assert!(read_placement_file(root.path(), name, DIRECTORY, "state.0.json", 16).is_err());
            assert!(read_placement_file(root.path(), "api", name, "state.0.json", 16).is_err());
            assert!(read_placement_file(root.path(), "api", DIRECTORY, name, 16).is_err());
        }
        Ok(())
    }

    /// The revisions of the design's state files.
    const DESIGN: (u64, u64) = (12, 7);
    /// 2026-10-02T12:00:00Z, the day of the design's bot file.
    const DESIGN_DAY: i64 = 1_790_942_400;

    /// A placement whose one replica runs the design's revisions, with these events pinned
    /// at these versions.
    fn record_with(events: &[(&str, [u32; 3])]) -> PlacementRecord {
        let mut record = running_record(&[]);
        record.config["events"] = events
            .iter()
            .map(|(id, version)| json!({"event_id":id,"event_version":version,"board_version":[3,0,1]}))
            .collect();
        (record.config_revision, record.intent_revision) = DESIGN;
        (
            record.replicas[0].config_revision,
            record.replicas[0].intent_revision,
        ) = DESIGN;
        record
    }

    fn design_record() -> PlacementRecord {
        record_with(&[
            ("evt_once", [0, 0, 4]),
            ("evt_helper", [0, 0, 1]),
            ("evt_notes_form", [1, 0, 0]),
        ])
    }

    /// The row a reader gets on the design's day when nothing is cached yet.
    fn design_row(root: &Path, record: &PlacementRecord, snapshot: bool) -> Value {
        Rows::new(&Diagnostics::default(), root, snapshot)
            .at(DESIGN_DAY)
            .placement(record, false, Detail::Full)
    }

    /// `state` with its document changed.
    fn changed(state: &[u8], change: impl Fn(&mut Value)) -> Vec<u8> {
        let mut state: Value = serde_json::from_slice(state).unwrap();
        change(&mut state);
        serde_json::to_vec(&state).unwrap()
    }

    fn ids(list: Option<&Value>) -> Option<Vec<String>> {
        list.map(|list| {
            list.as_array()
                .unwrap()
                .iter()
                .map(|entry| entry["event_id"].as_str().unwrap().to_owned())
                .collect()
        })
    }

    #[test]
    fn the_state_files_of_the_design_become_its_row_facts() {
        let root = tempfile::tempdir().unwrap();
        let record = design_record();
        let once = json!({"evt_once": design_once_entry()});
        write_schedule_state(root.path(), "api", &design_schedules(once, DESIGN));
        let bots = json!({"evt_helper": design_bot_entry()});
        write_bot_state(root.path(), "api", &design_bots(bots, DESIGN));
        let runs = json!({"evt_notes_form": design_action_entry()});
        write_run_state(root.path(), "api", 0, &design_runs(runs, DESIGN));

        let live = design_row(root.path(), &record, false);
        assert_eq!(
            live["schedules"],
            json!([{"event_id":"evt_once","once_at":1790233200,"timezone":"Europe/Berlin","once_state":"pending",
                "hold":null,"next_at":1790233200,"running":false,"last_at":null,"last_outcome":null}])
        );
        assert_eq!(
            live["bots"],
            json!([{"event_id":"evt_helper","provider":"telegram","state":"connected","hold":null,"bot_name":"helper_bot",
                "connected_at":1790000003,"last_message_at":1790003600,"last_outcome":"succeeded",
                "running":1,"runs":57,"runs_today":12,"failed":2,"dropped":3}])
        );
        assert_eq!(
            live["actions"],
            json!([{"event_id":"evt_notes_form","kind":"form","fields":1,"file_fields":0,"running":0,
                "last_at":1790035200,"last_outcome":"succeeded","runs":12,"failed":1}])
        );
        for (_, flag) in TRIGGER_LISTS {
            assert_eq!(live[flag], false, "{flag}");
        }
        // The bot's own ID and its read position stay on the device.
        for private in ["7123456789", "412345678"] {
            assert!(!live.to_string().contains(private), "{private}");
        }

        let snapshot = design_row(root.path(), &record, true);
        assert_eq!(
            snapshot["schedules"],
            json!([{"event_id":"evt_once","once_at":1790233200,"timezone":"Europe/Berlin","once_state":"pending","hold":null,"last_outcome":null}])
        );
        assert_eq!(
            snapshot["bots"],
            json!([{"event_id":"evt_helper","provider":"telegram","hold":null,"state":"ok"}])
        );
        assert_eq!(
            snapshot["actions"],
            json!([{"event_id":"evt_notes_form","kind":"form","fields":1,"file_fields":0}])
        );
    }

    #[test]
    fn one_time_schedules_say_when_they_run_and_stay_reported_once_they_are_over() {
        let root = tempfile::tempdir().unwrap();
        let record = record_with(&[("evt_once", [0, 0, 4]), ("evt_report", [1, 0, 0])]);
        let entry = |change: &dyn Fn(&mut Value)| {
            let mut entry = design_once_entry();
            change(&mut entry);
            entry
        };
        let schedules = |events: Value, record: &PlacementRecord, snapshot: bool| {
            write_schedule_state(root.path(), "api", &design_schedules(events, DESIGN));
            design_row(root.path(), record, snapshot)
                .get("schedules")
                .cloned()
        };
        let live =
            |once: Value| schedules(json!({"evt_once": once}), &record, false).unwrap()[0].clone();
        // Held, or not armed by this service: its instant is not the next run.
        let held = live(entry(&|entry| entry["hold"] = json!("runs_elsewhere")));
        assert_eq!(
            (&held["hold"], &held["next_at"]),
            (&json!("runs_elsewhere"), &Value::Null)
        );
        let unarmed = live(entry(&|entry| entry["once_armed_at"] = Value::Null));
        assert_eq!(unarmed["next_at"], Value::Null);
        let started = live(entry(&|entry| {
            entry["once_state"] = json!("started");
            entry["running_since"] = json!(1790233200);
        }));
        assert_eq!(
            (&started["running"], &started["next_at"]),
            (&json!(true), &Value::Null)
        );
        // The keys a repeating entry leaves empty do not make it one.
        let written = live(entry(&|entry| {
            entry["expression"] = json!("");
            for key in ["next_at", "watermark", "armed_at", "last_skip"] {
                entry[key] = Value::Null;
            }
            for key in ["skipped_overlap", "skipped_missed", "skipped_busy"] {
                entry[key] = json!(0);
            }
        }));
        assert_eq!(written["once_at"], 1790233200);

        // A value out of bounds drops the entry, not the file.
        let changes: [(&str, Value); 9] = [
            ("once_at", json!(946_684_799)),
            ("once_at", json!(4_102_444_801i64)),
            ("once_at", json!("2026-09-24T07:00:00Z")),
            ("once_state", json!("exploded")),
            ("once_state", Value::Null),
            ("once_armed_at", json!(-1)),
            ("event_version", json!([0, 4])),
            ("event_version", json!("0.0.4")),
            ("timezone", json!("Europe/Ber lin")),
        ];
        for (key, value) in changes {
            let once = entry(&|entry| entry[key] = value.clone());
            let events = json!({"evt_once": once, "evt_report": schedule_entry()});
            assert_eq!(
                ids(schedules(events, &record, false).as_ref()),
                Some(vec!["evt_report".to_owned()]),
                "{key} = {value}"
            );
        }

        // Over: reported whatever runs, as long as the config pins the version it was for.
        let ran = entry(&|entry| {
            entry["once_state"] = json!("ran");
            entry["last"] = json!({"at":1790233200,"finished_at":1790233204,"outcome":"succeeded"});
        });
        let events = json!({"evt_once": ran, "evt_report": schedule_entry()});
        assert_eq!(
            ids(schedules(events.clone(), &record, false).as_ref()),
            Some(vec!["evt_once".to_owned(), "evt_report".to_owned()])
        );
        let mut stopped = record.clone();
        stopped.replicas[0].observed_state = ObservedState::Stopped;
        let finished = json!([{"event_id":"evt_once","once_at":1790233200,"timezone":"Europe/Berlin","once_state":"ran",
            "hold":null,"next_at":null,"running":false,"last_at":1790233200,"last_outcome":"succeeded"}]);
        assert_eq!(
            schedules(events.clone(), &stopped, false),
            Some(finished.clone())
        );
        assert_eq!(
            schedules(events.clone(), &stopped, true),
            Some(
                json!([{"event_id":"evt_once","once_at":1790233200,"timezone":"Europe/Berlin","once_state":"ran","hold":null,"last_outcome":"succeeded"}])
            )
        );
        let file = design_schedules(events.clone(), DESIGN);
        for other in [
            changed(&file, |state| state["decided"] = json!(false)),
            changed(&file, |state| state["intent_revision"] = json!(6)),
            changed(&file, |state| state["config_revision"] = json!(11)),
        ] {
            write_schedule_state(root.path(), "api", &other);
            let row = design_row(root.path(), &record, false);
            assert_eq!(row["schedules"], finished);
        }
        for state in ["missed", "passed"] {
            let over = entry(&|entry| entry["once_state"] = json!(state));
            let reported = schedules(json!({"evt_once": over}), &stopped, false).unwrap();
            assert_eq!(reported[0]["once_state"], state);
        }
        // Not after the event moved to another version, and never what is still to come.
        let mut moved = stopped.clone();
        moved.config["events"][0]["event_version"] = json!([0, 0, 5]);
        assert_eq!(schedules(events, &moved, false), None);
        for state in ["pending", "started"] {
            let open = entry(&|entry| entry["once_state"] = json!(state));
            let events = json!({"evt_once": open, "evt_report": schedule_entry()});
            assert_eq!(schedules(events, &stopped, false), None, "{state}");
        }
        let unversioned = entry(&|entry| {
            entry["once_state"] = json!("ran");
            entry["event_version"] = Value::Null;
        });
        assert_eq!(
            schedules(json!({"evt_once": unversioned}), &stopped, false),
            None
        );
    }

    #[test]
    fn bot_state_is_untrusted_and_only_the_running_process_is_reported() {
        let root = tempfile::tempdir().unwrap();
        let record = record_with(&[("evt_helper", [0, 0, 1]), ("evt_support", [0, 0, 1])]);
        let reported = |state: &[u8], record: &PlacementRecord| {
            write_bot_state(root.path(), "api", state);
            ids(design_row(root.path(), record, false).get("bots"))
        };
        let good = json!({"evt_helper": design_bot_entry(), "evt_support": design_bot_entry()});
        let both = Some(vec!["evt_helper".to_owned(), "evt_support".to_owned()]);
        assert_eq!(reported(&design_bots(good.clone(), DESIGN), &record), both);

        let changes: [(&str, Value); 10] = [
            ("provider", json!("slack")),
            ("provider", Value::Null),
            ("state", json!("asleep")),
            ("hold", json!("a_newer_reason")),
            ("last", json!({"at": 1790003600, "outcome": "exploded"})),
            ("connected_at", json!(-5)),
            ("runs", json!("many")),
            ("dropped_flood", json!(-1)),
            ("day", json!({"date": "2026-10-2", "runs": 12})),
            ("day", json!({"date": "2026-10-02"})),
        ];
        for (key, value) in changes {
            let mut bots = good.clone();
            bots["evt_support"][key] = value.clone();
            assert_eq!(
                reported(&design_bots(bots, DESIGN), &record),
                Some(vec!["evt_helper".to_owned()]),
                "{key} = {value}"
            );
        }
        // A name that is not a plain handle is left out; the bot is still reported.
        for name in [
            json!("Hélène ✨"),
            json!(""),
            json!("x".repeat(65)),
            json!(42),
        ] {
            let mut bots = good.clone();
            bots["evt_support"]["bot_name"] = name.clone();
            write_bot_state(root.path(), "api", &design_bots(bots, DESIGN));
            let row = design_row(root.path(), &record, false);
            assert_eq!(row["bots"][1]["event_id"], "evt_support", "{name}");
            assert_eq!(row["bots"][1]["bot_name"], Value::Null, "{name}");
        }
        let mut foreign = good.clone();
        foreign["evt_other"] = design_bot_entry();
        assert_eq!(reported(&design_bots(foreign, DESIGN), &record), both);

        // Written by another process than the one that runs, or too large to be one.
        let file = design_bots(good.clone(), DESIGN);
        for stale in [
            changed(&file, |state| state["decided"] = json!(false)),
            changed(&file, |state| state["intent_revision"] = json!(6)),
            changed(&file, |state| state["config_revision"] = json!(11)),
            changed(&file, |state| state["version"] = json!(2)),
            changed(&file, |state| state["bots"] = json!([])),
            changed(&file, |state| {
                state["padding"] = json!("x".repeat(MAX_STATE_FILE_BYTES as usize))
            }),
            b"{ torn".to_vec(),
        ] {
            assert_eq!(reported(&stale, &record), None);
        }
        for observed in [
            ObservedState::Starting,
            ObservedState::Backoff,
            ObservedState::Stopped,
            ObservedState::Failed,
        ] {
            let mut idle = record.clone();
            idle.replicas[0].observed_state = observed;
            assert_eq!(reported(&file, &idle), None, "{observed:?}");
        }
    }

    #[test]
    fn bot_rows_count_runs_of_the_clock_day_and_snapshots_hide_connection_changes() {
        let root = tempfile::tempdir().unwrap();
        let record = record_with(&[("evt_helper", [0, 0, 1])]);
        let write = |change: &dyn Fn(&mut Value)| {
            let mut bot = design_bot_entry();
            change(&mut bot);
            let bots = json!({"evt_helper": bot});
            write_bot_state(root.path(), "api", &design_bots(bots, DESIGN));
        };
        write(&|_| {});
        let runs_today = |now: i64| {
            Rows::new(&Diagnostics::default(), root.path(), false)
                .at(now)
                .placement(&record, false, Detail::Full)["bots"][0]["runs_today"]
                .clone()
        };
        let midnight = 1_790_899_200;
        assert_eq!(runs_today(midnight), 12);
        assert_eq!(runs_today(midnight + 86_399), 12);
        assert_eq!(runs_today(midnight + 86_400), 0);
        assert_eq!(runs_today(midnight - 1), 0);
        write(&|bot| bot["day"] = Value::Null);
        assert_eq!(runs_today(midnight), 0);

        for (state, snapshot) in [
            ("connecting", "ok"),
            ("connected", "ok"),
            ("reconnecting", "ok"),
            ("waiting", "waiting"),
            ("token_refused", "token_refused"),
            ("intents_refused", "intents_refused"),
            ("conflict", "conflict"),
            ("webhook_set", "webhook_set"),
        ] {
            write(&|bot| bot["state"] = json!(state));
            assert_eq!(
                design_row(root.path(), &record, false)["bots"][0]["state"],
                state
            );
            assert_eq!(
                design_row(root.path(), &record, true)["bots"][0]["state"],
                snapshot
            );
        }
        write(&|bot| {
            bot["state"] = json!("waiting");
            bot["hold"] = json!("not_released");
        });
        assert_eq!(
            design_row(root.path(), &record, true)["bots"],
            json!([{"event_id":"evt_helper","provider":"telegram","hold":"not_released","state":"waiting"}])
        );
    }

    #[test]
    fn action_rows_add_up_the_instances_that_run() {
        let root = tempfile::tempdir().unwrap();
        let mut record = record_with(&[("evt_notes_form", [1, 0, 0]), ("evt_ping", [1, 0, 0])]);
        let mut second = record.replicas[0].clone();
        (second.slot, second.process_id) = (1, Some(4212));
        record.replicas.push(second);
        let ping = json!({"kind":"action","fields":0,"file_fields":0,"running":1,"runs":3,"failed":0,
            "last":{"at":1790035300,"finished_at":1790035301,"outcome":"failed","origin":"service_page"}});
        let first = design_runs(
            json!({"evt_notes_form": design_action_entry(), "evt_ping": ping}),
            DESIGN,
        );
        write_run_state(root.path(), "api", 0, &first);
        // Started before the first instance's last run and ended after it.
        let mut later = design_action_entry();
        later["running"] = json!(2);
        later["runs"] = json!(5);
        later["failed"] = json!(2);
        later["last"] = json!({"at":1790035100,"finished_at":1790035900,"outcome":"timed_out","origin":"management"});
        let second = design_runs(json!({"evt_notes_form": later}), DESIGN);
        write_run_state(root.path(), "api", 1, &second);
        // The file of an instance that does not run.
        let idle = design_runs(json!({"evt_notes_form": design_action_entry()}), DESIGN);
        write_run_state(root.path(), "api", 2, &idle);
        let notes = |row: &Value| row["actions"][0].clone();

        let live = design_row(root.path(), &record, false);
        assert_eq!(
            live["actions"],
            json!([
                {"event_id":"evt_notes_form","kind":"form","fields":1,"file_fields":0,"running":2,
                    "last_at":1790035100,"last_outcome":"timed_out","runs":17,"failed":3},
                {"event_id":"evt_ping","kind":"action","fields":0,"file_fields":0,"running":1,
                    "last_at":1790035300,"last_outcome":"failed","runs":3,"failed":0},
            ])
        );
        let alone = json!({"event_id":"evt_notes_form","kind":"form","fields":1,"file_fields":0,"running":0,
            "last_at":1790035200,"last_outcome":"succeeded","runs":12,"failed":1});
        // An instance of another revision, an unusable file, or a replica that stopped.
        for unusable in [
            changed(&second, |state| state["intent_revision"] = json!(6)),
            changed(&second, |state| {
                state["padding"] = json!("x".repeat(MAX_STATE_FILE_BYTES as usize))
            }),
            b"[]".to_vec(),
        ] {
            write_run_state(root.path(), "api", 1, &unusable);
            assert_eq!(notes(&design_row(root.path(), &record, false)), alone);
        }
        write_run_state(root.path(), "api", 1, &second);
        let mut stopping = record.clone();
        stopping.replicas[1].observed_state = ObservedState::Stopping;
        assert_eq!(notes(&design_row(root.path(), &stopping, false)), alone);

        // Entries out of bounds or of other events are dropped; the others stay.
        for (key, value) in [
            ("kind", json!("wizard")),
            ("fields", json!(-1)),
            ("running", json!(1.5)),
            ("last", json!({"at": 1790035300, "outcome": "exploded"})),
            (
                "last",
                json!({"finished_at": 1790035300, "outcome": "failed"}),
            ),
        ] {
            let mut ping = design_action_entry();
            ping[key] = value.clone();
            let events = json!({"evt_notes_form": design_action_entry(), "evt_ping": ping, "evt_other": design_action_entry()});
            write_run_state(root.path(), "api", 0, &design_runs(events, DESIGN));
            assert_eq!(
                ids(design_row(root.path(), &stopping, false).get("actions")),
                Some(vec!["evt_notes_form".to_owned()]),
                "{key} = {value}"
            );
        }
        // Nothing reports while no instance runs the current revisions, or while the one
        // that runs has not written its file yet.
        let mut stopped = stopping.clone();
        stopped.replicas[0].observed_state = ObservedState::Stopped;
        let mut updated = record.clone();
        updated.intent_revision = 8;
        let older = changed(&first, |state| state["intent_revision"] = json!(6));
        write_run_state(root.path(), "api", 0, &older);
        for record in [&stopped, &updated, &stopping] {
            let row = design_row(root.path(), record, false);
            assert!(row.get("actions").is_none() && row.get("actions_truncated").is_none());
        }
    }

    #[test]
    fn action_state_is_read_again_when_other_instances_run_or_after_fifteen_seconds() {
        let root = tempfile::tempdir().unwrap();
        let diagnostics = Diagnostics::default();
        let mut record = record_with(&[("evt_notes_form", [1, 0, 0])]);
        let runs = |record: &PlacementRecord, at: Instant| {
            diagnostics
                .actions_at(root.path(), record, at)
                .map(|actions| actions[0].runs)
        };
        let write = |slot: u8, runs: u64| {
            let mut entry = design_action_entry();
            entry["runs"] = json!(runs);
            let state = design_runs(json!({"evt_notes_form": entry}), DESIGN);
            write_run_state(root.path(), "api", slot, &state);
        };
        let start = Instant::now();
        write(0, 1);
        write(1, 10);
        assert_eq!(runs(&record, start), Some(1));
        write(0, 2);
        let almost = start + FACT_CACHE - Duration::from_millis(1);
        assert_eq!(runs(&record, almost), Some(1));
        assert_eq!(runs(&record, start + FACT_CACHE), Some(2));
        let mut second = record.replicas[0].clone();
        (second.slot, second.process_id) = (1, Some(4212));
        record.replicas.push(second);
        assert_eq!(runs(&record, start + FACT_CACHE), Some(12));
        assert_eq!(diagnostics.lock().actions.len(), 1);
    }

    #[test]
    fn the_three_lists_share_one_budget_and_give_up_actions_then_schedules_then_bots() {
        // Entries of exactly `bytes` bytes once written.
        let entries = |count: usize, bytes: usize| -> Vec<Value> {
            (0..count)
                .map(|index| json!(format!("{index:02}{}", "x".repeat(bytes - 4))))
                .collect()
        };
        let lists = |sizes: [(usize, usize); 3]| {
            sizes.map(|(count, bytes)| Some((entries(count, bytes), false)))
        };
        let lengths = |facts: &Map<String, Value>| {
            TRIGGER_LISTS.map(|(key, _)| facts.get(key).map(|list| list.as_array().unwrap().len()))
        };
        let flags = |facts: &Map<String, Value>| {
            TRIGGER_LISTS.map(|(_, flag)| facts[flag].as_bool().unwrap())
        };
        let bytes = |facts: &Map<String, Value>| Value::Object(facts.clone()).to_string().len();

        let small = trigger_facts(lists([(2, 100), (2, 100), (2, 100)]));
        assert_eq!(lengths(&small), [Some(2); 3]);
        assert_eq!(flags(&small), [false; 3]);
        let capped = trigger_facts([Some((entries(2, 100), true)), None, None]);
        assert_eq!(capped["schedules_truncated"], true);
        assert!(capped.get("bots").is_none() && capped.get("bots_truncated").is_none());

        let over = trigger_facts(lists([(16, 100), (8, 100), (16, 100)]));
        let [schedules, bots, actions] = lengths(&over).map(Option::unwrap);
        assert_eq!((schedules, bots), (16, 8));
        assert!((1..16).contains(&actions), "{actions}");
        assert_eq!(flags(&over), [false, false, true]);
        assert!(bytes(&over) + 101 > TRIGGER_FACTS_BUDGET);

        let crowded = trigger_facts(lists([(16, 250), (8, 250), (16, 250)]));
        let [schedules, bots, actions] = lengths(&crowded).map(Option::unwrap);
        assert_eq!((bots, actions), (8, 0));
        assert!((1..16).contains(&schedules), "{schedules}");
        assert_eq!(flags(&crowded), [true, false, true]);

        let bots_only = trigger_facts(lists([(1, 100), (8, 600), (1, 100)]));
        let [schedules, bots, actions] = lengths(&bots_only).map(Option::unwrap);
        assert_eq!((schedules, actions), (0, 0));
        assert!((1..8).contains(&bots), "{bots}");
        assert_eq!(flags(&bots_only), [true; 3]);

        for facts in [&small, &capped, &over, &crowded, &bots_only] {
            assert!(bytes(facts) <= TRIGGER_FACTS_BUDGET, "{}", bytes(facts));
        }
        // `{"schedules":[…],"schedules_truncated":false}` around one entry of 4,052 bytes is
        // exactly the budget; a byte more is over it.
        let exact = trigger_facts([Some((entries(1, 4052), false)), None, None]);
        assert_eq!(
            (bytes(&exact), lengths(&exact)[0]),
            (TRIGGER_FACTS_BUDGET, Some(1))
        );
        let above = trigger_facts([Some((entries(1, 4053), false)), None, None]);
        assert_eq!(
            (lengths(&above)[0], &above["schedules_truncated"]),
            (Some(0), &json!(true))
        );
    }

    #[test]
    fn a_row_with_all_three_lists_never_grows_by_more_than_the_budget() {
        let root = tempfile::tempdir().unwrap();
        let named = |kind: &str, count: usize| -> Vec<String> {
            (0..count)
                .map(|index| format!("{kind}-{index:02}-{}", "e".repeat(110)))
                .collect()
        };
        let (schedules, bots, actions) = (named("cron", 20), named("bot", 10), named("form", 20));
        let pinned: Vec<(&str, [u32; 3])> = schedules
            .iter()
            .chain(&bots)
            .chain(&actions)
            .map(|id| (id.as_str(), [0, 0, 1]))
            .collect();
        let record = record_with(&pinned);
        let all = |ids: &[String], entry: fn() -> Value| -> Value {
            Value::Object(ids.iter().map(|id| (id.clone(), entry())).collect())
        };
        let schedule_state = design_schedules(all(&schedules, schedule_entry), DESIGN);
        write_schedule_state(root.path(), "api", &schedule_state);
        write_bot_state(
            root.path(),
            "api",
            &design_bots(all(&bots, design_bot_entry), DESIGN),
        );
        let runs = design_runs(all(&actions, design_action_entry), DESIGN);
        write_run_state(root.path(), "api", 0, &runs);
        for snapshot in [false, true] {
            let row = design_row(root.path(), &record, snapshot);
            assert!(
                trigger_bytes(&row) <= TRIGGER_FACTS_BUDGET,
                "{}",
                trigger_bytes(&row)
            );
            let count = |key: &str| row[key].as_array().unwrap().len();
            let (schedules, bots, actions) = (count("schedules"), count("bots"), count("actions"));
            assert!(schedules <= MAX_SCHEDULES && bots <= MAX_BOTS && actions <= MAX_ACTIONS);
            assert_eq!(row["bots_truncated"], true);
            assert_eq!(row["schedules_truncated"], true);
            assert_eq!(row["actions_truncated"], true);
            // A list loses entries only once the lists after it in the order are empty.
            if schedules < MAX_SCHEDULES {
                assert_eq!(actions, 0, "snapshot {snapshot}");
            }
            if bots < MAX_BOTS {
                assert_eq!((schedules, actions), (0, 0), "snapshot {snapshot}");
            }
            assert!(bots > 0);
        }
    }

    #[test]
    fn the_three_lists_are_shed_with_the_events() {
        let root = tempfile::tempdir().unwrap();
        let record = design_record();
        let once = json!({"evt_once": design_once_entry()});
        write_schedule_state(root.path(), "api", &design_schedules(once, DESIGN));
        let bots = json!({"evt_helper": design_bot_entry()});
        write_bot_state(root.path(), "api", &design_bots(bots, DESIGN));
        let runs = json!({"evt_notes_form": design_action_entry()});
        write_run_state(root.path(), "api", 0, &design_runs(runs, DESIGN));
        let none = tempfile::tempdir().unwrap();
        for snapshot in [false, true] {
            for detail in DETAILS {
                let row = |root: &Path| {
                    Rows::new(&Diagnostics::default(), root, snapshot)
                        .at(DESIGN_DAY)
                        .placement(&record, false, detail)
                };
                let (reported, empty) = (row(root.path()), row(none.path()));
                for (key, flag) in TRIGGER_LISTS {
                    let shed = detail >= Detail::NoEvents;
                    assert_eq!(reported.get(key).is_some(), !shed, "{key} {detail:?}");
                    assert_eq!(reported[flag], json!(shed), "{flag} {detail:?}");
                    assert!(empty.get(key).is_none() && empty.get(flag).is_none());
                }
            }
        }
    }

    #[test]
    fn utc_dates_follow_the_calendar() {
        for (unix, date) in [
            (0, "1970-01-01"),
            (-1, "1969-12-31"),
            (951_782_400, "2000-02-29"),
            (1_790_899_200, "2026-10-02"),
            (1_790_985_599, "2026-10-02"),
            (4_102_444_800, "2100-01-01"),
            (LATEST_TIME, "9999-12-31"),
        ] {
            assert_eq!(utc_date(unix), date, "{unix}");
            assert!(is_date(date));
        }
        for text in [
            "2026-10-2",
            "2026/10/02",
            "2026-10-02T",
            "２０２６-10-02",
            "",
        ] {
            assert!(!is_date(text), "{text}");
        }
    }

    #[test]
    fn agent_release_is_null_for_development_builds() {
        let root = tempfile::tempdir().unwrap();
        let agent = agent_json(root.path());
        assert_eq!(agent["version"], env!("CARGO_PKG_VERSION"));
        assert!(agent["release_version"].is_null());
        assert!(agent["release_sequence"].is_null());
    }
}
