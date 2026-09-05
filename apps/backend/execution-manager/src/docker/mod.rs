//! Single-use Docker sandboxes prepared before admission and destroyed after use.
pub mod engine;
pub mod registry;

use std::{
    collections::{HashMap, VecDeque},
    env,
    path::PathBuf,
    sync::{
        Arc, Mutex, Weak,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::Duration,
};

use async_trait::async_trait;
use serde_json::{Value, json};
use tokio::{
    io::AsyncWriteExt,
    sync::{Mutex as AsyncMutex, Notify, mpsc},
    time::{Instant, timeout, timeout_at},
};

use crate::{
    Backend, CommonConfig, Dispatch, Error, EventSink, MAX_EVENT, MAX_INPUT, MAX_OUTPUT, Mode,
    Reservation, Result, config::positive,
};
use engine::{Attached, Engine, EngineError};
use registry::{Record, Registry, now};

const OWNER_LABEL: &str = "io.flow-like.execution-installation";
const RUN_LABEL: &str = "io.flow-like.execution-run";
const DEADLINE_LABEL: &str = "io.flow-like.execution-deadline";
const KIND_LABEL: &str = "io.flow-like.execution-kind";

impl From<EngineError> for Error {
    fn from(error: EngineError) -> Self {
        Error::internal(error)
    }
}

struct DockerConfig {
    image: String,
    gateway_image: String,
    network: String,
    memory_mb: u64,
    cpus: u64,
    pids: u64,
    tmp_mb: u64,
}

impl DockerConfig {
    fn from_env() -> Result<Self> {
        let required =
            |key: &str| env::var(key).map_err(|_| Error::invalid(format!("{key} is required")));
        let config = Self {
            image: required("SANDBOX_IMAGE")?,
            gateway_image: required("SANDBOX_GATEWAY_IMAGE")?,
            network: required("SANDBOX_GATEWAY_NETWORK")?,
            memory_mb: positive("SANDBOX_MEMORY_MB", 2048, 262144)?,
            cpus: positive("SANDBOX_CPUS", 1, 128)?,
            pids: positive("SANDBOX_PIDS", 256, 4096)?,
            tmp_mb: positive("SANDBOX_TMP_MB", 512, 65536)?,
        };
        if env::var("SANDBOX_RUNTIME").as_deref().unwrap_or("runsc") != "runsc" {
            return Err(Error::invalid(
                "SANDBOX_RUNTIME must be runsc; shared-kernel fallback is forbidden",
            ));
        }
        for image in [&config.image, &config.gateway_image] {
            let digest = image
                .rsplit_once('@')
                .map_or(image.as_str(), |(_, digest)| digest);
            if !digest.strip_prefix("sha256:").is_some_and(|hex| {
                hex.len() == 64
                    && hex
                        .bytes()
                        .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
            }) || image.chars().any(char::is_whitespace)
            {
                return Err(Error::invalid(
                    "Sandbox and gateway images must use an immutable sha256 digest",
                ));
            }
        }
        if config.network.is_empty()
            || !config
                .network
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || b"_.-".contains(&c))
        {
            return Err(Error::invalid("Invalid sandbox gateway network"));
        }
        Ok(config)
    }
}

struct Slot {
    record: Record,
    ready_at: Instant,
    transport: AsyncMutex<Option<(Attached, Attached)>>,
    closed: AtomicBool,
    cleanup: AsyncMutex<()>,
}

#[derive(Default)]
struct Inventory {
    available: VecDeque<Arc<Slot>>,
    active: HashMap<String, Arc<Slot>>,
    creating: usize,
    retiring: usize,
}

struct Manager {
    this: Weak<Manager>,
    common: Arc<CommonConfig>,
    config: DockerConfig,
    engine: Arc<Engine>,
    registry: Registry,
    owner: String,
    inventory: Mutex<Inventory>,
    changed: Notify,
    ready: AtomicBool,
    draining: AtomicBool,
    stopped: AtomicBool,
    created: AtomicU64,
    errors: AtomicU64,
    assignments: AtomicU64,
    assignment_nanos: AtomicU64,
}

pub async fn from_env(common: Arc<CommonConfig>) -> Result<Arc<dyn Backend>> {
    let config = DockerConfig::from_env()?;
    let host = env::var("DOCKER_HOST").unwrap_or_else(|_| "unix:///var/run/docker.sock".into());
    let path = host
        .strip_prefix("unix://")
        .filter(|path| path.starts_with('/'))
        .ok_or_else(|| Error::invalid("Docker execution requires a local Unix socket"))?;
    let engine = Engine::new(PathBuf::from(path));
    let state_path = env::var("EXECUTION_MANAGER_STATE_PATH")
        .unwrap_or_else(|_| "/state/executions.sqlite3".into());
    let registry = Registry::open(state_path, common.installation.clone())
        .await
        .map_err(Error::internal)?;
    let manager = Arc::new_cyclic(|this| Manager {
        this: this.clone(),
        common,
        config,
        engine,
        registry,
        owner: uuid::Uuid::new_v4().simple().to_string(),
        inventory: Mutex::new(Inventory::default()),
        changed: Notify::new(),
        ready: AtomicBool::new(false),
        draining: AtomicBool::new(false),
        stopped: AtomicBool::new(false),
        created: AtomicU64::new(0),
        errors: AtomicU64::new(0),
        assignments: AtomicU64::new(0),
        assignment_nanos: AtomicU64::new(0),
    });
    Ok(manager)
}

impl Manager {
    fn fail_closed(&self) {
        self.ready.store(false, Ordering::Release);
        self.draining.store(true, Ordering::Release);
        self.changed.notify_waiters();
    }

    fn labels(&self, row: &Record, deadline: f64) -> Value {
        json!({ OWNER_LABEL: self.common.installation, RUN_LABEL: row.name, DEADLINE_LABEL: (deadline as u64).to_string(), KIND_LABEL: "warm-slot" })
    }

    fn specification(&self, row: &Record, deadline: f64, lifetime: u64, runner: bool) -> Value {
        let mut host = json!({
            "ReadonlyRootfs": true, "SecurityOpt": ["no-new-privileges:true"], "CapDrop": ["ALL"],
            "RestartPolicy": {"Name": "no"}, "LogConfig": {"Type": "none", "Config": {}},
            "Mounts": [{"Type": "volume", "Source": row.volume, "Target": "/gateway", "ReadOnly": runner}],
            "NetworkMode": if runner { "none" } else { &self.config.network },
            "Memory": if runner { self.config.memory_mb * 1024 * 1024 } else { 128 * 1024 * 1024 },
            "MemorySwap": if runner { self.config.memory_mb * 1024 * 1024 } else { 128 * 1024 * 1024 },
            "NanoCpus": if runner { self.config.cpus * 1_000_000_000 } else { 500_000_000 },
            "PidsLimit": if runner { self.config.pids } else { 80 },
            "Ulimits": [{"Name":"nofile","Soft":1024,"Hard":1024},{"Name":"core","Soft":0,"Hard":0}],
        });
        let environment = if runner {
            host["Runtime"] = json!("runsc");
            host["Tmpfs"] = json!({"/tmp": format!("rw,noexec,nosuid,nodev,size={}m,uid=1000,gid=1000,mode=1700", self.config.tmp_mb)});
            vec![
                "HOME=/tmp".to_owned(),
                "TMPDIR=/tmp".into(),
                "RUST_LOG=warn".into(),
                "HTTP_PROXY=http://127.0.0.1:3128".into(),
                "HTTPS_PROXY=http://127.0.0.1:3128".into(),
                "http_proxy=http://127.0.0.1:3128".into(),
                "https_proxy=http://127.0.0.1:3128".into(),
                "NO_PROXY=".into(),
                "no_proxy=".into(),
                "SANDBOX_PROXY_SOCKET=/gateway/proxy.sock".into(),
                format!("API_URL={}/api/v1", self.common.callback_url),
                format!("BACKEND_PUB={}", self.common.backend_pub),
                "EXECUTOR_REQUIRE_DISPATCH_BINDING=true".into(),
                format!("EXECUTION_TIMEOUT_SECONDS={}", self.common.timeout),
            ]
        } else {
            // timeout remains UID 0 while the gateway drops to 65532. It needs
            // KILL to enforce the independent deadline across that UID change.
            host["CapAdd"] = json!(["CHOWN", "SETUID", "SETGID", "KILL"]);
            vec!["RUST_LOG=warn".to_owned()]
        };
        let command = if runner {
            vec![
                "--signal=KILL".to_owned(),
                format!("{lifetime}s"),
                "/app/runtime".into(),
                "--once".into(),
                "warm".into(),
            ]
        } else {
            vec![
                "--signal=KILL".to_owned(),
                format!("{lifetime}s"),
                "/app/execution-gateway".into(),
                "--unix-warm".into(),
            ]
        };
        json!({
            "Image": if runner { &self.config.image } else { &self.config.gateway_image },
            "User": if runner { "1000:1000" } else { "0:0" }, "Env": environment,
            "OpenStdin": true, "StdinOnce": true, "AttachStdin": true, "AttachStdout": true,
            "AttachStderr": false, "Tty": false, "Entrypoint": ["/usr/bin/timeout"],
            "Cmd": command, "Labels": self.labels(row, deadline), "HostConfig": host,
        })
    }

    async fn create_slot(&self) -> Result<Arc<Slot>> {
        let name = format!("flowwarm-{}", uuid::Uuid::new_v4().simple());
        let record = Record {
            gateway: format!("{name}-gateway"),
            volume: format!("{name}-socket"),
            name,
        };
        // The process deadline continues to apply if every manager disappears.
        let lifetime =
            self.common.startup_grace + self.common.warm_idle_seconds + self.common.budget();
        let deadline = now() + lifetime as f64;
        self.registry
            .add(record.clone(), self.owner.clone(), deadline)
            .await
            .map_err(Error::internal)?;
        let create = async {
            self.engine
                .volume(&record.volume, self.labels(&record, deadline))
                .await?;
            self.engine
                .create(
                    &record.gateway,
                    self.specification(&record, deadline, lifetime, false),
                )
                .await?;
            let mut gateway = self.engine.attach(&record.gateway).await?;
            let until = Instant::now() + Duration::from_secs(self.common.startup_grace);
            gateway.ready(b"ready\n", until).await?;
            self.engine
                .create(
                    &record.name,
                    self.specification(&record, deadline, lifetime, true),
                )
                .await?;
            let mut runner = self.engine.attach(&record.name).await?;
            runner.ready(b"ready\n", until).await?;
            self.registry
                .ready(record.name.clone())
                .await
                .map_err(Error::internal)?;
            Ok::<_, Error>(Arc::new(Slot {
                record: record.clone(),
                ready_at: Instant::now(),
                transport: AsyncMutex::new(Some((runner, gateway))),
                closed: AtomicBool::new(false),
                cleanup: AsyncMutex::new(()),
            }))
        };
        match timeout(Duration::from_secs(self.common.startup_grace), create).await {
            Ok(Ok(slot)) => Ok(slot),
            result => {
                self.remove_resources(&record).await?;
                match result {
                    Ok(Err(error)) => Err(error),
                    _ => Err(Error::internal("Sandbox preparation deadline expired")),
                }
            }
        }
    }

    async fn replenish(self: Arc<Self>) {
        while !self.draining.load(Ordering::Acquire) {
            let create = {
                let mut inventory = self.inventory.lock().unwrap();
                if self.ready()
                    && inventory.available.len() + inventory.creating + inventory.retiring
                        < self.common.warm_pool_size
                {
                    inventory.creating += 1;
                    true
                } else {
                    false
                }
            };
            if !create {
                tokio::select! { _ = self.changed.notified() => {}, _ = tokio::time::sleep(Duration::from_secs(1)) => {} }
                continue;
            }
            let mut slot = match self.create_slot().await {
                Ok(slot) => Some(slot),
                Err(error) => {
                    self.errors.fetch_add(1, Ordering::Relaxed);
                    tracing::warn!(error = %error, "Sandbox preparation failed");
                    None
                }
            };
            let failed = slot.is_none();
            {
                let mut inventory = self.inventory.lock().unwrap();
                if self.ready()
                    && !self.draining.load(Ordering::Acquire)
                    && let Some(slot) = slot.take()
                {
                    inventory.available.push_back(slot);
                    self.created.fetch_add(1, Ordering::Relaxed);
                }
            }
            if let Some(slot) = slot {
                let _ = self.discard(&slot).await;
            }
            self.inventory.lock().unwrap().creating -= 1;
            self.changed.notify_waiters();
            if failed {
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
        }
    }

    async fn remove_resources(&self, row: &Record) -> Result<()> {
        let remove = async {
            self.engine.remove(&row.name).await?;
            self.engine.remove(&row.gateway).await?;
            self.engine.remove_volume(&row.volume).await?;
            self.registry
                .remove(row.name.clone())
                .await
                .map_err(Error::internal)
        };
        match timeout(Duration::from_secs(self.common.cleanup_timeout), remove).await {
            Ok(Ok(())) => Ok(()),
            _ => {
                self.fail_closed();
                let _ = self.registry.cleanup_failed(row.name.clone()).await;
                Err(Error::internal("Sandbox cleanup failed; admission closed"))
            }
        }
    }

    async fn discard(&self, slot: &Arc<Slot>) -> Result<()> {
        // A second caller waits for confirmed cleanup. It cannot acknowledge a
        // cancellation merely because another task has started removing it.
        let _cleanup = slot.cleanup.lock().await;
        if slot.closed.load(Ordering::Acquire) {
            return Ok(());
        }
        let result = self.remove_resources(&slot.record).await;
        if result.is_ok() {
            slot.closed.store(true, Ordering::Release);
        }
        slot.transport.lock().await.take();
        self.inventory
            .lock()
            .unwrap()
            .active
            .remove(&slot.record.name);
        self.changed.notify_waiters();
        result
    }

    async fn reconcile(&self) -> Result<()> {
        for row in self
            .registry
            .expired(self.owner.clone())
            .await
            .map_err(Error::internal)?
        {
            self.remove_resources(&row).await?;
        }
        let mut expired = Vec::new();
        {
            let mut inventory = self.inventory.lock().unwrap();
            while inventory.available.front().is_some_and(|slot| {
                slot.ready_at.elapsed().as_secs() >= self.common.warm_idle_seconds
            }) {
                expired.push(inventory.available.pop_front().unwrap());
            }
            inventory.retiring += expired.len();
        }
        let mut failure = None;
        for slot in expired {
            if let Err(error) = self.discard(&slot).await {
                failure = Some(error);
            }
            self.inventory.lock().unwrap().retiring -= 1;
        }
        self.changed.notify_waiters();
        failure.map_or(Ok(()), Err)
    }

    async fn execute_slot(
        &self,
        slot: &Arc<Slot>,
        deadline: Instant,
        payload: Dispatch,
        mode: Mode,
        events: EventSink,
    ) -> Result<Value> {
        // The shared budget already includes cleanup. Reserve that final slice
        // rather than allowing execution to consume it and adding it again.
        let execution_deadline = deadline - Duration::from_secs(self.common.cleanup_timeout);
        let operation = async {
            let (mut runner, mut gateway) =
                slot.transport.lock().await.take().ok_or(Error::Cancelled)?;
            let policy = json!({
                "callback_url": self.common.callback_url, "object_store_url": self.common.object_store_url,
                "app_id": payload.app_id, "run_id": payload.run_id, "executor_jwt": payload.executor_jwt,
                "deadline": now() + execution_deadline.saturating_duration_since(Instant::now()).as_secs_f64(),
                "allowed_https_hosts": self.common.allowed_https_hosts, "buckets": self.common.buckets,
                "object_store_tls_gateway": self.common.object_store_tls_gateway,
            });
            let mut policy = serde_json::to_vec(&policy)?;
            policy.push(b'\n');
            if policy.len() > 65536 {
                return Err(Error::invalid("Gateway policy exceeds its budget"));
            }
            gateway.input.write_all(&policy).await?;
            gateway
                .ready(
                    b"assigned\n",
                    Instant::now() + Duration::from_secs(self.common.startup_grace),
                )
                .await?;
            gateway.input.shutdown().await?;
            let mut envelope = serde_json::to_vec(&json!({"mode": mode, "payload": payload}))?;
            envelope.push(b'\n');
            if envelope.len() > MAX_INPUT {
                return Err(Error::invalid("Warm dispatch exceeds its input budget"));
            }
            // The registry binding precedes these tenant bytes. Cancellation
            // only removes this already-running sandbox, with no later start.
            runner.input.write_all(&envelope).await?;
            runner.input.shutdown().await?;
            let (sender, mut receiver) = mpsc::channel(2);
            let reader = tokio::spawn(async move {
                loop {
                    let line = runner.output.line(MAX_EVENT).await;
                    let done = !line.as_ref().is_ok_and(|line| !line.is_empty());
                    if sender.send(line).await.is_err() || done {
                        break;
                    }
                }
            });
            let reader = AbortOnDrop(reader);
            let mut heartbeat = tokio::time::interval(Duration::from_secs(10));
            heartbeat.tick().await;
            let mut total = 0;
            let mut completed = None;
            loop {
                tokio::select! {
                    line = receiver.recv() => {
                        let line = line.ok_or_else(|| Error::internal("Execution output transport stopped"))??;
                        if line.is_empty() { break; }
                        total += line.len();
                        if total > MAX_OUTPUT { return Err(Error::internal("Execution exceeded its output budget")); }
                        let event: Value = serde_json::from_slice(&line)?;
                        let kind = event.get("event_type").and_then(Value::as_str).ok_or_else(|| Error::internal("Invalid execution event"))?;
                        if kind == "completed" { completed = event.get("payload").cloned(); }
                        events.send(event).await;
                    }
                    _ = heartbeat.tick() => {
                        events.send(json!({"event_id": uuid::Uuid::new_v4().simple().to_string(), "timestamp": {"secs_since_epoch": now() as u64, "nanos_since_epoch": 0}, "event_type": "heartbeat", "payload": {}})).await;
                    }
                }
            }
            drop(reader);
            if self.engine.wait(&slot.record.name).await? != 0
                || !completed.as_ref().is_some_and(Value::is_object)
            {
                return Err(Error::internal("Execution did not settle"));
            }
            Ok(completed.unwrap())
        };
        let outcome = timeout_at(execution_deadline, operation)
            .await
            .unwrap_or_else(|_| Err(Error::internal("Execution deadline expired")));
        self.discard(slot).await?;
        outcome
    }
}

struct AbortOnDrop<T>(tokio::task::JoinHandle<T>);
impl<T> Drop for AbortOnDrop<T> {
    fn drop(&mut self) {
        self.0.abort();
    }
}

struct DockerReservation {
    manager: Arc<Manager>,
    slot: Option<Arc<Slot>>,
    deadline: Instant,
}

impl Drop for DockerReservation {
    fn drop(&mut self) {
        if let Some(slot) = self.slot.take() {
            let manager = self.manager.clone();
            tokio::spawn(async move {
                let _ = manager.discard(&slot).await;
            });
        }
    }
}

#[async_trait]
impl Reservation for DockerReservation {
    async fn execute(
        mut self: Box<Self>,
        payload: Dispatch,
        mode: Mode,
        events: EventSink,
    ) -> Result<Value> {
        let slot = self.slot.as_ref().unwrap().clone();
        let result = self
            .manager
            .execute_slot(&slot, self.deadline, payload, mode, events)
            .await;
        self.slot.take();
        result
    }
}

#[async_trait]
impl Backend for Manager {
    fn ready(&self) -> bool {
        self.ready.load(Ordering::Acquire) && !self.draining.load(Ordering::Acquire)
    }

    fn metrics(&self) -> String {
        let inventory = self.inventory.lock().unwrap();
        format!(
            "executor_active_executions {}\nexecutor_ready_sandboxes {}\nexecutor_creating_sandboxes {}\nexecutor_retiring_sandboxes {}\nexecutor_sandboxes_created_total {}\nexecutor_sandbox_creation_errors_total {}\nexecutor_assignment_seconds_sum {}\nexecutor_assignment_seconds_count {}\n",
            inventory.active.len(),
            inventory.available.len(),
            inventory.creating,
            inventory.retiring,
            self.created.load(Ordering::Relaxed),
            self.errors.load(Ordering::Relaxed),
            self.assignment_nanos.load(Ordering::Relaxed) as f64 / 1e9,
            self.assignments.load(Ordering::Relaxed)
        )
    }

    async fn prepare(self: Arc<Self>) -> Result<()> {
        self.registry
            .heartbeat(self.owner.clone())
            .await
            .map_err(Error::internal)?;
        let heartbeat = self.clone();
        tokio::spawn(async move {
            while !heartbeat.stopped.load(Ordering::Acquire) {
                tokio::time::sleep(Duration::from_secs(5)).await;
                if let Err(error) = heartbeat.registry.heartbeat(heartbeat.owner.clone()).await {
                    tracing::error!(error, "Execution ownership heartbeat failed");
                    heartbeat.fail_closed();
                    break;
                }
            }
        });
        let preparation = async {
            let info = self.engine.request("GET", "/info", None).await?;
            let args = info["Runtimes"]["runsc"]["runtimeArgs"].as_array().ok_or_else(|| Error::invalid("runsc runtime is unavailable or lacks runtimeArgs"))?;
            if !["--network=none", "--host-uds=open"].iter().all(|expected| args.iter().any(|argument| argument.as_str() == Some(expected))) {
                return Err(Error::invalid("runsc must configure --network=none and --host-uds=open"));
            }
            for image in [&self.config.image, &self.config.gateway_image] {
                self.engine.request("GET", &format!("/images/{}/json", engine::encode(image)), None).await?;
            }
            self.engine.request("GET", &format!("/networks/{}", engine::encode(&self.config.network)), None).await?;
            self.reconcile().await?;
            self.ready.store(true, Ordering::Release);
            for _ in 0..self.common.warm_create_concurrency.min(self.common.warm_pool_size) {
                tokio::spawn(self.clone().replenish());
            }
            let sweep = self.clone();
            tokio::spawn(async move {
                while !sweep.stopped.load(Ordering::Acquire) {
                    tokio::time::sleep(Duration::from_secs(5)).await;
                    if let Err(error) = sweep.reconcile().await {
                        tracing::error!(error = %error, "Execution reconciliation failed");
                        sweep.fail_closed();
                    }
                }
            });
            timeout(Duration::from_secs(self.common.startup_grace), async {
                loop {
                    if !self.ready() { return Err(Error::Unavailable); }
                    if !self.inventory.lock().unwrap().available.is_empty() { return Ok(()); }
                    tokio::select! { _ = self.changed.notified() => {}, _ = tokio::time::sleep(Duration::from_millis(100)) => {} }
                }
            }).await.map_err(|_| Error::internal("No sandbox became ready during startup"))?
        }.await;
        if preparation.is_err() {
            self.fail_closed();
        }
        preparation
    }

    async fn reserve(&self, payload: &Dispatch) -> Result<Box<dyn Reservation>> {
        let started = Instant::now();
        let slot = {
            let mut inventory = self.inventory.lock().unwrap();
            if !self.ready() || inventory.active.len() >= self.common.capacity {
                return Err(Error::NoCapacity);
            }
            let slot = inventory.available.pop_front().ok_or(Error::NoCapacity)?;
            inventory
                .active
                .insert(slot.record.name.clone(), slot.clone());
            slot
        };
        self.changed.notify_waiters();
        let deadline = Instant::now() + Duration::from_secs(self.common.budget());
        let assignment = async {
            if slot.ready_at.elapsed().as_secs() >= self.common.warm_idle_seconds {
                return Err(Error::NoCapacity);
            }
            if !self
                .registry
                .assign(
                    slot.record.name.clone(),
                    payload.run_id.clone(),
                    now() + self.common.budget() as f64,
                )
                .await
                .map_err(Error::internal)?
            {
                return Err(Error::Cancelled);
            }
            Ok(())
        }
        .await;
        if let Err(error) = assignment {
            self.discard(&slot).await?;
            return Err(error);
        }
        self.assignments.fetch_add(1, Ordering::Relaxed);
        self.assignment_nanos.fetch_add(
            started.elapsed().as_nanos().min(u64::MAX as u128) as u64,
            Ordering::Relaxed,
        );
        Ok(Box::new(DockerReservation {
            manager: self.this.upgrade().ok_or(Error::Unavailable)?,
            slot: Some(slot),
            deadline,
        }))
    }

    async fn cancel(&self, run_id: &str) -> Result<Value> {
        if !crate::config::safe_id(run_id) {
            return Err(Error::invalid("Invalid run_id"));
        }
        let until = now() + 60.0 + 86400_u64.max(self.common.timeout + 3600) as f64;
        let records = self
            .registry
            .cancel(run_id.to_owned(), until)
            .await
            .map_err(Error::internal)?;
        for row in &records {
            let local = self
                .inventory
                .lock()
                .unwrap()
                .active
                .get(&row.name)
                .cloned();
            if let Some(slot) = local {
                self.discard(&slot).await?;
            } else {
                self.remove_resources(row).await?;
            }
        }
        Ok(json!({"run_id": run_id, "terminated": true, "containers_removed": records.len() * 2}))
    }

    async fn shutdown(&self) -> Result<()> {
        self.fail_closed();
        let until = Instant::now() + Duration::from_secs(self.common.budget());
        let unused: Vec<_> = self.inventory.lock().unwrap().available.drain(..).collect();
        for slot in unused {
            let _ = self.discard(&slot).await;
        }
        loop {
            let done = {
                let inventory = self.inventory.lock().unwrap();
                inventory.active.is_empty() && inventory.creating == 0 && inventory.retiring == 0
            };
            if done || Instant::now() >= until {
                break;
            }
            tokio::select! { _ = self.changed.notified() => {}, _ = tokio::time::sleep(Duration::from_secs(1)) => {} }
        }
        let active: Vec<_> = self
            .inventory
            .lock()
            .unwrap()
            .active
            .values()
            .cloned()
            .collect();
        let mut failure = None;
        for slot in active {
            if let Err(error) = self.discard(&slot).await {
                failure = Some(error);
            }
        }
        self.stopped.store(true, Ordering::Release);
        if let Some(error) = failure {
            Err(error)
        } else {
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests;
