use crate::{
    bit::{Bit, BitTypes},
    models::{ModelMeta, local_utils::ensure_local_weights},
    state::FlowLikeState,
    utils::execute::{LlamaServerRuntime, failed_to_load},
};
use flow_like_model_provider::{
    llm::{ModelConstructor, ModelLogic, llamacpp::LlamaCppClient},
    provider::ModelProvider,
};
use flow_like_storage::files::store::FlowLikeStore;
use flow_like_types::{
    Result, Value as JsonValue, reqwest,
    tokio::time::{Instant, sleep},
};
use parking_lot::Mutex;
use portpicker::pick_unused_port;
use std::{
    collections::{HashSet, VecDeque},
    io::{BufRead, BufReader, Read},
    net::{Ipv4Addr, TcpListener},
    path::{Path, PathBuf},
    process::{Child, ExitStatus, Stdio},
    sync::{
        Arc, LazyLock, Weak,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::Duration,
};

use super::{DEFAULT_MAX_CONTEXT_SIZE, ExecutionSettings, LOCAL_ENGINE_LOADS};

const READY_TIMEOUT: Duration = Duration::from_secs(60);
const READY_POLL_INTERVAL: Duration = Duration::from_millis(250);
const HEALTH_REQUEST_TIMEOUT: Duration = Duration::from_secs(5);
const PORT_RETRIES: usize = 3;
const PORT_PICKS: usize = 16;
const STDERR_TAIL_LINES: usize = 12;
const OUTPUT_SETTLE_TIMEOUT: Duration = Duration::from_millis(500);
const CACHE_REUSE_TOKENS: &str = "256";

pub struct LocalModel {
    bit: Bit,
    client: LlamaCppClient,
    default_model: Option<String>,
    pub port: u16,
    _server: LlamaServerProcess,
    /// Every client holds the model, so the factory sees it in use and keeps its one server.
    this: Weak<LocalModel>,
}

/// Owns the llama-server child; dropping it kills and reaps the process, including when startup
/// fails or is cancelled before a `LocalModel` exists. The `LocalModel` owns it, so the server
/// stops once neither the factory nor any client holds the model.
#[derive(Default)]
struct LlamaServerProcess {
    child: Option<Child>,
    output: Arc<ServerOutput>,
}

impl LlamaServerProcess {
    fn attach(&mut self, mut child: Child) -> &mut Child {
        self.stop();
        self.output = Arc::default();
        if let Some(stdout) = child.stdout.take() {
            spawn_reader(
                "llama-server-stdout",
                Box::new(move || {
                    drain_lines(stdout, |line| tracing::debug!(%line, "[LLM] stdout"));
                }),
            );
        }
        if let Some(stderr) = child.stderr.take() {
            let output = self.output.clone();
            spawn_reader(
                "llama-server-stderr",
                Box::new(move || {
                    drain_lines(stderr, |line| {
                        eprintln!("[LLM ERROR] stderr: {}", line);
                        output.push(line);
                    });
                    output.closed.store(true, Ordering::Release);
                }),
            );
        }
        self.child.insert(child)
    }

    fn stop(&mut self) {
        let Some(mut child) = self.child.take() else {
            return;
        };
        let pid = child.id();
        if let Err(error) = child.kill() {
            tracing::warn!(pid, %error, "Failed to kill local model server");
        }
        match child.wait() {
            Ok(status) => tracing::debug!(pid, %status, "Local model server stopped"),
            Err(error) => tracing::warn!(pid, %error, "Failed to reap local model server"),
        }
    }

    fn exit_status(&mut self) -> Option<ExitStatus> {
        self.child.as_mut()?.try_wait().ok().flatten()
    }

    /// Waits for `/health`, failing as soon as the process exits. Another process can answer on
    /// the port, so health only counts while this server still runs.
    async fn wait_until_ready(&mut self, port: u16) -> std::result::Result<(), StartFailure> {
        let client = reqwest::Client::new();
        let deadline = Instant::now() + READY_TIMEOUT;
        loop {
            if let Some(status) = self.exit_status() {
                return Err(StartFailure::Exited(status));
            }
            if is_healthy(&client, port).await {
                return self
                    .exit_status()
                    .map_or(Ok(()), |status| Err(StartFailure::Exited(status)));
            }
            if Instant::now() >= deadline {
                return Err(StartFailure::TimedOut);
            }
            sleep(READY_POLL_INTERVAL).await;
        }
    }
}

impl Drop for LlamaServerProcess {
    fn drop(&mut self) {
        self.stop();
    }
}

/// The server's last stderr lines, for start errors: release builds have no console for them.
#[derive(Default)]
struct ServerOutput {
    tail: Mutex<VecDeque<String>>,
    closed: AtomicBool,
}

impl ServerOutput {
    fn push(&self, line: String) {
        let mut tail = self.tail.lock();
        if tail.len() == STDERR_TAIL_LINES {
            tail.pop_front();
        }
        tail.push_back(line);
    }

    /// The tail once the reader drained a stopped server's stderr, or after a short wait.
    async fn settled(&self) -> String {
        let deadline = Instant::now() + OUTPUT_SETTLE_TIMEOUT;
        while !self.closed.load(Ordering::Acquire) && Instant::now() < deadline {
            sleep(Duration::from_millis(10)).await;
        }
        Vec::from(self.tail.lock().clone()).join("\n")
    }
}

/// Server pipes are drained on threads: a blocking read in a Tokio task would hold a runtime
/// worker for as long as the server runs.
fn spawn_reader(name: &str, read: Box<dyn FnOnce() + Send>) {
    if let Err(error) = thread::Builder::new().name(name.to_owned()).spawn(read) {
        tracing::warn!(%error, name, "Failed to start a local model server output reader");
    }
}

fn drain_lines(reader: impl Read, mut on_line: impl FnMut(String)) {
    let mut reader = BufReader::new(reader);
    let mut line = Vec::new();
    while matches!(reader.read_until(b'\n', &mut line), Ok(read) if read > 0) {
        on_line(
            String::from_utf8_lossy(&line)
                .trim_end_matches(['\r', '\n'])
                .to_owned(),
        );
        line.clear();
    }
}

async fn is_healthy(client: &reqwest::Client, port: u16) -> bool {
    client
        .get(format!("http://127.0.0.1:{port}/health"))
        .timeout(HEALTH_REQUEST_TIMEOUT)
        .send()
        .await
        .is_ok_and(|response| response.status().is_success())
}

/// Ports of servers that are still starting. Parallel cold starts could otherwise pick the same
/// free port, and the start that loses the bind would take the other server's health as its own.
static STARTING_PORTS: LazyLock<Mutex<HashSet<u16>>> = LazyLock::new(Default::default);

struct PortReservation {
    port: u16,
}

impl PortReservation {
    fn pick(mut candidate: impl FnMut() -> Option<u16>) -> Result<Self> {
        let mut starting = STARTING_PORTS.lock();
        (0..PORT_PICKS)
            .filter_map(|_| candidate())
            .find(|port| starting.insert(*port))
            .map(|port| Self { port })
            .ok_or_else(|| {
                flow_like_types::anyhow!(
                    "Found no free local port for llama-server in {PORT_PICKS} picks"
                )
            })
    }
}

impl Drop for PortReservation {
    fn drop(&mut self) {
        STARTING_PORTS.lock().remove(&self.port);
    }
}

/// portpicker probes the wildcard address with SO_REUSEADDR, which macOS grants while another
/// server listens on 127.0.0.1, so a running llama-server's port must be ruled out on loopback.
fn free_loopback_port() -> Option<u16> {
    pick_unused_port().filter(|port| !port_in_use(*port))
}

fn port_in_use(port: u16) -> bool {
    TcpListener::bind((Ipv4Addr::LOCALHOST, port)).is_err()
}

enum StartFailure {
    Exited(ExitStatus),
    TimedOut,
}

#[derive(Debug, PartialEq, Eq)]
enum Retry {
    WithFallback,
    OnAnotherPort,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LlamaServerMode {
    Chat,
    SystemOne,
}

/// Everything a llama-server start needs besides its port and chat template.
struct ServerLaunch<'a> {
    runtime: &'a LlamaServerRuntime,
    gguf_path: &'a Path,
    context_length: u32,
    gpu_mode: bool,
    projection_path: Option<&'a str>,
    mode: LlamaServerMode,
}

impl ServerLaunch<'_> {
    fn spawn(
        &self,
        port: u16,
        template_override: &LlamaServerTemplateOverride,
        with_fallback: bool,
    ) -> Result<LlamaServerProcess> {
        let args = LocalModel::server_args(
            self.gguf_path,
            self.context_length,
            port,
            self.gpu_mode,
            self.projection_path,
            template_override,
            self.mode,
        );

        tracing::debug!(
            ?args,
            entrypoint = %self.runtime.entrypoint.display(),
            with_fallback,
            "Starting LLM Server"
        );

        let child = self
            .runtime
            .command(with_fallback)
            .args(&args)
            .stderr(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .map_err(|error| {
                flow_like_types::anyhow!(
                    "Failed to spawn llama-server at {}: {}",
                    self.runtime.entrypoint.display(),
                    error
                )
            })?;
        let mut server = LlamaServerProcess::default();
        server.attach(child);
        Ok(server)
    }

    /// Starts llama-server on a port reserved until it is healthy. A port another process took in
    /// the meantime is retried on a new one, and a server that cannot load its libraries is
    /// retried once with the runtime's fallback libraries.
    async fn start(
        &self,
        template_override: &LlamaServerTemplateOverride,
    ) -> Result<(LlamaServerProcess, u16)> {
        let mut with_fallback = false;
        let mut port_retries = 0;
        loop {
            let reservation = PortReservation::pick(free_loopback_port)?;
            let port = reservation.port;
            let mut server = self.spawn(port, template_override, with_fallback)?;
            let failure = match server.wait_until_ready(port).await {
                Ok(()) => return Ok((server, port)),
                Err(failure) => failure,
            };
            match self.retry_after(&failure, with_fallback, port_in_use(port)) {
                Some(Retry::WithFallback) => {
                    tracing::warn!(
                        entrypoint = %self.runtime.entrypoint.display(),
                        "llama-server could not load; retrying with the fallback libraries"
                    );
                    with_fallback = true;
                }
                Some(Retry::OnAnotherPort) if port_retries < PORT_RETRIES => {
                    tracing::warn!(port, "Another process took the llama-server port; retrying");
                    port_retries += 1;
                }
                _ => return Err(self.start_error(failure, server, port).await),
            }
        }
    }

    /// A model whose own template does not advertise tool use is restarted with chatml. No other
    /// local engine loads meanwhile, so `--fit on` sizes the offload against what others hold.
    async fn start_with_tool_support(
        &self,
        template_override: &LlamaServerTemplateOverride,
    ) -> Result<(LlamaServerProcess, u16)> {
        let _one_at_a_time = LOCAL_ENGINE_LOADS.lock().await;
        let (server, port) = self.start(template_override).await?;
        let should_probe_tool_template = self.mode == LlamaServerMode::Chat
            && self.projection_path.is_none()
            && template_override.chat_template.is_none()
            && template_override.chat_template_file.is_none();

        if !should_probe_tool_template
            || LocalModel::server_supports_tool_use(port)
                .await
                .unwrap_or(false)
        {
            return Ok((server, port));
        }

        tracing::warn!(
            "Local model template does not advertise tool support. Restarting llama-server with chatml fallback."
        );
        drop(server);

        let fallback_template = LlamaServerTemplateOverride {
            chat_template: Some("chatml".to_string()),
            chat_template_file: None,
        };
        self.start(&fallback_template).await
    }

    fn retry_after(
        &self,
        failure: &StartFailure,
        with_fallback: bool,
        port_taken: bool,
    ) -> Option<Retry> {
        let StartFailure::Exited(status) = failure else {
            return None;
        };
        if failed_to_load(status) && !with_fallback && self.runtime.fallback_dir.is_some() {
            return Some(Retry::WithFallback);
        }
        port_taken.then_some(Retry::OnAnotherPort)
    }

    async fn start_error(
        &self,
        failure: StartFailure,
        mut server: LlamaServerProcess,
        port: u16,
    ) -> flow_like_types::Error {
        server.stop();
        let output = server.output.settled().await;
        let entrypoint = self.runtime.entrypoint.display();
        match failure {
            StartFailure::Exited(status) => flow_like_types::anyhow!(
                "llama-server at {entrypoint} exited ({status}) before it served port {port}:\n{output}"
            ),
            StartFailure::TimedOut => flow_like_types::anyhow!(
                "llama-server at {entrypoint} did not answer on port {port} within {}s:\n{output}",
                READY_TIMEOUT.as_secs()
            ),
        }
    }
}

#[derive(Clone, Debug, Default)]
struct LlamaServerTemplateOverride {
    chat_template: Option<String>,
    chat_template_file: Option<String>,
}

fn provider_param_as_string(provider: &ModelProvider, key: &str) -> Option<String> {
    provider
        .params
        .as_ref()
        .and_then(|params| params.get(key))
        .and_then(|value| value.as_str())
        .map(str::to_owned)
}

fn resolve_template_override(provider: &ModelProvider) -> LlamaServerTemplateOverride {
    LlamaServerTemplateOverride {
        chat_template: provider_param_as_string(provider, "chat_template"),
        chat_template_file: provider_param_as_string(provider, "chat_template_file"),
    }
}

fn find_string_field(value: &JsonValue, key: &str) -> Option<String> {
    match value {
        JsonValue::Object(map) => {
            if let Some(string_value) = map.get(key).and_then(|value| value.as_str()) {
                return Some(string_value.to_owned());
            }

            map.values().find_map(|child| find_string_field(child, key))
        }
        JsonValue::Array(values) => values
            .iter()
            .find_map(|child| find_string_field(child, key)),
        _ => None,
    }
}

fn template_supports_tool_use(template: &str) -> bool {
    let template = template.to_lowercase();
    ["tool", "tool_call", "tool_calls", "function", "functions"]
        .iter()
        .any(|marker| template.contains(marker))
}

fn props_support_tool_use(props: &JsonValue) -> bool {
    find_string_field(props, "chat_template_tool_use")
        .is_some_and(|template| !template.trim().is_empty())
        || find_string_field(props, "chat_template")
            .is_some_and(|template| template_supports_tool_use(&template))
}

impl ModelMeta for LocalModel {
    fn get_bit(&self) -> Bit {
        self.bit.clone()
    }
}

#[flow_like_types::async_trait]
impl ModelLogic for LocalModel {
    async fn provider(&self) -> Result<ModelConstructor> {
        let this = self.this.upgrade().ok_or_else(|| {
            flow_like_types::anyhow!("The local model server of {} stopped", self.bit.id)
        })?;
        Ok(ModelConstructor {
            inner: Box::new(self.client.clone().with_keepalive(this)),
        })
    }

    async fn default_model(&self) -> Option<String> {
        self.default_model.clone()
    }
}

impl LocalModel {
    pub async fn check_health(port: &str) -> Result<bool> {
        let response = reqwest::get(format!("http://127.0.0.1:{}/health", port)).await?;

        if response.status().is_success() {
            Ok(true)
        } else {
            Err(flow_like_types::anyhow!(
                "Model is not healthy: {}",
                response.status()
            ))
        }
    }

    async fn server_supports_tool_use(port: u16) -> Result<bool> {
        let response = reqwest::get(format!("http://127.0.0.1:{port}/props")).await?;
        if !response.status().is_success() {
            return Ok(false);
        }

        let props = response.json::<JsonValue>().await?;
        Ok(props_support_tool_use(&props))
    }

    fn resolve_context_length(model_context_length: Option<u32>, max_context_size: usize) -> u32 {
        let max_context_size = if max_context_size == 0 {
            DEFAULT_MAX_CONTEXT_SIZE as u32
        } else {
            u32::try_from(max_context_size).unwrap_or(u32::MAX)
        };
        let model_context_length = model_context_length
            .filter(|context_length| *context_length > 0)
            .unwrap_or(max_context_size);

        std::cmp::min(model_context_length, max_context_size)
    }

    fn server_args(
        gguf_path: &Path,
        context_length: u32,
        port: u16,
        gpu_mode: bool,
        projection_path: Option<&str>,
        template_override: &LlamaServerTemplateOverride,
        mode: LlamaServerMode,
    ) -> Vec<String> {
        let mut args = vec![
            "--model".to_string(),
            gguf_path.to_string_lossy().into_owned(),
            "--ctx-size".to_string(),
            context_length.to_string(),
            "--host".to_string(),
            "127.0.0.1".to_string(),
            "--port".to_string(),
            port.to_string(),
            "--parallel".to_string(),
            "1".to_string(),
            "--no-webui".to_string(),
            // Auto probes support on CPU as well as GPU and falls back when unavailable.
            "--flash-attn".to_string(),
            "auto".to_string(),
        ];

        if gpu_mode {
            args.extend([
                "--n-gpu-layers".to_string(),
                "auto".to_string(),
                "--fit".to_string(),
                "on".to_string(),
            ]);
        } else {
            args.extend([
                "--device".to_string(),
                "none".to_string(),
                "--n-gpu-layers".to_string(),
                "0".to_string(),
            ]);
        }

        if let Some(projection_path) = projection_path {
            args.push("--mmproj".to_string());
            args.push(projection_path.to_string());
        }

        if mode == LlamaServerMode::SystemOne {
            // Laya and Clef need the whole prompt in one physical batch.
            args.extend([
                "--batch-size".to_string(),
                context_length.to_string(),
                "--ubatch-size".to_string(),
                context_length.to_string(),
            ]);
            return args;
        }

        args.push("--jinja".to_string());
        if projection_path.is_none() {
            // Reuse unchanged prompt chunks after an edit. Multimodal contexts cannot shift KV.
            args.extend(["--cache-reuse".to_string(), CACHE_REUSE_TOKENS.to_string()]);
        }

        if let Some(chat_template_file) = template_override.chat_template_file.as_ref() {
            args.push("--chat-template-file".to_string());
            args.push(chat_template_file.clone());
        } else if let Some(chat_template) = template_override.chat_template.as_ref() {
            args.push("--chat-template".to_string());
            args.push(chat_template.clone());
        }

        args
    }

    fn serving(
        bit: Bit,
        provider: &ModelProvider,
        server: LlamaServerProcess,
        port: u16,
    ) -> Arc<Self> {
        Arc::new_cyclic(|this| LocalModel {
            bit,
            client: LlamaCppClient::new(&format!("http://127.0.0.1:{port}")),
            default_model: provider.model_id.clone(),
            port,
            _server: server,
            this: this.clone(),
        })
    }

    /// The Bit's GGUF and its projector, if any, once their weights are in the local store.
    async fn local_files(
        bit: &Bit,
        app_state: &Arc<FlowLikeState>,
    ) -> Result<(PathBuf, Option<String>)> {
        let FlowLikeStore::Local(bit_store) = FlowLikeState::bit_store(app_state).await? else {
            return Err(flow_like_types::anyhow!("Only local store supported"));
        };

        let gguf_path = bit
            .to_path(&bit_store)
            .ok_or(flow_like_types::anyhow!("No model path"))?;
        let pack = bit.pack(app_state.clone()).await?;
        ensure_local_weights(&pack, app_state, bit.id.as_str(), "local model").await?;

        let projection_path = pack
            .bits
            .iter()
            .find(|b| b.bit_type == BitTypes::Projection)
            .and_then(|bit| bit.to_path(&bit_store))
            .map(|path| path.to_string_lossy().into_owned());
        Ok((gguf_path, projection_path))
    }

    pub async fn new(
        bit: &Bit,
        app_state: Arc<FlowLikeState>,
        execution_settings: &ExecutionSettings,
    ) -> flow_like_types::Result<Arc<LocalModel>> {
        let runtime = app_state.runtime_locator.installed_llama_server()?;
        let (gguf_path, projection_path) = Self::local_files(bit, &app_state).await?;
        let provider = bit
            .try_to_served_provider()
            .ok_or_else(|| flow_like_types::anyhow!("Failed to get provider from bit"))?;
        let mode = if bit.bit_type == BitTypes::SystemOne {
            LlamaServerMode::SystemOne
        } else {
            LlamaServerMode::Chat
        };
        let template_override = if mode == LlamaServerMode::Chat {
            resolve_template_override(&provider)
        } else {
            LlamaServerTemplateOverride::default()
        };

        let launch = ServerLaunch {
            runtime,
            gguf_path: &gguf_path,
            context_length: Self::resolve_context_length(
                bit.try_to_context_length(),
                execution_settings.max_context_size,
            ),
            gpu_mode: execution_settings.gpu_mode,
            projection_path: projection_path.as_deref(),
            mode,
        };

        tracing::debug!(?execution_settings, "Execution settings");

        let (server, port) = launch.start_with_tool_support(&template_override).await?;
        Ok(Self::serving(bit.clone(), &provider, server, port))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn arg_value<'a>(args: &'a [String], key: &str) -> Option<&'a str> {
        args.windows(2)
            .find(|window| window[0] == key)
            .map(|window| window[1].as_str())
    }

    #[cfg(unix)]
    fn sleeping_child() -> Child {
        std::process::Command::new("sleep")
            .arg("60")
            .spawn()
            .expect("spawn sleep")
    }

    /// True while the pid is still in the process table, including as an unreaped zombie.
    #[cfg(unix)]
    fn process_listed(pid: u32) -> bool {
        let output = std::process::Command::new("ps")
            .args(["-o", "stat=", "-p", &pid.to_string()])
            .output()
            .expect("run ps");
        !String::from_utf8_lossy(&output.stdout).trim().is_empty()
    }

    /// The child sleeps for 60s, so a stop that waited instead of killing fails this bound.
    #[cfg(unix)]
    fn assert_killed_promptly(started: std::time::Instant) {
        assert!(
            started.elapsed() < Duration::from_secs(30),
            "server was waited out instead of killed"
        );
    }

    #[cfg(unix)]
    fn provider() -> ModelProvider {
        ModelProvider {
            api_surface: None,
            provider_name: "Local".to_string(),
            model_id: Some("local-model".to_string()),
            version: None,
            params: None,
        }
    }

    #[cfg(unix)]
    fn runtime_in(dir: &Path, fallback: bool) -> LlamaServerRuntime {
        LlamaServerRuntime {
            entrypoint: dir.join("llama-server"),
            library_dir: dir.to_path_buf(),
            fallback_dir: fallback.then(|| dir.join("fallback")),
        }
    }

    #[cfg(unix)]
    fn launch_with(runtime: &LlamaServerRuntime) -> ServerLaunch<'_> {
        ServerLaunch {
            runtime,
            gguf_path: Path::new("/models/model.gguf"),
            context_length: 8192,
            gpu_mode: false,
            projection_path: None,
            mode: LlamaServerMode::Chat,
        }
    }

    /// A stand-in llama-server: a shell script that runs `script`.
    #[cfg(unix)]
    fn fake_server(dir: &Path, script: &str) -> LlamaServerRuntime {
        use std::os::unix::fs::PermissionsExt;

        let runtime = runtime_in(dir, true);
        std::fs::write(&runtime.entrypoint, format!("#!/bin/sh\n{script}\n")).unwrap();
        std::fs::set_permissions(&runtime.entrypoint, std::fs::Permissions::from_mode(0o755))
            .unwrap();
        runtime
    }

    /// Answers every request with 200 once the fake server wrote the port it was given.
    #[cfg(unix)]
    fn serve_health_on_recorded_port(port_file: PathBuf) {
        use flow_like_types::tokio::{self, io::AsyncReadExt, io::AsyncWriteExt};

        tokio::spawn(async move {
            let port = loop {
                let recorded = std::fs::read_to_string(&port_file).unwrap_or_default();
                if let Ok(port) = recorded.trim().parse::<u16>() {
                    break port;
                }
                sleep(Duration::from_millis(10)).await;
            };
            let listener = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, port))
                .await
                .unwrap();
            while let Ok((mut stream, _)) = listener.accept().await {
                let mut request = [0_u8; 1024];
                let _ = stream.read(&mut request).await;
                let _ = stream
                    .write_all(
                        b"HTTP/1.1 200 OK\r\ncontent-length: 2\r\nconnection: close\r\n\r\nok",
                    )
                    .await;
            }
        });
    }

    #[cfg(unix)]
    #[test]
    fn dropping_the_server_kills_and_reaps_the_process() {
        let mut server = LlamaServerProcess::default();
        let pid = server.attach(sleeping_child()).id();
        assert!(process_listed(pid));

        let started = std::time::Instant::now();
        drop(server);

        assert_killed_promptly(started);
        assert!(!process_listed(pid));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn abandoning_a_start_stops_the_spawned_server() {
        let (pid_sender, pid_receiver) = std::sync::mpsc::channel();
        let start = async move {
            let mut server = LlamaServerProcess::default();
            pid_sender
                .send(server.attach(sleeping_child()).id())
                .unwrap();
            std::future::pending::<()>().await;
        };

        let started = std::time::Instant::now();
        let _ = flow_like_types::tokio::time::timeout(Duration::from_millis(50), start).await;

        assert_killed_promptly(started);
        assert!(!process_listed(pid_receiver.recv().unwrap()));
    }

    #[cfg(unix)]
    #[test]
    fn restarting_reaps_the_previous_server() {
        let mut server = LlamaServerProcess::default();
        let first = server.attach(sleeping_child()).id();

        let started = std::time::Instant::now();
        server.stop();
        assert!(!process_listed(first));

        let second = server.attach(sleeping_child()).id();
        let third = server.attach(sleeping_child()).id();
        assert_killed_promptly(started);
        assert!(!process_listed(second));
        assert!(process_listed(third));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_client_keeps_the_server_alive_after_the_model_is_dropped() {
        let mut server = LlamaServerProcess::default();
        let pid = server.attach(sleeping_child()).id();
        let model = LocalModel::serving(Bit::default(), &provider(), server, 9650);
        assert_eq!(model.default_model().await.as_deref(), Some("local-model"));
        let client = model.provider().await.unwrap().into_client();

        drop(model);
        assert!(
            process_listed(pid),
            "the factory evicting the model must not stop a server a client still uses"
        );

        let started = std::time::Instant::now();
        drop(client);
        assert_killed_promptly(started);
        assert!(!process_listed(pid));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn the_factory_keeps_a_model_whose_client_is_still_in_use() {
        use crate::models::factory_cache::{FactoryCache, MODEL_IDLE_TTL};

        let mut server = LlamaServerProcess::default();
        let pid = server.attach(sleeping_child()).id();
        let cache = FactoryCache::<dyn ModelLogic>::default();
        let cached = cache
            .get_or_build("local-model", || async move {
                Ok(
                    LocalModel::serving(Bit::default(), &provider(), server, 9650)
                        as Arc<dyn ModelLogic>,
                )
            })
            .await
            .unwrap();
        let client = cached.provider().await.unwrap().into_client();
        drop(cached);

        let idle = std::time::Instant::now() + MODEL_IDLE_TTL * 3;
        cache.gc(idle, MODEL_IDLE_TTL);
        assert!(
            cache.contains("local-model"),
            "a build while the client runs must reuse its server, not start a second one"
        );
        assert!(process_listed(pid));

        drop(client);
        let started = std::time::Instant::now();
        cache.gc(
            idle + MODEL_IDLE_TTL + Duration::from_secs(1),
            MODEL_IDLE_TTL,
        );
        assert!(!cache.contains("local-model"));
        assert_killed_promptly(started);
        assert!(!process_listed(pid));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_server_starts_only_while_no_other_local_engine_loads() {
        let dir = tempfile::tempdir().unwrap();
        let port_file = dir.path().join("port");
        let runtime = fake_server(
            dir.path(),
            &format!(
                "while [ $# -gt 0 ]; do [ \"$1\" = --port ] && echo \"$2\" > '{}'; shift; done\nexec sleep 60",
                port_file.display()
            ),
        );
        serve_health_on_recorded_port(port_file.clone());
        let chatml = LlamaServerTemplateOverride {
            chat_template: Some("chatml".to_string()),
            chat_template_file: None,
        };
        let launch = launch_with(&runtime);

        let loading = LOCAL_ENGINE_LOADS.lock().await;
        let mut start = std::pin::pin!(launch.start_with_tool_support(&chatml));
        let waited = tokio::time::timeout(Duration::from_millis(500), &mut start).await;
        assert!(waited.is_err(), "the start must wait for the other load");
        assert!(
            !port_file.exists(),
            "no server spawns while another engine loads"
        );

        drop(loading);
        let (mut server, port) = tokio::time::timeout(Duration::from_secs(30), start)
            .await
            .expect("the start proceeds once the other load is done")
            .unwrap();
        assert!(server.exit_status().is_none());
        assert_eq!(
            std::fs::read_to_string(&port_file).unwrap().trim(),
            port.to_string()
        );
    }

    #[test]
    fn a_reserved_port_is_not_handed_out_again_until_released() {
        const PORT: u16 = 1;
        let first = PortReservation::pick(|| Some(PORT)).unwrap();
        assert!(PortReservation::pick(|| Some(PORT)).is_err());

        let mut offered = [PORT, 2].into_iter();
        let second = PortReservation::pick(|| offered.next()).unwrap();
        assert_eq!(second.port, 2);

        drop(first);
        assert_eq!(PortReservation::pick(|| Some(PORT)).unwrap().port, PORT);
    }

    /// Only the held direction is checked: a closed listener can linger for a moment in a child
    /// that another test forks, so "free right after close" is not deterministic here.
    #[test]
    fn a_port_another_listener_holds_is_in_use() {
        let holder = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        assert!(port_in_use(holder.local_addr().unwrap().port()));
    }

    #[cfg(unix)]
    #[test]
    fn a_server_that_lost_its_port_to_another_process_is_retried() {
        use std::os::unix::process::ExitStatusExt;

        let runtime = runtime_in(Path::new("/opt/llamacpp"), false);
        let launch = launch_with(&runtime);
        let bind_failure = StartFailure::Exited(ExitStatus::from_raw(1 << 8));

        assert_eq!(
            launch.retry_after(&bind_failure, false, true),
            Some(Retry::OnAnotherPort)
        );
        assert_eq!(launch.retry_after(&bind_failure, false, false), None);
        assert_eq!(
            launch.retry_after(&StartFailure::TimedOut, false, true),
            None
        );
    }

    #[cfg(unix)]
    #[test]
    fn only_a_load_failure_retries_once_with_the_fallback_libraries() {
        use std::os::unix::process::ExitStatusExt;

        let load_failure = StartFailure::Exited(ExitStatus::from_raw(127 << 8));
        let pack = runtime_in(Path::new("/opt/llamacpp"), true);
        let bundle = runtime_in(Path::new("/opt/llamacpp"), false);

        assert_eq!(
            launch_with(&pack).retry_after(&load_failure, false, false),
            cfg!(target_os = "linux").then_some(Retry::WithFallback)
        );
        assert_eq!(
            launch_with(&pack).retry_after(&load_failure, true, false),
            None
        );
        assert_eq!(
            launch_with(&bundle).retry_after(&load_failure, false, false),
            None
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_server_that_cannot_start_reports_its_output() {
        let dir = tempfile::tempdir().unwrap();
        let starts = dir.path().join("starts");
        let runtime = fake_server(
            dir.path(),
            &format!(
                "echo \"${{LD_LIBRARY_PATH}}\" >> '{}'\necho 'error while loading shared libraries: libstdc++.so.6' >&2\nexit 127",
                starts.display()
            ),
        );

        let error = launch_with(&runtime)
            .start(&LlamaServerTemplateOverride::default())
            .await
            .err()
            .expect("the fake server never becomes ready")
            .to_string();

        assert!(
            error.contains("error while loading shared libraries"),
            "{error}"
        );
        assert!(
            error.contains(&runtime.entrypoint.display().to_string()),
            "{error}"
        );
        let starts = std::fs::read_to_string(&starts).unwrap();
        if cfg!(target_os = "linux") {
            let library_paths = starts.lines().collect::<Vec<_>>();
            let library_dir = runtime.library_dir.display().to_string();
            let fallback = format!(
                "{library_dir}:{}",
                runtime.fallback_dir.as_ref().unwrap().display()
            );
            assert_eq!(library_paths, [library_dir.as_str(), fallback.as_str()]);
        } else {
            assert_eq!(starts.lines().count(), 1, "{starts}");
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_start_returns_once_its_running_server_is_healthy() {
        let dir = tempfile::tempdir().unwrap();
        let port_file = dir.path().join("port");
        let runtime = fake_server(
            dir.path(),
            &format!(
                "while [ $# -gt 0 ]; do [ \"$1\" = --port ] && echo \"$2\" > '{}'; shift; done\nexec sleep 60",
                port_file.display()
            ),
        );
        serve_health_on_recorded_port(port_file.clone());

        let (mut server, port) = launch_with(&runtime)
            .start(&LlamaServerTemplateOverride::default())
            .await
            .unwrap();

        assert_eq!(
            std::fs::read_to_string(&port_file).unwrap().trim(),
            port.to_string()
        );
        assert!(server.exit_status().is_none());
        assert!(!STARTING_PORTS.lock().contains(&port));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn systemone_starts_once_without_probing_chat_tool_support() {
        let dir = tempfile::tempdir().unwrap();
        let port_file = dir.path().join("port");
        let starts_file = dir.path().join("starts");
        let runtime = fake_server(
            dir.path(),
            &format!(
                "echo started >> '{}'\nwhile [ $# -gt 0 ]; do [ \"$1\" = --port ] && echo \"$2\" > '{}'; shift; done\nexec sleep 60",
                starts_file.display(),
                port_file.display()
            ),
        );
        // This server answers health but has no chat-template metadata.
        serve_health_on_recorded_port(port_file);
        let launch = ServerLaunch {
            mode: LlamaServerMode::SystemOne,
            ..launch_with(&runtime)
        };
        let (mut server, _) = launch
            .start_with_tool_support(&LlamaServerTemplateOverride::default())
            .await
            .unwrap();
        assert!(server.exit_status().is_none());
        assert_eq!(std::fs::read_to_string(starts_file).unwrap(), "started\n");
    }

    #[test]
    fn server_args_use_automatic_gpu_offload() {
        let args = LocalModel::server_args(
            &PathBuf::from("/models/model.gguf"),
            8192,
            9650,
            true,
            None,
            &LlamaServerTemplateOverride::default(),
            LlamaServerMode::Chat,
        );

        assert_eq!(arg_value(&args, "--n-gpu-layers"), Some("auto"));
        assert_eq!(arg_value(&args, "--flash-attn"), Some("auto"));
        assert_eq!(arg_value(&args, "--fit"), Some("on"));
        assert_eq!(arg_value(&args, "--parallel"), Some("1"));
        assert_eq!(arg_value(&args, "--host"), Some("127.0.0.1"));
        assert_eq!(arg_value(&args, "--ctx-size"), Some("8192"));
        assert_eq!(arg_value(&args, "--cache-reuse"), Some("256"));
        assert!(!args.iter().any(|arg| arg == "-ngl" || arg == "45"));
    }

    #[test]
    fn context_length_is_bounded_by_desktop_default() {
        assert_eq!(
            LocalModel::resolve_context_length(Some(128_000), DEFAULT_MAX_CONTEXT_SIZE),
            DEFAULT_MAX_CONTEXT_SIZE as u32
        );
        assert_eq!(
            LocalModel::resolve_context_length(Some(128_000), 0),
            DEFAULT_MAX_CONTEXT_SIZE as u32
        );
    }

    #[test]
    fn server_args_can_pin_context_size() {
        let args = LocalModel::server_args(
            &PathBuf::from("/models/model.gguf"),
            16_384,
            9650,
            true,
            None,
            &LlamaServerTemplateOverride::default(),
            LlamaServerMode::Chat,
        );

        assert_eq!(arg_value(&args, "--ctx-size"), Some("16384"));
    }

    #[test]
    fn server_args_can_disable_gpu_offload() {
        let args = LocalModel::server_args(
            &PathBuf::from("/models/model.gguf"),
            8192,
            9650,
            false,
            None,
            &LlamaServerTemplateOverride::default(),
            LlamaServerMode::Chat,
        );

        assert_eq!(arg_value(&args, "--device"), Some("none"));
        assert_eq!(arg_value(&args, "--n-gpu-layers"), Some("0"));
        assert_eq!(arg_value(&args, "--flash-attn"), Some("auto"));
    }

    #[test]
    fn vision_keeps_its_projector_without_unsupported_kv_shifting() {
        let args = LocalModel::server_args(
            Path::new("/models/model.gguf"),
            8192,
            9650,
            true,
            Some("/models/mmproj.gguf"),
            &LlamaServerTemplateOverride::default(),
            LlamaServerMode::Chat,
        );
        assert_eq!(arg_value(&args, "--mmproj"), Some("/models/mmproj.gguf"));
        assert!(args.iter().any(|arg| arg == "--jinja"));
        assert!(!args.iter().any(|arg| arg == "--cache-reuse"));
    }

    #[test]
    fn systemone_fits_the_whole_prompt_and_preserves_its_native_template() {
        let args = LocalModel::server_args(
            Path::new("/models/decision.gguf"),
            4096,
            9650,
            true,
            Some("/models/mmproj.gguf"),
            &LlamaServerTemplateOverride {
                chat_template: Some("chatml".into()),
                chat_template_file: Some("/templates/chat.jinja".into()),
            },
            LlamaServerMode::SystemOne,
        );
        for flag in ["--ctx-size", "--batch-size", "--ubatch-size"] {
            assert_eq!(arg_value(&args, flag), Some("4096"));
        }
        assert_eq!(arg_value(&args, "--mmproj"), Some("/models/mmproj.gguf"));
        assert_eq!(arg_value(&args, "--parallel"), Some("1"));
        for flag in [
            "--jinja",
            "--chat-template",
            "--chat-template-file",
            "--cache-reuse",
        ] {
            assert!(!args.iter().any(|arg| arg == flag), "{flag}");
        }
    }
}
