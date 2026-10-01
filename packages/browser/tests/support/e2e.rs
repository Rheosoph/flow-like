#![allow(dead_code)]

use std::collections::{HashMap, VecDeque};
use std::fmt::Write as _;
use std::future::Future;
use std::net::{Ipv4Addr, Ipv6Addr};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use flow_like_browser::attach::{self, AttachEndpoint};
use flow_like_browser::connection::{Connection, EventHook};
use flow_like_browser::event_log::Event;
use flow_like_browser::launch::cft::{self, CftVersion};
use flow_like_browser::launch::{
    self, BrowserKind, Executable, ExecutableSource, Flavor, LaunchOptions, discovery,
};
use flow_like_browser::refs::{NodeRef, RefTable};
use flow_like_browser::script::ScriptOptions;
use flow_like_browser::snapshot::SnapshotOptions;
use flow_like_browser::test_hooks;
use flow_like_browser::types::TargetId;
use flow_like_browser::{Browser, Element, Frame, Page, PageInfo};
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{OnceCell, Semaphore};
use tokio::time::Instant;

pub const TEST_BUDGET: Duration = Duration::from_secs(60);
pub const RECORD_ENV: &str = "FLOW_LIKE_CDP_RECORD";
pub const SNAPSHOT_OPTIONS: SnapshotOptions = SnapshotOptions {
    max_child_frames: 20,
    capture_dom_snapshots: false,
};

const CHROME_ENV: &str = "FLOW_LIKE_BROWSER_E2E_CHROME";
const PROVISION_ENV: &str = "FLOW_LIKE_BROWSER_E2E_PROVISION";
const CACHE_ENV: &str = "FLOW_LIKE_BROWSER_CACHE_DIR";
const HEADFUL_ENV: &str = "FLOW_LIKE_BROWSER_E2E_HEADFUL";
const PROFILE_ROOT_ENV: &str = "FLOW_LIKE_BROWSER_E2E_PROFILE_ROOT";
const NO_BROWSER: &str = "The Chromium e2e tests were run (--ignored or --include-ignored) but no browser was found. Install Google Chrome, set FLOW_LIKE_BROWSER_E2E_CHROME=<path to the executable>, or set FLOW_LIKE_BROWSER_E2E_PROVISION=stable (the build Settings > Automation installs) or download (the pinned build) to install Chrome for Testing.";
const MAX_BROWSERS: usize = 4;
const WINDOW_SIZE: (u32, u32) = (1280, 800);
const PAGE_LOAD_TIMEOUT: Duration = Duration::from_secs(20);
const LAUNCH_TIMEOUT: Duration = Duration::from_secs(30);
const POLL: Duration = Duration::from_millis(50);
const CLEANUP_WAIT: Duration = Duration::from_secs(10);
const SNAPSHOT_WAIT: Duration = Duration::from_secs(5);
const TRANSCRIPT_LINES: usize = 4000;
const TRANSCRIPT_PARAMS: usize = 300;
const SCRATCH_NAME: usize = 24;
const MAX_REQUEST_HEAD: usize = 64 * 1024;
const LARGE_PAYLOAD: usize = 4096;
const LARGE_SENT_TEXT: usize = 1024;
const REF_ROLES: [&str; 6] = [
    "button", "textbox", "link", "checkbox", "combobox", "heading",
];

static BROWSERS: Semaphore = Semaphore::const_new(MAX_BROWSERS);
static EXECUTABLE: OnceCell<Executable> = OnceCell::const_new();

pub fn recording_enabled() -> bool {
    std::env::var(RECORD_ENV).is_ok_and(|value| value == "1")
}

pub fn executable_overridden() -> bool {
    env_path(CHROME_ENV).is_some()
}

pub fn headless() -> bool {
    std::env::var_os(HEADFUL_ENV).is_none()
}

/// The Chrome for Testing build `FLOW_LIKE_BROWSER_E2E_PROVISION` installs: `stable` the newest
/// Stable one, as Settings > Automation > Install does, or `download` the pinned one.
fn provision_version() -> Option<CftVersion> {
    let mode = std::env::var(PROVISION_ENV)
        .ok()
        .filter(|mode| !mode.is_empty())?;
    match mode.as_str() {
        "stable" => Some(CftVersion::Stable),
        "download" => Some(CftVersion::Pinned),
        _ => panic!("{PROVISION_ENV}={mode} is neither stable nor download"),
    }
}

fn provisioning() -> bool {
    provision_version().is_some()
}

/// A provisioning run never installs into (and prunes) the developer's own browser cache.
pub fn cache_dir() -> PathBuf {
    if provisioning() && env_path(CACHE_ENV).is_none() {
        return logs_dir().with_file_name("browser-e2e-cache");
    }
    launch::default_cache_dir()
}

pub fn profile_root() -> PathBuf {
    env_path(PROFILE_ROOT_ENV)
        .unwrap_or_else(|| std::env::temp_dir().join("flow-like-e2e-profiles"))
}

pub fn logs_dir() -> PathBuf {
    std::env::var_os("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target"))
        .join("browser-e2e-logs")
}

fn env_path(name: &str) -> Option<PathBuf> {
    std::env::var_os(name)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

pub async fn executable() -> Executable {
    EXECUTABLE.get_or_init(resolve_executable).await.clone()
}

async fn resolve_executable() -> Executable {
    if let Some(path) = env_path(CHROME_ENV) {
        assert!(
            path.is_file(),
            "{CHROME_ENV}={} is not a file",
            path.display()
        );
        let flavor = flavor_of(&path);
        return Executable {
            version: discovery::executable_version(&path, flavor),
            path,
            flavor,
            source: ExecutableSource::Explicit,
        };
    }
    if let Some(version) = provision_version() {
        let cache = cache_dir();
        return cft::install(version.clone(), &cache, |_| {})
            .await
            .unwrap_or_else(|error| {
                panic!(
                    "{PROVISION_ENV} could not install Chrome for Testing ({version:?}) into {}: {error}",
                    cache.display()
                )
            });
    }
    launch::find(BrowserKind::Chrome, &cache_dir())
        .unwrap_or_else(|error| panic!("{NO_BROWSER}\n{error}"))
}

fn flavor_of(path: &Path) -> Flavor {
    let text = path.to_string_lossy().to_lowercase().replace('\\', "/");
    let testing = [
        "chrome for testing",
        "chrome-linux64/",
        "chrome-win64/",
        "chrome-mac-",
    ];
    if testing.iter().any(|marker| text.contains(marker)) {
        Flavor::ChromeForTesting
    } else if text.starts_with("/snap/") {
        Flavor::SnapChromium
    } else if text.contains("msedge") || text.contains("microsoft edge") {
        Flavor::Edge
    } else if text.contains("chromium") {
        Flavor::Chromium
    } else {
        Flavor::Chrome
    }
}

/// Writes the executable's path, then its version (empty when unknown), one per line.
pub fn record_provisioned(executable: &Executable) -> PathBuf {
    let path = logs_dir().join("provision_browser.txt");
    let record = format!(
        "{}\n{}\n",
        executable.path.display(),
        executable.version.as_deref().unwrap_or_default()
    );
    let written = std::fs::create_dir_all(logs_dir()).and_then(|()| std::fs::write(&path, record));
    if let Err(error) = written {
        eprintln!("could not write {}: {error}", path.display());
    }
    path
}

pub async fn run<F, Fut>(name: &str, body: F)
where
    F: FnOnce(E2e) -> Fut,
    Fut: Future<Output = ()>,
{
    let executable = executable().await;
    let _permit = BROWSERS
        .acquire()
        .await
        .expect("the e2e browser semaphore stays open");
    let log = Log::new(name);
    let tracked = Tracked::default();
    let scratch = create_scratch(name);
    let mut guard = FailureGuard {
        log: log.clone(),
        tracked: tracked.clone(),
        scratch: Some(scratch),
        armed: true,
    };
    let context = E2e {
        log: log.clone(),
        executable,
        tracked: tracked.clone(),
        scratch: guard.scratch_path(),
    };
    let finished = tokio::time::timeout(TEST_BUDGET, body(context)).await;
    if finished.is_err() {
        guard.armed = false;
        let reason = format!("timed out after {TEST_BUDGET:?}");
        let path = log.write_failure(&reason, &tracked.stderr_tails());
        panic!("{name} {reason}; log: {}", path.display());
    }
    guard.armed = false;
}

fn create_scratch(name: &str) -> tempfile::TempDir {
    let root = profile_root();
    std::fs::create_dir_all(&root)
        .unwrap_or_else(|error| panic!("creating {}: {error}", root.display()));
    let short: String = name.chars().take(SCRATCH_NAME).collect();
    tempfile::Builder::new()
        .prefix(&format!("e2e-{short}-"))
        .tempdir_in(&root)
        .unwrap_or_else(|error| panic!("creating a scratch dir in {}: {error}", root.display()))
}

pub struct E2e {
    log: Log,
    executable: Executable,
    tracked: Tracked,
    scratch: PathBuf,
}

impl E2e {
    pub fn executable(&self) -> &Executable {
        &self.executable
    }

    pub fn options(&self) -> LaunchOptions {
        LaunchOptions {
            kind: BrowserKind::Chrome,
            executable: Some(self.executable.clone()),
            headless: headless(),
            window_size: WINDOW_SIZE,
            user_agent: None,
            user_data_dir: None,
            proxy: None,
            locale: None,
            ignore_https_errors: false,
            cache_dir: cache_dir(),
            page_load_timeout: PAGE_LOAD_TIMEOUT,
            launch_timeout: LAUNCH_TIMEOUT,
        }
    }

    pub async fn launch(&self) -> Browser {
        self.launch_with(self.options()).await
    }

    pub async fn launch_with(&self, options: LaunchOptions) -> Browser {
        let browser = self.launch_untracked_with(options).await;
        self.tracked.push(browser.clone());
        browser
    }

    pub async fn launch_untracked(&self) -> Browser {
        self.launch_untracked_with(self.options()).await
    }

    pub async fn launch_untracked_with(&self, options: LaunchOptions) -> Browser {
        let started = Instant::now();
        let browser = match Browser::launch(options).await {
            Ok(browser) => browser,
            Err(error) => {
                self.note(format!("Browser::launch failed: {error}"));
                panic!("Browser::launch failed: {error}");
            }
        };
        self.note(format!(
            "launched {} at {} in {:?}",
            browser.version().product,
            browser.debugger_address().unwrap_or("-"),
            started.elapsed()
        ));
        self.watch(&browser);
        browser
    }

    pub fn watch(&self, browser: &Browser) {
        browser.connection().add_hook(Arc::new(EventTranscript {
            log: self.log.clone(),
        }));
    }

    pub fn scratch_dir(&self, name: &str) -> PathBuf {
        let dir = self.scratch.join(name);
        std::fs::create_dir_all(&dir)
            .unwrap_or_else(|error| panic!("creating {}: {error}", dir.display()));
        dir
    }

    pub fn scratch_root(&self) -> &Path {
        &self.scratch
    }

    pub fn note(&self, line: impl Into<String>) {
        self.log.note(line.into());
    }

    pub fn observe(&self, line: impl Into<String>) {
        let line = line.into();
        eprintln!("[{}] {line}", self.log.name);
        self.log.observe(&line);
    }
}

#[derive(Clone, Default)]
struct Tracked(Arc<Mutex<Vec<Browser>>>);

impl Tracked {
    fn push(&self, browser: Browser) {
        lock(&self.0).push(browser);
    }

    fn abort_all(&self) {
        let browsers: Vec<Browser> = lock(&self.0).drain(..).collect();
        for browser in browsers {
            browser.abort();
        }
    }

    fn stderr_tails(&self) -> Vec<String> {
        lock(&self.0)
            .iter()
            .filter_map(test_hooks::stderr_tail)
            .collect()
    }
}

/// Aborts every tracked browser before the scratch dir (persistent profiles, download dirs) goes.
struct FailureGuard {
    log: Log,
    tracked: Tracked,
    scratch: Option<tempfile::TempDir>,
    armed: bool,
}

impl FailureGuard {
    fn scratch_path(&self) -> PathBuf {
        self.scratch
            .as_ref()
            .map(|scratch| scratch.path().to_path_buf())
            .expect("the scratch dir lives as long as the guard")
    }
}

impl Drop for FailureGuard {
    fn drop(&mut self) {
        if self.armed && std::thread::panicking() {
            let reason = "panicked; the panic message is in the test output";
            let path = self.log.write_failure(reason, &self.tracked.stderr_tails());
            eprintln!("browser e2e log: {}", path.display());
        }
        self.tracked.abort_all();
        let Some(scratch) = self.scratch.take() else {
            return;
        };
        let deadline = std::time::Instant::now() + CLEANUP_WAIT;
        while std::time::Instant::now() < deadline {
            match try_processes_using(scratch.path()) {
                Ok(processes) if processes.is_empty() => break,
                Ok(_) => std::thread::sleep(POLL),
                Err(error) => {
                    eprintln!("browser e2e cleanup stopped waiting for its browsers: {error}");
                    break;
                }
            }
        }
        drop(scratch);
    }
}

#[derive(Default)]
struct LogLines {
    notes: Vec<String>,
    events: VecDeque<String>,
}

#[derive(Clone)]
struct Log {
    name: String,
    lines: Arc<Mutex<LogLines>>,
}

impl Log {
    fn new(name: &str) -> Log {
        for stale in [format!("{name}.log"), format!("{name}.observed.log")] {
            let _ = std::fs::remove_file(logs_dir().join(stale));
        }
        Log {
            name: name.to_owned(),
            lines: Arc::default(),
        }
    }

    fn note(&self, line: String) {
        lock(&self.lines).notes.push(line);
    }

    fn event(&self, event: &Event) {
        let session = event
            .session
            .as_ref()
            .map_or("-", |session| session.as_str());
        let params = event.params.to_string();
        let line = format!(
            "{:>7} {session} {} {}",
            event.seq,
            event.method,
            truncate(&params, TRANSCRIPT_PARAMS)
        );
        let mut lines = lock(&self.lines);
        if lines.events.len() == TRANSCRIPT_LINES {
            lines.events.pop_front();
        }
        lines.events.push_back(line);
    }

    fn observe(&self, line: &str) {
        self.note(format!("observed: {line}"));
        let path = logs_dir().join(format!("{}.observed.log", self.name));
        let mut text = std::fs::read_to_string(&path).unwrap_or_default();
        let _ = writeln!(text, "{line}");
        let appended =
            std::fs::create_dir_all(logs_dir()).and_then(|()| std::fs::write(&path, text));
        if let Err(error) = appended {
            eprintln!("could not append to {}: {error}", path.display());
        }
    }

    fn write_failure(&self, reason: &str, stderr_tails: &[String]) -> PathBuf {
        let path = logs_dir().join(format!("{}.log", self.name));
        let lines = lock(&self.lines);
        let mut text = format!("test: {}\nreason: {reason}\n\n== notes\n", self.name);
        for note in &lines.notes {
            let _ = writeln!(text, "{note}");
        }
        for (index, tail) in stderr_tails.iter().enumerate() {
            let _ = writeln!(
                text,
                "\n== Chrome stderr tail of launched browser {index}\n{tail}"
            );
        }
        let _ = writeln!(
            text,
            "\n== CDP events seen by the test connections (last {TRANSCRIPT_LINES})"
        );
        for event in &lines.events {
            let _ = writeln!(text, "{event}");
        }
        let written =
            std::fs::create_dir_all(logs_dir()).and_then(|()| std::fs::write(&path, text));
        if let Err(error) = written {
            eprintln!("could not write {}: {error}", path.display());
        }
        path
    }
}

struct EventTranscript {
    log: Log,
}

impl EventHook for EventTranscript {
    fn on_event(&self, _connection: &Connection, event: &Event) {
        self.log.event(event);
    }
}

/// Keeps the events of one domain, including those the connection's event log does not retain
/// (it logs only Page, Target, Browser, Inspector and execution-context events).
pub struct Captured {
    prefix: &'static str,
    events: Mutex<Vec<Event>>,
}

impl EventHook for Captured {
    fn on_event(&self, _connection: &Connection, event: &Event) {
        if event.method.starts_with(self.prefix) {
            lock(&self.events).push(event.clone());
        }
    }
}

impl Captured {
    pub fn on(connection: &Connection, prefix: &'static str) -> Arc<Captured> {
        let captured = Arc::new(Captured {
            prefix,
            events: Mutex::new(Vec::new()),
        });
        connection.add_hook(captured.clone());
        captured
    }

    pub async fn wait_for(
        &self,
        timeout: Duration,
        matches: impl Fn(&Event) -> bool,
    ) -> Option<Event> {
        let mut found = None;
        eventually(timeout, || {
            found = lock(&self.events)
                .iter()
                .find(|event| matches(event))
                .cloned();
            found.is_some()
        })
        .await;
        found
    }
}

fn truncate(text: &str, max_chars: usize) -> &str {
    text.char_indices()
        .nth(max_chars)
        .map_or(text, |(end, _)| &text[..end])
}

pub async fn first_page(browser: &Browser) -> Page {
    let info = browser
        .pages()
        .into_iter()
        .next()
        .expect("the browser reports at least one tab");
    browser
        .page(&info.target_id)
        .await
        .unwrap_or_else(|error| panic!("attaching the first tab {}: {error}", info.target_id))
}

pub async fn first_element(frame: &Frame, css: &str) -> Element {
    frame
        .find_css(css, None)
        .await
        .unwrap_or_else(|error| panic!("finding {css}: {error}"))
        .into_iter()
        .next()
        .unwrap_or_else(|| panic!("no element matches {css}"))
}

pub async fn text_of(frame: &Frame, css: &str) -> String {
    first_element(frame, css)
        .await
        .text()
        .await
        .unwrap_or_else(|error| panic!("reading the text of {css}: {error}"))
}

pub async fn child_frame(parent: &Frame, owner_css: &str) -> Frame {
    let owner = first_element(parent, owner_css).await;
    parent
        .child_frame(&owner)
        .await
        .unwrap_or_else(|error| panic!("entering the frame of {owner_css}: {error}"))
}

pub async fn script_value(frame: &Frame, body: &str) -> Value {
    frame
        .execute_script(body, Vec::new(), ScriptOptions::USER)
        .await
        .unwrap_or_else(|error| panic!("running {body:?}: {error}"))
        .into_json()
}

pub async fn eventually(timeout: Duration, mut ready: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + timeout;
    loop {
        if ready() {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        tokio::time::sleep(POLL).await;
    }
}

pub async fn wait_until(timeout: Duration, what: &str, ready: impl FnMut() -> bool) {
    assert!(
        eventually(timeout, ready).await,
        "timed out after {timeout:?} waiting for {what}"
    );
}

pub async fn wait_for_popup(browser: &Browser, opener: &TargetId, url_part: &str) -> PageInfo {
    let mut found = None;
    wait_until(Duration::from_secs(10), "the popup tab", || {
        found = browser
            .pages()
            .into_iter()
            .find(|info| info.opener_id.as_ref() == Some(opener) && info.url.contains(url_part));
        found.is_some()
    })
    .await;
    found.expect("wait_until returns only after the popup appeared")
}

pub fn title_of(pages: &[PageInfo], target: &TargetId) -> Option<String> {
    pages
        .iter()
        .find(|info| info.target_id == *target)
        .map(|info| info.title.clone())
}

pub fn stale_ref_text(reference: &str) -> String {
    format!("Stale element ref '{reference}' — take a new browser snapshot")
}

pub fn png_size(bytes: &[u8]) -> Option<(u32, u32)> {
    const SIGNATURE: &[u8] = b"\x89PNG\r\n\x1a\n";
    if !bytes.starts_with(SIGNATURE) || bytes.len() < 24 {
        return None;
    }
    let width = u32::from_be_bytes(bytes[16..20].try_into().ok()?);
    let height = u32::from_be_bytes(bytes[20..24].try_into().ok()?);
    Some((width, height))
}

pub fn debug_port(browser: &Browser) -> u16 {
    let address = browser
        .debugger_address()
        .expect("a launched browser reports its debugger address");
    address
        .rsplit(':')
        .next()
        .and_then(|port| port.parse().ok())
        .unwrap_or_else(|| panic!("unexpected debugger address {address}"))
}

pub async fn browser_ws_url(browser: &Browser) -> String {
    let address = format!("127.0.0.1:{}", debug_port(browser));
    match attach::resolve_endpoint_with(&address, BrowserKind::Chrome, None).await {
        Ok(AttachEndpoint::PortMode { ws_url }) => ws_url,
        other => panic!("{address} did not resolve to a port-mode endpoint: {other:?}"),
    }
}

pub fn chrome_version(browser: &Browser) -> String {
    let product = &browser.version().product;
    product.rsplit('/').next().unwrap_or(product).to_owned()
}

pub fn url_port(url: &str) -> u16 {
    url::Url::parse(url)
        .ok()
        .and_then(|parsed| parsed.port())
        .unwrap_or_else(|| panic!("{url} has no explicit port"))
}

/// A scan that fails panics, so "no process left" never comes from a scan that did not run.
pub fn processes_using(path: &Path) -> Vec<u32> {
    try_processes_using(path).unwrap_or_else(|error| panic!("{error}"))
}

/// For cleanup that may run while a test unwinds, where a second panic would abort the binary.
#[cfg(unix)]
pub fn try_processes_using(path: &Path) -> Result<Vec<u32>, String> {
    let output = std::process::Command::new("pgrep")
        .arg("-f")
        .arg(regex_escape(&path.to_string_lossy()))
        .output()
        .map_err(|error| format!("the e2e cleanup checks need pgrep: {error}"))?;
    // pgrep exits 1 when nothing matches and 2 or 3 when the scan itself failed.
    if !matches!(output.status.code(), Some(0 | 1)) {
        return Err(scan_failure("pgrep", path, &output));
    }
    Ok(parse_pids(&output.stdout))
}

/// A substring match, not `-like`, so `[` and `]` in a path are not wildcards that match nothing.
#[cfg(windows)]
pub fn try_processes_using(path: &Path) -> Result<Vec<u32>, String> {
    let pattern = path.display().to_string().replace('\'', "''");
    let script = format!(
        "Get-CimInstance Win32_Process | Where-Object {{ $_.ProcessId -ne $PID -and $_.CommandLine -and $_.CommandLine.IndexOf('{pattern}', [StringComparison]::OrdinalIgnoreCase) -ge 0 }} | ForEach-Object {{ $_.ProcessId }}"
    );
    let output = std::process::Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", &script])
        .output()
        .map_err(|error| format!("the e2e cleanup checks need PowerShell: {error}"))?;
    if !output.status.success() {
        return Err(scan_failure("PowerShell", path, &output));
    }
    Ok(parse_pids(&output.stdout))
}

fn scan_failure(tool: &str, path: &Path, output: &std::process::Output) -> String {
    format!(
        "{tool} could not scan for {} ({}): {}",
        path.display(),
        output.status,
        String::from_utf8_lossy(&output.stderr)
    )
}

fn regex_escape(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());
    for character in text.chars() {
        if "\\.+*?()|[]{}^$".contains(character) {
            escaped.push('\\');
        }
        escaped.push(character);
    }
    escaped
}

fn parse_pids(output: &[u8]) -> Vec<u32> {
    String::from_utf8_lossy(output)
        .lines()
        .filter_map(|line| line.trim().parse().ok())
        .collect()
}

pub fn profile_of(browser: &Browser) -> PathBuf {
    let port = debug_port(browser);
    let prefix = format!("flow-like-browser-{}-", std::process::id());
    scratch_roots()
        .iter()
        .filter_map(|root| std::fs::read_dir(root).ok())
        .flatten()
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .find(|path| has_prefix(path, &prefix) && active_port(path) == Some(port))
        .unwrap_or_else(|| panic!("no temporary profile of this test process serves port {port}"))
}

fn scratch_roots() -> Vec<PathBuf> {
    let mut roots = vec![std::env::temp_dir()];
    roots.extend(dirs::home_dir().map(|home| home.join("snap/chromium/common")));
    roots
}

fn has_prefix(path: &Path, prefix: &str) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.starts_with(prefix))
}

fn active_port(dir: &Path) -> Option<u16> {
    attach::read_devtools_active_port(dir)
        .ok()
        .flatten()
        .map(|(port, _)| port)
}

pub async fn assert_gone(profile: &Path) {
    let gone = eventually(CLEANUP_WAIT, || {
        processes_using(profile).is_empty() && !profile.exists()
    })
    .await;
    assert!(
        gone,
        "{CLEANUP_WAIT:?} after teardown processes {:?} still use {} (profile exists: {})",
        processes_using(profile),
        profile.display(),
        profile.exists()
    );
}

pub async fn assert_removed(dir: &Path, what: &str) {
    let removed = eventually(CLEANUP_WAIT, || !dir.exists()).await;
    assert!(removed, "{what}: {} still exists", dir.display());
}

pub async fn assert_no_process(profile: &Path) {
    let gone = eventually(CLEANUP_WAIT, || processes_using(profile).is_empty()).await;
    assert!(
        gone,
        "{CLEANUP_WAIT:?} after teardown processes {:?} still use {}",
        processes_using(profile),
        profile.display()
    );
}

impl E2e {
    /// Starts Chrome through the crate's own spawn path (sandbox preflight, lifeline, flavor
    /// rules) with `--remote-debugging-port=0` and the given profile, without connecting to it.
    pub async fn spawn_unconnected(&self, profile: &Path) -> test_hooks::LaunchedProcess {
        let mut options = self.options();
        options.user_data_dir = Some(profile.to_path_buf());
        test_hooks::spawn(&self.executable, &options)
            .await
            .unwrap_or_else(|error| panic!("spawning Chrome with {}: {error}", profile.display()))
    }
}

pub async fn stop_spawned(launched: &test_hooks::LaunchedProcess) {
    launched.process.kill();
    if !launched.process.wait_exit(CLEANUP_WAIT).await {
        eprintln!("the spawned Chrome did not exit within {CLEANUP_WAIT:?}");
    }
}

#[derive(Clone, Debug)]
pub struct SnapshotNode {
    pub reference: String,
    pub role: String,
    pub name: String,
}

pub struct Snapshot {
    pub table: RefTable,
    pub nodes: Vec<SnapshotNode>,
    pub warnings: Vec<String>,
}

impl Snapshot {
    pub async fn take(page: &Page, previous: &RefTable) -> Snapshot {
        let forest = page
            .accessibility_forest(SNAPSHOT_OPTIONS)
            .await
            .unwrap_or_else(|error| panic!("accessibility snapshot: {error}"));
        let main = forest
            .frames
            .first()
            .expect("a snapshot always holds the main frame");
        let mut allocator = previous.begin(&forest.page, &main.loader_id);
        let mut nodes = Vec::new();
        for frame in &forest.frames {
            for (role, name, backend_node_id) in ax_candidates(&frame.nodes) {
                let node = NodeRef {
                    page: forest.page.clone(),
                    local_root: frame.local_root.clone(),
                    frame_id: frame.frame_id.clone(),
                    loader_id: frame.loader_id.clone(),
                    backend_node_id,
                };
                let proposed = allocator.propose(&node, &role, &name);
                nodes.push(SnapshotNode {
                    reference: proposed.reference.clone(),
                    role: role.clone(),
                    name: name.clone(),
                });
                allocator.commit(proposed, node, role, name);
            }
        }
        Snapshot {
            table: allocator.finish(),
            nodes,
            warnings: forest.warnings,
        }
    }

    pub async fn until(
        page: &Page,
        previous: &RefTable,
        ready: impl Fn(&Snapshot) -> bool,
    ) -> Snapshot {
        let deadline = Instant::now() + SNAPSHOT_WAIT;
        loop {
            let snapshot = Snapshot::take(page, previous).await;
            if ready(&snapshot) || Instant::now() >= deadline {
                return snapshot;
            }
            tokio::time::sleep(POLL * 4).await;
        }
    }

    pub fn has(&self, role: &str, name: &str) -> bool {
        self.nodes
            .iter()
            .any(|node| node.role == role && node.name == name)
    }

    pub fn reference(&self, role: &str, name: &str) -> String {
        self.nodes
            .iter()
            .find(|node| node.role == role && node.name == name)
            .map(|node| node.reference.clone())
            .unwrap_or_else(|| {
                panic!(
                    "no {role} named {name:?} in the snapshot {:?} (warnings {:?})",
                    self.nodes, self.warnings
                )
            })
    }

    pub async fn element(
        &self,
        page: &Page,
        reference: &str,
    ) -> flow_like_browser::Result<Element> {
        let entry = self.table.lookup(reference)?;
        page.resolve_node(&entry.node, reference, self.table.main_loader())
            .await
    }

    pub async fn resolve(&self, page: &Page, reference: &str) -> Element {
        self.element(page, reference)
            .await
            .unwrap_or_else(|error| panic!("resolving {reference}: {error}"))
    }

    pub fn local_root(&self, reference: &str) -> TargetId {
        self.table
            .lookup(reference)
            .unwrap_or_else(|error| panic!("{error}"))
            .node
            .local_root
            .clone()
    }
}

fn ax_candidates(nodes: &Value) -> Vec<(String, String, i64)> {
    nodes
        .as_array()
        .into_iter()
        .flatten()
        .filter(|node| !node["ignored"].as_bool().unwrap_or(false))
        .filter_map(|node| {
            let role = node["role"]["value"]
                .as_str()
                .filter(|role| REF_ROLES.contains(role))?;
            let backend_node_id = node["backendDOMNodeId"].as_i64()?;
            let name = node["name"]["value"].as_str().unwrap_or_default();
            Some((role.to_owned(), name.to_owned(), backend_node_id))
        })
        .collect()
}

pub struct TestServer {
    state: Arc<ServerState>,
}

struct ServerState {
    port: u16,
    third_site: bool,
    requests: Mutex<Vec<String>>,
}

impl TestServer {
    pub async fn start() -> TestServer {
        let v4 = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
            .await
            .expect("bind the e2e HTTP server");
        let port = v4
            .local_addr()
            .expect("the e2e HTTP server has an address")
            .port();
        let v6 = TcpListener::bind((Ipv6Addr::LOCALHOST, port)).await.ok();
        let state = Arc::new(ServerState {
            port,
            third_site: v6.is_some(),
            requests: Mutex::new(Vec::new()),
        });
        tokio::spawn(accept(v4, state.clone()));
        if let Some(v6) = v6 {
            tokio::spawn(accept(v6, state.clone()));
        }
        TestServer { state }
    }

    pub fn port(&self) -> u16 {
        self.state.port
    }

    pub fn url(&self, path: &str) -> String {
        format!("http://127.0.0.1:{}{path}", self.state.port)
    }

    pub fn localhost_url(&self, path: &str) -> String {
        format!("http://localhost:{}{path}", self.state.port)
    }

    pub fn has_third_site(&self) -> bool {
        self.state.third_site
    }

    pub fn requests(&self) -> Vec<String> {
        lock(&self.state.requests).clone()
    }

    pub fn oopif_buttons(&self) -> Vec<String> {
        let mut names = vec![
            "Top button",
            "Frame button same",
            "Frame button cross",
            "Frame button leaf",
        ];
        if self.state.third_site {
            names.push("Frame button third");
        }
        names.into_iter().map(str::to_owned).collect()
    }
}

async fn accept(listener: TcpListener, state: Arc<ServerState>) {
    while let Ok((stream, _)) = listener.accept().await {
        tokio::spawn(serve(stream, state.clone()));
    }
}

async fn serve(mut stream: TcpStream, state: Arc<ServerState>) {
    let Some(request) = read_request(&mut stream).await else {
        return;
    };
    lock(&state.requests).push(format!("{} {}", request.method, request.target));
    let response = route(&state, &request);
    if !response.delay.is_zero() {
        tokio::time::sleep(response.delay).await;
    }
    if response.write(&mut stream).await.is_ok() {
        let _ = stream.shutdown().await;
    }
}

struct HttpRequest {
    method: String,
    target: String,
    path: String,
    query: String,
    content_length: usize,
}

impl HttpRequest {
    fn parse(head: &str) -> Option<HttpRequest> {
        let mut lines = head.split("\r\n");
        let mut first = lines.next()?.split(' ');
        let method = first.next()?.to_owned();
        let target = first.next()?.to_owned();
        let content_length = lines
            .filter_map(|line| line.split_once(':'))
            .find(|(name, _)| name.trim().eq_ignore_ascii_case("content-length"))
            .and_then(|(_, value)| value.trim().parse().ok())
            .unwrap_or(0);
        let (path, query) = target.split_once('?').unwrap_or((target.as_str(), ""));
        let (path, query) = (path.to_owned(), query.to_owned());
        Some(HttpRequest {
            method,
            target,
            path,
            query,
            content_length,
        })
    }

    fn param(&self, name: &str) -> Option<&str> {
        self.query
            .split('&')
            .filter_map(|pair| pair.split_once('='))
            .find(|(key, _)| *key == name)
            .map(|(_, value)| value)
    }

    fn millis(&self, default: u64) -> Duration {
        let millis = self
            .param("ms")
            .and_then(|value| value.parse().ok())
            .unwrap_or(default);
        Duration::from_millis(millis)
    }
}

async fn read_request(stream: &mut TcpStream) -> Option<HttpRequest> {
    let mut buffer = Vec::new();
    let mut chunk = [0_u8; 4096];
    loop {
        let read = stream
            .read(&mut chunk)
            .await
            .ok()
            .filter(|read| *read > 0)?;
        buffer.extend_from_slice(&chunk[..read]);
        if let Some(end) = buffer.windows(4).position(|window| window == b"\r\n\r\n") {
            let request = HttpRequest::parse(&String::from_utf8_lossy(&buffer[..end]))?;
            let body_seen = buffer.len() - end - 4;
            discard(stream, request.content_length.saturating_sub(body_seen)).await;
            return Some(request);
        }
        if buffer.len() > MAX_REQUEST_HEAD {
            return None;
        }
    }
}

async fn discard(stream: &mut TcpStream, mut remaining: usize) {
    let mut chunk = [0_u8; 4096];
    while remaining > 0 {
        match stream.read(&mut chunk).await {
            Ok(0) | Err(_) => return,
            Ok(read) => remaining = remaining.saturating_sub(read),
        }
    }
}

enum Body {
    Full(Vec<u8>),
    Streamed {
        head: String,
        pause: Duration,
        tail: String,
    },
}

struct Response {
    status: u16,
    headers: Vec<(String, String)>,
    body: Body,
    delay: Duration,
}

impl Response {
    fn new(status: u16, content_type: &str, body: Vec<u8>) -> Response {
        Response {
            status,
            headers: vec![("content-type".to_owned(), content_type.to_owned())],
            body: Body::Full(body),
            delay: Duration::ZERO,
        }
    }

    fn html(body: String) -> Response {
        Response::cacheable(body).header("cache-control", "no-store")
    }

    fn cacheable(body: String) -> Response {
        Response::new(200, "text/html; charset=utf-8", body.into_bytes())
    }

    fn empty(status: u16) -> Response {
        Response::new(status, "text/plain", Vec::new())
    }

    fn header(mut self, name: &str, value: &str) -> Response {
        self.headers.push((name.to_owned(), value.to_owned()));
        self
    }

    fn delayed(mut self, delay: Duration) -> Response {
        self.delay = delay;
        self
    }

    async fn write(self, stream: &mut TcpStream) -> std::io::Result<()> {
        let mut head = format!(
            "HTTP/1.1 {} {}\r\nconnection: close\r\n",
            self.status,
            reason(self.status)
        );
        for (name, value) in &self.headers {
            head.push_str(&format!("{name}: {value}\r\n"));
        }
        match self.body {
            Body::Full(bytes) => {
                head.push_str(&format!("content-length: {}\r\n\r\n", bytes.len()));
                stream.write_all(head.as_bytes()).await?;
                stream.write_all(&bytes).await
            }
            Body::Streamed {
                head: first,
                pause,
                tail,
            } => {
                head.push_str("\r\n");
                stream.write_all(head.as_bytes()).await?;
                stream.write_all(first.as_bytes()).await?;
                stream.flush().await?;
                tokio::time::sleep(pause).await;
                stream.write_all(tail.as_bytes()).await
            }
        }
    }
}

fn reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        204 => "No Content",
        302 => "Found",
        _ => "Not Found",
    }
}

const BUTTON_STYLE: &str = r#"style="position:absolute;left:40px;top:120px;width:300px;height:60px;display:block;font-size:24px""#;
const COUNT: &str = "var o=document.getElementById('out');o.textContent=Number(o.textContent)+1";
const FORM_BODY: &str = r#"<label>Name <input id=name aria-label="Name"></label><button id=greet onclick="document.getElementById('out').textContent='Hello '+document.getElementById('name').value">Greet</button><p id=out></p>"#;
const DIALOGS_BODY: &str = r#"<button id=alert onclick="alert('hi');show('alerted')">Alert</button><button id=confirm onclick="show(String(confirm('sure?')))">Confirm</button><button id=prompt onclick="show(prompt('name?','default'))">Prompt</button><p id=out>none</p><script>function show(v){document.getElementById('out').textContent=v}</script>"#;
const TALL_BODY: &str =
    r#"<div style="height:4000px;background:linear-gradient(white,steelblue)">tall</div>"#;
const PRERENDER_BODY: &str = r#"<script type="speculationrules">{"prerender":[{"source":"list","urls":["/pr/next"]}]}</script><a id=go href=/pr/next style="display:block;width:300px;height:60px;font-size:24px">go</a>"#;
const UNTITLED: &str = "<!doctype html><html><head><meta charset=utf-8></head><body><h1 id=t>UNTITLED</h1></body></html>";
const UPLOAD_BODY: &str = "<input id=single type=file><input id=many type=file multiple>";
const COOKIE: &str = "e2e_cookie=kept; Max-Age=86400; Path=/; SameSite=Lax";
const DOWNLOAD_BODY: &[u8] = b"hello download\n";

fn route(state: &ServerState, request: &HttpRequest) -> Response {
    if let Some(case) = request.path.strip_prefix("/case/") {
        return g1_case(case).map_or_else(not_found, Response::html);
    }
    match request.path.as_str() {
        "/next" => Response::html(page("NEXT", "")),
        "/frame" => Response::html(page("FRAME", "")),
        "/slow" => Response::html(page("NEXT", "")).delayed(request.millis(3000)),
        "/slowbody" => slow_body(request.millis(1500)),
        "/204" => Response::empty(204),
        "/r302" => Response::empty(302).header("location", "/next"),
        "/download" => Response::new(200, "application/octet-stream", DOWNLOAD_BODY.to_vec())
            .header("content-disposition", "attachment; filename=\"d.bin\""),
        "/file.txt" => Response::new(200, "text/plain", b"hello file\n".to_vec()),
        "/form" => Response::html(page("Form", FORM_BODY)),
        "/upload" => Response::html(page("Upload", UPLOAD_BODY)),
        "/oopif" => Response::html(oopif_top(state)),
        "/oopif/frame" => Response::html(oopif_frame(request.param("n").unwrap_or("frame"))),
        "/oopif/nested" => Response::html(oopif_nested(state, request.param("leaf"))),
        "/dialogs" => Response::html(page("Dialogs", DIALOGS_BODY)),
        "/tall" => Response::html(doc("Tall", TALL_BODY)),
        "/setcookie" => Response::html(page("Cookie", "")).header("set-cookie", COOKIE),
        "/titled" => Response::html(page("Titled", "")),
        "/untitled" => Response::html(UNTITLED.to_owned()),
        "/bf/a" => Response::cacheable(bf_page("A", "b")),
        "/bf/b" => Response::cacheable(bf_page("B", "a")),
        "/pr/start" => Response::html(page("Prerender start", PRERENDER_BODY)),
        "/pr/next" => Response::html(page("PRERENDERED", "")),
        "/popup" => Response::html(opener("/slowbody?ms=2000")),
        "/popup-quick" => Response::html(opener("/next")),
        "/popup-untitled" => Response::html(opener("/untitled")),
        "/overlay" => Response::html(overlay(state)),
        "/emulation" => Response::html(emulation_top(state)),
        "/emulation/probe" => Response::html(doc("probe", "<p>probe</p>")),
        _ => not_found(),
    }
}

fn not_found() -> Response {
    Response::new(404, "text/html; charset=utf-8", b"not found".to_vec())
}

fn page(title: &str, body: &str) -> String {
    format!(
        "<!doctype html><html><head><meta charset=utf-8><title>{title}</title></head><body><h1 id=t>{title}</h1>{body}</body></html>"
    )
}

fn doc(title: &str, body: &str) -> String {
    format!(
        "<!doctype html><html><head><meta charset=utf-8><title>{title}</title><style>body{{margin:0}}</style></head><body>{body}</body></html>"
    )
}

fn g1_case(name: &str) -> Option<String> {
    let body = match name {
        "noop" => format!("<button id=go {BUTTON_STYLE}>go</button>"),
        "anchor" => format!("<a id=go href=/next {BUTTON_STYLE}>go</a>"),
        "anchor-204" => format!("<a id=go href=/204 {BUTTON_STYLE}>go</a>"),
        "anchor-hash" => format!(
            "<a id=go href=#sec {BUTTON_STYLE}>go</a><div style=\"height:3000px\"></div><h2 id=sec>SEC</h2>"
        ),
        "anchor-slowbody" => format!("<a id=go href=\"/slowbody?ms=1500\" {BUTTON_STYLE}>go</a>"),
        "form-get" => format!(
            "<form action=/next><input name=q value=x><button id=go {BUTTON_STYLE}>go</button></form>"
        ),
        "form-post" => format!(
            "<form method=post action=/next><input name=q value=x><button id=go {BUTTON_STYLE}>go</button></form>"
        ),
        "download" => format!("<a id=go href=/download {BUTTON_STYLE}>go</a>"),
        "iframe-nav" => format!(
            "<button id=go {BUTTON_STYLE} onclick=\"document.getElementById('f').src='/next?frame=1'\">go</button><iframe id=f src=/frame style=\"position:absolute;top:300px;width:400px;height:200px\"></iframe>"
        ),
        _ => return None,
    };
    Some(page("START", &body))
}

fn slow_body(pause: Duration) -> Response {
    let mut response = Response::html(String::new());
    response.body = Body::Streamed {
        head: format!(
            "<!doctype html><html><head><meta charset=utf-8><title>NEXT</title></head><body>{}",
            " ".repeat(2048)
        ),
        pause,
        tail: "<h1 id=t>NEXT</h1></body></html>".to_owned(),
    };
    response
}

fn iframe(id: &str, src: &str, style: &str) -> String {
    format!(r#"<iframe id={id} title="{id}" src="{src}" style="{style}"></iframe>"#)
}

fn oopif_top(state: &ServerState) -> String {
    let port = state.port;
    let nested = |id: &str, leaf: &str| {
        iframe(
            id,
            &format!("http://127.0.0.1:{port}/oopif/nested?leaf={leaf}"),
            "width:420px;height:130px;border:0",
        )
    };
    let same = iframe(
        "same",
        "/oopif/frame?n=same",
        "border:5px solid red;padding:7px;width:360px;height:80px",
    );
    let cross = iframe(
        "cross",
        &format!("http://127.0.0.1:{port}/oopif/frame?n=cross"),
        "border:10px solid blue;padding:3px;width:360px;height:80px;margin-left:30px",
    );
    let leaf = nested("nested", "leaf");
    let third = if state.third_site {
        nested("nested-third", "third")
    } else {
        String::new()
    };
    let body = format!(
        r#"<style>iframe{{display:block;margin:6px 0}}</style><button id=b onclick="{COUNT}">Top button</button><p id=out>0</p><a id=leave href=/case/noop>Leave</a>{same}{cross}{leaf}{third}"#
    );
    page("OOPIF top", &body)
}

fn oopif_frame(name: &str) -> String {
    doc(
        &format!("frame {name}"),
        &format!(
            r#"<button id=b onclick="{COUNT}">Frame button {name}</button><p id=out>0</p><input id=i aria-label="Frame input {name}">"#
        ),
    )
}

fn oopif_nested(state: &ServerState, leaf: Option<&str>) -> String {
    let (host, name) = match leaf {
        Some("third") => ("[::1]", "third"),
        _ => ("localhost", "leaf"),
    };
    let child = iframe(
        "leaf",
        &format!("http://{host}:{}/oopif/frame?n={name}", state.port),
        "display:block;width:380px;height:100px;border:0",
    );
    doc("nested", &format!("<span>Nested B</span>{child}"))
}

fn bf_page(name: &str, other: &str) -> String {
    page(
        &format!("Page {name}"),
        &format!(
            r#"<button id=b onclick="{COUNT}">{name} button</button><p id=out>0</p><a id=to href=/bf/{other}>to {other}</a><script>addEventListener('pageshow',function(e){{document.body.dataset.persisted=String(e.persisted)}})</script>"#
        ),
    )
}

fn opener(url: &str) -> String {
    page(
        "Opener",
        &format!(
            r#"<button id=open style="width:300px;height:60px" onclick="window.open('{url}')">open</button>"#
        ),
    )
}

fn overlay(state: &ServerState) -> String {
    let frame = iframe(
        "covered",
        &format!("http://127.0.0.1:{}/oopif/frame?n=covered", state.port),
        "position:absolute;left:20px;top:80px;width:360px;height:120px;border:0",
    );
    doc(
        "Overlay",
        &format!(
            r#"<p id=hits>0</p>{frame}<div id=overlay class=cover onclick="var h=document.getElementById('hits');h.textContent=Number(h.textContent)+1" style="position:absolute;left:0;top:60px;width:420px;height:180px;background:rgba(0,0,0,0.2)"></div>"#
        ),
    )
}

fn emulation_top(state: &ServerState) -> String {
    let frame = iframe(
        "probe",
        &format!("http://127.0.0.1:{}/emulation/probe", state.port),
        "display:block;border:0;width:300px;height:150px",
    );
    format!(
        r#"<!doctype html><html><head><meta charset=utf-8><meta name=viewport content="width=device-width"><title>Emulation</title><style>body{{margin:0}}</style></head><body>{frame}</body></html>"#
    )
}

pub fn fixture_path(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/cdp")
        .join(format!("{name}.jsonl"))
}

pub fn write_fixture(name: &str, chrome: &str, frames: &[Value]) -> PathBuf {
    let mut text = json!({"chrome": chrome, "recorded": today()}).to_string();
    text.push('\n');
    for frame in frames {
        text.push_str(&frame.to_string());
        text.push('\n');
    }
    let path = fixture_path(name);
    std::fs::write(&path, text)
        .unwrap_or_else(|error| panic!("writing {}: {error}", path.display()));
    path
}

/// Rewrites a proxy transcript for committing: DevTools ids (32 upper-case hex) and download
/// GUIDs become value-based aliases (`T1`, `S1`, `F1`, `L1`, `C1`, `G1`, `X1`), the given ports
/// `{port}`, the given local directories `{dir}`, and large base64 payloads and the long scripts
/// of sent commands (injected atoms) length markers.
pub fn normalize_transcript(lines: &[String], ports: &[u16], dirs: &[PathBuf]) -> Vec<Value> {
    let entries: Vec<Value> = lines
        .iter()
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect();
    let mut aliases = Aliases::default();
    for entry in &entries {
        aliases.collect("", &entry["frame"], false);
    }
    let scrubber = Scrubber::new(ports, dirs);
    entries
        .iter()
        .map(|entry| {
            let mut frame = aliases.rewrite(&entry["frame"], &scrubber);
            if entry["dir"] == "send" {
                shorten_long_text(&mut frame);
            }
            json!({"dir": entry["dir"], "frame": frame})
        })
        .collect()
}

fn shorten_long_text(value: &mut Value) {
    match value {
        Value::String(text) if text.chars().count() > LARGE_SENT_TEXT => {
            *text = format!("<text {} chars>", text.chars().count());
        }
        Value::Array(items) => items.iter_mut().for_each(shorten_long_text),
        Value::Object(map) => map.values_mut().for_each(shorten_long_text),
        _ => {}
    }
}

pub fn target_events(frames: &[Value]) -> Vec<String> {
    frames
        .iter()
        .filter(|entry| entry["dir"] == "recv")
        .map(|entry| &entry["frame"])
        .filter(|frame| {
            frame["method"]
                .as_str()
                .is_some_and(|method| method.starts_with("Target."))
        })
        .map(describe_target_event)
        .collect()
}

fn describe_target_event(frame: &Value) -> String {
    let params = &frame["params"];
    let info = if params["targetInfo"].is_object() {
        &params["targetInfo"]
    } else {
        params
    };
    format!(
        "{} {} {}/{} {}",
        frame["method"].as_str().unwrap_or_default(),
        info["targetId"]
            .as_str()
            .or(params["sessionId"].as_str())
            .unwrap_or("-"),
        info["type"].as_str().unwrap_or("-"),
        info["subtype"].as_str().unwrap_or("-"),
        info["url"].as_str().unwrap_or_default()
    )
}

#[derive(Default)]
struct Aliases {
    names: HashMap<String, String>,
    counts: HashMap<char, usize>,
}

impl Aliases {
    fn collect(&mut self, key: &str, value: &Value, frame_like: bool) {
        match value {
            Value::String(text) if is_id_shaped(text) => {
                self.assign(text, id_kind(key, frame_like))
            }
            Value::Object(map) => {
                let frame_like = map.contains_key("loaderId");
                for (key, inner) in map {
                    self.collect(key, inner, frame_like);
                }
            }
            Value::Array(items) => {
                for item in items {
                    self.collect(key, item, frame_like);
                }
            }
            _ => {}
        }
    }

    fn assign(&mut self, id: &str, kind: char) {
        if self.names.contains_key(id) {
            return;
        }
        let count = self.counts.entry(kind).or_insert(0);
        *count += 1;
        self.names.insert(id.to_owned(), format!("{kind}{count}"));
    }

    fn rewrite(&self, value: &Value, scrubber: &Scrubber) -> Value {
        match value {
            Value::String(text) => Value::String(self.rewrite_text(text, scrubber)),
            Value::Array(items) => Value::Array(
                items
                    .iter()
                    .map(|item| self.rewrite(item, scrubber))
                    .collect(),
            ),
            Value::Object(map) => Value::Object(
                map.iter()
                    .map(|(key, inner)| (key.clone(), self.rewrite_field(key, inner, scrubber)))
                    .collect(),
            ),
            other => other.clone(),
        }
    }

    fn rewrite_field(&self, key: &str, value: &Value, scrubber: &Scrubber) -> Value {
        match value.as_str() {
            Some(text) if key == "data" && text.len() > LARGE_PAYLOAD => {
                Value::String(format!("<base64 {} bytes>", text.len()))
            }
            _ => self.rewrite(value, scrubber),
        }
    }

    fn rewrite_text(&self, text: &str, scrubber: &Scrubber) -> String {
        if let Some(alias) = self.names.get(text) {
            return alias.clone();
        }
        let mut rewritten = scrubber.dirs(text);
        for (id, alias) in &self.names {
            if rewritten.contains(id.as_str()) {
                rewritten = rewritten.replace(id.as_str(), alias);
            }
        }
        scrubber.ports(rewritten)
    }
}

fn is_id_shaped(text: &str) -> bool {
    let hex_upper = |byte: u8| byte.is_ascii_digit() || (b'A'..=b'F').contains(&byte);
    let devtools = text.len() == 32 && text.bytes().all(hex_upper);
    devtools || is_guid(text)
}

fn is_guid(text: &str) -> bool {
    let groups: Vec<&str> = text.split('-').collect();
    groups.iter().map(|group| group.len()).eq([8, 4, 4, 4, 12])
        && groups
            .iter()
            .all(|group| group.bytes().all(|byte| byte.is_ascii_hexdigit()))
}

fn id_kind(key: &str, frame_like: bool) -> char {
    match key {
        "sessionId" => 'S',
        "targetId" | "openerId" => 'T',
        "frameId" | "parentFrameId" | "openerFrameId" | "initiatingFrameId" => 'F',
        "id" | "parentId" if frame_like => 'F',
        "loaderId" => 'L',
        "browserContextId" => 'C',
        "guid" => 'G',
        _ => 'X',
    }
}

struct Scrubber {
    ports: Vec<u16>,
    dirs: Vec<String>,
}

impl Scrubber {
    fn new(ports: &[u16], dirs: &[PathBuf]) -> Scrubber {
        let mut spelled: Vec<String> = dirs.iter().flat_map(|dir| dir_spellings(dir)).collect();
        spelled.sort_by_key(|dir| std::cmp::Reverse(dir.len()));
        spelled.dedup();
        Scrubber {
            ports: ports.to_vec(),
            dirs: spelled,
        }
    }

    fn dirs(&self, text: &str) -> String {
        self.dirs.iter().fold(text.to_owned(), |current, dir| {
            current.replace(dir.as_str(), "{dir}")
        })
    }

    fn ports(&self, text: String) -> String {
        self.ports
            .iter()
            .fold(text, |current, port| mask_port(&current, *port))
    }
}

/// Every spelling Chrome may report for a local directory: as given, canonical, and with the
/// macOS `/private` prefix added or removed; each without a trailing separator.
fn dir_spellings(dir: &Path) -> Vec<String> {
    let mut spellings = vec![dir.to_string_lossy().into_owned()];
    if let Ok(canonical) = std::fs::canonicalize(dir) {
        spellings.push(canonical.to_string_lossy().into_owned());
    }
    let mut variants = Vec::new();
    for spelling in &spellings {
        match spelling.strip_prefix("/private") {
            Some(public) => variants.push(public.to_owned()),
            None if spelling.starts_with('/') => variants.push(format!("/private{spelling}")),
            None => {}
        }
    }
    spellings.extend(variants);
    spellings
        .into_iter()
        .map(|spelling| spelling.trim_end_matches(['/', '\\']).to_owned())
        .filter(|spelling| spelling.len() > 1)
        .collect()
}

fn mask_port(text: &str, port: u16) -> String {
    let needle = format!(":{port}");
    let mut masked = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find(&needle) {
        let after = &rest[at + needle.len()..];
        masked.push_str(&rest[..at]);
        let longer_number = after.starts_with(|character: char| character.is_ascii_digit());
        masked.push_str(if longer_number {
            needle.as_str()
        } else {
            ":{port}"
        });
        rest = after;
    }
    masked.push_str(rest);
    masked
}

fn today() -> String {
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs());
    let (year, month, day) = civil_from_days(i64::try_from(seconds / 86_400).unwrap_or(0));
    format!("{year:04}-{month:02}-{day:02}")
}

pub fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let shifted = days + 719_468;
    let era = shifted.div_euclid(146_097);
    let day_of_era = shifted.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_index = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_index + 2) / 5 + 1;
    let month = if month_index < 10 {
        month_index + 3
    } else {
        month_index - 9
    };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    (
        year,
        u32::try_from(month).unwrap_or(1),
        u32::try_from(day).unwrap_or(1),
    )
}
