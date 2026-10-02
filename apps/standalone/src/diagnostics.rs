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

pub fn features() -> Value {
    Value::Object(
        FEATURES
            .iter()
            .map(|flag| ((*flag).to_owned(), json!(1)))
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

struct Inner {
    agent_started_at: i64,
    replicas: HashMap<(String, u8), ReplicaRestarts>,
    tasks: BTreeMap<String, TaskHealth>,
    /// Per state directory and placement; `None` while the queues could not be read.
    offline_writes: HashMap<(PathBuf, String), (Instant, Option<Value>)>,
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
        facts
    }
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
        }
    }

    pub(crate) fn snapshot(diagnostics: &'a Diagnostics, state_dir: &'a Path) -> Self {
        Self::new(diagnostics, state_dir, true)
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
        assert_eq!(
            features(),
            json!({"placement_diagnostics":1,"task_health":1,"placement_events":1,"offline_summary":1,"host_operation":1,"network_interfaces":1,"rollout_history":1,"operations":1,"metrics_history":1,"offline_lookup":1,"reader_bindings":1,"acme_failure_detail":1,"archive_status":1,"artifact_capacity":1})
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

    #[test]
    fn agent_release_is_null_for_development_builds() {
        let root = tempfile::tempdir().unwrap();
        let agent = agent_json(root.path());
        assert_eq!(agent["version"], env!("CARGO_PKG_VERSION"));
        assert!(agent["release_version"].is_null());
        assert!(agent["release_sequence"].is_null());
    }
}
