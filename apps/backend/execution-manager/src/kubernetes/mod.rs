//! Async supervision of single-use gVisor Pods with external egress enforcement.
mod manifests;
pub mod slot;
mod transport;

use crate::{
    Backend, CommonConfig, Dispatch, Error, EventSink, MAX_EVENT, MAX_OUTPUT, Mode, Reservation,
    Result,
    config::{positive, safe_id, secret},
};
use async_trait::async_trait;
use futures_util::TryStreamExt;
use manifests::*;
use reqwest::{Client, Method};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::{HashMap, VecDeque},
    env,
    sync::{
        Arc, Mutex, Weak,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::{
    io::{AsyncBufRead, AsyncBufReadExt, BufReader},
    sync::{Mutex as AsyncMutex, Notify, OwnedSemaphorePermit, Semaphore},
    time::Instant,
};
use tokio_util::{io::StreamReader, sync::CancellationToken};
use transport::{ClaimStore, Claims, Kube, KubeApi, TransportError};
use uuid::Uuid;

struct Config {
    common: Arc<CommonConfig>,
    image: String,
    gateway_image: String,
    namespace: String,
    pod_name: String,
    pod_uid: String,
    memory_mb: u64,
    cpus: u64,
    tmp_mb: u64,
    runtime_class: String,
    key_id: String,
    node_selector: Value,
    tolerations: Value,
    pull_secrets: Value,
    app_name: String,
    kubernetes_host: String,
    kubernetes_port: u16,
}

impl Config {
    fn from_env(common: Arc<CommonConfig>) -> Result<Self> {
        let value =
            |key: &str| env::var(key).map_err(|_| Error::invalid(format!("{key} is required")));
        let json_env = |key: &str, default: &str| -> Result<Value> {
            serde_json::from_str(&env::var(key).unwrap_or_else(|_| default.into()))
                .map_err(|_| Error::invalid(format!("{key} must contain valid JSON")))
        };
        let result = Self {
            common,
            image: value("SANDBOX_IMAGE")?,
            gateway_image: value("SANDBOX_GATEWAY_IMAGE")?,
            namespace: value("NAMESPACE")?,
            pod_name: value("POD_NAME")?,
            pod_uid: value("POD_UID")?,
            memory_mb: positive("SANDBOX_MEMORY_MB", 1024, 262144)?,
            cpus: positive("SANDBOX_CPUS", 1, 128)?,
            tmp_mb: positive("SANDBOX_TMP_MB", 256, 65536)?,
            runtime_class: env::var("SANDBOX_RUNTIME_CLASS").unwrap_or_else(|_| "runsc".into()),
            key_id: value("BACKEND_KID")?,
            node_selector: json_env("SANDBOX_NODE_SELECTOR", "{}")?,
            tolerations: json_env("SANDBOX_TOLERATIONS", "[]")?,
            pull_secrets: json_env("SANDBOX_IMAGE_PULL_SECRETS", "[]")?,
            app_name: env::var("APP_NAME").unwrap_or_else(|_| "flow-like".into()),
            kubernetes_host: value("KUBERNETES_SERVICE_HOST")?,
            kubernetes_port: positive("KUBERNETES_SERVICE_PORT_HTTPS", 443, 65535)? as u16,
        };
        let digest = regex::Regex::new(r"^[A-Za-z0-9][A-Za-z0-9._:/-]*@sha256:[a-f0-9]{64}$")
            .expect("static image expression");
        let dns = regex::Regex::new(r"^[a-z0-9](?:[a-z0-9.-]{0,61}[a-z0-9])?$")
            .expect("static DNS expression");
        if !digest.is_match(&result.image) || !digest.is_match(&result.gateway_image) {
            return Err(Error::invalid(
                "Sandbox images require immutable sha256 digests",
            ));
        }
        if !dns.is_match(&result.common.installation)
            || result.common.warm_pool_size > result.common.capacity
            || result.common.warm_idle_seconds < 60
        {
            return Err(Error::invalid(
                "Invalid installation name, warm reserve or slot age",
            ));
        }
        if result.runtime_class.is_empty()
            || matches!(result.runtime_class.as_str(), "runc" | "default")
        {
            return Err(Error::invalid(
                "A configured gVisor RuntimeClass is required",
            ));
        }
        if !result.node_selector.is_object()
            || !result.tolerations.is_array()
            || !result.pull_secrets.is_array()
        {
            return Err(Error::invalid("Invalid sandbox scheduling configuration"));
        }
        Ok(result)
    }

    fn execution_budget(&self) -> u64 {
        self.common.timeout + self.common.startup_grace + self.common.terminal_grace
    }
    fn marker_name(&self, run_id: &str) -> String {
        format!(
            "flow-cancel-{:x}",
            Sha256::digest(format!("{}\0{run_id}", self.common.installation).as_bytes())
        )[..60]
            .to_owned()
    }
}

pub(super) fn now() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
}
fn run_hash(run_id: &str) -> String {
    format!("{:x}", Sha256::digest(run_id.as_bytes()))[..48].to_owned()
}
fn endpoint(ip: &str, port: u16) -> String {
    if ip.contains(':') {
        format!("http://[{ip}]:{port}")
    } else {
        format!("http://{ip}:{port}")
    }
}
fn transport_error(_: TransportError) -> Error {
    Error::internal("Kubernetes supervision request failed")
}

async fn send_with_header_deadline(
    request: reqwest::RequestBuilder,
    remaining: Duration,
) -> Result<reqwest::Response> {
    tokio::time::timeout(remaining, request.send())
        .await
        .map_err(|_| Error::internal("Execution admission exceeded its startup budget"))?
        .map_err(|_| Error::internal("Execution transport failed"))
}

struct Slot {
    name: String,
    runner_token: String,
    gateway_token: String,
    born: f64,
    runner_ip: String,
    gateway_ip: String,
    runner_started: f64,
    gateway_started: f64,
    cleanup: AsyncMutex<bool>,
}

impl Slot {
    fn new() -> Self {
        Self {
            name: format!("flow-slot-{}", Uuid::new_v4().simple()),
            runner_token: format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple()),
            gateway_token: format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple()),
            born: now(),
            runner_ip: String::new(),
            gateway_ip: String::new(),
            runner_started: 0.0,
            gateway_started: 0.0,
            cleanup: AsyncMutex::new(false),
        }
    }
}

#[derive(Default)]
struct State {
    warm: VecDeque<Arc<Slot>>,
    active: HashMap<String, Arc<Slot>>,
    creating: usize,
    reserving: usize,
    retiring: usize,
    warm_failures: u64,
    assignment_seconds: f64,
    assignments: u64,
}

struct Manager {
    config: Config,
    kube: Arc<dyn KubeApi>,
    claims: Arc<dyn ClaimStore>,
    client: Client,
    state: Mutex<State>,
    capacity: Arc<Semaphore>,
    ready: AtomicBool,
    stopping: CancellationToken,
    changed: Notify,
    owner: Weak<Self>,
}

pub async fn from_env(common: Arc<CommonConfig>) -> Result<Arc<dyn Backend>> {
    let config = Config::from_env(common)?;
    let kube = Arc::new(
        Kube::new(
            &config.namespace,
            &config.kubernetes_host,
            config.kubernetes_port,
        )
        .await
        .map_err(transport_error)?,
    );
    let claims = Claims::new(
        &secret("REDIS_URL")?,
        &config.namespace,
        &config.common.installation,
        86400 + config.common.budget() + 120,
    )
    .map_err(|_| {
        Error::invalid(
            "REDIS_URL must be authenticated redis:// or rediss:// without query overrides",
        )
    })?;
    Ok(Manager::new(config, kube, claims)?)
}

impl Manager {
    fn new(
        config: Config,
        kube: Arc<dyn KubeApi>,
        claims: Arc<dyn ClaimStore>,
    ) -> Result<Arc<Self>> {
        let client = Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .retry(reqwest::retry::never())
            .pool_idle_timeout(Duration::from_secs(30))
            .pool_max_idle_per_host(2)
            .connect_timeout(Duration::from_secs(5))
            .build()?;
        Ok(Arc::new_cyclic(|owner| Self {
            capacity: Arc::new(Semaphore::new(config.common.capacity)),
            config,
            kube,
            claims,
            client,
            state: Mutex::new(State::default()),
            ready: AtomicBool::new(false),
            stopping: CancellationToken::new(),
            changed: Notify::new(),
            owner: owner.clone(),
        }))
    }

    fn fail_closed(&self) {
        self.ready.store(false, Ordering::Release);
        self.stopping.cancel();
        self.changed.notify_waiters();
    }

    async fn wait_ready(&self, name: &str) -> Result<(String, f64)> {
        let until = Instant::now() + Duration::from_secs(180);
        loop {
            if self.stopping.is_cancelled() || Instant::now() >= until {
                return Err(Error::internal("Warm Pod did not become ready"));
            }
            if let Some(pod) = self.kube.get("pods", name).await.map_err(transport_error)? {
                if matches!(
                    pod["status"]["phase"].as_str(),
                    Some("Failed" | "Succeeded")
                ) {
                    return Err(Error::internal("Warm Pod exited before assignment"));
                }
                let ready = pod["status"]["conditions"].as_array().is_some_and(|items| {
                    items
                        .iter()
                        .any(|item| item["type"] == "Ready" && item["status"] == "True")
                });
                if ready {
                    let ip = pod["status"]["podIP"]
                        .as_str()
                        .ok_or_else(|| Error::internal("Warm Pod has no address"))?;
                    let started = pod_start(&pod)
                        .ok_or_else(|| Error::internal("Warm Pod has no start time"))?;
                    return Ok((ip.into(), started));
                }
            }
            tokio::select! { _ = self.stopping.cancelled()=>return Err(Error::Unavailable), _ = tokio::time::sleep(Duration::from_millis(250))=>{} }
        }
    }

    async fn create_slot(&self) -> Result<Arc<Slot>> {
        let mut slot = Slot::new();
        let result = async {
            // Both restrictive policies exist before either Pod can be scheduled.
            for gateway in [false, true] {
                self.kube
                    .request(
                        Method::POST,
                        "networkpolicies",
                        None,
                        Some(self.config.policy(&slot, gateway)),
                        &[],
                    )
                    .await
                    .map_err(transport_error)?;
            }
            self.kube
                .request(
                    Method::POST,
                    "pods",
                    None,
                    Some(self.config.pod(&slot, true)),
                    &[],
                )
                .await
                .map_err(transport_error)?;
            (slot.gateway_ip, slot.gateway_started) =
                self.wait_ready(&format!("{}-gateway", slot.name)).await?;
            self.kube
                .request(
                    Method::POST,
                    "pods",
                    None,
                    Some(self.config.pod(&slot, false)),
                    &[],
                )
                .await
                .map_err(transport_error)?;
            (slot.runner_ip, slot.runner_started) =
                self.wait_ready(&format!("{}-runner", slot.name)).await?;
            Ok(())
        }
        .await;
        if let Err(error) = result {
            self.discard(&slot).await?;
            return Err(error);
        }
        Ok(Arc::new(slot))
    }

    async fn refill(self: Arc<Self>) {
        while !self.stopping.is_cancelled() {
            let needed = {
                let mut state = self.state.lock().expect("manager state");
                if state.warm.len() + state.creating + state.retiring + state.reserving
                    < self.config.common.warm_pool_size
                    && state.creating < self.config.common.warm_create_concurrency
                {
                    state.creating += 1;
                    true
                } else {
                    false
                }
            };
            if needed {
                let manager = self.clone();
                tokio::spawn(async move {
                    match manager.create_slot().await {
                        Ok(slot) => {
                            let retire = {
                                let mut state = manager.state.lock().expect("manager state");
                                if manager.stopping.is_cancelled() {
                                    true
                                } else {
                                    state.warm.push_back(slot.clone());
                                    false
                                }
                            };
                            if retire {
                                let _ = manager.discard(&slot).await;
                            }
                        }
                        Err(_) => {
                            manager.state.lock().expect("manager state").warm_failures += 1;
                            tracing::warn!(
                                "Warm slot initialization failed; inspect execution Pod events"
                            );
                            tokio::time::sleep(Duration::from_secs(2)).await;
                        }
                    }
                    manager.state.lock().expect("manager state").creating -= 1;
                    manager.changed.notify_one();
                });
                continue;
            }
            tokio::select! {_ = self.stopping.cancelled()=>break,_ = self.changed.notified()=>{},_ = tokio::time::sleep(Duration::from_secs(1))=>{}}
        }
    }

    async fn sweep(self: Arc<Self>) {
        loop {
            tokio::select! {_ = self.stopping.cancelled()=>return,_ = tokio::time::sleep(Duration::from_secs(5))=>{}}
            if self.sweep_once().await.is_err() {
                self.fail_closed();
                return;
            }
        }
    }

    async fn sweep_once(&self) -> Result<()> {
        self.claims
            .ping()
            .await
            .map_err(|_| Error::internal("Execution assignment registry unavailable"))?;
        let expired = {
            let mut state = self.state.lock().expect("manager state");
            let mut expired = Vec::new();
            state.warm.retain(|slot| {
                if now() - slot.born >= (self.config.common.warm_idle_seconds - 30) as f64 {
                    expired.push(slot.clone());
                    false
                } else {
                    true
                }
            });
            state.retiring += expired.len();
            expired
        };
        for slot in expired {
            let result = self.discard(&slot).await;
            self.state.lock().expect("manager state").retiring -= 1;
            self.changed.notify_one();
            result?;
        }
        let selector = format!("{INSTALLATION}={}", self.config.common.installation);
        for pod in self
            .kube
            .list("pods", &format!("{selector},{MANAGER}"))
            .await
            .map_err(transport_error)?
        {
            if expired_object(&pod) {
                self.finish_pod(
                    pod,
                    Instant::now() + Duration::from_secs(self.config.common.cleanup_timeout),
                )
                .await?;
            }
        }
        for marker in self
            .kube
            .list(
                "configmaps",
                &format!("{selector},{COMPONENT}=execution-cancellation"),
            )
            .await
            .map_err(transport_error)?
        {
            if expired_object(&marker) {
                self.kube
                    .delete("configmaps", &marker)
                    .await
                    .map_err(transport_error)?;
            }
        }
        Ok(())
    }

    async fn request_gateway(&self, slot: &Slot, path: &str, policy: Option<Value>) -> Result<()> {
        let mut request = self
            .client
            .post(format!("{}{path}", endpoint(&slot.gateway_ip, 9001)))
            .header("X-Gateway-Token", &slot.gateway_token)
            .timeout(Duration::from_secs(10));
        if let Some(policy) = policy {
            request = request.json(&policy);
        } else {
            request = request.body("");
        }
        let response = request
            .send()
            .await
            .map_err(|_| Error::internal("Execution gateway unavailable"))?;
        if !matches!(response.status().as_u16(), 200 | 204) {
            return Err(Error::internal("Gateway refused execution policy"));
        }
        Ok(())
    }

    async fn cancelled(&self, run_id: &str) -> Result<bool> {
        Ok(self
            .kube
            .get("configmaps", &self.config.marker_name(run_id))
            .await
            .map_err(transport_error)?
            .is_some())
    }

    async fn execute_slot(
        &self,
        slot: &Slot,
        payload: Dispatch,
        mode: Mode,
        events: EventSink,
    ) -> Result<Value> {
        let started = Instant::now();
        let admitted = now();
        let deadline = admitted + self.config.execution_budget() as f64;
        let common = &self.config.common;
        tokio::time::timeout(Duration::from_secs(common.startup_grace),async {
            let gateway_name=format!("{}-gateway",slot.name);
            let runner_name=format!("{}-runner",slot.name);
            // The two Pod bindings are independent. Wait for both replies even
            // when one fails, then clean up the complete slot on any failure.
            let (gateway,runner)=tokio::join!(
                self.kube.request(Method::PATCH,"pods",Some(&gateway_name),Some(self.config.bind(&payload.run_id,deadline,slot.gateway_started)),&[]),
                self.kube.request(Method::PATCH,"pods",Some(&runner_name),Some(self.config.bind(&payload.run_id,deadline,slot.runner_started)),&[]),
            );
            gateway.map_err(transport_error)?;
            runner.map_err(transport_error)?;
            if self.cancelled(&payload.run_id).await? { return Err(Error::Cancelled); }
            self.request_gateway(slot,"/configure",Some(json!({"callback_url":common.callback_url,"object_store_url":common.object_store_url,
                "app_id":payload.app_id,"run_id":payload.run_id,"executor_jwt":payload.executor_jwt,"deadline":deadline,
                "buckets":common.buckets,"allowed_https_hosts":common.allowed_https_hosts,"object_store_tls_gateway":common.object_store_tls_gateway}))).await?;
            if self.cancelled(&payload.run_id).await? { return Err(Error::Cancelled); }
            Ok(())
        }).await.map_err(|_|Error::internal("Execution admission exceeded its startup budget"))??;
        let request = self
            .client
            .post(format!("{}/execute", endpoint(&slot.runner_ip, 8080)))
            .header("X-Slot-Token", &slot.runner_token)
            .timeout(Duration::from_secs(self.config.execution_budget()))
            .json(&json!({"mode":mode,"payload":payload}));
        let remaining = Duration::from_secs(common.startup_grace)
            .checked_sub(started.elapsed())
            .ok_or_else(|| Error::internal("Execution admission exceeded its startup budget"))?;
        // Bound admission through the slot's HTTP acknowledgement. The request
        // itself retains the execution budget while its NDJSON body is read.
        let response = send_with_header_deadline(request, remaining).await?;
        if response.status() != reqwest::StatusCode::OK {
            return Err(Error::internal("Single-use runner refused dispatch"));
        }
        {
            let mut state = self.state.lock().expect("manager state");
            state.assignment_seconds += now() - admitted;
            state.assignments += 1;
        }
        let stream = response.bytes_stream().map_err(std::io::Error::other);
        let mut reader = BufReader::new(StreamReader::new(stream));
        let mut total = 0usize;
        let mut completed = None;
        let mut exited = false;
        let mut heartbeat = tokio::time::interval(Duration::from_secs(10));
        heartbeat.tick().await;
        loop {
            // Keep one read future alive across heartbeat ticks: cancelling a
            // partially consumed line would corrupt NDJSON framing.
            let line = read_line(&mut reader, MAX_EVENT);
            tokio::pin!(line);
            let raw = loop {
                tokio::select! {
                    result=&mut line=>break result?,
                    _=heartbeat.tick()=>events.send(json!({"event_id":Uuid::new_v4().simple().to_string(),"timestamp":{"secs_since_epoch":now() as u64,"nanos_since_epoch":0},"event_type":"heartbeat","payload":{}})).await,
                }
            };
            if raw.is_empty() {
                break;
            }
            total += raw.len();
            if total > MAX_OUTPUT {
                return Err(Error::internal("Execution exceeded its output budget"));
            }
            let event: Value = serde_json::from_slice(&raw)
                .map_err(|_| Error::internal("Execution emitted invalid NDJSON"))?;
            if !event.is_object() {
                return Err(Error::internal("Execution emitted invalid event"));
            }
            if let Some(exit) = event.get("slot_exit_code") {
                exited = exit.as_i64() == Some(0);
                continue;
            }
            if event["event_type"] == "completed" {
                completed = event.get("payload").cloned();
            }
            events.send(event).await;
        }
        match completed {
            Some(value) if value.is_object() && exited => Ok(value),
            _ => Err(Error::internal(
                "Execution ended without terminal acknowledgement",
            )),
        }
    }

    async fn finish_pod(&self, mut pod: Value, until: Instant) -> Result<()> {
        if !terminal(&pod) {
            let name = pod["metadata"]["name"]
                .as_str()
                .ok_or_else(|| Error::internal("Pod identity missing"))?
                .to_owned();
            if let Some(started) = pod_start(&pod) {
                let requested = ((now() - started).max(0.0) as u64) + 1;
                let deadline = pod["spec"]["activeDeadlineSeconds"]
                    .as_u64()
                    .unwrap_or(requested)
                    .min(requested);
                match self
                    .kube
                    .request(
                        Method::PATCH,
                        "pods",
                        Some(&name),
                        Some(json!({"spec":{"activeDeadlineSeconds":deadline}})),
                        &[],
                    )
                    .await
                {
                    Err(TransportError::Status(404)) => return Ok(()),
                    Err(error) => return Err(transport_error(error)),
                    Ok(_) => {}
                }
            } else {
                self.kube
                    .delete("pods", &pod)
                    .await
                    .map_err(transport_error)?;
            }
            loop {
                if Instant::now() >= until {
                    return Err(Error::internal(
                        "Kubelet has not confirmed sandbox termination",
                    ));
                }
                match self
                    .kube
                    .get("pods", &name)
                    .await
                    .map_err(transport_error)?
                {
                    None => return Ok(()),
                    Some(current) if terminal(&current) => {
                        pod = current;
                        break;
                    }
                    Some(_) => {}
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        }
        self.kube
            .delete("pods", &pod)
            .await
            .map_err(transport_error)
    }

    async fn discard(&self, slot: &Slot) -> Result<()> {
        let mut discarded = slot.cleanup.lock().await;
        if *discarded {
            return Ok(());
        }
        let result = tokio::time::timeout(
            Duration::from_secs(self.config.common.cleanup_timeout),
            async {
                let until =
                    Instant::now() + Duration::from_secs(self.config.common.cleanup_timeout);
                // Preserve restrictive policies until the runner's termination is confirmed.
                for suffix in ["-runner", "-gateway"] {
                    if let Some(pod) = self
                        .kube
                        .get("pods", &format!("{}{suffix}", slot.name))
                        .await
                        .map_err(transport_error)?
                    {
                        self.finish_pod(pod, until).await?;
                    }
                }
                for name in [&slot.name, &format!("{}-gateway", slot.name)] {
                    if let Some(policy) = self
                        .kube
                        .get("networkpolicies", name)
                        .await
                        .map_err(transport_error)?
                    {
                        self.kube
                            .delete("networkpolicies", &policy)
                            .await
                            .map_err(transport_error)?;
                    }
                }
                Ok(())
            },
        )
        .await
        .unwrap_or_else(|_| Err(Error::internal("Sandbox cleanup exceeded its time budget")));
        if result.is_ok() {
            *discarded = true;
        } else {
            self.fail_closed();
        }
        self.state
            .lock()
            .expect("manager state")
            .active
            .remove(&slot.name);
        self.changed.notify_one();
        result
    }
}

fn pod_start(pod: &Value) -> Option<f64> {
    chrono::DateTime::parse_from_rfc3339(pod["status"]["startTime"].as_str()?)
        .ok()
        .map(|value| value.timestamp_millis() as f64 / 1000.0)
}
fn terminal(pod: &Value) -> bool {
    matches!(
        pod["status"]["phase"].as_str(),
        Some("Failed" | "Succeeded")
    )
}
fn expired_object(object: &Value) -> bool {
    object["metadata"]["annotations"][DEADLINE]
        .as_str()
        .and_then(|value| value.parse::<f64>().ok())
        .is_some_and(|value| value <= now())
}

pub(super) async fn read_line<R: AsyncBufRead + Unpin>(
    reader: &mut R,
    maximum: usize,
) -> Result<Vec<u8>> {
    let mut line = Vec::new();
    loop {
        let buffer = reader.fill_buf().await?;
        if buffer.is_empty() {
            return Ok(line);
        }
        let consumed = buffer
            .iter()
            .position(|byte| *byte == b'\n')
            .map_or(buffer.len(), |index| index + 1);
        if line.len() + consumed > maximum {
            return Err(Error::internal("Execution event exceeds its size limit"));
        }
        let end = buffer[consumed - 1] == b'\n';
        line.extend_from_slice(&buffer[..consumed]);
        reader.consume(consumed);
        if end {
            return Ok(line);
        }
    }
}

struct Assigned {
    manager: Arc<Manager>,
    slot: Option<Arc<Slot>>,
    permit: Option<OwnedSemaphorePermit>,
    claim_pending: bool,
}

impl Drop for Assigned {
    fn drop(&mut self) {
        if self.claim_pending {
            self.manager.state.lock().expect("manager state").reserving -= 1;
        }
        if let Some(slot) = self.slot.take() {
            let manager = self.manager.clone();
            let permit = self.permit.take();
            if let Ok(runtime) = tokio::runtime::Handle::try_current() {
                runtime.spawn(async move {
                    let _ = manager.discard(&slot).await;
                    drop(permit);
                });
            }
        }
    }
}

#[async_trait]
impl Reservation for Assigned {
    async fn execute(
        mut self: Box<Self>,
        payload: Dispatch,
        mode: Mode,
        events: EventSink,
    ) -> Result<Value> {
        let slot = self.slot.as_ref().expect("assigned slot").clone();
        let result = tokio::time::timeout(
            Duration::from_secs(self.manager.config.execution_budget()),
            self.manager.execute_slot(&slot, payload, mode, events),
        )
        .await
        .unwrap_or_else(|_| Err(Error::internal("Execution exceeded its time budget")));
        let cleanup = self.manager.discard(&slot).await;
        self.slot = None;
        cleanup?;
        result
    }
}

#[async_trait]
impl Backend for Manager {
    fn ready(&self) -> bool {
        self.ready.load(Ordering::Acquire) && !self.stopping.is_cancelled()
    }
    fn metrics(&self) -> String {
        let state = self.state.lock().expect("manager state");
        format!(
            "executor_warm_slots {}\nexecutor_warm_target {}\nexecutor_warm_initializing {}\nexecutor_warm_failures_total {}\nexecutor_warm_retiring {}\nexecutor_assignment_seconds_sum {}\nexecutor_assignment_seconds_count {}\nexecutor_active {}\n",
            state.warm.len(),
            self.config.common.warm_pool_size,
            state.creating,
            state.warm_failures,
            state.retiring,
            state.assignment_seconds,
            state.assignments,
            state.active.len()
        )
    }
    async fn prepare(self: Arc<Self>) -> Result<()> {
        let owner = self
            .kube
            .get("pods", &self.config.pod_name)
            .await
            .map_err(transport_error)?;
        if owner
            .as_ref()
            .and_then(|pod| pod["metadata"]["uid"].as_str())
            != Some(self.config.pod_uid.as_str())
        {
            return Err(Error::internal(
                "Manager Pod ownership could not be verified",
            ));
        }
        self.claims
            .ping()
            .await
            .map_err(|_| Error::internal("Execution assignment registry unavailable"))?;
        self.ready.store(true, Ordering::Release);
        tokio::spawn(self.clone().refill());
        tokio::spawn(self.sweep());
        Ok(())
    }
    async fn reserve(&self, payload: &Dispatch) -> Result<Box<dyn Reservation>> {
        if !self.ready() {
            return Err(Error::Unavailable);
        }
        let permit = self
            .capacity
            .clone()
            .try_acquire_owned()
            .map_err(|_| Error::NoCapacity)?;
        let slot = {
            let mut state = self.state.lock().expect("manager state");
            let index = state
                .warm
                .iter()
                .position(|slot| {
                    now() - slot.born < (self.config.common.warm_idle_seconds - 30) as f64
                })
                .ok_or(Error::NoCapacity)?;
            let slot = state.warm.remove(index).expect("selected warm slot");
            state.reserving += 1;
            state.active.insert(slot.name.clone(), slot.clone());
            slot
        };
        let mut reservation = Box::new(Assigned {
            manager: self.owner.upgrade().ok_or(Error::Unavailable)?,
            slot: Some(slot.clone()),
            permit: Some(permit),
            claim_pending: true,
        });
        let claim = self.claims.claim(&payload.run_id, &slot.name).await;
        self.state.lock().expect("manager state").reserving -= 1;
        reservation.claim_pending = false;
        match claim {
            Ok(true) => {}
            other => {
                // No tenant data entered this pristine slot. A lost claim reply
                // still leaves its Redis tombstone retained against replay.
                let mut state = self.state.lock().expect("manager state");
                if !self.stopping.is_cancelled() {
                    state.active.remove(&slot.name);
                    state.warm.push_back(slot);
                    reservation.slot = None;
                }
                drop(state);
                return Err(match other {
                    Ok(false) => Error::Cancelled,
                    _ => Error::internal("Execution assignment could not be confirmed"),
                });
            }
        }
        self.changed.notify_one();
        Ok(reservation)
    }
    async fn cancel(&self, run_id: &str) -> Result<Value> {
        if !safe_id(run_id) {
            return Err(Error::invalid("Invalid run identifier"));
        }
        let result = async {
            match self
                .kube
                .request(
                    Method::POST,
                    "configmaps",
                    None,
                    Some(self.config.cancellation(run_id)),
                    &[],
                )
                .await
            {
                Ok(_) | Err(TransportError::Status(409)) => {}
                Err(error) => return Err(transport_error(error)),
            }
            let selector = format!(
                "{INSTALLATION}={},{RUN}={}",
                self.config.common.installation,
                run_hash(run_id)
            );
            let until = Instant::now() + Duration::from_secs(self.config.common.cleanup_timeout);
            for pod in self
                .kube
                .list("pods", &selector)
                .await
                .map_err(transport_error)?
            {
                if pod["metadata"]["annotations"]["flow-like.io/run-id"] == run_id {
                    self.finish_pod(pod, until).await?;
                }
            }
            Ok(json!({"run_id":run_id,"terminated":true}))
        }
        .await;
        if result.is_err() {
            self.fail_closed();
        }
        result
    }
    async fn shutdown(&self) -> Result<()> {
        self.fail_closed();
        let until = Instant::now() + Duration::from_secs(self.config.common.budget() + 60);
        let unused = {
            self.state
                .lock()
                .expect("manager state")
                .warm
                .drain(..)
                .collect::<Vec<_>>()
        };
        for slot in unused {
            self.discard(&slot).await?;
        }
        loop {
            let done = {
                let state = self.state.lock().expect("manager state");
                state.active.is_empty() && state.creating == 0
            };
            if done {
                return Ok(());
            }
            if Instant::now() >= until {
                return Err(Error::internal("Sandbox drain exceeded its time budget"));
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }
}

#[cfg(test)]
mod tests;
