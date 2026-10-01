use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use serde_json::json;
use tokio::time::Instant;

use crate::attach::AttachEndpoint;
use crate::connection::{Connection, ConnectionOptions, without_op_deadline};
use crate::downloads::DownloadTracker;
use crate::error::BrowserError;
use crate::launch::{BrowserProcess, LaunchOptions, LaunchedProcess};
use crate::page::Page;
use crate::session::Session;
use crate::target::TargetManager;
use crate::transport::Transport;
use crate::transport::ws::{ConnectOptions, WsTransport};
use crate::types::{SessionId, TargetId, VersionResult};

const DEFAULT_PAGE_LOAD_TIMEOUT: Duration = Duration::from_secs(300);
const BROWSER_CLOSE_TIMEOUT: Duration = Duration::from_secs(2);
const GRACEFUL_EXIT: Duration = Duration::from_secs(3);
const KILLED_EXIT: Duration = Duration::from_secs(2);
const DOWNLOAD_MOVES_GRACE: Duration = Duration::from_secs(2);
const ABORT_WATCHDOG_GRACE: Duration = Duration::from_secs(3);
#[cfg(any(test, feature = "test-support"))]
const TEST_LAUNCH_TIMEOUT: Duration = Duration::from_secs(30);
#[cfg(any(test, feature = "test-support"))]
const TEST_WINDOW_SIZE: (u32, u32) = (1920, 1080);
const APPROVAL_REFUSED: &str = "rejected: HTTP 403";
const INTERNAL_URL_PREFIXES: [&str; 4] = [
    "chrome://",
    "chrome-untrusted://",
    "chrome-extension://",
    "devtools://",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConnectionKind {
    Launched,
    AttachedPort,
    AttachedApproval,
    Direct,
}

pub struct BrowserSettings {
    kind: ConnectionKind,
    headless: bool,
    page_load_timeout: Mutex<Duration>,
}

impl BrowserSettings {
    pub fn new(
        kind: ConnectionKind,
        headless: bool,
        page_load_timeout: std::time::Duration,
    ) -> Self {
        Self {
            kind,
            headless,
            page_load_timeout: Mutex::new(page_load_timeout),
        }
    }

    pub fn kind(&self) -> ConnectionKind {
        self.kind
    }

    pub fn headless(&self) -> bool {
        self.headless
    }

    pub fn page_load_timeout(&self) -> std::time::Duration {
        *self
            .page_load_timeout
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    pub fn set_page_load_timeout(&self, timeout: std::time::Duration) {
        *self
            .page_load_timeout
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = timeout;
    }
}

#[derive(Clone, Debug)]
pub struct VersionInfo {
    pub product: String,
    pub protocol_version: String,
    pub user_agent: String,
}

#[derive(Clone, Debug)]
pub struct PageInfo {
    pub target_id: TargetId,
    pub url: String,
    pub title: String,
    pub opener_id: Option<TargetId>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ClosePageOutcome {
    Remaining { next: TargetId },
    LastPageClosed { browser_closed: bool },
}

#[derive(Clone)]
pub struct Browser {
    pub(crate) inner: std::sync::Arc<BrowserInner>,
}

pub(crate) struct BrowserInner {
    pub(crate) connection: Connection,
    pub(crate) targets: std::sync::Arc<crate::target::TargetManager>,
    pub(crate) settings: std::sync::Arc<BrowserSettings>,
    pub(crate) downloads: crate::downloads::DownloadTracker,
    pub(crate) process: Option<crate::launch::BrowserProcess>,
    pub(crate) debugger_address: Option<String>,
    pub(crate) version: VersionInfo,
    pub(crate) closed: std::sync::atomic::AtomicBool,
}

struct Construction {
    kind: ConnectionKind,
    headless: bool,
    page_load_timeout: Duration,
    process: Option<BrowserProcess>,
    staging: Option<PathBuf>,
    debugger_address: Option<String>,
}

impl Construction {
    fn attached(kind: ConnectionKind) -> Self {
        Self {
            kind,
            headless: false,
            page_load_timeout: DEFAULT_PAGE_LOAD_TIMEOUT,
            process: None,
            staging: None,
            debugger_address: None,
        }
    }
}

struct OwnedSetup {
    ignore_https_errors: bool,
    window_size: (u32, u32),
    launch_timeout: Duration,
}

async fn construct(
    transport: Box<dyn Transport>,
    construction: Construction,
) -> crate::Result<Browser> {
    let connection = Connection::start(transport, ConnectionOptions::default());
    let version = match fetch_version(&connection).await {
        Ok(version) => version,
        Err(error) => {
            connection.abort();
            if let Some(process) = &construction.process {
                terminate(process).await;
            }
            return Err(error);
        }
    };
    let browser = Browser::assemble(connection, version, construction);
    if let Err(error) = browser.inner.targets.start().await {
        browser.discard().await;
        return Err(error);
    }
    Ok(browser)
}

async fn fetch_version(connection: &Connection) -> crate::Result<VersionInfo> {
    let raw = connection
        .send_raw("Browser.getVersion", json!({}), None)
        .await?;
    let version: VersionResult = crate::protocol::decode("Browser.getVersion", raw)?;
    Ok(VersionInfo {
        product: version.product,
        protocol_version: version.protocol_version,
        user_agent: version.user_agent,
    })
}

async fn terminate(process: &BrowserProcess) {
    process.kill();
    process.wait_exit(KILLED_EXIT).await;
    process.cleanup();
}

fn approval_refusal(kind: ConnectionKind, error: BrowserError) -> BrowserError {
    match error {
        BrowserError::Connect { message }
            if kind == ConnectionKind::AttachedApproval && message.contains(APPROVAL_REFUSED) =>
        {
            BrowserError::Connect {
                message: "The browser refused the connection (Allow was not clicked, or no browser window was open)"
                    .to_owned(),
            }
        }
        other => other,
    }
}

fn is_internal_url(url: &str) -> bool {
    INTERNAL_URL_PREFIXES
        .iter()
        .any(|prefix| url.starts_with(prefix))
}

fn title_value(reply: &serde_json::Value) -> Option<String> {
    reply["result"]["value"].as_str().map(str::to_owned)
}

fn title_expression() -> serde_json::Value {
    json!({"expression": "document.title", "returnByValue": true})
}

async fn held_title(page: &Page) -> Option<String> {
    let reply = page
        .session()
        .send("Runtime.evaluate", title_expression())
        .await
        .ok()?;
    title_value(&reply)
}

async fn temporary_title(
    connection: Connection,
    target: TargetId,
    deadline: Instant,
) -> Option<String> {
    let root = connection.session(None);
    let attached = root
        .send(
            "Target.attachToTarget",
            json!({"targetId": target, "flatten": true}),
        )
        .await
        .ok()?;
    let session = SessionId::from(attached["sessionId"].as_str()?);
    let probe = connection.session(Some(session.clone()));
    let remaining = deadline.saturating_duration_since(Instant::now());
    let title = if remaining.is_zero() {
        None
    } else {
        probe
            .send_with_timeout("Runtime.evaluate", title_expression(), remaining)
            .await
            .ok()
            .and_then(|reply| title_value(&reply))
    };
    let resumed = probe.send_nowait("Runtime.runIfWaitingForDebugger", json!({}));
    let detached = root.send_nowait("Target.detachFromTarget", json!({"sessionId": session}));
    if let Err(error) = resumed.and(detached) {
        tracing::debug!(%target, %error, "releasing a title probe session failed");
    }
    title
}

// Chrome 154 sends no Target.targetInfoChanged when a background tab sets its title, so the
// registry can still hold the URL-derived placeholder; Target.getTargets reads the current one.
async fn target_titles(root: Session, deadline: Instant) -> HashMap<TargetId, String> {
    let listed = tokio::time::timeout_at(deadline, root.send("Target.getTargets", json!({}))).await;
    let Ok(Ok(reply)) = listed else {
        return HashMap::new();
    };
    reply["targetInfos"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|info| {
            let target = TargetId::from(info["targetId"].as_str()?);
            Some((target, info["title"].as_str()?.to_owned()))
        })
        .collect()
}

impl Browser {
    pub async fn launch(options: crate::launch::LaunchOptions) -> crate::Result<Browser> {
        let executable = match &options.executable {
            Some(executable) => executable.clone(),
            None => crate::launch::find(options.kind, &options.cache_dir)?,
        };
        let LaunchedProcess {
            ws_url,
            port,
            process,
        } = crate::launch::spawn(&executable, &options).await?;
        let mut connect = ConnectOptions::new(ws_url);
        connect.handshake_timeout = options.launch_timeout;
        let transport = match WsTransport::connect(&connect).await {
            Ok(transport) => transport,
            Err(error) => {
                terminate(&process).await;
                return Err(error);
            }
        };
        let construction = Construction {
            kind: ConnectionKind::Launched,
            headless: options.headless,
            page_load_timeout: options.page_load_timeout,
            staging: process.staging_dir().map(Path::to_path_buf),
            process: Some(process),
            debugger_address: Some(format!("localhost:{port}")),
        };
        let browser = construct(Box::new(transport), construction).await?;
        browser.finish_owned_setup(&launch_setup(&options)).await
    }

    pub async fn attach(endpoint: crate::attach::AttachEndpoint) -> crate::Result<Browser> {
        let (kind, handshake_timeout) = match &endpoint {
            AttachEndpoint::PortMode { .. } => (ConnectionKind::AttachedPort, None),
            AttachEndpoint::ApprovalMode {
                handshake_timeout, ..
            } => (ConnectionKind::AttachedApproval, Some(*handshake_timeout)),
            AttachEndpoint::DirectWs {
                handshake_timeout, ..
            } => (ConnectionKind::Direct, Some(*handshake_timeout)),
        };
        let mut options = ConnectOptions::new(endpoint.ws_url());
        if let Some(timeout) = handshake_timeout {
            options.handshake_timeout = timeout;
        }
        let transport = WsTransport::connect(&options)
            .await
            .map_err(|error| approval_refusal(kind, error))?;
        construct(Box::new(transport), Construction::attached(kind)).await
    }

    pub async fn connect(options: crate::transport::ws::ConnectOptions) -> crate::Result<Browser> {
        let transport = WsTransport::connect(&options).await?;
        construct(
            Box::new(transport),
            Construction::attached(ConnectionKind::Direct),
        )
        .await
    }

    #[cfg(any(test, feature = "test-support"))]
    pub async fn connect_transport(
        transport: Box<dyn crate::transport::Transport>,
        setup: crate::test_hooks::TestSetup,
    ) -> crate::Result<Browser> {
        let crate::test_hooks::TestSetup {
            kind,
            headless,
            page_load_timeout,
            process,
            staging,
            run_owned_setup,
        } = setup;
        let construction = Construction {
            kind,
            headless,
            page_load_timeout,
            process,
            staging,
            debugger_address: None,
        };
        let browser = construct(transport, construction).await?;
        if kind != ConnectionKind::Launched || !run_owned_setup {
            return Ok(browser);
        }
        let owned = OwnedSetup {
            ignore_https_errors: false,
            window_size: TEST_WINDOW_SIZE,
            launch_timeout: TEST_LAUNCH_TIMEOUT,
        };
        browser.finish_owned_setup(&owned).await
    }

    fn assemble(connection: Connection, version: VersionInfo, construction: Construction) -> Self {
        let settings = Arc::new(BrowserSettings::new(
            construction.kind,
            construction.headless,
            construction.page_load_timeout,
        ));
        let inner = Arc::new_cyclic(|browser| BrowserInner {
            targets: TargetManager::new(connection.clone(), settings.clone(), browser.clone()),
            downloads: DownloadTracker::new(construction.kind, construction.staging),
            connection: connection.clone(),
            settings,
            process: construction.process,
            debugger_address: construction.debugger_address,
            version,
            closed: AtomicBool::new(false),
        });
        connection.add_hook(inner.targets.clone());
        connection.add_hook(Arc::new(inner.downloads.clone()));
        inner.downloads.attach(&connection);
        Browser { inner }
    }

    async fn finish_owned_setup(self, setup: &OwnedSetup) -> crate::Result<Browser> {
        match self.owned_setup(setup).await {
            Ok(()) => Ok(self),
            Err(error) => {
                self.discard().await;
                Err(error)
            }
        }
    }

    async fn owned_setup(&self, setup: &OwnedSetup) -> crate::Result<()> {
        if setup.ignore_https_errors {
            self.root()
                .send(
                    "Security.setIgnoreCertificateErrors",
                    json!({"ignore": true}),
                )
                .await?;
        }
        self.configure_downloads().await?;
        let deadline = Instant::now() + setup.launch_timeout;
        let first = self.inner.targets.wait_first_page(deadline).await?;
        let page = self.page(&first).await?;
        let (width, height) = setup.window_size;
        page.fit_viewport(width, height).await
    }

    async fn discard(&self) {
        self.inner.closed.store(true, Ordering::Release);
        self.inner.connection.abort();
        if let Some(process) = &self.inner.process {
            terminate(process).await;
        }
    }

    pub fn connection(&self) -> &Connection {
        &self.inner.connection
    }

    pub fn root(&self) -> crate::session::Session {
        self.inner.connection.session(None)
    }

    pub fn kind(&self) -> ConnectionKind {
        self.inner.settings.kind()
    }

    pub fn is_owned(&self) -> bool {
        self.kind() == ConnectionKind::Launched
    }

    pub fn debugger_address(&self) -> Option<&str> {
        self.inner.debugger_address.as_deref()
    }

    pub fn version(&self) -> &VersionInfo {
        &self.inner.version
    }

    pub fn settings(&self) -> &BrowserSettings {
        &self.inner.settings
    }

    pub fn pages(&self) -> Vec<PageInfo> {
        self.inner.targets.pages()
    }

    pub async fn list_pages(&self, title_budget: std::time::Duration) -> Vec<PageInfo> {
        let deadline = Instant::now() + title_budget;
        let pages = self.pages();
        let probes = futures_util::future::join_all(
            pages.iter().map(|info| self.probe_title(info, deadline)),
        );
        let (probed, current) =
            futures_util::future::join(probes, target_titles(self.root(), deadline)).await;
        pages
            .into_iter()
            .zip(probed)
            .map(|(info, probed)| {
                let title = probed
                    .or_else(|| current.get(&info.target_id).cloned())
                    .unwrap_or_else(|| info.title.clone());
                PageInfo { title, ..info }
            })
            .collect()
    }

    async fn probe_title(&self, info: &PageInfo, deadline: Instant) -> Option<String> {
        if info.url.starts_with("about:blank") {
            return Some(String::new());
        }
        let probed = match self.inner.targets.attached_page(&info.target_id) {
            Some(page) if page.pending_dialog().is_none() => {
                tokio::time::timeout_at(deadline, held_title(&page)).await
            }
            None if self.is_owned() && !is_internal_url(&info.url) => {
                self.temporary_title(&info.target_id, deadline).await
            }
            _ => return None,
        };
        probed.ok().flatten()
    }

    async fn temporary_title(
        &self,
        target: &TargetId,
        deadline: Instant,
    ) -> Result<Option<String>, tokio::time::error::Elapsed> {
        let probe = tokio::spawn(temporary_title(
            self.inner.connection.clone(),
            target.clone(),
            deadline,
        ));
        tokio::time::timeout_at(deadline, probe)
            .await
            .map(|joined| joined.ok().flatten())
    }

    pub async fn page(&self, target: &TargetId) -> crate::Result<Page> {
        self.inner.targets.page(target).await
    }

    pub async fn new_page(&self) -> crate::Result<Page> {
        self.inner.targets.create_page().await
    }

    pub async fn close_page(&self, target: &TargetId) -> crate::Result<ClosePageOutcome> {
        self.inner.targets.close_page(target).await?;
        if !self.inner.connection.is_closed()
            && let Some(next) = self.pages().into_iter().next()
        {
            return Ok(ClosePageOutcome::Remaining {
                next: next.target_id,
            });
        }
        let browser_closed = self.is_owned();
        self.close().await?;
        Ok(ClosePageOutcome::LastPageClosed { browser_closed })
    }

    pub async fn activate(&self, target: &TargetId) -> crate::Result<()> {
        self.root()
            .send(
                "Target.activateTarget",
                serde_json::json!({"targetId": target}),
            )
            .await
            .map(drop)
    }

    pub fn downloads(&self) -> &crate::downloads::DownloadTracker {
        &self.inner.downloads
    }

    pub async fn close(&self) -> crate::Result<()> {
        if self.inner.closed.swap(true, Ordering::AcqRel) {
            return Ok(());
        }
        if self.is_owned() {
            self.close_owned().await;
        } else {
            self.inner.connection.close().await;
        }
        Ok(())
    }

    async fn close_owned(&self) {
        let mut interrupted = StopOnDrop(Some(self));
        request_browser_close(&self.root()).await;
        if let Some(process) = &self.inner.process {
            process.wait_exit(GRACEFUL_EXIT).await;
            process.kill();
            process.wait_exit(KILLED_EXIT).await;
        }
        interrupted.disarm();
        self.inner.connection.close().await;
        let moves_deadline = Instant::now() + DOWNLOAD_MOVES_GRACE;
        if !self.inner.downloads.moves_finished(moves_deadline).await {
            tracing::debug!("download moves were still running when the browser closed");
        }
        if let Some(process) = &self.inner.process {
            process.cleanup();
        }
    }

    pub async fn shutdown(&self) {
        if let Err(error) = self.close().await {
            tracing::warn!(%error, "closing the browser failed");
        }
    }

    pub fn abort(&self) {
        if !self.inner.closed.swap(true, Ordering::AcqRel) {
            self.stop_now();
        }
    }

    fn stop_now(&self) {
        let connection = &self.inner.connection;
        match (&self.inner.process, self.kind()) {
            (Some(process), ConnectionKind::Launched) if !process.profile().temporary => {
                let requested = without_op_deadline(|| {
                    connection.send_nowait("Browser.close", json!({}), None)
                });
                if let Err(error) = requested {
                    tracing::debug!(%error, "Browser.close could not be queued during abort");
                }
                process.close_lifeline();
                connection.abort();
                process.arm_watchdog(ABORT_WATCHDOG_GRACE);
            }
            (Some(process), ConnectionKind::Launched) => {
                connection.abort();
                process.kill();
            }
            _ => connection.abort(),
        }
    }

    pub fn is_alive(&self) -> bool {
        !self.inner.closed.load(Ordering::Acquire) && !self.inner.connection.is_closed()
    }
}

/// Gives a close that is dropped before the browser exited the teardown of `abort`.
struct StopOnDrop<'a>(Option<&'a Browser>);

impl StopOnDrop<'_> {
    fn disarm(&mut self) {
        self.0 = None;
    }
}

impl Drop for StopOnDrop<'_> {
    fn drop(&mut self) {
        if let Some(browser) = self.0.take() {
            browser.stop_now();
        }
    }
}

async fn request_browser_close(root: &Session) {
    match root
        .send_with_timeout("Browser.close", json!({}), BROWSER_CLOSE_TIMEOUT)
        .await
    {
        Ok(_)
        | Err(
            BrowserError::Disconnected { .. }
            | BrowserError::TargetClosed { .. }
            | BrowserError::Timeout { .. },
        ) => {}
        Err(error) => {
            tracing::debug!(%error, "Browser.close failed; stopping the process instead");
        }
    }
}

fn launch_setup(options: &LaunchOptions) -> OwnedSetup {
    OwnedSetup {
        ignore_https_errors: options.ignore_https_errors,
        window_size: options.window_size,
        launch_timeout: options.launch_timeout,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_hooks::TestSetup;
    use crate::testing::default_auto_reply;
    use crate::transport::memory::InMemoryTransport;
    use std::sync::atomic::AtomicUsize;

    fn titled_tab(title: &str) -> serde_json::Value {
        json!({"targetId": "T1", "type": "page", "title": title, "url": "http://127.0.0.1/titled", "attached": false})
    }

    #[tokio::test]
    async fn attached_titles_come_from_a_fresh_target_list_without_attaching() {
        let (transport, control) = InMemoryTransport::new();
        let lists = Arc::new(AtomicUsize::new(0));
        let listed = lists.clone();
        control.set_auto_reply(move |command| match command.method.as_str() {
            "Browser.getVersion" => Some(
                json!({"protocolVersion": "1.3", "product": "Chrome/154.0.8037.92", "revision": "@1", "userAgent": "Mozilla/5.0", "jsVersion": "15.4"}),
            ),
            "Target.setDiscoverTargets" => Some(json!({})),
            "Target.getTargets" => {
                let first = listed.fetch_add(1, Ordering::SeqCst) == 0;
                let title = if first { "127.0.0.1/titled" } else { "Titled" };
                Some(json!({"targetInfos": [titled_tab(title)]}))
            }
            _ => default_auto_reply(command),
        });
        let setup = TestSetup {
            kind: ConnectionKind::AttachedPort,
            headless: true,
            page_load_timeout: Duration::from_secs(5),
            process: None,
            staging: None,
            run_owned_setup: false,
        };
        let browser = Browser::connect_transport(Box::new(transport), setup)
            .await
            .unwrap();
        assert_eq!(browser.pages()[0].title, "127.0.0.1/titled");
        let pages = browser.list_pages(Duration::from_secs(2)).await;
        assert_eq!(pages[0].title, "Titled");
        assert_eq!(lists.load(Ordering::SeqCst), 2);
        assert!(
            !control
                .commands_seen()
                .iter()
                .any(|command| command.method == "Target.attachToTarget")
        );
    }
}
