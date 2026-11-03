use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use flow_like_types::tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};
use flow_like_types::tokio::process::Command;
use flow_like_types::tokio::{self, time};

use super::backend::{ShellBackend, ShellRequest, ShellResponse};

const KEPT_ENV: &[&str] = &[
    "PATH",
    "LANG",
    "LC_ALL",
    "LC_CTYPE",
    "TZ",
    "SYSTEMROOT",
    "COMSPEC",
    "PATHEXT",
    "WINDIR",
];
const READER_GRACE: Duration = Duration::from_millis(300);
const READER_KILL_GRACE: Duration = Duration::from_secs(2);
const POLL: Duration = Duration::from_millis(40);

/// The child's whole environment: a short allow-list of the parent's, a home inside the working
/// directory, and the bot's own secrets. Nothing else of the parent leaks in.
pub fn scrubbed_env(
    parent: impl IntoIterator<Item = (String, String)>,
    home: &Path,
    extra: &[(String, String)],
) -> Vec<(String, String)> {
    let mut env: Vec<(String, String)> = parent
        .into_iter()
        .filter(|(name, _)| KEPT_ENV.iter().any(|kept| kept.eq_ignore_ascii_case(name)))
        .collect();
    let home = home.to_string_lossy().to_string();
    for (name, value) in [
        ("HOME", home.as_str()),
        ("USERPROFILE", home.as_str()),
        ("TERM", "dumb"),
        ("NO_COLOR", "1"),
    ] {
        env.retain(|(existing, _)| !existing.eq_ignore_ascii_case(name));
        env.push((name.to_string(), value.to_string()));
    }
    for (name, value) in extra {
        env.retain(|(existing, _)| !existing.eq_ignore_ascii_case(name));
        env.push((name.clone(), value.clone()));
    }
    env
}

/// `bwrap` arguments that make the whole filesystem read-only except the working directory.
pub fn bubblewrap_args(root: &Path, cwd: &Path) -> Vec<String> {
    let root = root.to_string_lossy().to_string();
    let cwd = cwd.to_string_lossy().to_string();
    [
        "--ro-bind",
        "/",
        "/",
        "--dev",
        "/dev",
        "--proc",
        "/proc",
        "--tmpfs",
        "/tmp",
        "--bind",
        root.as_str(),
        root.as_str(),
        "--unshare-pid",
        "--unshare-ipc",
        "--die-with-parent",
        "--chdir",
        cwd.as_str(),
        "--",
    ]
    .into_iter()
    .map(str::to_string)
    .collect()
}

#[cfg(target_os = "linux")]
fn bubblewrap() -> Option<&'static PathBuf> {
    use std::sync::OnceLock;
    static BWRAP: OnceLock<Option<PathBuf>> = OnceLock::new();
    BWRAP
        .get_or_init(|| {
            let path = PathBuf::from("/usr/bin/bwrap");
            let usable = path.is_file()
                && std::process::Command::new(&path)
                    .args(["--ro-bind", "/", "/", "--", "true"])
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .status()
                    .is_ok_and(|status| status.success());
            usable.then_some(path)
        })
        .as_ref()
}

#[cfg(not(target_os = "linux"))]
fn bubblewrap() -> Option<&'static PathBuf> {
    None
}

#[derive(Default)]
struct Capture {
    head: Vec<u8>,
    tail: VecDeque<u8>,
    omitted: u64,
}

impl Capture {
    fn push(&mut self, chunk: &[u8], budget: usize) {
        let head_cap = budget / 2;
        let tail_cap = budget - head_cap;
        let room = head_cap.saturating_sub(self.head.len()).min(chunk.len());
        self.head.extend_from_slice(&chunk[..room]);
        self.tail.extend(&chunk[room..]);
        while self.tail.len() > tail_cap {
            self.tail.pop_front();
            self.omitted += 1;
        }
    }

    fn text(&self) -> String {
        let head = String::from_utf8_lossy(&self.head);
        if self.omitted == 0 && self.tail.is_empty() {
            return head.into_owned();
        }
        let tail: Vec<u8> = self.tail.iter().copied().collect();
        let tail = String::from_utf8_lossy(&tail);
        if self.omitted == 0 {
            return format!("{head}{tail}");
        }
        format!("{head}\n[... {} bytes omitted ...]\n{tail}", self.omitted)
    }
}

fn read_stream<R: AsyncRead + Unpin + Send + 'static>(
    mut stream: R,
    budget: usize,
    capture: Arc<Mutex<Capture>>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut buffer = vec![0u8; 8192];
        while let Ok(read) = stream.read(&mut buffer).await {
            if read == 0 {
                break;
            }
            if let Ok(mut capture) = capture.lock() {
                capture.push(&buffer[..read], budget);
            }
        }
    })
}

#[cfg(unix)]
fn kill_tree(pid: Option<u32>) {
    if let Some(pid) = pid {
        // SAFETY: killpg only signals; the child leads its own group (process_group(0) at spawn).
        unsafe { libc::killpg(pid as i32, libc::SIGKILL) };
    }
}

#[cfg(windows)]
fn kill_tree(pid: Option<u32>) {
    if let Some(pid) = pid {
        let _ = std::process::Command::new("taskkill")
            .args(["/PID", &pid.to_string(), "/T", "/F"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
}

#[cfg(not(any(unix, windows)))]
fn kill_tree(_pid: Option<u32>) {}

fn command_line(request: &ShellRequest) -> (String, Vec<String>) {
    if !request.script {
        return (request.program.clone(), request.args.clone());
    }
    if cfg!(windows) {
        (
            "cmd".to_string(),
            vec!["/C".into(), request.program.clone()],
        )
    } else {
        ("sh".to_string(), vec!["-c".into(), request.program.clone()])
    }
}

/// Runs commands as plain child processes of this device, in one working directory.
pub struct LocalShell {
    root: PathBuf,
}

impl LocalShell {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }
}

#[async_trait]
impl ShellBackend for LocalShell {
    fn root(&self) -> PathBuf {
        self.root.clone()
    }

    async fn run(
        &self,
        request: ShellRequest,
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ShellResponse {
        let started = Instant::now();
        let cwd = self.root.join(&request.cwd);
        let _ = tokio::fs::create_dir_all(&cwd).await;
        let (program, args) = command_line(&request);
        let wrapper = bubblewrap();
        let mut command = match wrapper {
            Some(bwrap) => {
                let mut command = Command::new(bwrap);
                command
                    .args(bubblewrap_args(&self.root, &cwd))
                    .arg(&program);
                command
            }
            None => {
                let mut command = Command::new(&program);
                command.current_dir(&cwd);
                command
            }
        };
        command
            .args(&args)
            .env_clear()
            .envs(scrubbed_env(std::env::vars(), &self.root, &request.env))
            .stdin(if request.stdin.is_some() {
                Stdio::piped()
            } else {
                Stdio::null()
            })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        #[cfg(unix)]
        command.process_group(0);
        #[cfg(windows)]
        command.creation_flags(0x0800_0000);

        let mut child = match command.spawn() {
            Ok(child) => child,
            Err(error) => {
                return ShellResponse {
                    spawn_error: Some(format!("Could not start '{program}': {error}")),
                    duration_ms: started.elapsed().as_millis() as u64,
                    ..ShellResponse::default()
                };
            }
        };
        if let (Some(mut pipe), Some(text)) = (child.stdin.take(), request.stdin.clone()) {
            tokio::spawn(async move {
                let _ = pipe.write_all(text.as_bytes()).await;
                let _ = pipe.shutdown().await;
            });
        }
        let out = Arc::new(Mutex::new(Capture::default()));
        let err = Arc::new(Mutex::new(Capture::default()));
        let mut readers = Vec::new();
        if let Some(stream) = child.stdout.take() {
            readers.push(read_stream(stream, request.output_bytes, out.clone()));
        }
        if let Some(stream) = child.stderr.take() {
            readers.push(read_stream(stream, request.output_bytes, err.clone()));
        }

        let pid = child.id();
        let mut response = ShellResponse {
            sandboxed: wrapper.is_some(),
            ..ShellResponse::default()
        };
        let status = loop {
            tokio::select! {
                status = child.wait() => break status.ok(),
                _ = time::sleep(POLL) => {
                    if cancelled() {
                        response.cancelled = true;
                    } else if started.elapsed() >= request.wall_time {
                        response.timed_out = true;
                    } else {
                        continue;
                    }
                    kill_tree(pid);
                    let _ = child.start_kill();
                    break child.wait().await.ok();
                }
            }
        };
        response.exit_code = status.and_then(|status| status.code());

        let drained = time::timeout(READER_GRACE, join_all(&mut readers)).await;
        if drained.is_err() {
            kill_tree(pid);
            let _ = time::timeout(READER_KILL_GRACE, join_all(&mut readers)).await;
        }
        for reader in &readers {
            reader.abort();
        }
        if let Ok(capture) = out.lock() {
            response.stdout = capture.text();
            response.stdout_omitted = capture.omitted;
        }
        if let Ok(capture) = err.lock() {
            response.stderr = capture.text();
            response.stderr_omitted = capture.omitted;
        }
        response.duration_ms = started.elapsed().as_millis() as u64;
        response
    }
}

async fn join_all(readers: &mut [tokio::task::JoinHandle<()>]) {
    for reader in readers.iter_mut() {
        let _ = reader.await;
    }
}
