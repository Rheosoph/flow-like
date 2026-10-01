// Derived from agent-browser cli/src/native/cdp/chrome.rs @d01253d, Copyright 2025 Vercel Inc., Apache-2.0, and rustwright src/lib.rs @fca1438, Copyright (c) 2026 Ikonomos Inc (dba Skyvern), MIT; modified by Rheosoph GmbH. See NOTICE.
use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex, PoisonError, Weak};
use std::time::{Duration, Instant};

use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::task::JoinHandle;

use crate::BrowserError;
use crate::launch::cft::IN_USE_PREFIX;
use crate::launch::{
    Executable, ExecutableSource, Flavor, LaunchOptions, args, browser_name, discovery, profile,
    sandbox,
};

const STDERR_TAIL_BYTES: usize = 64 * 1024;
const MESSAGE_OUTPUT_CHARS: usize = 2000;
const ENDPOINT_POLL: Duration = Duration::from_millis(50);
const EXIT_POLL: Duration = Duration::from_millis(25);
const STDERR_DRAIN: Duration = Duration::from_millis(500);
const KILLED_EXIT_WAIT: Duration = Duration::from_secs(2);
const REMOVE_ATTEMPTS: u32 = 5;
const REMOVE_RETRY: Duration = Duration::from_millis(100);
const PROFILE_IN_USE_EXIT_CODE: i64 = 21;
const PROCESS_SINGLETON_FAILURE: &str = "Failed to create a ProcessSingleton";
const DEVTOOLS_LISTENING: &str = "DevTools listening on ";
const BROWSER_WS_PATH: &str = "/devtools/browser";
const DEVTOOLS_ACTIVE_PORT: &str = "DevToolsActivePort";
const CFT_BLOCKED: &str = "Chrome for Testing is blocked by policy on this machine; install Google Chrome or use Browser Type Edge";

static CFT_MARKER_USERS: Mutex<BTreeMap<PathBuf, usize>> = Mutex::new(BTreeMap::new());

type ChildStream = Box<dyn AsyncRead + Send + Unpin>;

pub struct LaunchedProcess {
    pub ws_url: String,
    pub port: u16,
    pub process: BrowserProcess,
}

pub async fn spawn(
    executable: &Executable,
    options: &LaunchOptions,
) -> crate::Result<LaunchedProcess> {
    if let Some(proxy) = &options.proxy {
        args::validate_proxy(&proxy.server)?;
    }
    let env = sandbox::preflight(executable)?;
    let workspace = Workspace::prepare(executable, options)?;
    let arguments = args::launch_args(
        executable,
        options,
        &workspace.profile.dir,
        workspace.profile.temporary,
    );
    #[cfg(unix)]
    let (process, stderr) = start_unix_child(executable, &arguments, &env, workspace)?;
    #[cfg(windows)]
    let (process, stderr) =
        start_windows_child(executable, &arguments, &env, workspace, options.headless)?;
    let mut stderr_reader = spawn_stderr_tail(stderr, Arc::clone(&process.stderr));
    match process
        .wait_for_endpoint(
            executable.flavor,
            options.launch_timeout,
            &mut stderr_reader,
        )
        .await
    {
        Ok((port, path)) => Ok(LaunchedProcess {
            ws_url: format!("ws://127.0.0.1:{port}{path}"),
            port,
            process,
        }),
        Err(error) => {
            process.kill();
            process.wait_exit(KILLED_EXIT_WAIT).await;
            process.cleanup();
            Err(error)
        }
    }
}

pub struct BrowserProcess {
    handle: Arc<ProcessHandle>,
    #[cfg(unix)]
    lifeline: Mutex<Option<std::os::fd::OwnedFd>>,
    watchdog: Sender<WatchdogSignal>,
    armed: AtomicBool,
    stderr: Arc<Mutex<Vec<u8>>>,
}

#[derive(Clone, Debug)]
pub struct Profile {
    pub dir: std::path::PathBuf,
    pub temporary: bool,
}

impl BrowserProcess {
    #[cfg(all(unix, any(test, feature = "test-support")))]
    pub(crate) fn adopt(
        child: tokio::process::Child,
        profile: Profile,
        staging: Option<PathBuf>,
    ) -> BrowserProcess {
        let workspace = Workspace {
            profile,
            staging,
            cft_marker: Mutex::new(None),
        };
        BrowserProcess::assemble(ProcessHandle::new(child, workspace), None)
    }

    fn assemble(
        handle: ProcessHandle,
        #[cfg(unix)] lifeline: Option<std::os::fd::OwnedFd>,
    ) -> BrowserProcess {
        let handle = Arc::new(handle);
        let (watchdog, signals) = std::sync::mpsc::channel();
        let watched = Arc::downgrade(&handle);
        let pid = handle.pid;
        if let Err(error) = std::thread::Builder::new()
            .name(format!("browser-watchdog-{pid}"))
            .spawn(move || run_watchdog(&signals, &watched))
        {
            tracing::warn!(pid, %error, "the browser watchdog thread could not start; arm_watchdog will not kill");
        }
        BrowserProcess {
            handle,
            #[cfg(unix)]
            lifeline: Mutex::new(lifeline),
            watchdog,
            armed: AtomicBool::new(false),
            stderr: Arc::new(Mutex::new(Vec::new())),
        }
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn pid(&self) -> u32 {
        self.handle.pid
    }

    pub fn profile(&self) -> &Profile {
        &self.handle.workspace.profile
    }

    pub fn staging_dir(&self) -> Option<&std::path::Path> {
        self.handle.workspace.staging.as_deref()
    }

    pub fn close_lifeline(&self) {
        #[cfg(unix)]
        drop(with_lock(&self.lifeline, Option::take));
    }

    pub fn arm_watchdog(&self, grace: std::time::Duration) {
        if self.watchdog.send(WatchdogSignal::Arm(grace)).is_ok() {
            self.armed.store(true, Ordering::Release);
        }
    }

    pub fn kill(&self) {
        self.handle.kill();
    }

    pub async fn wait_exit(&self, timeout: std::time::Duration) -> bool {
        let deadline = Instant::now().checked_add(timeout);
        loop {
            match self.handle.try_exit() {
                Ok(Some(_)) => {
                    let _ = self.watchdog.send(WatchdogSignal::Exited);
                    return true;
                }
                Ok(None) => {}
                Err(error) => {
                    tracing::debug!(pid = self.handle.pid, %error, "could not poll the browser process");
                    return false;
                }
            }
            let remaining =
                deadline.map(|deadline| deadline.saturating_duration_since(Instant::now()));
            if remaining == Some(Duration::ZERO) {
                return false;
            }
            tokio::time::sleep(remaining.map_or(EXIT_POLL, |remaining| remaining.min(EXIT_POLL)))
                .await;
        }
    }

    pub fn cleanup(&self) {
        self.handle.workspace.remove(REMOVE_ATTEMPTS);
    }

    pub fn stderr_tail(&self) -> String {
        with_lock(&self.stderr, |tail| {
            String::from_utf8_lossy(tail).into_owned()
        })
    }

    async fn wait_for_endpoint(
        &self,
        flavor: Flavor,
        timeout: Duration,
        stderr_reader: &mut JoinHandle<()>,
    ) -> crate::Result<(u16, String)> {
        let deadline = Instant::now().checked_add(timeout);
        let profile_dir = &self.handle.workspace.profile.dir;
        let mut problem = None;
        loop {
            match crate::attach::read_devtools_active_port(profile_dir) {
                Ok(Some(endpoint)) => return Ok(endpoint),
                Ok(None) => {}
                Err(error) => problem = Some(error.to_string()),
            }
            if let Some(endpoint) = endpoint_from_stderr(&self.stderr_tail()) {
                return Ok(endpoint);
            }
            let exit = self.handle.try_exit().map_err(|error| {
                BrowserError::io(
                    format!(
                        "Checking whether {} (process {}) is still starting",
                        browser_name(flavor),
                        self.handle.pid
                    ),
                    &error,
                )
            })?;
            if let Some(exit) = exit {
                let _ = tokio::time::timeout(STDERR_DRAIN, stderr_reader).await;
                return Err(startup_failure(
                    flavor,
                    profile_dir,
                    exit,
                    &self.stderr_tail(),
                ));
            }
            if deadline.is_some_and(|deadline| Instant::now() >= deadline) {
                return Err(endpoint_timeout(
                    flavor,
                    profile_dir,
                    timeout,
                    problem,
                    &self.stderr_tail(),
                ));
            }
            tokio::time::sleep(ENDPOINT_POLL).await;
        }
    }
}

impl Drop for BrowserProcess {
    fn drop(&mut self) {
        let handed_to_watchdog = *self.armed.get_mut()
            && self
                .watchdog
                .send(WatchdogSignal::Detach(Arc::clone(&self.handle)))
                .is_ok();
        if !handed_to_watchdog {
            self.handle.kill_unless_reaped();
        }
    }
}

#[cfg(unix)]
type PlatformChild = tokio::process::Child;
#[cfg(windows)]
type PlatformChild = crate::launch::windows_process::WindowsChild;

struct ProcessHandle {
    pid: u32,
    #[cfg(unix)]
    child: Mutex<PlatformChild>,
    #[cfg(windows)]
    child: PlatformChild,
    workspace: Workspace,
}

impl ProcessHandle {
    fn new(child: PlatformChild, workspace: Workspace) -> ProcessHandle {
        #[cfg(unix)]
        let (pid, child) = (child.id().unwrap_or_default(), Mutex::new(child));
        #[cfg(windows)]
        let pid = child.pid();
        ProcessHandle {
            pid,
            child,
            workspace,
        }
    }

    fn kill(&self) {
        #[cfg(unix)]
        self.kill_group(true);
        #[cfg(windows)]
        self.child.terminate_job();
    }

    fn kill_unless_reaped(&self) {
        #[cfg(unix)]
        self.kill_group(false);
        #[cfg(windows)]
        self.child.terminate_job();
    }

    #[cfg(unix)]
    fn kill_group(&self, even_if_reaped: bool) {
        with_lock(&self.child, |child| {
            // An unreaped leader, even a zombie, keeps its pid reserved as the group id; once
            // reaped, the number can be reissued to an unrelated group.
            let reaped = child.id().is_none();
            if (even_if_reaped || !reaped)
                && let Ok(group) = libc::pid_t::try_from(self.pid)
                && group > 1
            {
                // SAFETY: killpg only sends a signal; spawn made this child the leader of its own group.
                unsafe { libc::killpg(group, libc::SIGKILL) };
            }
            let _ = child.start_kill();
        });
    }

    /// Waits until no process of the killed group is left, so the one removal attempt of the
    /// workspace does not race a dying browser that still creates files in the profile.
    #[cfg(unix)]
    fn wait_group_gone(&self, timeout: Duration) {
        let Ok(group) = libc::pid_t::try_from(self.pid) else {
            return;
        };
        if group <= 1 {
            return;
        }
        let deadline = Instant::now().checked_add(timeout);
        loop {
            let _ = self.try_exit();
            // SAFETY: signal 0 only checks whether a process of the group still exists.
            let probed = unsafe { libc::killpg(group, 0) };
            let alive =
                probed == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM);
            if !alive || deadline.is_none_or(|deadline| Instant::now() >= deadline) {
                return;
            }
            std::thread::sleep(EXIT_POLL);
        }
    }

    fn wait_exit_blocking(&self, timeout: Duration) -> bool {
        let deadline = Instant::now().checked_add(timeout);
        loop {
            match self.try_exit() {
                Ok(Some(_)) => return true,
                Ok(None) if deadline.is_some_and(|deadline| Instant::now() < deadline) => {
                    std::thread::sleep(EXIT_POLL);
                }
                Ok(None) | Err(_) => return false,
            }
        }
    }

    fn try_exit(&self) -> std::io::Result<Option<ExitSummary>> {
        #[cfg(unix)]
        {
            Ok(with_lock(&self.child, tokio::process::Child::try_wait)?
                .map(ExitSummary::from_status))
        }
        #[cfg(windows)]
        {
            Ok(self
                .child
                .try_wait()?
                .map(|code| ExitSummary::Code(i64::from(code))))
        }
    }
}

#[cfg(unix)]
impl Drop for ProcessHandle {
    fn drop(&mut self) {
        // Only an unreaped leader proves the group id is still ours (see kill_group).
        let unreaped = with_lock(&self.child, |child| child.id().is_some());
        if unreaped && self.workspace.removes_anything() {
            self.wait_group_gone(KILLED_EXIT_WAIT);
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ExitSummary {
    Code(i64),
    #[cfg_attr(not(unix), allow(dead_code))]
    Signal(i32),
    #[cfg_attr(not(unix), allow(dead_code))]
    Unknown,
}

impl ExitSummary {
    #[cfg(unix)]
    fn from_status(status: std::process::ExitStatus) -> Self {
        use std::os::unix::process::ExitStatusExt;
        match (status.code(), status.signal()) {
            (Some(code), _) => Self::Code(i64::from(code)),
            (None, Some(signal)) => Self::Signal(signal),
            (None, None) => Self::Unknown,
        }
    }
}

impl std::fmt::Display for ExitSummary {
    fn fmt(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            Self::Code(code) => write!(formatter, "code {code}"),
            Self::Signal(signal) => write!(formatter, "signal {signal}"),
            Self::Unknown => formatter.write_str("unknown status"),
        }
    }
}

enum WatchdogSignal {
    Arm(Duration),
    Exited,
    Detach(Arc<ProcessHandle>),
}

fn run_watchdog(signals: &Receiver<WatchdogSignal>, process: &Weak<ProcessHandle>) {
    let mut deadline: Option<Instant> = None;
    loop {
        let signal = match deadline {
            Some(at) => signals.recv_timeout(at.saturating_duration_since(Instant::now())),
            None => signals.recv().map_err(|_| RecvTimeoutError::Disconnected),
        };
        match signal {
            Ok(WatchdogSignal::Arm(grace)) => {
                if let Some(at) = Instant::now().checked_add(grace) {
                    deadline = Some(deadline.map_or(at, |current| current.min(at)));
                }
            }
            Ok(WatchdogSignal::Detach(owned)) => {
                if let Some(at) = deadline {
                    std::thread::sleep(at.saturating_duration_since(Instant::now()));
                }
                end_grace(&owned);
                owned.wait_exit_blocking(KILLED_EXIT_WAIT);
                owned.workspace.remove(REMOVE_ATTEMPTS);
                return;
            }
            Ok(WatchdogSignal::Exited) | Err(RecvTimeoutError::Disconnected) => return,
            Err(RecvTimeoutError::Timeout) => {
                if let Some(process) = process.upgrade() {
                    end_grace(&process);
                }
                return;
            }
        }
    }
}

fn end_grace(process: &ProcessHandle) {
    tracing::debug!(
        pid = process.pid,
        "the browser shutdown grace is over; killing what is left of its process group"
    );
    process.kill_unless_reaped();
}

struct Workspace {
    profile: Profile,
    staging: Option<PathBuf>,
    cft_marker: Mutex<Option<CftMarker>>,
}

impl Workspace {
    fn prepare(executable: &Executable, options: &LaunchOptions) -> crate::Result<Workspace> {
        sweep_scratch_roots();
        let scratch = profile::scratch_root(executable.flavor)?;
        let locale = options.locale.as_deref();
        let mut workspace = match &options.user_data_dir {
            Some(dir) => {
                profile::ensure_profile_available(dir, executable)?;
                profile::prepare_user_data_dir(dir, locale)?;
                Workspace::new(dir.clone(), false)
            }
            None => {
                let workspace = Workspace::new(profile::create_temp_profile(executable)?, true);
                profile::prepare_user_data_dir(&workspace.profile.dir, locale)?;
                workspace
            }
        };
        workspace.staging = Some(create_staging_dir(&scratch)?);
        workspace.cft_marker = Mutex::new(CftMarker::acquire(executable));
        Ok(workspace)
    }

    fn new(dir: PathBuf, temporary: bool) -> Workspace {
        Workspace {
            profile: Profile { dir, temporary },
            staging: None,
            cft_marker: Mutex::new(None),
        }
    }

    #[cfg(unix)]
    fn removes_anything(&self) -> bool {
        self.staging.is_some() || self.profile.temporary
    }

    fn remove(&self, attempts: u32) {
        drop(with_lock(&self.cft_marker, Option::take));
        if let Some(staging) = &self.staging {
            remove_dir(staging, attempts);
        }
        if self.profile.temporary {
            remove_dir(&self.profile.dir, attempts);
        }
    }
}

impl Drop for Workspace {
    fn drop(&mut self) {
        // On Windows a process that left the job before WindowsChild held it, or a scanner,
        // can keep a file open for a moment after the drop wait.
        self.remove(if cfg!(windows) { REMOVE_ATTEMPTS } else { 1 });
    }
}

fn sweep_scratch_roots() {
    profile::sweep_stale_profiles(&std::env::temp_dir());
    #[cfg(not(any(target_os = "macos", windows)))]
    if let Some(snap_root) = profile::snap_profile_root() {
        profile::sweep_stale_profiles(&snap_root);
    }
}

fn create_staging_dir(root: &Path) -> crate::Result<PathBuf> {
    let dir = root.join(format!(
        "{}{}-{}",
        profile::STAGING_PREFIX,
        std::process::id(),
        uuid::Uuid::new_v4().simple()
    ));
    profile::create_private_dir(&dir).map_err(|error| {
        BrowserError::io(
            format!("Creating the download staging folder {}", dir.display()),
            &error,
        )
    })?;
    Ok(dir)
}

fn remove_dir(dir: &Path, attempts: u32) {
    for attempt in 1..=attempts {
        match std::fs::remove_dir_all(dir) {
            Ok(()) => return,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return,
            Err(error) if attempt == attempts => {
                tracing::warn!(dir = %dir.display(), %error, "could not remove a browser scratch folder; the sweep of a later run removes it once this process has exited");
            }
            Err(_) => std::thread::sleep(REMOVE_RETRY),
        }
    }
}

struct CftMarker {
    path: PathBuf,
}

impl CftMarker {
    fn acquire(executable: &Executable) -> Option<CftMarker> {
        if executable.source != ExecutableSource::CachedCft {
            return None;
        }
        let install = discovery::cft_install_dir(&executable.path)?;
        let path = install.join(format!("{IN_USE_PREFIX}{}", std::process::id()));
        with_lock(&CFT_MARKER_USERS, |users| {
            let count = users.get(&path).copied().unwrap_or(0);
            if count == 0
                && let Err(error) = std::fs::write(&path, b"")
            {
                tracing::warn!(marker = %path.display(), %error, "could not mark Chrome for Testing as in use");
                return None;
            }
            users.insert(path.clone(), count + 1);
            Some(CftMarker { path })
        })
    }
}

impl Drop for CftMarker {
    fn drop(&mut self) {
        with_lock(&CFT_MARKER_USERS, |users| {
            let remaining = users
                .get(&self.path)
                .map_or(0, |count| count.saturating_sub(1));
            if remaining > 0 {
                users.insert(self.path.clone(), remaining);
                return;
            }
            users.remove(&self.path);
            if let Err(error) = std::fs::remove_file(&self.path)
                && error.kind() != std::io::ErrorKind::NotFound
            {
                tracing::warn!(marker = %self.path.display(), %error, "could not remove the Chrome for Testing in-use marker");
            }
        });
    }
}

fn spawn_stderr_tail(mut stream: ChildStream, tail: Arc<Mutex<Vec<u8>>>) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut chunk = [0u8; 4096];
        while let Ok(read) = stream.read(&mut chunk).await {
            if read == 0 {
                break;
            }
            with_lock(&tail, |tail| append_bounded(tail, &chunk[..read]));
        }
    })
}

fn append_bounded(tail: &mut Vec<u8>, bytes: &[u8]) {
    tail.extend_from_slice(bytes);
    if tail.len() > STDERR_TAIL_BYTES {
        let excess = tail.len() - STDERR_TAIL_BYTES;
        tail.drain(..excess);
    }
}

fn endpoint_from_stderr(stderr: &str) -> Option<(u16, String)> {
    stderr
        .split_inclusive('\n')
        .filter(|line| line.ends_with('\n'))
        .filter_map(|line| line.split_once(DEVTOOLS_LISTENING))
        .filter_map(|(_, url)| url::Url::parse(url.trim()).ok())
        .filter(|url| url.scheme() == "ws" && url.path().starts_with(BROWSER_WS_PATH))
        .find_map(|url| Some((url.port().filter(|port| *port != 0)?, url.path().to_owned())))
}

fn startup_failure(
    flavor: Flavor,
    profile_dir: &Path,
    exit: ExitSummary,
    stderr: &str,
) -> BrowserError {
    if exit == ExitSummary::Code(PROFILE_IN_USE_EXIT_CODE)
        || stderr.contains(PROCESS_SINGLETON_FAILURE)
    {
        return profile::in_use_error(profile_dir, profile::lock_owner(profile_dir));
    }
    if let Some(error) = sandbox::classify_stderr(stderr) {
        return error;
    }
    BrowserError::Launch {
        message: format!(
            "{} exited during startup ({exit}): {}",
            browser_name(flavor),
            output_excerpt(stderr)
        ),
    }
}

fn endpoint_timeout(
    flavor: Flavor,
    profile_dir: &Path,
    timeout: Duration,
    problem: Option<String>,
    stderr: &str,
) -> BrowserError {
    let detail = problem.unwrap_or_else(|| {
        format!(
            "{} was not written",
            profile_dir.join(DEVTOOLS_ACTIVE_PORT).display()
        )
    });
    BrowserError::Launch {
        message: format!(
            "{} did not report its DevTools endpoint within {} s ({detail}). Browser output: {}",
            browser_name(flavor),
            timeout.as_secs_f64(),
            output_excerpt(stderr)
        ),
    }
}

fn output_excerpt(stderr: &str) -> String {
    let output = stderr.trim();
    if output.is_empty() {
        return "no output".to_owned();
    }
    match output.char_indices().rev().nth(MESSAGE_OUTPUT_CHARS - 1) {
        Some((start, _)) if start > 0 => format!("...{}", &output[start..]),
        _ => output.to_owned(),
    }
}

fn spawn_failure(
    executable: &Executable,
    error: &std::io::Error,
    policy_denial: bool,
) -> BrowserError {
    if policy_denial && executable.source == ExecutableSource::CachedCft {
        return BrowserError::Launch {
            message: CFT_BLOCKED.to_owned(),
        };
    }
    BrowserError::Launch {
        message: format!(
            "Could not start {} at {}: {error}",
            browser_name(executable.flavor),
            executable.path.display()
        ),
    }
}

#[cfg(any(test, windows))]
fn is_policy_denial(raw_os_error: Option<i32>) -> bool {
    const ERROR_ACCESS_DENIED: i32 = 5;
    const ERROR_ACCESS_DISABLED_BY_POLICY: i32 = 1260;
    const ERROR_SYSTEM_INTEGRITY_POLICY_VIOLATION: i32 = 4551;
    matches!(
        raw_os_error,
        Some(
            ERROR_ACCESS_DENIED
                | ERROR_ACCESS_DISABLED_BY_POLICY
                | ERROR_SYSTEM_INTEGRITY_POLICY_VIOLATION
        )
    )
}

#[cfg(windows)]
fn start_windows_child(
    executable: &Executable,
    arguments: &[OsString],
    env: &[(OsString, OsString)],
    workspace: Workspace,
    private_desktop: bool,
) -> crate::Result<(BrowserProcess, ChildStream)> {
    let (child, stderr) = PlatformChild::spawn(&executable.path, arguments, env, private_desktop)
        .map_err(|error| {
        spawn_failure(executable, &error, is_policy_denial(error.raw_os_error()))
    })?;
    Ok((
        BrowserProcess::assemble(ProcessHandle::new(child, workspace)),
        Box::new(tokio::fs::File::from_std(stderr)),
    ))
}

#[cfg(unix)]
fn start_unix_child(
    executable: &Executable,
    arguments: &[OsString],
    env: &[(OsString, OsString)],
    workspace: Workspace,
) -> crate::Result<(BrowserProcess, ChildStream)> {
    use std::os::fd::AsRawFd;
    use std::process::Stdio;

    let pipes = args::uses_lifeline(executable.flavor)
        .then(lifeline::Pipes::create)
        .transpose()
        .map_err(|error| BrowserError::io("Creating the browser lifeline pipes", &error))?;
    let mut command = tokio::process::Command::new(&executable.path);
    command
        .args(arguments)
        .envs(env.iter().cloned())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .process_group(0);
    if let Some(pipes) = &pipes {
        let chrome_reads = pipes.chrome_reads.as_raw_fd();
        let chrome_writes = pipes.chrome_writes.as_raw_fd();
        // SAFETY: the hook runs between fork and exec and only calls fcntl, dup2 and close, which are async-signal-safe.
        unsafe {
            command.pre_exec(move || lifeline::install(chrome_reads, chrome_writes));
        }
    }
    let mut child = command
        .spawn()
        .map_err(|error| spawn_failure(executable, &error, false))?;
    let stderr: ChildStream = match child.stderr.take() {
        Some(stderr) => Box::new(stderr),
        None => Box::new(tokio::io::empty()),
    };
    let lifeline_writer = pipes.map(lifeline::Pipes::into_parent_writer);
    Ok((
        BrowserProcess::assemble(ProcessHandle::new(child, workspace), lifeline_writer),
        stderr,
    ))
}

#[cfg(unix)]
mod lifeline {
    use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};

    use tokio::io::AsyncReadExt;

    // Chrome started with --remote-debugging-pipe reads fd 3 and writes fd 4 and quits on EOF.
    const CHROME_READ_FD: RawFd = 3;
    const CHROME_WRITE_FD: RawFd = 4;
    const FIRST_SPARE_FD: RawFd = 5;

    pub(super) struct Pipes {
        pub(super) chrome_reads: OwnedFd,
        pub(super) chrome_writes: OwnedFd,
        parent_writes: OwnedFd,
        parent_reads: OwnedFd,
    }

    impl Pipes {
        pub(super) fn create() -> std::io::Result<Pipes> {
            let (chrome_reads, parent_writes) = cloexec_pipe()?;
            let (parent_reads, chrome_writes) = cloexec_pipe()?;
            Ok(Pipes {
                chrome_reads,
                chrome_writes,
                parent_writes,
                parent_reads,
            })
        }

        pub(super) fn into_parent_writer(self) -> OwnedFd {
            drain(self.parent_reads);
            self.parent_writes
        }
    }

    pub(super) fn install(chrome_reads: RawFd, chrome_writes: RawFd) -> std::io::Result<()> {
        let reads = duplicate_above_stdio(chrome_reads)?;
        let writes = duplicate_above_stdio(chrome_writes)?;
        move_onto(reads, CHROME_READ_FD)?;
        move_onto(writes, CHROME_WRITE_FD)
    }

    fn duplicate_above_stdio(fd: RawFd) -> std::io::Result<RawFd> {
        // SAFETY: F_DUPFD only duplicates a descriptor this process owns.
        let duplicate = unsafe { libc::fcntl(fd, libc::F_DUPFD, FIRST_SPARE_FD) };
        if duplicate < 0 {
            Err(std::io::Error::last_os_error())
        } else {
            Ok(duplicate)
        }
    }

    fn move_onto(fd: RawFd, target: RawFd) -> std::io::Result<()> {
        // SAFETY: fd is a private duplicate at or above FIRST_SPARE_FD, so it never equals target.
        let result = unsafe { libc::dup2(fd, target) };
        let error = (result < 0).then(std::io::Error::last_os_error);
        // SAFETY: fd was created by duplicate_above_stdio and is closed exactly once.
        unsafe { libc::close(fd) };
        error.map_or(Ok(()), Err)
    }

    fn cloexec_pipe() -> std::io::Result<(OwnedFd, OwnedFd)> {
        let mut fds: [libc::c_int; 2] = [-1, -1];
        #[cfg(any(target_os = "linux", target_os = "android"))]
        // SAFETY: fds is a writable array of two descriptors.
        let created = unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC) };
        #[cfg(not(any(target_os = "linux", target_os = "android")))]
        // SAFETY: fds is a writable array of two descriptors.
        let created = unsafe { libc::pipe(fds.as_mut_ptr()) };
        if created != 0 {
            return Err(std::io::Error::last_os_error());
        }
        // SAFETY: pipe returned two fresh descriptors that nothing else owns.
        let pair = unsafe { (OwnedFd::from_raw_fd(fds[0]), OwnedFd::from_raw_fd(fds[1])) };
        #[cfg(not(any(target_os = "linux", target_os = "android")))]
        for fd in [&pair.0, &pair.1] {
            set_cloexec(fd)?;
        }
        Ok(pair)
    }

    #[cfg(not(any(target_os = "linux", target_os = "android")))]
    fn set_cloexec(fd: &OwnedFd) -> std::io::Result<()> {
        // SAFETY: F_SETFD only changes the descriptor flags of an owned descriptor.
        if unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_SETFD, libc::FD_CLOEXEC) } < 0 {
            return Err(std::io::Error::last_os_error());
        }
        Ok(())
    }

    fn drain(fd: OwnedFd) {
        let raw = fd.as_raw_fd();
        match tokio::net::unix::pipe::Receiver::from_owned_fd(fd) {
            Ok(mut receiver) => {
                tokio::spawn(async move {
                    let mut sink = [0u8; 1024];
                    while matches!(receiver.read(&mut sink).await, Ok(read) if read > 0) {}
                });
            }
            Err(error) => {
                tracing::debug!(fd = raw, %error, "the browser lifeline output cannot be drained");
            }
        }
    }
}

fn with_lock<T, R>(mutex: &Mutex<T>, action: impl FnOnce(&mut T) -> R) -> R {
    action(&mut mutex.lock().unwrap_or_else(PoisonError::into_inner))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn executable(source: ExecutableSource, path: &str) -> Executable {
        Executable {
            path: PathBuf::from(path),
            flavor: Flavor::ChromeForTesting,
            version: None,
            source,
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn dropping_a_killed_process_waits_for_its_group_before_removing_the_profile() {
        let profile = tempfile::Builder::new()
            .prefix("flow-like-group-wait-")
            .tempdir()
            .unwrap()
            .keep();
        std::fs::write(profile.join("Local State"), "{}").unwrap();
        let child = tokio::process::Command::new("/bin/sh")
            .args(["-c", "sleep 30 & sleep 30"])
            .process_group(0)
            .spawn()
            .unwrap();
        let handle = ProcessHandle::new(child, Workspace::new(profile.clone(), true));
        let group = libc::pid_t::try_from(handle.pid).unwrap();
        handle.kill();
        drop(handle);
        // SAFETY: signal 0 only checks whether a process of the group still exists.
        let probed = unsafe { libc::killpg(group, 0) };
        assert_eq!(
            probed, -1,
            "a process of the killed group outlived the drop"
        );
        assert_eq!(
            std::io::Error::last_os_error().raw_os_error(),
            Some(libc::ESRCH)
        );
        assert!(!profile.exists(), "{} survived the drop", profile.display());
    }

    #[cfg(windows)]
    #[test]
    fn dropping_a_killed_process_waits_for_its_job_before_removing_the_profile() {
        let dir = tempfile::tempdir().unwrap();
        let (child, _leaf, profile) =
            crate::launch::windows_process::tests::spawn_leaf_holding_a_profile_file(dir.path());
        let handle = ProcessHandle::new(child, Workspace::new(profile.clone(), true));
        handle.kill();
        drop(handle);
        assert!(!profile.exists(), "{} survived the drop", profile.display());
    }

    #[test]
    fn stderr_announces_the_endpoint_only_on_complete_lines() {
        let line = "DevTools listening on ws://127.0.0.1:39123/devtools/browser/0b4c6f2e\n";
        assert_eq!(
            endpoint_from_stderr(&format!("[warn] noise\n{line}")),
            Some((39123, "/devtools/browser/0b4c6f2e".to_owned()))
        );
        assert_eq!(endpoint_from_stderr(line.trim_end()), None);
        assert_eq!(
            endpoint_from_stderr("DevTools listening on ws://127.0.0.1:0/devtools/browser/x\n"),
            None
        );
        assert_eq!(
            endpoint_from_stderr("DevTools listening on ws://127.0.0.1:9222/json/version\n"),
            None
        );
        assert_eq!(
            endpoint_from_stderr(
                "DevTools listening on http://127.0.0.1:9222/devtools/browser/x\n"
            ),
            None
        );
    }

    #[test]
    fn the_active_port_file_is_parsed_strictly() {
        let dir = tempfile::tempdir().expect("tempdir");
        let file = dir.path().join(DEVTOOLS_ACTIVE_PORT);
        let read = || crate::attach::read_devtools_active_port(dir.path());
        assert!(matches!(read(), Ok(None)));
        std::fs::write(&file, "41234\n/devtools/browser/5f1c").expect("write");
        assert_eq!(
            read().expect("valid"),
            Some((41234, "/devtools/browser/5f1c".to_owned()))
        );
        for partial in [
            "41234",
            "41234\n",
            "0\n/devtools/browser/x",
            "70000\n/devtools/browser/x",
            "41234\n/json/version",
            "41234\n/devtools/browser/x\nextra",
        ] {
            std::fs::write(&file, partial).expect("write");
            assert!(
                !matches!(read(), Ok(Some(_))),
                "{partial:?} is not an endpoint"
            );
        }
    }

    #[test]
    fn startup_failures_are_classified() {
        let dir = tempfile::tempdir().expect("tempdir");
        let in_use =
            startup_failure(Flavor::Chrome, dir.path(), ExitSummary::Code(21), "").to_string();
        assert!(
            in_use.contains("is in use by another browser process"),
            "{in_use}"
        );
        let singleton = startup_failure(
            Flavor::Chrome,
            dir.path(),
            ExitSummary::Code(0),
            "[1:1:ERROR:process_singleton_posix.cc] Failed to create a ProcessSingleton for your profile directory.",
        )
        .to_string();
        assert!(singleton.contains("is in use by"), "{singleton}");
        let sandbox = startup_failure(
            Flavor::ChromeForTesting,
            dir.path(),
            ExitSummary::Code(1),
            "No usable sandbox! Update your kernel",
        )
        .to_string();
        assert!(sandbox.contains("AppArmor"), "{sandbox}");
        assert_eq!(
            startup_failure(Flavor::Edge, dir.path(), ExitSummary::Code(3), "  boom\n").to_string(),
            "Microsoft Edge exited during startup (code 3): boom"
        );
        assert_eq!(
            startup_failure(Flavor::Chrome, dir.path(), ExitSummary::Signal(9), "").to_string(),
            "Google Chrome exited during startup (signal 9): no output"
        );
    }

    #[test]
    fn timeouts_name_the_missing_file_or_the_parse_problem() {
        let missing = endpoint_timeout(
            Flavor::Chrome,
            Path::new("/p"),
            Duration::from_millis(1500),
            None,
            "",
        )
        .to_string();
        assert_eq!(
            missing,
            format!(
                "Google Chrome did not report its DevTools endpoint within 1.5 s ({} was not written). Browser output: no output",
                Path::new("/p").join(DEVTOOLS_ACTIVE_PORT).display()
            )
        );
        let invalid = endpoint_timeout(
            Flavor::Chromium,
            Path::new("/p"),
            Duration::from_secs(30),
            Some("bad port".to_owned()),
            "log line",
        )
        .to_string();
        assert_eq!(
            invalid,
            "Chromium did not report its DevTools endpoint within 30 s (bad port). Browser output: log line"
        );
    }

    #[test]
    fn output_excerpts_keep_the_end_and_respect_characters() {
        let long = format!("{}é", "x".repeat(MESSAGE_OUTPUT_CHARS + 10));
        let excerpt = output_excerpt(&long);
        assert!(excerpt.starts_with("..."));
        assert!(excerpt.ends_with('é'));
        assert_eq!(excerpt.chars().count(), MESSAGE_OUTPUT_CHARS + 3);
        let exact = "y".repeat(MESSAGE_OUTPUT_CHARS);
        assert_eq!(output_excerpt(&exact), exact);
    }

    #[test]
    fn the_stderr_tail_is_bounded() {
        let mut tail = Vec::new();
        append_bounded(&mut tail, &vec![b'a'; STDERR_TAIL_BYTES - 1]);
        append_bounded(&mut tail, b"bcd");
        assert_eq!(tail.len(), STDERR_TAIL_BYTES);
        assert!(tail.ends_with(b"abcd"));
        assert_eq!(tail[0], b'a');
    }

    #[test]
    fn spawn_denials_by_policy_only_rewrite_chrome_for_testing() {
        assert!(is_policy_denial(Some(5)));
        assert!(is_policy_denial(Some(1260)));
        assert!(is_policy_denial(Some(4551)));
        assert!(!is_policy_denial(Some(2)));
        assert!(!is_policy_denial(None));
        let error = std::io::Error::from_raw_os_error(5);
        let cft = executable(ExecutableSource::CachedCft, "/cache/chrome");
        assert_eq!(spawn_failure(&cft, &error, true).to_string(), CFT_BLOCKED);
        let installed = executable(ExecutableSource::Installed, "/opt/chrome");
        let message = spawn_failure(&installed, &error, true).to_string();
        assert!(
            message.starts_with("Could not start Chrome for Testing at /opt/chrome: "),
            "{message}"
        );
        assert!(
            spawn_failure(&cft, &error, false)
                .to_string()
                .starts_with("Could not start")
        );
    }

    #[test]
    fn chrome_for_testing_markers_are_shared_by_concurrent_launches() {
        let cache = tempfile::tempdir().expect("tempdir");
        let install = cache
            .path()
            .join(crate::launch::cft::CACHE_SUBDIR)
            .join("linux64-154.0.8037.92");
        std::fs::create_dir_all(install.join("chrome-linux64")).expect("mkdir");
        let cft = executable(
            ExecutableSource::CachedCft,
            &install
                .join("chrome-linux64")
                .join("chrome")
                .to_string_lossy(),
        );
        let marker = install.join(format!("{IN_USE_PREFIX}{}", std::process::id()));
        let first = CftMarker::acquire(&cft).expect("marked");
        let second = CftMarker::acquire(&cft).expect("marked again");
        assert!(marker.is_file());
        drop(first);
        assert!(marker.is_file(), "the second launch still uses the build");
        drop(second);
        assert!(!marker.exists());
        let explicit = executable(ExecutableSource::Explicit, &cft.path.to_string_lossy());
        assert!(CftMarker::acquire(&explicit).is_none());
    }

    fn chrome() -> Executable {
        Executable {
            path: PathBuf::from("/nonexistent/chrome"),
            flavor: Flavor::Chrome,
            version: None,
            source: ExecutableSource::Explicit,
        }
    }

    fn launch_options(user_data_dir: Option<PathBuf>) -> LaunchOptions {
        LaunchOptions {
            kind: crate::launch::BrowserKind::Chrome,
            executable: None,
            headless: true,
            window_size: (800, 600),
            user_agent: None,
            user_data_dir,
            proxy: None,
            locale: Some("de-DE".to_owned()),
            ignore_https_errors: false,
            cache_dir: PathBuf::from("/nonexistent/cache"),
            page_load_timeout: Duration::from_secs(30),
            launch_timeout: Duration::from_secs(30),
        }
    }

    #[test]
    fn a_temporary_workspace_is_removed_when_dropped() {
        let workspace = Workspace::prepare(&chrome(), &launch_options(None)).expect("prepared");
        let profile_dir = workspace.profile.dir.clone();
        let staging = workspace.staging.clone().expect("staging folder");
        assert!(workspace.profile.temporary);
        assert!(profile_dir.join("Default").join("Preferences").is_file());
        assert!(profile_dir.join("First Run").is_file());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&staging)
                .expect("staging")
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o700);
        }
        drop(workspace);
        assert!(!profile_dir.exists());
        assert!(!staging.exists());
    }

    #[test]
    fn a_persistent_profile_survives_workspace_removal() {
        let persistent = tempfile::tempdir().expect("tempdir");
        let options = launch_options(Some(persistent.path().to_path_buf()));
        let workspace = Workspace::prepare(&chrome(), &options).expect("prepared");
        let staging = workspace.staging.clone().expect("staging folder");
        workspace.remove(REMOVE_ATTEMPTS);
        let preferences = persistent.path().join("Default").join("Preferences");
        assert!(preferences.is_file());
        assert!(!staging.exists());
    }
}
