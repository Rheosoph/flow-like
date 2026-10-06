//! Engine processes: one child per loaded model, serving an OpenAI-compatible API on a
//! private socket (sandboxed Linux) or loopback port behind a bearer key per start.
//!
//! A TCP engine binds a port the kernel picks and holds it from then on, and announces it on an
//! output only the agent reads. No port is picked first and handed over later, so no other
//! process can bind it in between and pose as the engine.

mod endpoint;
pub mod gguf;

use endpoint::EngineSocket;
pub mod llamacpp;
pub mod mlx;
#[cfg(feature = "runtime")]
pub mod onnx;

use anyhow::{Context, Result};
use std::{
    collections::VecDeque,
    ffi::OsString,
    path::PathBuf,
    process::{ExitStatus, Stdio},
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::{
    io::{AsyncBufReadExt, AsyncRead, AsyncWriteExt},
    sync::watch,
};
use zeroize::Zeroizing;

/// The line an engine announces its port with, the port ending the line.
const ANNOUNCEMENT: &str = "listening on http://127.0.0.1:";
const OUTPUT_TAIL_LINES: usize = 16;
const READY_POLL: Duration = Duration::from_millis(250);
const STOP_GRACE: Duration = Duration::from_secs(5);
/// What an engine inherits from the agent: lookup, locale and logging settings, the loader
/// path its runtime pack was probed with, and GPU selection. Never the agent's credentials.
const ENGINE_ENV: [&str; 11] = [
    "PATH",
    "HOME",
    "TMPDIR",
    "LANG",
    "LC_ALL",
    "RUST_LOG",
    "LD_LIBRARY_PATH",
    "CUDA_VISIBLE_DEVICES",
    "GGML_VK_VISIBLE_DEVICES",
    "VK_ICD_FILENAMES",
    "VK_DRIVER_FILES",
];
#[cfg(feature = "runtime")]
const MAX_WORKER_CONFIG_BYTES: u64 = 64 * 1024;

/// Bytes a loaded model holds: weights, KV cache and compute buffers.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MemoryEstimate {
    pub weights: u64,
    pub kv_cache: u64,
    pub overhead: u64,
}

impl MemoryEstimate {
    pub fn total(&self) -> u64 {
        self.weights
            .saturating_add(self.kv_cache)
            .saturating_add(self.overhead)
    }
}

/// Where an engine announces the port it bound: the first line there reading
/// `listening on http://127.0.0.1:<port>`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Announce {
    /// llama-server logs it after loading the model and binding.
    Stderr,
    /// Model workers print it; nothing else of theirs goes there.
    Stdout,
}

/// How to start one engine.
#[derive(Clone)]
pub struct EngineLaunch {
    pub program: PathBuf,
    pub args: Vec<OsString>,
    pub env: Vec<(OsString, OsString)>,
    /// Written to the child's stdin, which then closes.
    pub stdin: Option<Zeroizing<Vec<u8>>>,
    /// Paths the engine reads; a sandbox binds them read-only where they are.
    pub read_only: Vec<PathBuf>,
    pub announce: Announce,
}

/// A started engine plus what the supervisor needs to serve and account for it.
pub struct EnginePlan {
    pub launch: EngineLaunch,
    pub key: Zeroizing<String>,
    pub estimate: MemoryEstimate,
    pub slots: u8,
    /// A llama.cpp chat model is restarted with the chatml template when its own
    /// template does not advertise tool use.
    pub probe_tool_template: bool,
}

pub trait EngineLauncher: Send + Sync {
    fn command(&self, launch: &EngineLaunch) -> Result<tokio::process::Command>;

    fn prepare(
        &self,
        launch: &EngineLaunch,
        _estimate: MemoryEstimate,
        _model_id: &str,
    ) -> Result<PreparedLaunch> {
        Ok(PreparedLaunch {
            command: self.command(launch)?,
            socket: None,
            isolation: None,
        })
    }
}

pub struct PreparedLaunch {
    command: tokio::process::Command,
    socket: Option<EngineSocket>,
    isolation: Option<Box<dyn Send + Sync>>,
}

/// Starts engines as children of the agent; on Linux inside the engine sandbox whenever
/// the host offers one, and always when the host requires isolation.
pub struct ProcessLauncher {
    pub sandbox: bool,
    pub state_dir: Option<PathBuf>,
}

impl EngineLauncher for ProcessLauncher {
    fn command(&self, launch: &EngineLaunch) -> Result<tokio::process::Command> {
        anyhow::ensure!(
            !self.sandbox,
            "Sandboxed engines require a prepared isolation lease"
        );
        let mut command = tokio::process::Command::new(&launch.program);
        command.args(&launch.args);
        die_with_agent(&mut command);
        command.env_clear();
        for key in ENGINE_ENV {
            if let Some(value) = std::env::var_os(key) {
                command.env(key, value);
            }
        }
        command.envs(launch.env.iter().map(|(key, value)| (key, value)));
        Ok(command)
    }

    fn prepare(
        &self,
        launch: &EngineLaunch,
        estimate: MemoryEstimate,
        model_id: &str,
    ) -> Result<PreparedLaunch> {
        if !self.sandbox {
            return Ok(PreparedLaunch {
                command: self.command(launch)?,
                socket: None,
                isolation: None,
            });
        }
        self.sandboxed(launch, estimate, model_id)
    }
}

/// A sandboxed engine serves only through its private filesystem socket.
impl ProcessLauncher {
    #[cfg(target_os = "linux")]
    fn sandboxed(
        &self,
        launch: &EngineLaunch,
        estimate: MemoryEstimate,
        model_id: &str,
    ) -> Result<PreparedLaunch> {
        let state_dir = self
            .state_dir
            .as_deref()
            .context("An engine sandbox needs the agent state directory")?;
        let socket = EngineSocket::new_sandboxed()?;
        let launch = socket.launch(launch)?;
        let (mut command, isolation) = crate::isolation::engine_command(
            &launch.program,
            &launch.args,
            &launch.read_only,
            &socket.path(),
            state_dir,
            estimate.total(),
            model_id,
        )?;
        command.env_clear();
        for key in ENGINE_ENV {
            if let Some(value) = std::env::var_os(key) {
                command.env(key, value);
            }
        }
        command.envs(launch.env.iter().map(|(key, value)| (key, value)));
        Ok(PreparedLaunch {
            command,
            socket: Some(socket),
            isolation: Some(Box::new(isolation)),
        })
    }

    #[cfg(not(target_os = "linux"))]
    fn sandboxed(&self, _: &EngineLaunch, _: MemoryEstimate, _: &str) -> Result<PreparedLaunch> {
        anyhow::bail!("The engine sandbox needs Linux")
    }
}

fn die_with_agent(command: &mut tokio::process::Command) {
    #[cfg(target_os = "linux")]
    // SAFETY: prctl is async-signal-safe and only changes this child's death signal.
    unsafe {
        command.pre_exec(|| {
            if libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL, 0, 0, 0) != 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    #[cfg(not(target_os = "linux"))]
    let _ = command;
}

#[derive(Debug)]
pub enum StartFailure {
    Exited(ExitStatus),
    TimedOut,
}

/// A running engine. Dropping it kills the process.
pub struct EngineProcess {
    child: tokio::process::Child,
    /// The port the engine announced, once it has.
    port: watch::Receiver<Option<u16>>,
    output: Tail,
    client: reqwest::Client,
    socket: Option<EngineSocket>,
    _isolation: Option<Box<dyn Send + Sync>>,
}

/// The last lines an engine wrote to stderr.
type Tail = Arc<Mutex<VecDeque<String>>>;

fn pipe_if(wanted: bool) -> Stdio {
    if wanted {
        Stdio::piped()
    } else {
        Stdio::null()
    }
}

/// Reads the engine's outputs: the port it announces where its launch says, and the last
/// lines of its stderr.
fn watch_outputs(
    child: &mut tokio::process::Child,
    announce: Announce,
    label: &str,
) -> (watch::Receiver<Option<u16>>, Tail) {
    let (announced, port) = watch::channel(None);
    let (on_stderr, on_stdout) = match announce {
        Announce::Stderr => (Some(announced), None),
        Announce::Stdout => (None, Some(announced)),
    };
    let output = Tail::default();
    if let Some(stderr) = child.stderr.take() {
        let tail = Arc::clone(&output);
        tokio::spawn(keep_tail(stderr, tail, label.to_owned(), on_stderr));
    }
    if let Some(stdout) = child.stdout.take() {
        tokio::spawn(read_announcement(stdout, on_stdout));
    }
    (port, output)
}

impl EngineProcess {
    pub async fn spawn(
        launcher: &dyn EngineLauncher,
        launch: &EngineLaunch,
        label: &str,
        estimate: MemoryEstimate,
    ) -> Result<Self> {
        let PreparedLaunch {
            mut command,
            socket,
            isolation,
        } = launcher.prepare(launch, estimate, label)?;
        let client = endpoint::engine_client(socket.as_ref().map(EngineSocket::path))?;
        command
            .stdin(pipe_if(launch.stdin.is_some()))
            .stdout(pipe_if(launch.announce == Announce::Stdout))
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        let mut child = crate::process_spawner::spawn(command)
            .await
            .with_context(|| {
                format!("Start the engine of {label} ({})", launch.program.display())
            })?;
        if let (Some(bytes), Some(stdin)) = (&launch.stdin, child.stdin.take()) {
            hand_over(stdin, bytes).await?;
        }
        let (port, output) = watch_outputs(&mut child, launch.announce, label);
        Ok(Self {
            child,
            port,
            output,
            client,
            socket,
            _isolation: isolation,
        })
    }

    pub fn client(&self) -> &reqwest::Client {
        &self.client
    }

    pub fn id(&self) -> Option<u32> {
        self.child.id()
    }

    /// The last lines the engine wrote, for error messages.
    pub fn output(&self) -> String {
        lock!(self.output)
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>()
            .join("\n")
    }

    pub fn exited(&mut self) -> Option<ExitStatus> {
        self.child.try_wait().ok().flatten()
    }

    /// Waits until `/health` answers 200 on the private socket or announced TCP port,
    /// the process exits, or `timeout` passes. Answers the base URL of the engine.
    pub async fn wait_ready(
        &mut self,
        key: &str,
        timeout: Duration,
    ) -> std::result::Result<String, StartFailure> {
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            if let Some(status) = self.exited() {
                return Err(StartFailure::Exited(status));
            }
            let announced = *self.port.borrow();
            let base_url = self
                .socket
                .as_ref()
                .map(|_| "http://localhost".to_owned())
                .or_else(|| announced.map(|port| format!("http://127.0.0.1:{port}")));
            if let Some(base_url) = base_url
                && healthy(&self.client, &base_url, key).await
            {
                return Ok(base_url);
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(StartFailure::TimedOut);
            }
            tokio::time::sleep(READY_POLL).await;
        }
    }

    /// Asks the engine to stop and kills it after a grace period.
    pub async fn stop(mut self) {
        if self.exited().is_some() {
            return;
        }
        #[cfg(unix)]
        if let Some(pid) = self.child.id().and_then(|pid| i32::try_from(pid).ok()) {
            // SAFETY: the pid belongs to our unreaped child, so it cannot be reused yet.
            unsafe {
                libc::kill(pid, libc::SIGTERM);
            }
        }
        if tokio::time::timeout(STOP_GRACE, self.child.wait())
            .await
            .is_err()
        {
            let _ = self.child.kill().await;
        }
    }
}

async fn hand_over(mut stdin: tokio::process::ChildStdin, bytes: &[u8]) -> Result<()> {
    stdin
        .write_all(bytes)
        .await
        .context("Hand the engine its configuration")?;
    stdin
        .shutdown()
        .await
        .context("Close the engine configuration")
}

async fn healthy(client: &reqwest::Client, base_url: &str, key: &str) -> bool {
    client
        .get(format!("{base_url}/health"))
        .bearer_auth(key)
        .timeout(Duration::from_secs(5))
        .send()
        .await
        .is_ok_and(|response| response.status() == reqwest::StatusCode::OK)
}

/// The port of an announcement line: `listening on http://127.0.0.1:<port>` ending the line.
pub fn announced_port(line: &str) -> Option<u16> {
    let (_, port) = line.trim_end().rsplit_once(ANNOUNCEMENT)?;
    port.parse().ok().filter(|port| *port != 0)
}

/// Passes on the first announcement of `line`'s stream; later ones change nothing.
fn announce(announced: &mut Option<watch::Sender<Option<u16>>>, line: &str) {
    if announced.is_some()
        && let Some(port) = announced_port(line)
        && let Some(sender) = announced.take()
    {
        sender.send_replace(Some(port));
    }
}

/// Logs the engine's stderr and keeps its last lines for error messages.
async fn keep_tail(
    stderr: tokio::process::ChildStderr,
    output: Tail,
    label: String,
    mut announced: Option<watch::Sender<Option<u16>>>,
) {
    let mut lines = tokio::io::BufReader::new(stderr).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        tracing::debug!(model = %label, "{line}");
        announce(&mut announced, &line);
        let mut tail = lock!(output);
        if tail.len() == OUTPUT_TAIL_LINES {
            tail.pop_front();
        }
        tail.push_back(line);
    }
}

/// Reads a worker's stdout, which carries nothing but its announcement, to its end.
async fn read_announcement(
    stdout: impl AsyncRead + Unpin,
    mut announced: Option<watch::Sender<Option<u16>>>,
) {
    let mut lines = tokio::io::BufReader::new(stdout).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        announce(&mut announced, &line);
    }
}

/// Ready timeout for an engine that maps `bytes` of weights: a minute plus 50 MB/s.
pub fn ready_timeout(bytes: u64) -> Duration {
    Duration::from_secs(60 + bytes / 50_000_000).min(Duration::from_secs(1_800))
}

/// A random 256-bit bearer key, hex encoded.
pub fn random_key() -> Zeroizing<String> {
    use rand_core::RngCore;
    let mut bytes = Zeroizing::new([0u8; 32]);
    rand_core::OsRng.fill_bytes(bytes.as_mut());
    Zeroizing::new(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

/// `flow-like-standalone model-worker --engine <engine>`: the agent binary serving one model,
/// with its configuration, key included, on stdin instead of the command line.
fn worker_launch(
    engine: &str,
    config: &impl serde::Serialize,
    read_only: Vec<PathBuf>,
) -> Result<EngineLaunch> {
    Ok(EngineLaunch {
        program: std::env::current_exe()
            .with_context(|| format!("Locate the agent binary for the {engine} worker"))?,
        args: ["model-worker", "--engine", engine]
            .into_iter()
            .map(OsString::from)
            .collect(),
        env: Vec::new(),
        stdin: Some(Zeroizing::new(serde_json::to_vec(config)?)),
        read_only,
        announce: Announce::Stdout,
    })
}

/// A worker's loopback listener on a port the kernel picks, announced to the agent on stdout.
#[cfg(feature = "runtime")]
async fn worker_listener() -> Result<tokio::net::TcpListener> {
    let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
        .await
        .context("Listen on a loopback port")?;
    announce_port(listener.local_addr()?.port())?;
    Ok(listener)
}

/// Tells the agent the port a worker serves on.
#[cfg(feature = "runtime")]
fn announce_port(port: u16) -> Result<()> {
    use std::io::Write;
    let mut stdout = std::io::stdout().lock();
    writeln!(stdout, "{ANNOUNCEMENT}{port}").context("Announce the worker's port")?;
    stdout.flush().context("Announce the worker's port")
}

/// The configuration a model worker got on stdin.
#[cfg(feature = "runtime")]
fn read_worker_config<T: serde::de::DeserializeOwned>() -> Result<T> {
    use std::io::Read;
    let mut text = Zeroizing::new(Vec::new());
    std::io::stdin()
        .take(MAX_WORKER_CONFIG_BYTES)
        .read_to_end(&mut text)
        .context("Read the model worker configuration")?;
    serde_json::from_slice(&text).context("Parse the model worker configuration")
}

#[cfg(feature = "runtime")]
fn parent_id() -> u32 {
    #[cfg(unix)]
    {
        // SAFETY: getppid has no preconditions.
        unsafe { libc::getppid() as u32 }
    }
    #[cfg(not(unix))]
    {
        0
    }
}

/// The parent changes when the agent dies and the worker is reparented. In the engine
/// sandbox the parent is the sandbox init, which dies with the agent anyway.
#[cfg(feature = "runtime")]
async fn parent_gone(parent: u32) {
    loop {
        tokio::time::sleep(Duration::from_secs(1)).await;
        if parent_id() != parent {
            return;
        }
    }
}

/// Describes why an engine did not become ready, with the end of its output.
pub fn start_error(label: &str, failure: &StartFailure, output: &str) -> anyhow::Error {
    match failure {
        StartFailure::Exited(status) => {
            anyhow::anyhow!(
                "The engine of {label} exited ({status}) before it was ready:\n{output}"
            )
        }
        StartFailure::TimedOut => {
            anyhow::anyhow!("The engine of {label} did not become ready in time:\n{output}")
        }
    }
}

#[cfg(test)]
pub(crate) mod fake;

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn engines_inherit_only_the_allowed_environment() -> Result<()> {
        let launch = EngineLaunch {
            program: PathBuf::from("/usr/bin/env"),
            args: Vec::new(),
            env: vec![("LD_LIBRARY_PATH".into(), "/pack/fallback".into())],
            stdin: None,
            read_only: Vec::new(),
            announce: Announce::Stderr,
        };
        let output = ProcessLauncher {
            sandbox: false,
            state_dir: None,
        }
        .command(&launch)?
        .output()
        .await?;
        let listed = String::from_utf8_lossy(&output.stdout);
        let names: Vec<&str> = listed
            .lines()
            .filter_map(|line| line.split_once('=').map(|(name, _)| name))
            .collect();
        assert!(
            names.iter().all(|name| ENGINE_ENV.contains(name)),
            "{names:?}"
        );
        assert!(
            listed
                .lines()
                .any(|line| line == "LD_LIBRARY_PATH=/pack/fallback"),
            "{listed}"
        );
        Ok(())
    }

    #[test]
    fn engines_announce_their_port_at_the_end_of_a_line() {
        let llama = "0.02.503.200 I srv  llama_server: listening on http://127.0.0.1:64728";
        assert_eq!(announced_port(llama), Some(64_728));
        assert_eq!(announced_port("listening on http://127.0.0.1:9\r"), Some(9));
        for other in [
            "listening on http://127.0.0.1:0",
            "listening on http://127.0.0.1:65536",
            "listening on http://127.0.0.1:80/v1",
            "listening on http://0.0.0.0:80",
            "server is listening on http://127.0.0.1:8080 - starting the main loop",
        ] {
            assert_eq!(announced_port(other), None, "{other}");
        }
    }

    #[tokio::test]
    async fn the_first_announcement_counts() {
        let (announced, port) = watch::channel(None);
        let output: &[u8] = b"running 1 test\nlistening on http://127.0.0.1:41000\n\
                              listening on http://127.0.0.1:6666\n";
        read_announcement(output, Some(announced)).await;
        assert_eq!(*port.borrow(), Some(41_000));
    }
}
