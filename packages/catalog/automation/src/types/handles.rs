use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[cfg(feature = "execute")]
use flow_like::flow::execution::context::ExecutionContext;
#[cfg(feature = "execute")]
use flow_like_types::{Cacheable, Context, create_id};
#[cfg(feature = "execute")]
use std::sync::Arc;

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, Default)]
pub enum BrowserType {
    #[default]
    Chrome,
    Firefox,
    Edge,
    Safari,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub struct BrowserContextOptions {
    pub browser_type: BrowserType,
    pub headless: bool,
    pub user_data_dir: Option<String>,
    pub viewport_width: Option<u32>,
    pub viewport_height: Option<u32>,
    pub user_agent: Option<String>,
    pub locale: Option<String>,
    pub timezone_id: Option<String>,
    pub geolocation: Option<Geolocation>,
    pub permissions: Option<Vec<String>>,
    pub ignore_https_errors: bool,
    pub proxy: Option<ProxySettings>,
    pub webdriver_url: Option<String>,
}

impl Default for BrowserContextOptions {
    fn default() -> Self {
        Self {
            browser_type: BrowserType::Chrome,
            headless: true,
            user_data_dir: None,
            viewport_width: Some(1920),
            viewport_height: Some(1080),
            user_agent: None,
            locale: None,
            timezone_id: None,
            geolocation: None,
            permissions: None,
            ignore_https_errors: false,
            proxy: None,
            webdriver_url: Some("http://localhost:9515".to_string()),
        }
    }
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub struct Geolocation {
    pub latitude: f64,
    pub longitude: f64,
    pub accuracy: Option<f64>,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub struct ProxySettings {
    pub server: String,
    pub bypass: Option<String>,
    pub username: Option<String>,
    pub password: Option<String>,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq, Eq)]
pub enum Platform {
    Windows,
    MacOS,
    Linux,
}

/// Unified automation session that combines browser, desktop, and RPA capabilities
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub struct AutomationSession {
    pub session_ref: String,
    pub platform: Platform,
    pub default_delay_ms: u64,
    pub click_delay_ms: u64,
    pub debug_mode: bool,
    /// Browser context info if browser is attached
    pub browser_type: Option<BrowserType>,
    pub browser_headless: Option<bool>,
    pub browser_user_data_dir: Option<String>,
    /// Current page info if a page is open
    pub current_page_ref: Option<String>,
    pub current_window_handle: Option<String>,
    #[serde(default)]
    pub browser_frame_selectors: Vec<crate::types::selectors::Selector>,
}

/// The Chrome DevTools browser of one session, registered as a run resource so it
/// closes with the run even when the flow never reaches Stop Session.
#[cfg(feature = "execute")]
#[derive(Default)]
pub(crate) struct BrowserSlot {
    browser: std::sync::Mutex<Option<flow_like_browser::Browser>>,
    aborted: std::sync::atomic::AtomicBool,
    pub(crate) refs: std::sync::Mutex<RefState>,
    pub(crate) downloads: std::sync::Mutex<Option<DownloadArm>>,
}

#[cfg(feature = "execute")]
#[derive(Default)]
pub(crate) struct RefState {
    pub table: flow_like_browser::RefTable,
    pub elements: Vec<crate::browser::refs::SnapshotElement>,
}

#[cfg(feature = "execute")]
pub(crate) struct DownloadArm {
    pub directory: Option<std::path::PathBuf>,
    pub cursor: u64,
    pub include_in_progress: bool,
}

/// Runs `access` on the value behind a mutex; a panic of an earlier holder does not
/// poison the slot for the rest of the run.
#[cfg(feature = "execute")]
pub(crate) fn locked<T, R>(mutex: &std::sync::Mutex<T>, access: impl FnOnce(&mut T) -> R) -> R {
    access(
        &mut mutex
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner),
    )
}

#[cfg(feature = "execute")]
fn slot_occupied() -> flow_like_types::Error {
    flow_like_types::anyhow!("Close the attached browser before replacing it")
}

#[cfg(feature = "execute")]
impl BrowserSlot {
    pub(crate) fn browser(&self) -> Option<flow_like_browser::Browser> {
        locked(&self.browser, |browser| browser.clone())
    }

    /// Refuses a second browser and a browser that became ready after the run ended;
    /// a refused browser is aborted so no Chrome outlives its run.
    pub(crate) fn install(
        &self,
        browser: flow_like_browser::Browser,
    ) -> flow_like_types::Result<()> {
        let refusal = locked(&self.browser, |current| {
            if current.is_some() {
                Some(slot_occupied())
            } else if self.aborted.load(std::sync::atomic::Ordering::Acquire) {
                Some(flow_like_types::anyhow!(
                    "The run was cancelled before the browser was ready"
                ))
            } else {
                *current = Some(browser.clone());
                None
            }
        });
        match refusal {
            None => Ok(()),
            Some(refusal) => {
                browser.abort();
                Err(refusal)
            }
        }
    }

    pub(crate) fn take(&self) -> Option<flow_like_browser::Browser> {
        let browser = locked(&self.browser, Option::take);
        locked(&self.refs, |refs| *refs = RefState::default());
        locked(&self.downloads, |arm| *arm = None);
        browser
    }
}

#[cfg(feature = "execute")]
#[flow_like_types::async_trait]
impl flow_like::flow::execution::resources::RunResource for BrowserSlot {
    fn abort(&self) {
        self.aborted
            .store(true, std::sync::atomic::Ordering::Release);
        if let Some(browser) = self.take() {
            browser.abort();
        }
    }

    async fn shutdown(&self) {
        self.aborted
            .store(true, std::sync::atomic::Ordering::Release);
        if let Some(browser) = self.take() {
            browser.shutdown().await;
        }
    }
}

#[cfg(feature = "execute")]
impl Drop for BrowserSlot {
    fn drop(&mut self) {
        if let Some(browser) = self.take() {
            browser.abort();
        }
    }
}

/// A connected browser that no session slot owns yet. Dropping it disconnects, so a node that
/// is stopped or dropped between connecting and installing leaves no DevTools connection open.
#[cfg(feature = "execute")]
pub(crate) struct UnclaimedBrowser {
    browser: flow_like_browser::Browser,
    claimed: bool,
}

#[cfg(feature = "execute")]
impl UnclaimedBrowser {
    pub(crate) fn new(browser: flow_like_browser::Browser) -> Self {
        Self {
            browser,
            claimed: false,
        }
    }

    pub(crate) fn browser(&self) -> &flow_like_browser::Browser {
        &self.browser
    }

    pub(crate) fn claim(mut self) -> flow_like_browser::Browser {
        self.claimed = true;
        self.browser.clone()
    }
}

#[cfg(feature = "execute")]
impl Drop for UnclaimedBrowser {
    fn drop(&mut self) {
        if !self.claimed {
            self.browser.abort();
        }
    }
}

/// Marks the session inactive once the last wrapper clone is gone (the run ended without
/// Stop Session), so watchers such as a latched Mouse Down release what they hold.
#[cfg(feature = "execute")]
struct SessionLifetime(Arc<std::sync::atomic::AtomicBool>);

#[cfg(feature = "execute")]
impl Drop for SessionLifetime {
    fn drop(&mut self) {
        self.0.store(false, std::sync::atomic::Ordering::Release);
    }
}

#[cfg(feature = "execute")]
#[derive(Clone)]
pub struct AutomationSessionWrapper {
    slot: Arc<BrowserSlot>,
    browser_lock: Arc<tokio::sync::Mutex<()>>,
    active: Arc<std::sync::atomic::AtomicBool>,
    _lifetime: Arc<SessionLifetime>,
}

#[cfg(feature = "execute")]
impl AutomationSessionWrapper {
    fn new() -> Self {
        let active = Arc::new(std::sync::atomic::AtomicBool::new(true));
        Self {
            slot: Arc::new(BrowserSlot::default()),
            browser_lock: Arc::new(tokio::sync::Mutex::new(())),
            active: active.clone(),
            _lifetime: Arc::new(SessionLifetime(active)),
        }
    }
}

#[cfg(feature = "execute")]
impl Cacheable for AutomationSessionWrapper {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}

/// Keeps the current page, its replayed frame and every command in a node in one browser operation.
#[cfg(feature = "execute")]
pub struct BrowserOperationGuard {
    ctx: crate::browser::driver::PageContext,
    _guard: tokio::sync::OwnedMutexGuard<()>,
}
#[cfg(feature = "execute")]
impl std::ops::Deref for BrowserOperationGuard {
    type Target = crate::browser::driver::PageContext;
    fn deref(&self) -> &Self::Target {
        &self.ctx
    }
}

/// One browser operation that does not need a current page (tab selection, new tabs).
#[cfg(feature = "execute")]
pub struct BrowserGuard {
    pub browser: flow_like_browser::Browser,
    _guard: tokio::sync::OwnedMutexGuard<()>,
}

/// chromedriver window handles are `CDwindow-<target id>`; sessions saved before the
/// DevTools port still carry them.
#[cfg(feature = "execute")]
fn page_target(handle: &str) -> flow_like_browser::types::TargetId {
    flow_like_browser::types::TargetId::from(handle.strip_prefix("CDwindow-").unwrap_or(handle))
}

#[cfg(feature = "execute")]
fn attached_browser(slot: &BrowserSlot) -> flow_like_types::Result<flow_like_browser::Browser> {
    slot.browser()
        .ok_or_else(|| flow_like_types::anyhow!("No browser attached to this session"))
}

/// Idempotent for the same slot; a closed run (finished, cancelled or detached) refuses it.
#[cfg(feature = "execute")]
fn register_slot(
    resources: &flow_like::flow::execution::resources::RunResources,
    session_ref: &str,
    slot: &Arc<BrowserSlot>,
) -> flow_like_types::Result<()> {
    if slot.browser().is_some() {
        return Err(slot_occupied());
    }
    let slot = slot.clone();
    resources
        .get_or_insert_with(format!("automation:browser:{session_ref}"), move || slot)
        .context("Browser nodes need a live run to own the browser")?;
    Ok(())
}

impl AutomationSession {
    #[cfg(feature = "execute")]
    pub async fn new(
        ctx: &mut ExecutionContext,
        default_delay_ms: u64,
        click_delay_ms: u64,
        debug_mode: bool,
    ) -> flow_like_types::Result<Self> {
        let id = create_id();
        let platform = if cfg!(target_os = "windows") {
            Platform::Windows
        } else if cfg!(target_os = "macos") {
            Platform::MacOS
        } else {
            Platform::Linux
        };
        ctx.cache
            .write()
            .await
            .insert(id.clone(), Arc::new(AutomationSessionWrapper::new()));
        Ok(Self {
            session_ref: id,
            platform,
            default_delay_ms,
            click_delay_ms,
            debug_mode,
            browser_type: None,
            browser_headless: None,
            browser_user_data_dir: None,
            current_page_ref: None,
            current_window_handle: None,
            browser_frame_selectors: Vec::new(),
        })
    }

    #[cfg(feature = "execute")]
    async fn wrapper(
        &self,
        ctx: &ExecutionContext,
    ) -> flow_like_types::Result<AutomationSessionWrapper> {
        let cache = ctx.cache.read().await;
        let wrapper = cache
            .get(&self.session_ref)
            .and_then(|value| value.as_any().downcast_ref::<AutomationSessionWrapper>())
            .ok_or_else(|| {
                flow_like_types::anyhow!("Automation session is closed or belongs to another run")
            })?;
        if !wrapper.active.load(std::sync::atomic::Ordering::Acquire) {
            return Err(flow_like_types::anyhow!("Automation session is closed"));
        }
        Ok(wrapper.clone())
    }

    #[cfg(feature = "execute")]
    pub async fn ensure_active(&self, ctx: &ExecutionContext) -> flow_like_types::Result<()> {
        self.wrapper(ctx).await.map(|_| ())
    }

    #[cfg(feature = "execute")]
    pub(crate) async fn create_enigo(
        &self,
        ctx: &ExecutionContext,
    ) -> flow_like_types::Result<crate::computer::native::input::DesktopInput> {
        let wrapper = self.wrapper(ctx).await?;
        crate::computer::native::input::DesktopInput::new(
            wrapper.active,
            ctx.get_cancellation_token(),
        )
        .await
    }

    #[cfg(feature = "execute")]
    pub async fn apply_delay(&self, ctx: &ExecutionContext) -> flow_like_types::Result<()> {
        crate::rpa::branch::delay(
            ctx,
            std::time::Duration::from_millis(self.default_delay_ms.min(60_000)),
        )
        .await?;
        self.ensure_active(ctx).await
    }

    #[cfg(feature = "execute")]
    fn set_browser_options(&mut self, options: &BrowserContextOptions) {
        self.browser_type = Some(options.browser_type.clone());
        self.browser_headless = Some(options.headless);
        self.browser_user_data_dir = options.user_data_dir.clone();
    }

    /// Registers the browser slot of the session with the run before a browser is launched or
    /// connected, so a browser that becomes ready after the run ended is never left running.
    #[cfg(feature = "execute")]
    pub async fn prepare_browser_slot(
        &self,
        ctx: &ExecutionContext,
    ) -> flow_like_types::Result<()> {
        let wrapper = self.wrapper(ctx).await?;
        register_slot(&ctx.resources, &self.session_ref, &wrapper.slot)
    }

    #[cfg(feature = "execute")]
    pub async fn attach_cdp_browser(
        &mut self,
        ctx: &mut ExecutionContext,
        browser: flow_like_browser::Browser,
        options: &BrowserContextOptions,
    ) -> flow_like_types::Result<()> {
        let first_page = self.install_browser(ctx, browser).await?;
        self.set_browser_options(options);
        self.set_current_page_target(ctx, &first_page).await
    }

    /// Hands the browser to the slot of the session; on every failure, and when the node is
    /// dropped before the slot owns it, the browser is closed.
    #[cfg(feature = "execute")]
    async fn install_browser(
        &self,
        ctx: &ExecutionContext,
        browser: flow_like_browser::Browser,
    ) -> flow_like_types::Result<flow_like_browser::types::TargetId> {
        let browser = UnclaimedBrowser::new(browser);
        let wrapper = self.wrapper(ctx).await?;
        register_slot(&ctx.resources, &self.session_ref, &wrapper.slot)?;
        let Some(first_page) = browser.browser().pages().into_iter().next() else {
            browser.claim().shutdown().await;
            return Err(flow_like_types::anyhow!("The browser has no open tab"));
        };
        let _operation = wrapper.browser_lock.lock().await;
        wrapper.slot.install(browser.claim())?;
        Ok(first_page.target_id)
    }

    /// Serialises browser operations of parallel branches, then applies the session delay.
    #[cfg(feature = "execute")]
    async fn begin_operation(
        &self,
        ctx: &ExecutionContext,
    ) -> flow_like_types::Result<(AutomationSessionWrapper, tokio::sync::OwnedMutexGuard<()>)> {
        let wrapper = self.wrapper(ctx).await?;
        let guard = wrapper.browser_lock.clone().lock_owned().await;
        crate::rpa::branch::delay(
            ctx,
            std::time::Duration::from_millis(self.default_delay_ms.min(60_000)),
        )
        .await?;
        if !wrapper.active.load(std::sync::atomic::Ordering::Acquire) {
            return Err(flow_like_types::anyhow!("Automation session is closed"));
        }
        Ok((wrapper, guard))
    }

    /// The current page with its frame path replayed from the main frame (runs on every call).
    #[cfg(feature = "execute")]
    pub async fn browser_page(
        &self,
        ctx: &ExecutionContext,
    ) -> flow_like_types::Result<BrowserOperationGuard> {
        let (wrapper, guard) = self.begin_operation(ctx).await?;
        let page = self
            .page_context(&wrapper.slot, self.browser_frame_selectors.clone())
            .await?
            .enter_frame_path()
            .await?;
        Ok(BrowserOperationGuard {
            ctx: page,
            _guard: guard,
        })
    }

    /// The current page with its main frame as the current frame.
    #[cfg(feature = "execute")]
    pub async fn browser_top_page(
        &self,
        ctx: &ExecutionContext,
    ) -> flow_like_types::Result<BrowserOperationGuard> {
        let (wrapper, guard) = self.begin_operation(ctx).await?;
        let page = self.page_context(&wrapper.slot, Vec::new()).await?;
        Ok(BrowserOperationGuard {
            ctx: page,
            _guard: guard,
        })
    }

    #[cfg(feature = "execute")]
    pub async fn browser_guard(
        &self,
        ctx: &ExecutionContext,
    ) -> flow_like_types::Result<BrowserGuard> {
        let (wrapper, guard) = self.begin_operation(ctx).await?;
        Ok(BrowserGuard {
            browser: attached_browser(&wrapper.slot)?,
            _guard: guard,
        })
    }

    /// The browser without the operation lock or delay, for waits that must not block
    /// other branches.
    #[cfg(feature = "execute")]
    pub async fn cdp_browser(
        &self,
        ctx: &ExecutionContext,
    ) -> flow_like_types::Result<flow_like_browser::Browser> {
        attached_browser(&self.wrapper(ctx).await?.slot)
    }

    #[cfg(feature = "execute")]
    pub(crate) async fn browser_slot(
        &self,
        ctx: &ExecutionContext,
    ) -> flow_like_types::Result<Arc<BrowserSlot>> {
        Ok(self.wrapper(ctx).await?.slot)
    }

    #[cfg(feature = "execute")]
    async fn page_context(
        &self,
        slot: &Arc<BrowserSlot>,
        selectors: Vec<crate::types::selectors::Selector>,
    ) -> flow_like_types::Result<crate::browser::driver::PageContext> {
        let browser = attached_browser(slot)?;
        let handle = self
            .current_window_handle
            .as_deref()
            .ok_or_else(|| flow_like_types::anyhow!("Select or open a browser page first"))?;
        let page = browser.page(&page_target(handle)).await?;
        Ok(crate::browser::driver::PageContext::new(
            browser,
            page,
            slot.clone(),
            selectors,
        ))
    }

    pub fn has_browser(&self) -> bool {
        self.browser_type.is_some()
    }

    pub fn clear_current_page(&mut self) {
        self.current_page_ref = None;
        self.current_window_handle = None;
        self.browser_frame_selectors.clear();
    }

    #[cfg(feature = "execute")]
    pub async fn set_current_page_target(
        &mut self,
        ctx: &mut ExecutionContext,
        target: &flow_like_browser::types::TargetId,
    ) -> flow_like_types::Result<()> {
        self.ensure_active(ctx).await?;
        self.browser_frame_selectors.clear();
        self.current_page_ref = Some(create_id());
        self.current_window_handle = Some(page_target(target.as_str()).to_string());
        Ok(())
    }

    #[cfg(feature = "execute")]
    pub async fn detach_browser(
        &mut self,
        ctx: &mut ExecutionContext,
    ) -> flow_like_types::Result<()> {
        let wrapper = self.wrapper(ctx).await?;
        self.browser_type = None;
        self.browser_headless = None;
        self.browser_user_data_dir = None;
        self.clear_current_page();
        release_browser(&wrapper).await
    }

    /// Closes the browser of the session, then drops the session's cached resources.
    #[cfg(feature = "execute")]
    pub async fn close(&self, ctx: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let wrapper = self.wrapper(ctx).await?;
        wrapper
            .active
            .store(false, std::sync::atomic::Ordering::Release);
        let ended = release_browser(&wrapper).await;
        let prefixes = [
            "automation:auth:",
            "automation:network:",
            "automation:policy:",
            "automation:screen_state:",
        ]
        .map(|prefix| format!("{}{}", prefix, self.session_ref));
        ctx.cache.write().await.retain(|key, _| {
            key != &self.session_ref
                && !prefixes
                    .iter()
                    .any(|prefix| key == prefix || key.starts_with(&format!("{}:", prefix)))
        });
        ended
    }
}

/// Waits for an operation in flight on a parallel branch, then closes the DevTools browser
/// (owned: close and clean up; attached: disconnect).
#[cfg(feature = "execute")]
async fn release_browser(wrapper: &AutomationSessionWrapper) -> flow_like_types::Result<()> {
    let _guard = wrapper.browser_lock.lock().await;
    match wrapper.slot.take() {
        Some(browser) => close_cdp_browser(&browser).await,
        None => Ok(()),
    }
}

#[cfg(feature = "execute")]
async fn close_cdp_browser(browser: &flow_like_browser::Browser) -> flow_like_types::Result<()> {
    match browser.close().await {
        Ok(()) | Err(flow_like_browser::BrowserError::Disconnected { .. }) => Ok(()),
        Err(error) => Err(error.into()),
    }
}

#[cfg(all(test, feature = "execute"))]
pub(crate) mod tests {
    use super::*;
    use ahash::AHashMap;
    use flow_like::flow::board::ExecutionStage;
    use flow_like::flow::execution::internal_node::InternalNode;
    use flow_like::flow::execution::resources::{RunResource, RunResources};
    use flow_like::flow::execution::{LogLevel, Run};
    use flow_like::flow::node::NodeLogic;
    use flow_like::profile::Profile;
    use flow_like::state::{FlowLikeConfig, FlowLikeState};
    use flow_like::utils::http::HTTPClient;
    use flow_like_browser::test_hooks::TestSetup;
    use flow_like_browser::testing::default_auto_reply;
    use flow_like_browser::transport::memory::{InMemoryControl, InMemoryTransport, SentCommand};
    use flow_like_browser::types::{LoaderId, TargetId};
    use flow_like_browser::{Browser, ConnectionKind, RefTable};
    use flow_like_types::{Value, json::json};
    use std::time::Duration;

    const OCCUPIED: &str = "Close the attached browser before replacing it";
    const CANCELLED: &str = "The run was cancelled before the browser was ready";

    pub(crate) struct Scripted {
        pub(crate) browser: Browser,
        _control: InMemoryControl,
    }

    fn chrome_reply(command: &SentCommand) -> Option<Value> {
        let reply = match command.method.as_str() {
            "Browser.getVersion" => json!({
                "protocolVersion": "1.3",
                "product": "Chrome/154.0.8037.92",
                "revision": "@0",
                "userAgent": "Mozilla/5.0 Chrome/154.0.8037.92",
                "jsVersion": "15.4",
            }),
            "Target.getTargets" => json!({"targetInfos": [
                {"targetId": "T1", "type": "page", "url": "about:blank", "attached": false},
            ]}),
            "Target.attachToTarget" => json!({"sessionId": "S1"}),
            "Page.getFrameTree" => json!({"frameTree": {
                "frame": {"id": "T1", "loaderId": "L1", "url": "about:blank"},
                "childFrames": [],
            }}),
            _ => default_auto_reply(command).unwrap_or_else(|| json!({})),
        };
        Some(reply)
    }

    pub(crate) async fn scripted() -> Scripted {
        let (transport, control) = InMemoryTransport::new();
        control.set_auto_reply(chrome_reply);
        let setup = TestSetup {
            kind: ConnectionKind::Direct,
            headless: true,
            page_load_timeout: Duration::from_secs(30),
            process: None,
            staging: None,
            run_owned_setup: false,
        };
        let browser = Browser::connect_transport(Box::new(transport), setup)
            .await
            .expect("the scripted browser connects");
        Scripted {
            browser,
            _control: control,
        }
    }

    fn armed_slot(browser: &Browser) -> BrowserSlot {
        let slot = BrowserSlot::default();
        slot.install(browser.clone()).unwrap();
        locked(&slot.refs, |refs| {
            refs.table = RefTable::default()
                .begin(&TargetId::from("T1"), &LoaderId::from("L1"))
                .finish();
        });
        locked(&slot.downloads, |arm| {
            *arm = Some(DownloadArm {
                directory: None,
                cursor: 3,
                include_in_progress: false,
            });
        });
        slot
    }

    fn assert_cleared(slot: &BrowserSlot) {
        assert!(slot.browser().is_none());
        assert!(locked(&slot.refs, |refs| refs.table.page().is_none()));
        assert!(locked(&slot.downloads, |arm| arm.is_none()));
    }

    fn session_on(handle: Option<&str>) -> AutomationSession {
        AutomationSession {
            session_ref: "s1".into(),
            platform: Platform::Linux,
            default_delay_ms: 0,
            click_delay_ms: 0,
            debug_mode: false,
            browser_type: None,
            browser_headless: None,
            browser_user_data_dir: None,
            current_page_ref: None,
            current_window_handle: handle.map(str::to_owned),
            browser_frame_selectors: Vec::new(),
        }
    }

    #[tokio::test]
    async fn a_second_browser_is_refused_and_closed() {
        let (first, second) = (scripted().await, scripted().await);
        let slot = BrowserSlot::default();
        slot.install(first.browser.clone()).unwrap();
        assert_eq!(
            slot.install(second.browser.clone())
                .unwrap_err()
                .to_string(),
            OCCUPIED
        );
        assert!(!second.browser.is_alive());
        assert!(first.browser.is_alive());
        assert!(slot.browser().is_some_and(|browser| browser.is_alive()));
    }

    #[tokio::test]
    async fn a_browser_ready_after_the_run_ended_is_refused_and_closed() {
        let resources = RunResources::default();
        let slot = Arc::new(BrowserSlot::default());
        register_slot(&resources, "s1", &slot).unwrap();
        register_slot(&resources, "s1", &slot).expect("registering the same slot is idempotent");
        resources.abort();
        let late = scripted().await;
        assert_eq!(
            slot.install(late.browser.clone()).unwrap_err().to_string(),
            CANCELLED
        );
        assert!(!late.browser.is_alive());
        assert!(slot.browser().is_none());
    }

    #[tokio::test]
    async fn registering_needs_a_live_run_and_an_empty_slot() {
        let detached = RunResources::default();
        detached.abort();
        let slot = Arc::new(BrowserSlot::default());
        assert_eq!(
            register_slot(&detached, "s1", &slot)
                .unwrap_err()
                .to_string(),
            "Browser nodes need a live run to own the browser"
        );
        let attached = scripted().await;
        slot.install(attached.browser.clone()).unwrap();
        assert_eq!(
            register_slot(&RunResources::default(), "s1", &slot)
                .unwrap_err()
                .to_string(),
            OCCUPIED
        );
    }

    #[tokio::test]
    async fn ending_the_run_closes_the_browser_and_clears_the_slot() {
        let aborted = scripted().await;
        let slot = armed_slot(&aborted.browser);
        RunResource::abort(&slot);
        assert!(!aborted.browser.is_alive());
        assert_cleared(&slot);
        let late = scripted().await;
        assert_eq!(
            slot.install(late.browser.clone()).unwrap_err().to_string(),
            CANCELLED
        );

        let shut_down = scripted().await;
        let slot = armed_slot(&shut_down.browser);
        RunResource::shutdown(&slot).await;
        assert!(!shut_down.browser.is_alive());
        assert_cleared(&slot);

        let dropped = scripted().await;
        drop(armed_slot(&dropped.browser));
        assert!(!dropped.browser.is_alive());
    }

    #[tokio::test]
    async fn the_current_page_needs_a_browser_then_a_tab_and_reads_chromedriver_handles() {
        let slot = Arc::new(BrowserSlot::default());
        let no_browser = session_on(Some("T1"))
            .page_context(&slot, Vec::new())
            .await
            .err()
            .expect("an empty slot has no page");
        assert_eq!(
            no_browser.to_string(),
            "No browser attached to this session"
        );

        let scripted = scripted().await;
        slot.install(scripted.browser.clone()).unwrap();
        let no_tab = session_on(None)
            .page_context(&slot, Vec::new())
            .await
            .err()
            .expect("a session without a current tab has no page");
        assert_eq!(no_tab.to_string(), "Select or open a browser page first");

        let ctx = tokio::time::timeout(
            Duration::from_secs(10),
            session_on(Some("CDwindow-T1")).page_context(&slot, Vec::new()),
        )
        .await
        .expect("attaching the scripted tab finishes")
        .expect("a chromedriver window handle names the DevTools target");
        assert_eq!(ctx.page.target_id().as_str(), "T1");
        assert_eq!(ctx.frame().id(), ctx.page.main_frame().id());
    }

    #[tokio::test]
    async fn stop_session_waits_for_the_operation_in_flight_then_closes_the_browser() {
        let wrapper = AutomationSessionWrapper::new();
        let scripted = scripted().await;
        wrapper.slot.install(scripted.browser.clone()).unwrap();
        let operation = wrapper.browser_lock.clone().lock_owned().await;
        let releasing = tokio::spawn({
            let wrapper = wrapper.clone();
            async move { release_browser(&wrapper).await }
        });
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(scripted.browser.is_alive());
        assert!(wrapper.slot.browser().is_some());
        drop(operation);
        releasing
            .await
            .expect("releasing does not panic")
            .expect("closing the browser succeeds");
        assert!(!scripted.browser.is_alive());
        assert!(wrapper.slot.browser().is_none());
    }

    #[tokio::test]
    async fn an_unclaimed_browser_disconnects_when_dropped() {
        let dropped = scripted().await;
        drop(UnclaimedBrowser::new(dropped.browser.clone()));
        assert!(!dropped.browser.is_alive());
        let claimed = scripted().await;
        let browser = UnclaimedBrowser::new(claimed.browser.clone()).claim();
        assert!(browser.is_alive());
    }

    async fn live_run_context() -> ExecutionContext {
        let logic: Arc<dyn NodeLogic> = Arc::new(crate::browser::manage::BrowserAttachNode::new());
        let node = Arc::new(InternalNode::new(
            logic.get_node(),
            AHashMap::new(),
            logic,
            AHashMap::new(),
        ));
        let state = Arc::new(FlowLikeState::new(
            FlowLikeConfig::new(),
            HTTPClient::new_without_refetch(),
        ));
        let run: std::sync::Weak<flow_like_types::sync::Mutex<Run>> = std::sync::Weak::new();
        let mut context = ExecutionContext::new(
            Arc::new(AHashMap::from_iter([(
                node.node_id().to_string(),
                node.clone(),
            )])),
            &run,
            &state,
            &node,
            &Arc::new(flow_like_types::sync::Mutex::new(AHashMap::new())),
            &Arc::new(flow_like_types::sync::RwLock::new(AHashMap::new())),
            LogLevel::Info,
            ExecutionStage::Dev,
            Arc::new(Profile::default()),
            None,
            Arc::new(flow_like_types::sync::RwLock::new(Vec::new())),
            None,
            None,
            Arc::new(AHashMap::new()),
            None,
        )
        .await;
        context.resources = Arc::new(RunResources::default());
        context
    }

    #[tokio::test]
    async fn a_browser_dropped_before_the_session_owns_it_is_disconnected() {
        let mut context = live_run_context().await;
        let mut session = AutomationSession::new(&mut context, 0, 0, false)
            .await
            .expect("the session starts");
        let wrapper = session.wrapper(&context).await.unwrap();
        let operation = wrapper.browser_lock.clone().lock_owned().await;
        let attached = scripted().await;
        let options = BrowserContextOptions::default();
        {
            let installing =
                session.attach_cdp_browser(&mut context, attached.browser.clone(), &options);
            let mut installing = std::pin::pin!(installing);
            assert!(futures::poll!(installing.as_mut()).is_pending());
        }
        drop(operation);
        assert!(wrapper.slot.browser().is_none());
        assert!(
            !attached.browser.is_alive(),
            "a browser the node dropped before the slot owned it must disconnect"
        );
    }
}
