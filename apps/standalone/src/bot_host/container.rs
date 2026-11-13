//! The enrolled device supervises bot containers and signs only their scoped lifecycle calls.

use super::{
    container_broker::{self as policy, Action},
    container_wire::{self as wire, Frame},
    docker_probe,
    e2e::DeviceKeyVault,
    hub::DeviceApi,
};
use anyhow::{Context, Result, bail, ensure};
use async_trait::async_trait;
use flow_like_device_crypto::inbox::InboxKeyVault;
use flow_like_types::{
    bot::BotExecutionMode,
    device_bot::{DeviceBotInfo, DeviceDockerSupport, DevicePollResponse, DeviceTurnGrant},
};
use reqwest::Method;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    process::Stdio,
    sync::{
        Arc, Mutex, Weak,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
    sync::{OwnedSemaphorePermit, Semaphore, mpsc},
};
use tokio_util::sync::CancellationToken;

const POLL_CACHE: Duration = Duration::from_secs(2);
const PROBE_INTERVAL: Duration = Duration::from_secs(30);
const RESTART_AFTER: Duration = Duration::from_secs(30);
const STOP_TIMEOUT: Duration = Duration::from_secs(15);
const KEY_LIMIT: usize = 1024 * 1024;
const LABEL_DEVICE: &str = "io.flow-like.device";
const LABEL_BOT: &str = "io.flow-like.bot";

static ACTIVE: Mutex<Option<Weak<Coordinator>>> = Mutex::new(None);

fn lock<T>(value: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    value.lock().unwrap_or_else(|e| e.into_inner())
}

struct Lease {
    bot: String,
    generation: String,
    attempt: u32,
    renewed: Instant,
    _permit: OwnedSemaphorePermit,
}

#[derive(Clone)]
struct Offer {
    bot: String,
    conversation: String,
}

#[derive(Clone)]
struct Worker {
    generation: String,
    tx: mpsc::Sender<Frame>,
    views: Arc<Mutex<HashMap<u64, mpsc::Sender<Vec<u8>>>>>,
    cancel: CancellationToken,
    stopped: CancellationToken,
    ready: Arc<AtomicBool>,
    started: Instant,
}

struct Snapshot {
    at: Instant,
    response: DevicePollResponse,
}

struct PollRejection {
    status: u16,
    body: Vec<u8>,
}
impl std::fmt::Debug for PollRejection {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PollRejection")
            .field("status", &self.status)
            .finish_non_exhaustive()
    }
}
impl std::fmt::Display for PollRejection {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "Device bot poll failed with HTTP {}",
            self.status
        )
    }
}
impl std::error::Error for PollRejection {}

pub struct Coordinator {
    api: Arc<dyn DeviceApi>,
    state_dir: PathBuf,
    poll: tokio::sync::Mutex<Option<Snapshot>>,
    probe: tokio::sync::Mutex<Option<(Instant, DeviceDockerSupport)>>,
    bots: Mutex<HashMap<String, DeviceBotInfo>>,
    offers: Mutex<HashMap<String, Offer>>,
    workers: Mutex<HashMap<String, Worker>>,
    leases: tokio::sync::Mutex<HashMap<String, Lease>>,
    slots: Arc<Semaphore>,
    cancel: CancellationToken,
    view_ids: AtomicU64,
}

impl Coordinator {
    pub fn new(
        state_dir: PathBuf,
        api: Arc<dyn DeviceApi>,
        cancel: CancellationToken,
    ) -> Arc<Self> {
        let coordinator = Arc::new(Self {
            api,
            state_dir,
            poll: tokio::sync::Mutex::new(None),
            probe: tokio::sync::Mutex::new(None),
            bots: Mutex::default(),
            offers: Mutex::default(),
            workers: Mutex::default(),
            leases: tokio::sync::Mutex::new(HashMap::new()),
            slots: Arc::new(Semaphore::new(super::CONCURRENT_TURNS)),
            cancel,
            view_ids: AtomicU64::new(1),
        });
        *lock(&ACTIVE) = Some(Arc::downgrade(&coordinator));
        coordinator
    }

    pub fn native_api(self: &Arc<Self>) -> Arc<dyn DeviceApi> {
        Arc::new(NativeApi(self.clone()))
    }

    async fn support(&self) -> DeviceDockerSupport {
        let mut saved = self.probe.lock().await;
        if let Some((at, support)) = &*saved
            && at.elapsed() < PROBE_INTERVAL
        {
            return support.clone();
        }
        let support = docker_probe::probe(&self.state_dir, self.api.base_url()).await;
        *saved = Some((Instant::now(), support.clone()));
        support
    }

    async fn poll(self: &Arc<Self>) -> Result<DevicePollResponse> {
        let response = {
            let mut cache = self.poll.lock().await;
            if let Some(snapshot) = &*cache
                && snapshot.at.elapsed() < POLL_CACHE
            {
                return Ok(snapshot.response.clone());
            }
            let docker = self.support().await;
            let modes = if docker.available {
                vec![BotExecutionMode::Native, BotExecutionMode::Docker]
            } else {
                vec![BotExecutionMode::Native]
            };
            let mut ready: Vec<String> = lock(&self.bots)
                .values()
                .filter(|bot| bot.execution_mode == BotExecutionMode::Native)
                .map(|bot| bot.id.clone())
                .collect();
            ready.extend(
                lock(&self.workers)
                    .iter()
                    .filter(|(_, worker)| {
                        worker.ready.load(Ordering::Acquire)
                            && !worker.cancel.is_cancelled()
                            && !worker.stopped.is_cancelled()
                    })
                    .map(|(id, _)| id.clone()),
            );
            let body = json!({"host_id":super::host_id(self.api.device_id()), "execution_modes":modes, "docker":docker, "ready_bot_ids":ready});
            let (status, bytes) = self
                .api
                .call(
                    Method::POST,
                    &format!("/devices/{}/bots/poll", self.api.device_id()),
                    Some(&body),
                )
                .await?;
            if !(200..300).contains(&status) {
                if matches!(status, 401 | 403) {
                    for worker in lock(&self.workers).values() {
                        worker.cancel.cancel();
                    }
                }
                return Err(PollRejection {
                    status,
                    body: bytes,
                }
                .into());
            }
            let mut response: DevicePollResponse =
                serde_json::from_slice(&bytes).context("Read device bot assignments")?;
            if response.bots.iter().any(|bot| !ready.contains(&bot.id)) {
                response.next_poll_ms = response.next_poll_ms.min(2_000);
            }
            *lock(&self.offers) = response
                .turns
                .iter()
                .map(|turn| {
                    (
                        turn.id.clone(),
                        Offer {
                            bot: turn.bot_id.clone(),
                            conversation: turn.conversation_id.clone(),
                        },
                    )
                })
                .collect();
            *cache = Some(Snapshot {
                at: Instant::now(),
                response: response.clone(),
            });
            response
        };
        self.reconcile(&response).await;
        Ok(response)
    }

    async fn reconcile(self: &Arc<Self>, response: &DevicePollResponse) {
        let previous = std::mem::replace(
            &mut *lock(&self.bots),
            response
                .bots
                .iter()
                .map(|bot| (bot.id.clone(), bot.clone()))
                .collect(),
        );
        let stopping = {
            let mut workers = lock(&self.workers);
            let mut stopping = Vec::new();
            workers.retain(|id, worker| {
                if response
                    .bots
                    .iter()
                    .any(|bot| &bot.id == id && bot.execution_mode == BotExecutionMode::Docker)
                {
                    return true;
                }
                worker.cancel.cancel();
                if worker.stopped.is_cancelled() {
                    return false;
                }
                // Keep ownership until cleanup finishes, even if the bounded wait expires.
                stopping.push(worker.clone());
                true
            });
            stopping
        };
        for worker in stopping {
            let _ = tokio::time::timeout(STOP_TIMEOUT, worker.stopped.cancelled()).await;
        }
        #[cfg(all(
            feature = "playground",
            any(target_os = "linux", target_os = "macos", windows)
        ))]
        if let Some(host) = crate::playground::shared_machine_host() {
            for (id, old) in previous {
                if old.execution_mode == BotExecutionMode::Native
                    && !response
                        .bots
                        .iter()
                        .any(|bot| bot.id == id && bot.execution_mode == BotExecutionMode::Native)
                {
                    host.stop(&id).await;
                }
            }
        }
        #[cfg(not(all(
            feature = "playground",
            any(target_os = "linux", target_os = "macos", windows)
        )))]
        let _ = previous;
        if !self.support().await.available || self.cancel.is_cancelled() {
            return;
        }
        for bot in &response.bots {
            if bot.execution_mode == BotExecutionMode::Docker {
                self.ensure_worker(&bot.id);
            }
        }
    }

    fn ensure_worker(self: &Arc<Self>, bot: &str) {
        let mut workers = lock(&self.workers);
        if let Some(worker) = workers.get(bot)
            && (!worker.stopped.is_cancelled() || worker.started.elapsed() < RESTART_AFTER)
        {
            return;
        }
        let (tx, rx) = mpsc::channel(8);
        let worker = Worker {
            generation: uuid::Uuid::new_v4().to_string(),
            tx,
            views: Arc::default(),
            cancel: self.cancel.child_token(),
            stopped: CancellationToken::new(),
            ready: Arc::new(AtomicBool::new(false)),
            started: Instant::now(),
        };
        workers.insert(bot.into(), worker.clone());
        let this = self.clone();
        let bot = bot.to_owned();
        tokio::spawn(async move {
            if let Err(error) = this.supervise(&bot, &worker, rx).await {
                tracing::warn!(bot_id = %bot, "The bot's Docker container stopped: {error:#}");
            }
            worker.cancel.cancel();
            this.release_worker(&bot, &worker.generation).await;
            worker.ready.store(false, Ordering::Release);
            worker.stopped.cancel();
            worker
                .views
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .clear();
        });
    }

    fn configured(&self, bot: &str, mode: BotExecutionMode) -> bool {
        lock(&self.bots)
            .get(bot)
            .is_some_and(|info| info.execution_mode == mode)
    }

    fn cached_execution_mode(&self, bot: &str) -> Option<BotExecutionMode> {
        if self.cancel.is_cancelled() {
            return None;
        }
        lock(&self.bots).get(bot).map(|info| info.execution_mode)
    }

    fn worker_current(&self, bot: &str, generation: &str) -> bool {
        self.configured(bot, BotExecutionMode::Docker)
            && lock(&self.workers).get(bot).is_some_and(|worker| {
                worker.generation == generation
                    && !worker.cancel.is_cancelled()
                    && !worker.stopped.is_cancelled()
            })
    }

    async fn scoped_call(
        self: &Arc<Self>,
        bot: &str,
        generation: &str,
        method: Method,
        path: &str,
        body: Option<&Value>,
    ) -> Result<(u16, Vec<u8>)> {
        ensure!(
            self.worker_current(bot, generation),
            "This bot container is no longer selected"
        );
        let action = policy::action(method.as_str(), path, self.api.device_id())?;
        let body = policy::body_for(
            &action,
            body,
            bot,
            self.api.device_id(),
            BotExecutionMode::Docker,
        )?;
        match action {
            Action::Poll => {
                // The enrolled parent polls once for all hosts. A child cannot amplify polling.
                let mut response = self
                    .poll
                    .lock()
                    .await
                    .as_ref()
                    .context("The device has no current bot assignments")?
                    .response
                    .clone();
                response.bots.retain(|found| {
                    found.id == bot && found.execution_mode == BotExecutionMode::Docker
                });
                response.turns.retain(|turn| turn.bot_id == bot);
                if let Some(worker) = lock(&self.workers)
                    .get(bot)
                    .filter(|worker| worker.generation == generation)
                {
                    worker.ready.store(true, Ordering::Release);
                }
                Ok((200, serde_json::to_vec(&response)?))
            }
            Action::Forget => Ok((200, serde_json::to_vec(&json!({"removed":0}))?)),
            Action::PublishKey => self.api.call(method, path, body.as_ref()).await,
            Action::Claim(id) => {
                self.claim(
                    bot,
                    generation,
                    BotExecutionMode::Docker,
                    &id,
                    method,
                    path,
                    body.as_ref(),
                )
                .await
            }
            Action::Turn { id, terminal } => {
                self.held_call(bot, generation, &id, terminal, method, path, body.as_ref())
                    .await
            }
        }
    }

    async fn claim(
        &self,
        bot: &str,
        generation: &str,
        mode: BotExecutionMode,
        id: &str,
        method: Method,
        path: &str,
        body: Option<&Value>,
    ) -> Result<(u16, Vec<u8>)> {
        let offer = lock(&self.offers)
            .get(id)
            .filter(|offered| offered.bot == bot)
            .cloned()
            .context("The turn was not offered to this bot host")?;
        ensure!(
            self.configured(bot, mode),
            "This bot is assigned to another host mode"
        );
        let mut leases = self.leases.lock().await;
        // AccountHost stops a turn after one lease interval without a successful heartbeat.
        // Reclaim its local slot too, including when a disconnected host could not release it.
        leases.retain(|_, lease| {
            lease.renewed.elapsed()
                <= Duration::from_secs(flow_like_types::bot_turn::LEASE_SECS as u64 + 5)
        });
        let permit = self
            .slots
            .clone()
            .try_acquire_owned()
            .context("This device is already running its allowed bot turns")?;
        ensure!(!leases.contains_key(id), "The turn is already held here");
        let response = self.api.call(method, path, body).await?;
        if (200..300).contains(&response.0) {
            let grant: DeviceTurnGrant = serde_json::from_slice(&response.1)?;
            ensure!(
                grant.bot.bot_id == bot
                    && grant.turn.bot_id == bot
                    && grant.turn.conversation_id == offer.conversation
                    && grant.bot.execution_mode == mode,
                "The hub returned another bot's turn"
            );
            if mode == BotExecutionMode::Docker && !self.worker_current(bot, generation) {
                let release_path = format!(
                    "/devices/{}/bots/turns/{}/release",
                    self.api.device_id(),
                    grant.turn.id
                );
                let _ = self.api.call(Method::POST, &release_path, Some(&json!({"host_id":super::host_id(self.api.device_id()), "error_code":"BOT_CONTAINER_STOPPED", "retry":true}))).await;
                bail!("The container stopped while its turn was being claimed");
            }
            // A claim may batch newer queued messages into a different turn in this conversation.
            leases.insert(
                grant.turn.id,
                Lease {
                    bot: bot.into(),
                    generation: generation.into(),
                    attempt: grant.turn.attempts,
                    renewed: Instant::now(),
                    _permit: permit,
                },
            );
        }
        Ok(response)
    }

    async fn held_call(
        &self,
        bot: &str,
        generation: &str,
        id: &str,
        terminal: bool,
        method: Method,
        path: &str,
        body: Option<&Value>,
    ) -> Result<(u16, Vec<u8>)> {
        let mut leases = self.leases.lock().await;
        let lease = leases
            .get(id)
            .filter(|lease| lease.bot == bot && lease.generation == generation)
            .context("The container does not hold this turn")?;
        if path.contains("/checkpoint") {
            let attempt = if method == Method::GET {
                path.split_once('?').and_then(|(_, query)| {
                    url::form_urlencoded::parse(query.as_bytes())
                        .find(|(key, _)| key == "attempt")
                        .and_then(|(_, value)| value.parse::<u32>().ok())
                })
            } else {
                body.and_then(|value| value["attempt"].as_u64())
                    .and_then(|value| u32::try_from(value).ok())
            };
            ensure!(
                attempt == Some(lease.attempt),
                "The container does not hold this turn attempt"
            );
        }
        let response = self.api.call(method, path, body).await?;
        let lost = flow_like_bot_host::hub::answer(response.0, &response.1)
            .err()
            .is_some_and(|failure| {
                failure.lease_lost()
                    || matches!(failure.status, Some(401 | 403 | 404))
                    || (path.ends_with("/heartbeat") && failure.taken())
            });
        if matches!(response.0, 401 | 403) {
            for worker in lock(&self.workers).values() {
                worker.cancel.cancel();
            }
        }
        if lost {
            leases.remove(id);
            return Ok(response);
        }
        if path.ends_with("/heartbeat")
            && (200..300).contains(&response.0)
            && let Some(lease) = leases.get_mut(id)
        {
            lease.renewed = Instant::now();
        }
        let answer = serde_json::from_slice::<Value>(&response.1).unwrap_or(Value::Null);
        let parked = !path.ends_with("/wait") || answer["status"] == "parked";
        // Exhausted retries leave the turn held so the host can post the final failure reply.
        let needs_failure_reply = path.ends_with("/release")
            && body.is_some_and(|body| body["retry"] == true)
            && answer["requeued"] == false;
        if terminal && parked && !needs_failure_reply && (200..300).contains(&response.0) {
            leases.remove(id);
        }
        Ok(response)
    }

    async fn release_worker(&self, bot: &str, generation: &str) {
        let held = {
            let mut leases = self.leases.lock().await;
            let ids: Vec<_> = leases
                .iter()
                .filter(|(_, lease)| lease.bot == bot && lease.generation == generation)
                .map(|(id, _)| id.clone())
                .collect();
            for id in &ids {
                leases.remove(id);
            }
            ids
        };
        for id in held {
            let path = format!("/devices/{}/bots/turns/{id}/release", self.api.device_id());
            let _ = self.api.call(Method::POST, &path, Some(&json!({"host_id":super::host_id(self.api.device_id()),"error_code":"BOT_CONTAINER_STOPPED","retry":true}))).await;
        }
    }

    pub async fn shutdown(&self) {
        self.cancel.cancel();
        let workers: Vec<_> = lock(&self.workers).values().cloned().collect();
        for worker in workers {
            let _ = tokio::time::timeout(STOP_TIMEOUT, worker.stopped.cancelled()).await;
        }
        let _ = self
            .api
            .call(
                Method::POST,
                &format!("/devices/{}/bots/hosts/forget", self.api.device_id()),
                Some(&json!({"host_id":super::host_id(self.api.device_id())})),
            )
            .await;
    }

    async fn supervise(
        self: &Arc<Self>,
        bot: &str,
        worker: &Worker,
        mut outgoing: mpsc::Receiver<Frame>,
    ) -> Result<()> {
        let endpoint = docker_probe::local_endpoint()
            .await
            .map_err(anyhow::Error::msg)?;
        let image = crate::release::installed_playground_container(&self.state_dir)?.image;
        let directory =
            crate::supervisor::prepare_state_dir(&self.state_dir.join("bot-containers"))?;
        let name = resource_name(self.api.device_id(), bot);
        // Concurrent bot startups must not truncate a profile another Docker command is reading.
        let profile = directory.join(format!("{name}.seccomp.json"));
        std::fs::write(
            &profile,
            include_bytes!("../../packaging/chrome-seccomp.json"),
        )?;
        remove_owned(&endpoint, &name, self.api.device_id(), bot).await?;
        ensure_volume(
            &endpoint,
            &format!("{name}-data"),
            self.api.device_id(),
            bot,
        )
        .await?;
        let vault = DeviceKeyVault::open(&self.state_dir)?;
        let keys = vault
            .load(bot)
            .map_err(anyhow::Error::msg)?
            .map(|bytes| wire::encode(&bytes));
        ensure_image(&endpoint, &image, &worker.cancel).await?;
        ensure!(
            !worker.cancel.is_cancelled(),
            "The bot container was stopped before startup"
        );
        let args = run_args(&name, &image, &profile, self.api.device_id(), bot);
        // Creation never runs bot code. Finish it before attaching, so cancellation during an
        // image pull or filesystem setup cannot race cleanup and start an unobserved bot.
        let created = tokio::time::timeout(
            Duration::from_secs(30),
            docker_probe::command(&endpoint)
                .args(&args)
                .stdout(Stdio::null())
                .stderr(Stdio::inherit())
                .status(),
        )
        .await;
        let creation = match created {
            Ok(Ok(status)) if status.success() => Ok(()),
            Ok(Ok(status)) => Err(anyhow::anyhow!(
                "Creating the bot container failed with {status}"
            )),
            Ok(Err(error)) => Err(error.into()),
            Err(_) => Err(anyhow::anyhow!("Creating the bot container timed out")),
        };
        if creation.is_err() || worker.cancel.is_cancelled() {
            let cleanup = remove_owned(&endpoint, &name, self.api.device_id(), bot).await;
            return creation.and(cleanup);
        }
        let mut command = docker_probe::command(&endpoint);
        command
            .args(["start", "--attach", "--interactive", &name])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit());
        let mut child = match command.spawn() {
            Ok(child) => child,
            Err(error) => {
                let _ = remove_owned(&endpoint, &name, self.api.device_id(), bot).await;
                return Err(error).context("Start the bot container");
            }
        };
        let result = async {
            let mut input = child.stdin.take().context("Container input pipe is unavailable")?;
            let mut output = child.stdout.take().context("Container output pipe is unavailable")?;
            let init = Frame::Init { base_url: self.api.base_url().into(), device_id: self.api.device_id().into(), bot_id: bot.into(), keys };
            tokio::select! {
                _ = worker.cancel.cancelled() => return Ok(()),
                result = tokio::time::timeout(Duration::from_secs(300), wire::write(&mut input, &init)) => result.context("The bot image did not start in time")??,
            }
            let writing = async {
                let mut keepalive = tokio::time::interval(Duration::from_secs(5));
                keepalive.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
                loop {
                    let frame = tokio::select! {
                        frame = outgoing.recv() => match frame { Some(frame) => frame, None => return Ok::<_, anyhow::Error>(()) },
                        _ = keepalive.tick() => Frame::Ping,
                    };
                    wire::write(&mut input, &frame).await?;
                }
            };
            let reading = self.read_worker(bot, worker, &mut output);
            tokio::select! {
                _ = worker.cancel.cancelled() => Ok(()),
                result = writing => result,
                result = reading => result,
                result = child.wait() => { let status = result?; ensure!(status.success(), "The bot container exited with {status}"); Ok(()) },
            }
        }.await;
        // Killing the Docker CLI alone leaves its container alive. Remove only our labeled container.
        let _ = child.kill().await;
        let cleanup = remove_owned(&endpoint, &name, self.api.device_id(), bot).await;
        result.and(cleanup)
    }

    async fn read_worker(
        self: &Arc<Self>,
        bot: &str,
        worker: &Worker,
        output: &mut (impl AsyncRead + Unpin),
    ) -> Result<()> {
        let requests = Arc::new(Semaphore::new(32));
        while let Some(frame) = wire::read(output).await? {
            match frame {
                Frame::Call {
                    id,
                    method,
                    path,
                    body,
                } => {
                    let permit = requests
                        .clone()
                        .try_acquire_owned()
                        .context("Too many outstanding container requests")?;
                    let this = self.clone();
                    let bot = bot.to_owned();
                    let worker = worker.clone();
                    tokio::spawn(async move {
                        let _permit = permit;
                        let result = match Method::from_bytes(method.as_bytes()) {
                            Ok(method) => {
                                this.scoped_call(
                                    &bot,
                                    &worker.generation,
                                    method,
                                    &path,
                                    body.as_ref(),
                                )
                                .await
                            }
                            Err(_) => Err(anyhow::anyhow!("Invalid container request method")),
                        };
                        let (status, body) = result.unwrap_or_else(|error| (403, serde_json::to_vec(&json!({"error":"BOT_CONTAINER_REFUSED","message":error.to_string()})).unwrap_or_default()));
                        let _ = worker
                            .tx
                            .send(Frame::Reply {
                                id,
                                status,
                                body: wire::encode(&body),
                            })
                            .await;
                    });
                }
                Frame::Keys { data } => {
                    ensure!(
                        self.worker_current(bot, &worker.generation),
                        "This container is no longer selected"
                    );
                    let bytes = wire::decode(&data)?;
                    ensure!(bytes.len() <= KEY_LIMIT, "The bot key backup is too large");
                    // Only this bot's file is writable through this channel, never a path from the child.
                    DeviceKeyVault::open(&self.state_dir)?
                        .store(bot, &bytes)
                        .map_err(anyhow::Error::msg)?;
                }
                Frame::ViewData { id, data } => {
                    let bytes = wire::decode(&data)?;
                    ensure!(
                        bytes.len() <= 16 * 1024,
                        "The bot screen packet is too large"
                    );
                    let failed = lock(&worker.views)
                        .get(&id)
                        .is_some_and(|view| view.try_send(bytes).is_err());
                    if failed {
                        lock(&worker.views).remove(&id);
                        let _ = worker.tx.send(Frame::CloseView { id }).await;
                    }
                }
                Frame::CloseView { id } => {
                    lock(&worker.views).remove(&id);
                }
                _ => bail!("Unexpected message from the bot container"),
            }
        }
        Ok(())
    }
}

struct NativeApi(Arc<Coordinator>);

#[async_trait]
impl DeviceApi for NativeApi {
    fn base_url(&self) -> &str {
        self.0.api.base_url()
    }
    fn device_id(&self) -> &str {
        self.0.api.device_id()
    }
    async fn call(
        &self,
        method: Method,
        path: &str,
        body: Option<&Value>,
    ) -> Result<(u16, Vec<u8>)> {
        let action = policy::action(method.as_str(), path, self.device_id())?;
        match action {
            Action::Poll => {
                let mut response = match self.0.poll().await {
                    Ok(response) => response,
                    Err(error) => {
                        if let Some(answer) = error.downcast_ref::<PollRejection>() {
                            return Ok((answer.status, answer.body.clone()));
                        }
                        return Err(error);
                    }
                };
                response
                    .bots
                    .retain(|bot| bot.execution_mode == BotExecutionMode::Native);
                let ids: std::collections::HashSet<_> =
                    response.bots.iter().map(|bot| bot.id.as_str()).collect();
                response
                    .turns
                    .retain(|turn| ids.contains(turn.bot_id.as_str()));
                Ok((200, serde_json::to_vec(&response)?))
            }
            Action::Claim(id) => {
                let bot = lock(&self.0.offers)
                    .get(&id)
                    .map(|offer| offer.bot.clone())
                    .context("The turn was not offered to this device")?;
                let body = policy::body_for(
                    &Action::Claim(id.clone()),
                    body,
                    &bot,
                    self.device_id(),
                    BotExecutionMode::Native,
                )?;
                self.0
                    .claim(
                        &bot,
                        "native",
                        BotExecutionMode::Native,
                        &id,
                        method,
                        path,
                        body.as_ref(),
                    )
                    .await
            }
            Action::Turn { id, terminal } => {
                let bot = self
                    .0
                    .leases
                    .lock()
                    .await
                    .get(&id)
                    .map(|lease| lease.bot.clone())
                    .context("The native host does not hold this turn")?;
                self.0
                    .held_call(&bot, "native", &id, terminal, method, path, body)
                    .await
            }
            Action::Forget => Ok((200, serde_json::to_vec(&json!({"removed":0}))?)),
            Action::PublishKey => {
                let bot = body
                    .and_then(|body| body["principal_id"].as_str())
                    .context("Missing bot key principal")?;
                ensure!(
                    self.0.configured(bot, BotExecutionMode::Native),
                    "The native host cannot publish this bot's key"
                );
                self.0.api.call(method, path, body).await
            }
        }
    }
}

pub fn cached_execution_mode(bot: &str) -> Option<BotExecutionMode> {
    lock(&ACTIVE)
        .as_ref()
        .and_then(Weak::upgrade)
        .and_then(|host| host.cached_execution_mode(bot))
}

pub async fn serve_view<IO>(mut io: IO, bot: &str, observations: bool) -> Result<()>
where
    IO: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let host = lock(&ACTIVE)
        .as_ref()
        .and_then(Weak::upgrade)
        .context("The device's bot container host is not running")?;
    let worker = lock(&host.workers)
        .get(bot)
        .filter(|worker| !worker.stopped.is_cancelled() && !worker.cancel.is_cancelled())
        .cloned()
        .context("The bot's Docker container is not running")?;
    let id = host.view_ids.fetch_add(1, Ordering::Relaxed);
    let (incoming, mut rx) = mpsc::channel(32);
    {
        let mut views = lock(&worker.views);
        ensure!(views.len() < 4, "Too many viewers of this bot's screen");
        views.insert(id, incoming);
    }
    let _guard = ViewGuard {
        worker: worker.clone(),
        id,
    };
    worker.tx.send(Frame::OpenView { id, observations }).await?;
    let (mut read, mut write) = tokio::io::split(&mut io);
    let input = async {
        let mut bytes = [0u8; 16 * 1024];
        loop {
            let count = read.read(&mut bytes).await?;
            if count == 0 {
                break;
            }
            worker
                .tx
                .send(Frame::ViewData {
                    id,
                    data: wire::encode(&bytes[..count]),
                })
                .await?;
        }
        Ok::<_, anyhow::Error>(())
    };
    let output = async {
        while let Some(bytes) = rx.recv().await {
            write.write_all(&bytes).await?;
        }
        Ok::<_, anyhow::Error>(())
    };
    let result = tokio::select! { result = input => result, result = output => result, _ = worker.stopped.cancelled() => Ok(()) };
    result
}

/// Tunnel cancellation drops its future, so closing the child view cannot depend on reaching
/// the bottom of `serve_view`. The child's stream guard also hands input control back.
struct ViewGuard {
    worker: Worker,
    id: u64,
}

impl Drop for ViewGuard {
    fn drop(&mut self) {
        lock(&self.worker.views).remove(&self.id);
        if let Err(mpsc::error::TrySendError::Full(frame)) =
            self.worker.tx.try_send(Frame::CloseView { id: self.id })
        {
            let tx = self.worker.tx.clone();
            if let Ok(runtime) = tokio::runtime::Handle::try_current() {
                runtime.spawn(async move {
                    let _ = tx.send(frame).await;
                });
            }
        }
    }
}

fn resource_name(device: &str, bot: &str) -> String {
    let digest = Sha256::digest(format!("{device}\0{bot}").as_bytes());
    format!(
        "flow-like-bot-{}",
        digest[..16]
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    )
}

fn run_args(name: &str, image: &str, profile: &Path, device: &str, bot: &str) -> Vec<String> {
    // Stdout carries private broker frames, so Docker must not retain it as a container log.
    [
        "create".into(),
        "--log-driver=none".into(),
        "--rm".into(),
        "--interactive".into(),
        "--init".into(),
        "--pull=never".into(),
        "--name".into(),
        name.into(),
        "--hostname".into(),
        name.into(),
        "--label".into(),
        format!("{LABEL_DEVICE}={device}"),
        "--label".into(),
        format!("{LABEL_BOT}={bot}"),
        "--user=65532:65532".into(),
        "--cap-drop=ALL".into(),
        "--security-opt=no-new-privileges".into(),
        "--security-opt".into(),
        format!("seccomp={}", profile.display()),
        "--read-only".into(),
        "--shm-size=1g".into(),
        "--memory=4g".into(),
        "--memory-swap=4g".into(),
        "--cpus=2".into(),
        "--pids-limit=512".into(),
        "--tmpfs=/tmp:rw,nosuid,nodev,size=1g,mode=1777".into(),
        "--tmpfs=/run:rw,nosuid,nodev,size=16m,mode=1777".into(),
        "--mount".into(),
        format!("type=volume,source={name}-data,target=/var/lib/flow-like"),
        "--env=FLOW_LIKE_PLAYGROUND=virtual".into(),
        "--env=HOME=/var/lib/flow-like".into(),
        image.into(),
        "bot-container-worker".into(),
    ]
    .into()
}

async fn remove_owned(endpoint: &str, name: &str, device: &str, bot: &str) -> Result<()> {
    let inspected = tokio::time::timeout(
        Duration::from_secs(5),
        docker_probe::command(endpoint)
            .args([
                "container",
                "inspect",
                "--format",
                "{{json .Config.Labels}}",
                name,
            ])
            .output(),
    )
    .await
    .context("Docker container lookup timed out")??;
    if !inspected.status.success() {
        return Ok(());
    }
    ensure!(
        inspected.stdout.len() < 64 * 1024,
        "Docker container labels are too large"
    );
    let labels: Value = serde_json::from_slice(&inspected.stdout)?;
    ensure!(
        labels[LABEL_DEVICE] == device && labels[LABEL_BOT] == bot,
        "The bot container name is already used by another container"
    );
    let removed = tokio::time::timeout(
        Duration::from_secs(10),
        docker_probe::command(endpoint)
            .args(["container", "rm", "--force", name])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status(),
    )
    .await
    .context("Stopping the bot container timed out")??;
    ensure!(removed.success(), "Docker could not stop the bot container");
    Ok(())
}

async fn ensure_volume(endpoint: &str, name: &str, device: &str, bot: &str) -> Result<()> {
    let inspect = || async {
        tokio::time::timeout(
            Duration::from_secs(5),
            docker_probe::command(endpoint)
                .args(["volume", "inspect", "--format", "{{json .Labels}}", name])
                .output(),
        )
        .await
        .context("Docker volume lookup timed out")?
        .context("Inspect the bot volume")
    };
    let mut found = inspect().await?;
    if !found.status.success() {
        let created = tokio::time::timeout(
            Duration::from_secs(5),
            docker_probe::command(endpoint)
                .args([
                    "volume",
                    "create",
                    "--label",
                    &format!("{LABEL_DEVICE}={device}"),
                    "--label",
                    &format!("{LABEL_BOT}={bot}"),
                    name,
                ])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status(),
        )
        .await
        .context("Creating the bot volume timed out")??;
        ensure!(created.success(), "Docker could not create the bot volume");
        found = inspect().await?;
    }
    ensure!(
        found.status.success() && found.stdout.len() < 64 * 1024,
        "The bot volume could not be inspected"
    );
    let labels: Value = serde_json::from_slice(&found.stdout)?;
    ensure!(
        labels[LABEL_DEVICE] == device && labels[LABEL_BOT] == bot,
        "The bot volume name belongs to another volume"
    );
    Ok(())
}

async fn ensure_image(endpoint: &str, image: &str, cancel: &CancellationToken) -> Result<()> {
    let found = tokio::time::timeout(
        Duration::from_secs(5),
        docker_probe::command(endpoint)
            .args(["image", "inspect", image])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status(),
    )
    .await
    .context("Inspecting the bot image timed out")??;
    if found.success() {
        return Ok(());
    }
    let pulled = tokio::select! {
        _ = cancel.cancelled() => bail!("The bot container was stopped before its image was ready"),
        result = tokio::time::timeout(Duration::from_secs(300), docker_probe::command(endpoint).args(["pull", image]).stdout(Stdio::null()).stderr(Stdio::inherit()).status()) => result.context("Downloading the bot image timed out")??,
    };
    ensure!(
        pulled.success(),
        "Docker could not download the signed bot image"
    );
    Ok(())
}

#[cfg(test)]
mod tests;
