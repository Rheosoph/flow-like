//! Workload lifecycle and admission. Explicit Linux sandbox profiles add kernel
//! isolation; the default process profile shares the dedicated agent account.

use crate::{
    broker::{WorkloadBroker, drain_retirements},
    config::{PlacementConfig, RestartPolicy},
    enrollment::DeviceSession,
    state::{DesiredState, ObservedState, StateStore},
};
use anyhow::{Context, Result, bail};
use fs2::FileExt;
use std::{
    collections::HashMap,
    fs::{File, OpenOptions},
    path::{Path, PathBuf},
    process::Stdio,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::process::{Child, Command};
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;

pub(crate) const SERVICE_STARTUP_TIMEOUT: Duration = Duration::from_secs(60);
const SHUTDOWN_GRACE: Duration = Duration::from_secs(15);
const FORCED_REAP_GRACE: Duration = Duration::from_secs(2);
const STABLE_UPTIME: Duration = Duration::from_secs(60);

pub fn prepare_state_dir(path: &Path) -> Result<PathBuf> {
    std::fs::create_dir_all(path).context("Create agent state directory")?;
    let metadata = std::fs::symlink_metadata(path)?;
    anyhow::ensure!(
        metadata.is_dir() && !metadata.file_type().is_symlink(),
        "State directory must be a directory, not a symbolic link"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))?;
    }
    path.canonicalize().context("Resolve agent state directory")
}

pub fn lock_file(path: &Path) -> Result<File> {
    let mut options = OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
    }
    let file = options.open(path).context("Open agent lock")?;
    file.try_lock_exclusive()
        .context("Another agent or workload already holds this state lock")?;
    Ok(file)
}

pub fn agent_is_running(state_dir: &Path) -> Result<bool> {
    match lock_file(&state_dir.join("agent.lock")) {
        Ok(_lock) => Ok(false),
        Err(error)
            if error.chain().any(|cause| {
                cause
                    .downcast_ref::<std::io::Error>()
                    .is_some_and(|io| io.kind() == std::io::ErrorKind::WouldBlock)
            }) =>
        {
            Ok(true)
        }
        Err(error) => Err(error),
    }
}

struct RunningChild {
    child: Child,
    isolation: crate::isolation::IsolationLease,
    revision: u64,
    intent_revision: u64,
    started: Instant,
    stopping_since: Option<Instant>,
    startup_timed_out: bool,
    restart: RestartPolicy,
    broker_task: tokio::task::JoinHandle<()>,
    broker_cancel: CancellationToken,
    broker_drain: CancellationToken,
    instance_id: Option<String>,
}

impl Drop for RunningChild {
    fn drop(&mut self) {
        self.broker_cancel.cancel();
        self.broker_task.abort();
    }
}

struct RetryState {
    version: (u64, u64),
    failures: u32,
    next_attempt: Instant,
    configuration_failed: bool,
}

struct DataPreparation {
    version: (u64, u64),
    cancel: CancellationToken,
    task: Option<tokio::task::JoinHandle<Result<PathBuf>>>,
    result: Option<std::result::Result<PathBuf, String>>,
}

struct RolloutValidation(tokio::task::JoinHandle<bool>);

impl Drop for RolloutValidation {
    fn drop(&mut self) {
        self.0.abort();
    }
}

async fn poll_rollout_validations(
    store: &StateStore,
    jobs: &mut HashMap<String, RolloutValidation>,
    state_dir: &Path,
    session: Option<&Arc<DeviceSession>>,
) -> Result<()> {
    let pending = store.validating_rollouts()?;
    jobs.retain(|id, _| pending.iter().any(|record| &record.rollout_id == id));
    for record in pending {
        if jobs
            .get(&record.rollout_id)
            .is_some_and(|job| job.0.is_finished())
        {
            let mut job = jobs
                .remove(&record.rollout_id)
                .context("Missing rollout validation")?;
            let valid = (&mut job.0).await.unwrap_or(false);
            store.complete_rollout_validation(
                &record.rollout_id,
                valid,
                crate::enrollment::unix_time()?,
            )?;
            continue;
        }
        if jobs.contains_key(&record.rollout_id) || jobs.len() >= 2 {
            continue;
        }
        let id = record.rollout_id.clone();
        let session = session.cloned();
        let state_dir = state_dir.to_path_buf();
        let task = tokio::spawn(async move {
            #[cfg(feature = "runtime")]
            {
                tokio::time::timeout(
                    Duration::from_secs(
                        record
                            .deadline_at
                            .unwrap_or(0)
                            .saturating_sub(crate::enrollment::unix_time().unwrap_or(i64::MAX))
                            .max(0) as u64,
                    ),
                    async {
                        for config in [&record.previous_config, &record.candidate_config] {
                            if config.source == crate::config::ProjectSource::Offline {
                                crate::runtime::validate_rollout(config).await?;
                                continue;
                            }
                            let broker = Arc::new(WorkloadBroker::new_rollout_validation(
                                session
                                    .clone()
                                    .context("Online validation requires an enrolled device")?,
                                config.clone(),
                                state_dir.clone(),
                                record.rollout_id.clone(),
                            )?);
                            let result = crate::runtime::validate_rollout_authorized(
                                config,
                                Some(broker.clone()),
                                Some(broker.identity()),
                            )
                            .await;
                            broker.retire_validation().await?;
                            result?;
                        }
                        anyhow::Ok(())
                    },
                )
                .await
                .is_ok_and(|result| result.is_ok())
            }
            #[cfg(not(feature = "runtime"))]
            {
                let _ = (record, session, state_dir);
                false
            }
        });
        jobs.insert(id, RolloutValidation(task));
    }
    Ok(())
}

impl Drop for DataPreparation {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}

async fn poll_data_preparation(
    jobs: &mut HashMap<String, DataPreparation>,
    state_dir: &Path,
    config: &PlacementConfig,
    version: (u64, u64),
    cancel: &CancellationToken,
) -> Result<Option<PathBuf>> {
    if let Some(job) = jobs.get(&config.id) {
        if job.version != version {
            job.cancel.cancel();
            if job.task.as_ref().is_some_and(|task| !task.is_finished()) {
                return Ok(None);
            }
            jobs.remove(&config.id);
        }
    }
    if !jobs.contains_key(&config.id)
        && jobs
            .values()
            .filter(|job| job.task.as_ref().is_some_and(|task| !task.is_finished()))
            .count()
            >= 2
    {
        return Ok(None);
    }
    let job = jobs.entry(config.id.clone()).or_insert_with(|| {
        let root = state_dir.to_path_buf();
        let config = config.clone();
        let cancel = cancel.child_token();
        let task_cancel = cancel.clone();
        let task = tokio::task::spawn_blocking(move || {
            crate::placement_data::prepare_for_launch(
                &root,
                &config,
                version.0,
                version.1,
                &task_cancel,
            )
        });
        DataPreparation {
            version,
            cancel,
            task: Some(task),
            result: None,
        }
    });
    if job.task.as_ref().is_some_and(|task| task.is_finished()) {
        let task = job
            .task
            .take()
            .context("Missing placement data preparation")?;
        job.result = Some(match task.await {
            Ok(result) => result.map_err(|error| error.to_string()),
            Err(error) => Err(error.to_string()),
        });
    }
    match &job.result {
        Some(Ok(path)) => Ok(Some(path.clone())),
        Some(Err(error)) => anyhow::bail!("Placement data initialization failed: {error}"),
        None => Ok(None),
    }
}

impl RetryState {
    fn new(version: (u64, u64)) -> Self {
        Self {
            version,
            failures: 0,
            next_attempt: Instant::now(),
            configuration_failed: false,
        }
    }

    fn failed(&mut self, policy: &RestartPolicy) -> bool {
        self.failures = self.failures.saturating_add(1);
        self.next_attempt = Instant::now() + backoff(policy, self.failures);
        self.failures <= policy.max_restarts
    }
}

fn backoff(policy: &RestartPolicy, failures: u32) -> Duration {
    let factor = 1u64 << failures.saturating_sub(1).min(20);
    Duration::from_secs(
        policy
            .initial_backoff_secs
            .saturating_mul(factor)
            .min(policy.max_backoff_secs),
    )
}

pub async fn run(state_dir: &Path, program: &Path, cancel: CancellationToken) -> Result<()> {
    let session = DeviceSession::load(state_dir)?.map(Arc::new);
    run_with_session(state_dir, program, cancel, session).await
}

pub async fn run_with_session(
    state_dir: &Path,
    program: &Path,
    cancel: CancellationToken,
    session: Option<Arc<DeviceSession>>,
) -> Result<()> {
    run_with_session_and_ready(state_dir, program, cancel, session, || Ok(())).await
}

#[cfg(not(unix))]
pub async fn run_with_session_and_ready<F: FnOnce() -> Result<()>>(
    _state_dir: &Path,
    _program: &Path,
    _cancel: CancellationToken,
    _session: Option<Arc<DeviceSession>>,
    _ready: F,
) -> Result<()> {
    bail!("Standalone child credential channels currently require Unix");
}

#[cfg(unix)]
pub async fn run_with_session_and_ready<F: FnOnce() -> Result<()>>(
    state_dir: &Path,
    program: &Path,
    cancel: CancellationToken,
    session: Option<Arc<DeviceSession>>,
    ready: F,
) -> Result<()> {
    let _lock = lock_file(&state_dir.join("agent.lock"))?;
    let mut store = StateStore::open(&state_dir.join("management.sqlite"))?;
    store.reset_observed()?;
    store.reset_rollout_observations()?;
    store.retire_previous_instances()?;
    ready()?;
    let retire_cancel = CancellationToken::new();
    let retire_wake = Arc::new(Notify::new());
    let _retire_cancel_guard = retire_cancel.clone().drop_guard();
    let retire_task = session.clone().map(|session| {
        tokio::spawn(drain_retirements(
            state_dir.to_path_buf(),
            session,
            retire_cancel.clone(),
            retire_wake.clone(),
        ))
    });
    let mut children: HashMap<(String, u8), RunningChild> = HashMap::new();
    let mut retries: HashMap<(String, u8), RetryState> = HashMap::new();
    let mut data_preparations: HashMap<String, DataPreparation> = HashMap::new();
    let mut rollout_validations: HashMap<String, RolloutValidation> = HashMap::new();
    let mut listeners: HashMap<String, (std::net::SocketAddr, std::net::TcpListener)> =
        HashMap::new();
    let mut tick = tokio::time::interval(Duration::from_secs(1));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let result: Result<()> = async {
        loop {
            tokio::select! {_=cancel.cancelled()=>break,_=tick.tick()=>{}}
            poll_rollout_validations(
                &store,
                &mut rollout_validations,
                &state_dir,
                session.as_ref(),
            )
            .await?;
            let records = store.list_placements()?;
            data_preparations.retain(|id, _| records.iter().any(|record| &record.id == id));
            for record in records {
                if record.desired_state != DesiredState::Running {
                    if let Some(job) = data_preparations.get(&record.id) {
                        job.cancel.cancel();
                    }
                }
                let parsed = serde_json::from_value::<PlacementConfig>(record.config.clone())
                    .and_then(|c| {
                        c.validate().map_err(serde::de::Error::custom)?;
                        Ok(c)
                    });
                let version = (record.config_revision, record.intent_revision);
                // Every old worker must drain before a new revision uses the shared
                // listener and mutable project data, including during rollback.
                let draining_previous_cohort = children.iter().any(|((id, _), child)| {
                    id == &record.id && (child.revision, child.intent_revision) != version
                });
                let mut slots = record
                    .replicas
                    .iter()
                    .map(|r| r.slot)
                    .collect::<std::collections::BTreeSet<_>>();
                slots.extend(0..record.desired_replicas);
                slots.extend(
                    children
                        .keys()
                        .filter(|(id, _)| id == &record.id)
                        .map(|(_, slot)| *slot),
                );
                for slot in slots {
                    let key = (record.id.clone(), slot);
                    let retry = retries
                        .entry(key.clone())
                        .or_insert_with(|| RetryState::new(version));
                    if retry.version != version {
                        *retry = RetryState::new(version);
                    }
                    let wanted = record.desired_state == DesiredState::Running
                        && slot < record.desired_replicas;
                    if let Some(mut running) = children.remove(&key) {
                        if let Some(status) = running.child.try_wait()? {
                            running.broker_cancel.cancel();
                            running.broker_task.abort();
                            if let Some(instance) = &running.instance_id {
                                store.retire_instance(instance)?;
                                retire_wake.notify_one();
                            }
                            if wanted
                                && (running.stopping_since.is_none() || running.startup_timed_out)
                                && running.revision == record.config_revision
                                && running.intent_revision == record.intent_revision
                            {
                                if running.started.elapsed() >= STABLE_UPTIME
                                    && record.replicas.iter().any(|replica| {
                                        replica.slot == slot
                                            && replica.observed_state == ObservedState::Running
                                    })
                                {
                                    retry.failures = 0;
                                }
                                let restart = retry.failed(&running.restart);
                                store.record_replica(
                                    &record.id,
                                    slot,
                                    running.revision,
                                    running.intent_revision,
                                    if restart {
                                        ObservedState::Backoff
                                    } else {
                                        ObservedState::Failed
                                    },
                                    None,
                                    Some(&if running.startup_timed_out {
                                        "Service readiness timed out after 60 seconds".to_owned()
                                    } else { format!("Persistent service exited: {status}") }),
                                )?;
                            } else {
                                store.record_replica(
                                    &record.id,
                                    slot,
                                    running.revision,
                                    running.intent_revision,
                                    ObservedState::Stopped,
                                    None,
                                    None,
                                )?;
                            }
                        } else if !wanted
                            || running.revision != record.config_revision
                            || running.intent_revision != record.intent_revision
                            || running.stopping_since.is_some()
                        {
                            if let Some(since) = running.stopping_since {
                                if since.elapsed() >= SHUTDOWN_GRACE {
                                    if !running.isolation.signal(true)? {
                                        force_kill(&mut running.child)?;
                                    }
                                }
                            } else {
                                running.broker_drain.cancel();
                                if let Some(instance) = &running.instance_id {
                                    store.retire_instance(instance)?;
                                    retire_wake.notify_one();
                                }
                                store.record_replica(
                                    &record.id,
                                    slot,
                                    running.revision,
                                    running.intent_revision,
                                    ObservedState::Stopping,
                                    running.child.id(),
                                    None,
                                )?;
                                if !running.isolation.signal(false)? {
                                    signal_shutdown(&mut running.child)?;
                                }
                                running.stopping_since = Some(Instant::now());
                            }
                            children.insert(key, running);
                            continue;
                        } else if running.started.elapsed() >= SERVICE_STARTUP_TIMEOUT
                            && record.replicas.iter().any(|replica| {
                                replica.slot == slot
                                    && replica.observed_state == ObservedState::Starting
                            })
                        {
                            // Preparation and application readiness share the worker's
                            // deadline. A live process alone is never healthy.
                            running.broker_drain.cancel();
                            running.broker_cancel.cancel();
                            running.broker_task.abort();
                            if let Some(instance) = &running.instance_id {
                                store.retire_instance(instance)?;
                                retire_wake.notify_one();
                            }
                            if !running.isolation.signal(true)? {
                                force_kill(&mut running.child)?;
                            }
                            // Keep ownership and the PID until try_wait confirms exit.
                            // Even SIGKILL can wait on kernel I/O; other placements
                            // must continue reconciling while this child is reaped.
                            running.startup_timed_out = true;
                            running.stopping_since = Some(Instant::now());
                            store.record_replica(
                                &record.id, slot, running.revision, running.intent_revision,
                                ObservedState::Stopping, running.child.id(),
                                Some("Service readiness timed out after 60 seconds"),
                            )?;
                            children.insert(key, running);
                            continue;
                        } else {
                            children.insert(key, running);
                            continue;
                        }
                    }
                    if !wanted {
                        retries.remove(&key);
                        continue;
                    }
                    if draining_previous_cohort {
                        continue;
                    }
                    if retry.configuration_failed || Instant::now() < retry.next_attempt {
                        continue;
                    }
                    let config = match &parsed {
                        Ok(c) => c.clone(),
                        Err(_) => {
                            if store.claim_replica(
                                &record.id,
                                slot,
                                record.config_revision,
                                record.intent_revision,
                            )? {
                                store.record_replica(
                                    &record.id,
                                    slot,
                                    record.config_revision,
                                    record.intent_revision,
                                    ObservedState::Failed,
                                    None,
                                    Some("Invalid persisted placement configuration"),
                                )?;
                                retry.configuration_failed = true;
                            }
                            continue;
                        }
                    };
                    if retry.failures > config.restart.max_restarts {
                        continue;
                    }
                    #[cfg(feature = "runtime")]
                    let cached_start = if config.source == crate::config::ProjectSource::Online
                        && store.has_pending_serving_retirements(
                            &config.id,
                            crate::enrollment::unix_time()?,
                        )? {
                        session
                            .as_deref()
                            .map(|device| {
                                WorkloadBroker::can_restore_outage(device, &config, state_dir)
                            })
                            .transpose()
                            .unwrap_or_else(|_| {
                                tracing::warn!(placement = %config.id, "Cached startup snapshot could not be verified; waiting for serving leases before cloud recovery");
                                None
                            })
                            .unwrap_or(false)
                    } else {
                        false
                    };
                    #[cfg(not(feature = "runtime"))]
                    let cached_start = false;
                    if wait_for_serving_retirements(
                        &store,
                        &config,
                        slot,
                        version,
                        crate::enrollment::unix_time()?,
                        cached_start,
                    )? {
                        continue;
                    }
                    let data_root = match poll_data_preparation(
                        &mut data_preparations,
                        state_dir,
                        &config,
                        version,
                        &cancel,
                    )
                    .await
                    {
                        Ok(Some(path)) => path,
                        Ok(None) => continue,
                        Err(error) => {
                            data_preparations.remove(&record.id);
                            let restart = retry.failed(&config.restart);
                            if store.claim_replica(&record.id, slot, version.0, version.1)? {
                                store.record_replica(
                                    &record.id,
                                    slot,
                                    version.0,
                                    version.1,
                                    if restart {
                                        ObservedState::Backoff
                                    } else {
                                        ObservedState::Failed
                                    },
                                    None,
                                    Some(&error.to_string()),
                                )?;
                            }
                            continue;
                        }
                    };
                    if !store.claim_replica(
                        &record.id,
                        slot,
                        record.config_revision,
                        record.intent_revision,
                    )? {
                        continue;
                    }
                    if config.id != record.id {
                        store.record_replica(
                            &record.id,
                            slot,
                            record.config_revision,
                            record.intent_revision,
                            ObservedState::Failed,
                            None,
                            Some("Placement identity differs"),
                        )?;
                        retry.configuration_failed = true;
                        continue;
                    }
                    if let Some(hosting) = &config.hosting {
                        let address = std::net::SocketAddr::new(hosting.host, hosting.port);
                        if listeners
                            .get(&record.id)
                            .is_some_and(|(old, _)| *old != address)
                        {
                            if children.keys().any(|(id, _)| id == &record.id) {
                                continue;
                            }
                            listeners.remove(&record.id);
                        }
                        if !listeners.contains_key(&record.id) {
                            match std::net::TcpListener::bind(address).and_then(|listener| {
                                listener.set_nonblocking(true)?;
                                Ok(listener)
                            }) {
                                Ok(listener) => {
                                    listeners.insert(record.id.clone(), (address, listener));
                                }
                                Err(error) => {
                                    let restart = retry.failed(&config.restart);
                                    store.record_replica(
                                        &record.id,
                                        slot,
                                        record.config_revision,
                                        record.intent_revision,
                                        if restart {
                                            ObservedState::Backoff
                                        } else {
                                            ObservedState::Failed
                                        },
                                        None,
                                        Some(&format!("Cannot bind service listener: {error}")),
                                    )?;
                                    continue;
                                }
                            }
                        }
                    }
                    let listener = config
                        .hosting
                        .as_ref()
                        .and_then(|_| listeners.get(&record.id))
                        .map(|(_, listener)| listener);
                    match spawn_child(
                        program,
                        state_dir,
                        &record.id,
                        slot,
                        record.config_revision,
                        record.intent_revision,
                        listener,
                        &config,
                        &data_root,
                    ) {
                        Ok((mut child, channel, isolation)) => {
                            for (stream, source) in [
                                (
                                    child.stdout.take().map(|v| {
                                        Box::pin(v)
                                            as std::pin::Pin<Box<dyn tokio::io::AsyncRead + Send>>
                                    }),
                                    "stdout",
                                ),
                                (
                                    child.stderr.take().map(|v| {
                                        Box::pin(v)
                                            as std::pin::Pin<Box<dyn tokio::io::AsyncRead + Send>>
                                    }),
                                    "stderr",
                                ),
                            ] {
                                if let Some(stream) = stream {
                                    let root = state_dir.to_path_buf();
                                    let placement = record.id.clone();
                                    tokio::spawn(async move {
                                        let _ = crate::telemetry::capture(
                                            stream, root, placement, source,
                                        )
                                        .await;
                                    });
                                }
                            }
                            store.record_replica(
                                &record.id,
                                slot,
                                record.config_revision,
                                record.intent_revision,
                                ObservedState::Starting,
                                child.id(),
                                None,
                            )?;
                            let broker = if config.resource_grant.is_some() {
                                match session
                                    .clone()
                                    .context("Resource placement requires an enrolled device")
                                    .and_then(|session| {
                                        WorkloadBroker::new_replica(
                                            session,
                                            config.clone(),
                                            state_dir.to_path_buf(),
                                            record.config_revision,
                                            record.intent_revision,
                                            slot,
                                        )
                                    }) {
                                    Ok(broker) => Some(Arc::new(broker)),
                                    Err(error) => {
                                        drop(child);
                                        store.record_replica(
                                            &record.id,
                                            slot,
                                            record.config_revision,
                                            record.intent_revision,
                                            ObservedState::Failed,
                                            None,
                                            Some(&error.to_string()),
                                        )?;
                                        retry.configuration_failed = true;
                                        continue;
                                    }
                                }
                            } else {
                                None
                            };
                            let instance_id = broker.as_ref().map(|b| b.instance_id().to_string());
                            let bootstrap = crate::ipc::ChildBootstrap {
                                config: config.clone(),
                                data_root: Some(data_root),
                                replica_slot: slot,
                                inherited_listener: listener.is_some(),
                                config_revision: record.config_revision,
                                intent_revision: record.intent_revision,
                                parent_pid: std::process::id(),
                                api_base_url: session
                                    .as_ref()
                                    .map(|s| s.manifest().api_base_url.clone()),
                                workload_identity: broker.as_ref().map(|b| b.identity()),
                            };
                            let broker_cancel = CancellationToken::new();
                            let task_cancel = broker_cancel.clone();
                            let broker_drain = CancellationToken::new();
                            let task_drain = broker_drain.clone();
                            let task_state = state_dir.to_path_buf();
                            let process_id = child.id().context("Workload has no process ID")?;
                            let broker_task = tokio::spawn(async move {
                                if crate::ipc::serve_with_drain(
                                    channel,
                                    bootstrap,
                                    task_state,
                                    process_id,
                                    broker,
                                    task_cancel,
                                    task_drain,
                                )
                                .await
                                .is_err()
                                {
                                    tracing::debug!("Workload broker channel closed");
                                }
                            });
                            children.insert(
                                key,
                                RunningChild {
                                    child,
                                    isolation,
                                    revision: record.config_revision,
                                    intent_revision: record.intent_revision,
                                    started: Instant::now(),
                                    stopping_since: None,
                                    startup_timed_out: false,
                                    restart: config.restart,
                                    broker_task,
                                    broker_cancel,
                                    broker_drain,
                                    instance_id,
                                },
                            );
                        }
                        Err(error) => {
                            let restart = retry.failed(&config.restart);
                            store.record_replica(
                                &record.id,
                                slot,
                                record.config_revision,
                                record.intent_revision,
                                if restart {
                                    ObservedState::Backoff
                                } else {
                                    ObservedState::Failed
                                },
                                None,
                                Some(&error.to_string()),
                            )?;
                        }
                    }
                }
                if !children.keys().any(|(id, _)| id == &record.id) {
                    listeners.remove(&record.id);
                }
                store.aggregate_replicas(&record.id)?;
            }
            store.reconcile_rollouts(crate::enrollment::unix_time()?)?;
        }
        Ok(())
    }
    .await;
    rollout_validations.clear();
    let graceful_deadline = tokio::time::Instant::now() + SHUTDOWN_GRACE;
    let forced_deadline = graceful_deadline + FORCED_REAP_GRACE;
    // Shutdown observations are best-effort. A locked database must not add a
    // separate busy timeout for every child after the service's stop deadline.
    let _ = store.connection.busy_timeout(Duration::ZERO);
    for running in children.values_mut() {
        running.broker_drain.cancel();
        if !running.isolation.signal(false).unwrap_or(false) {
            let _ = signal_shutdown(&mut running.child);
        }
    }
    for ((id, slot), running) in &children {
        if let Some(instance) = &running.instance_id {
            let _ = store.retire_instance(instance);
            retire_wake.notify_one();
        }
        let _ = store.record_replica(
            id,
            *slot,
            running.revision,
            running.intent_revision,
            ObservedState::Stopping,
            running.child.id(),
            None,
        );
    }
    let mut indexed = children.iter_mut().collect::<Vec<_>>();
    let mut handles = indexed
        .iter_mut()
        .map(|(_, running)| &mut running.child)
        .collect::<Vec<_>>();
    let reaped = reap_children_until(&mut handles, graceful_deadline, forced_deadline).await;
    drop(handles);
    let mut unconfirmed = 0;
    for (((id, slot), running), reaped) in indexed.into_iter().zip(reaped) {
        if !reaped {
            unconfirmed += 1;
        }
        let _ = store.record_replica(
            id,
            *slot,
            running.revision,
            running.intent_revision,
            if reaped {
                ObservedState::Stopped
            } else {
                ObservedState::Stopping
            },
            if reaped { None } else { running.child.id() },
            if reaped {
                None
            } else {
                Some("Workload exit was not confirmed within the shutdown budget")
            },
        );
    }
    // Dropping each handle cancels and aborts its usage-only broker after the
    // shared reap budget, including a child whose exit could not be confirmed.
    drop(children);
    retire_cancel.cancel();
    if let Some(task) = retire_task {
        task.abort();
    }
    result.and(if unconfirmed == 0 {
        Ok(())
    } else {
        Err(anyhow::anyhow!(
            "Exit of {unconfirmed} workload processes was not confirmed within the shutdown budget"
        ))
    })
}

fn wait_for_serving_retirements(
    store: &StateStore,
    config: &PlacementConfig,
    slot: u8,
    version: (u64, u64),
    now: i64,
    authorized_cached_start: bool,
) -> Result<bool> {
    // Cached startup does not admit a replacement cloud instance. Its broker
    // keeps resource authorization unavailable until serving leases retire, and
    // the child verifies the exact sealed snapshot before reporting readiness.
    if authorized_cached_start
        || config.source != crate::config::ProjectSource::Online
        || !store.has_pending_serving_retirements(&config.id, now)?
    {
        return Ok(false);
    }
    if store.claim_replica(&config.id, slot, version.0, version.1)? {
        store.record_replica(
            &config.id,
            slot,
            version.0,
            version.1,
            ObservedState::Starting,
            None,
            Some("Waiting for previous instance resource leases to retire"),
        )?;
    }
    Ok(true)
}

async fn reap_children_until(
    children: &mut [&mut Child],
    graceful_deadline: tokio::time::Instant,
    forced_deadline: tokio::time::Instant,
) -> Vec<bool> {
    let mut reaped = vec![false; children.len()];
    for (child, reaped) in children.iter_mut().zip(&mut reaped) {
        *reaped = matches!(
            tokio::time::timeout_at(graceful_deadline, child.wait()).await,
            Ok(Ok(_))
        );
    }
    for (child, reaped) in children.iter_mut().zip(&reaped) {
        if !*reaped {
            let _ = force_kill(child);
        }
    }
    for (child, reaped) in children.iter_mut().zip(&mut reaped) {
        if !*reaped {
            *reaped = matches!(
                tokio::time::timeout_at(forced_deadline, child.wait()).await,
                Ok(Ok(_))
            );
        }
    }
    reaped
}

#[cfg(unix)]
fn spawn_child(
    program: &Path,
    state_dir: &Path,
    id: &str,
    slot: u8,
    revision: u64,
    intent: u64,
    listener: Option<&std::net::TcpListener>,
    config: &PlacementConfig,
    data_root: &Path,
) -> Result<(
    Child,
    tokio::net::UnixStream,
    crate::isolation::IsolationLease,
)> {
    let lock_path = state_dir.join(format!("placement-{id}-slot-{slot}.lock"));
    let placement_lock = if crate::isolation::sandboxed(config) {
        use std::os::unix::fs::OpenOptionsExt;
        // Writable inherited descriptors bypass both mount restrictions and
        // filesystem quotas. The lease file only needs a read-only flock.
        let created = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW)
            .open(&lock_path)?;
        drop(created);
        let file = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW)
            .open(&lock_path)?;
        file.try_lock_exclusive()?;
        file
    } else {
        lock_file(&lock_path)?
    };
    let (channel, child_socket) = crate::ipc::socket_pair()?;
    let (mut command, isolation) =
        crate::isolation::command(config, state_dir, data_root, program, slot)?;
    command
        .arg("--state-dir")
        .arg(state_dir)
        .arg("run-placement")
        .arg(id)
        .arg("--replica-slot")
        .arg(slot.to_string())
        .arg("--config-revision")
        .arg(revision.to_string())
        .arg("--intent-revision")
        .arg(intent.to_string())
        .arg("--parent-pid")
        .arg(std::process::id().to_string())
        .arg("--broker-fd")
        .arg(crate::ipc::CHILD_BROKER_FD.to_string())
        .arg("--placement-lock-fd")
        .arg(crate::ipc::CHILD_PLACEMENT_LOCK_FD.to_string());
    // Keep OS lookup/runtime settings, but never inherit the agent's cloud credentials.
    command.env_clear();
    for key in [
        "PATH",
        "HOME",
        "USERPROFILE",
        "SYSTEMROOT",
        "WINDIR",
        "TEMP",
        "TMP",
        "TMPDIR",
        "LANG",
        "LC_ALL",
        "RUST_LOG",
        "FLOW_LIKE_DEVICE_READ_CACHE_BYTES",
    ] {
        if let Some(value) = std::env::var_os(key) {
            command.env(key, value);
        }
    }
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    #[cfg(unix)]
    command.process_group(0);
    let inherited =
        crate::ipc::attach_child_descriptors(&mut command, &child_socket, &placement_lock)?;
    let _listener_descriptor = listener
        .map(|listener| crate::ipc::attach_listener(&mut command, listener))
        .transpose()?;
    isolation.attach(&mut command)?;
    let child = command
        .spawn()
        .context("Spawn standalone workload process")?;
    drop(inherited);
    drop(placement_lock);
    drop(child_socket);
    Ok((child, channel, isolation))
}

fn signal_shutdown(child: &mut Child) -> Result<()> {
    #[cfg(unix)]
    if let Some(pid) = child.id() {
        // The unreaped Child owns this PID; it is not a PID recovered from SQLite.
        let result = unsafe { libc::kill(-(pid as libc::pid_t), libc::SIGTERM) };
        if result != 0 && std::io::Error::last_os_error().raw_os_error() != Some(libc::ESRCH) {
            return Err(std::io::Error::last_os_error()).context("Signal workload shutdown");
        }
    }
    #[cfg(not(unix))]
    child.start_kill()?;
    Ok(())
}

fn force_kill(child: &mut Child) -> Result<()> {
    #[cfg(unix)]
    if let Some(pid) = child.id() {
        let result = unsafe { libc::kill(-(pid as libc::pid_t), libc::SIGKILL) };
        if result != 0 && std::io::Error::last_os_error().raw_os_error() != Some(libc::ESRCH) {
            return Err(std::io::Error::last_os_error()).context("Kill workload process group");
        }
    }
    #[cfg(not(unix))]
    child.start_kill()?;
    Ok(())
}

pub async fn shutdown_signal(cancel: CancellationToken) -> Result<()> {
    #[cfg(unix)]
    {
        let mut terminate =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
        tokio::select! { result = tokio::signal::ctrl_c() => result?, _ = terminate.recv() => {} }
    }
    #[cfg(not(unix))]
    tokio::signal::ctrl_c().await?;
    cancel.cancel();
    Ok(())
}

pub async fn watch_parent(parent_pid: u32, cancel: CancellationToken) {
    #[cfg(unix)]
    loop {
        if unsafe { libc::getppid() } as u32 != parent_pid {
            cancel.cancel();
            tokio::time::sleep(SHUTDOWN_GRACE).await;
            // Only supervised workers have their own process group.
            if unsafe { libc::getpgrp() == libc::getpid() } {
                unsafe {
                    libc::kill(-libc::getpid(), libc::SIGKILL);
                }
            }
            std::process::exit(1);
        }
        tokio::select! { _ = cancel.cancelled() => break, _ = tokio::time::sleep(Duration::from_secs(1)) => {} }
    }
    #[cfg(not(unix))]
    {
        let _ = parent_pid;
        cancel.cancel();
    }
}

pub fn require_supported_supervision() -> Result<()> {
    if !cfg!(unix) {
        bail!(
            "Native workload supervision currently requires Unix; Windows job-object supervision is not implemented yet"
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn online_rollout_waits_for_retirement_without_failing_or_blocking_offline() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let mut store = StateStore::open(&directory.path().join("management.sqlite"))?;
        let mut config: PlacementConfig = serde_json::from_value(serde_json::json!({
            "id":"service","project_id":"project","deployment_id":"deployment","revision":"one",
            "source":"online","project_path":directory.path(),
            "resource_grant":{"grant_id":"grant","authz_version":1},
            "hosting":{"host":"127.0.0.1","port":8080,"max_in_flight":4,"request_timeout_secs":30,"auth_secret":"auth"},
            "events":[{"event_id":"http","event_version":[1,0,0],"board_version":[1,0,0]}]
        }))?;
        store.upsert_placement(
            "service",
            &serde_json::to_value(&config)?,
            DesiredState::Running,
        )?;
        store.begin_instance(
            "previous-worker",
            "service",
            &serde_json::json!({"purpose":"workload"}),
            1000,
        )?;
        store.retire_instance("previous-worker")?;
        config.revision = "two".into();
        store.stage_rollout("update", &config, 1, 2, 30, 100)?;
        store.begin_rollout_validation("update", 100)?;
        store.complete_rollout_validation("update", true, 101)?;
        assert!(wait_for_serving_retirements(
            &store,
            &config,
            0,
            (2, 2),
            105,
            false
        )?);
        // The missing/mismatched/denied snapshot case stays on the normal gate.
        assert!(!wait_for_serving_retirements(
            &store,
            &config,
            0,
            (2, 2),
            105,
            true
        )?);
        assert!(store.has_pending_serving_retirements("service", 105)?);
        let placement = store.get_placement("service")?.unwrap();
        assert_eq!(placement.observed_state, ObservedState::Starting);
        assert_eq!(placement.process_id, None);
        assert_eq!(placement.ready_replicas, 0);
        store.reconcile_rollouts(105)?;
        assert_eq!(store.rollout("update")?.unwrap().state, "activating");
        let mut offline = config.clone();
        offline.source = crate::config::ProjectSource::Offline;
        assert!(!wait_for_serving_retirements(
            &store,
            &offline,
            0,
            (2, 2),
            105,
            false
        )?);
        store.reconcile_rollouts(130)?;
        assert_eq!(store.rollout("update")?.unwrap().state, "rolling_back");
        assert!(wait_for_serving_retirements(
            &store,
            &config,
            0,
            (3, 3),
            131,
            false
        )?);
        store.reconcile_rollouts(131)?;
        assert_eq!(store.rollout("update")?.unwrap().state, "rolling_back");
        store.forget_instance("previous-worker")?;
        assert!(!wait_for_serving_retirements(
            &store,
            &config,
            0,
            (3, 3),
            132,
            false
        )?);
        store.set_desired_state("service", DesiredState::Stopped)?;
        assert_eq!(store.rollout("update")?.unwrap().state, "cancelled");
        Ok(())
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn multiple_stubborn_children_share_one_shutdown_deadline() -> Result<()> {
        use tokio::io::{AsyncBufReadExt, BufReader};
        let mut children = Vec::new();
        for _ in 0..8 {
            let mut child = Command::new("/bin/sh")
                .args([
                    "-c",
                    "trap '' TERM; printf 'ready\\n'; while :; do sleep 1; done",
                ])
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .process_group(0)
                .kill_on_drop(true)
                .spawn()?;
            let mut output = BufReader::new(child.stdout.take().context("Missing fixture output")?);
            let mut line = String::new();
            tokio::time::timeout(Duration::from_secs(3), output.read_line(&mut line)).await??;
            anyhow::ensure!(
                line == "ready\n",
                "Fixture did not install its signal handler"
            );
            children.push(child);
        }
        for child in &mut children {
            signal_shutdown(child)?;
            assert!(child.try_wait()?.is_none());
        }
        let started = tokio::time::Instant::now();
        let mut handles = children.iter_mut().collect::<Vec<_>>();
        let reaped = reap_children_until(
            &mut handles,
            started + Duration::from_millis(120),
            started + Duration::from_millis(620),
        )
        .await;
        assert!(reaped.iter().all(|reaped| *reaped));
        assert!(
            started.elapsed() < Duration::from_millis(750),
            "Shutdown grace was multiplied by the number of children"
        );
        drop(handles);
        for child in &mut children {
            assert!(child.try_wait()?.is_some());
        }
        Ok(())
    }

    #[test]
    fn exclusive_lock_releases_without_deleting_lock_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("agent.lock");
        let held = lock_file(&path).unwrap();
        assert!(lock_file(&path).is_err());
        drop(held);
        // Parallel process tests can fork while the lock is held. Their copy
        // closes at exec, so release can briefly lag behind this thread's drop.
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            match lock_file(&path) {
                Ok(_reacquired) => break,
                Err(error) => {
                    assert!(
                        Instant::now() < deadline
                            && error.chain().any(|cause| cause
                                .downcast_ref::<std::io::Error>()
                                .is_some_and(|io| io.kind() == std::io::ErrorKind::WouldBlock)),
                        "Lock was not released: {error}"
                    );
                    std::thread::sleep(Duration::from_millis(5));
                }
            }
        }
        assert!(path.exists());
    }
}
