use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[cfg(feature = "execute")]
use flow_like::flow::execution::context::ExecutionContext;
#[cfg(feature = "execute")]
use flow_like_types::{Cacheable, create_id};
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

/// WebDriver server process (chromedriver, geckodriver, …) started for one session.
#[cfg(feature = "execute")]
pub(crate) struct DriverServer {
    child: tokio::process::Child,
}

#[cfg(feature = "execute")]
impl DriverServer {
    pub(crate) fn new(child: tokio::process::Child) -> Self {
        Self { child }
    }

    pub(crate) fn has_exited(&mut self) -> flow_like_types::Result<bool> {
        Ok(self.child.try_wait()?.is_some())
    }
}

#[cfg(feature = "execute")]
impl Drop for DriverServer {
    fn drop(&mut self) {
        let _ = self.child.start_kill();
    }
}

/// The server must outlive the WebDriver session: killing chromedriver before its
/// session is deleted orphans the browser it launched.
#[cfg(feature = "execute")]
#[derive(Default)]
struct BrowserState {
    driver: Option<Arc<thirtyfour::WebDriver>>,
    owned: bool,
    server: Option<Arc<DriverServer>>,
}

#[cfg(feature = "execute")]
impl Drop for BrowserState {
    fn drop(&mut self) {
        let Some(driver) = self.driver.take() else {
            return;
        };
        let server = self.server.take();
        let owned = self.owned;
        match tokio::runtime::Handle::try_current() {
            Ok(runtime) => {
                runtime.spawn(async move {
                    let _ = end_driver_session(&driver, owned).await;
                    drop(driver);
                    drop(server);
                });
            }
            Err(_) => {
                drop(driver);
                drop(server);
            }
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
    browser: Arc<tokio::sync::RwLock<BrowserState>>,
    browser_lock: Arc<tokio::sync::Mutex<()>>,
    active: Arc<std::sync::atomic::AtomicBool>,
    _lifetime: Arc<SessionLifetime>,
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

/// Keeps tab selection and every command in a node in one browser operation.
#[cfg(feature = "execute")]
pub struct BrowserOperationGuard {
    driver: Arc<thirtyfour::WebDriver>,
    _guard: tokio::sync::OwnedMutexGuard<()>,
}
#[cfg(feature = "execute")]
impl std::ops::Deref for BrowserOperationGuard {
    type Target = thirtyfour::WebDriver;
    fn deref(&self) -> &Self::Target {
        &self.driver
    }
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
        let active = Arc::new(std::sync::atomic::AtomicBool::new(true));
        let wrapper = AutomationSessionWrapper {
            browser: Arc::new(tokio::sync::RwLock::new(BrowserState::default())),
            browser_lock: Arc::new(tokio::sync::Mutex::new(())),
            active: active.clone(),
            _lifetime: Arc::new(SessionLifetime(active)),
        };
        ctx.cache
            .write()
            .await
            .insert(id.clone(), Arc::new(wrapper));
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
    pub async fn attach_browser(
        &mut self,
        ctx: &mut ExecutionContext,
        driver: thirtyfour::WebDriver,
        options: &BrowserContextOptions,
    ) -> flow_like_types::Result<()> {
        self.attach(ctx, driver, options, true).await
    }

    #[cfg(feature = "execute")]
    pub async fn attach_existing_browser(
        &mut self,
        ctx: &mut ExecutionContext,
        driver: thirtyfour::WebDriver,
        options: &BrowserContextOptions,
    ) -> flow_like_types::Result<()> {
        // Prevent the WebDriver destructor from closing a browser owned by the user.
        driver.clone().leak()?;
        self.attach(ctx, driver, options, false).await
    }

    #[cfg(feature = "execute")]
    async fn attach(
        &mut self,
        ctx: &mut ExecutionContext,
        driver: thirtyfour::WebDriver,
        options: &BrowserContextOptions,
        owned: bool,
    ) -> flow_like_types::Result<()> {
        let wrapper = self.wrapper(ctx).await?;
        let _operation = wrapper.browser_lock.lock().await;
        let mut state = wrapper.browser.write().await;
        if state.driver.is_some() {
            return Err(flow_like_types::anyhow!(
                "Close the attached browser before replacing it"
            ));
        }
        state.driver = Some(Arc::new(driver));
        state.owned = owned;
        self.browser_type = Some(options.browser_type.clone());
        self.browser_headless = Some(options.headless);
        self.browser_user_data_dir = options.user_data_dir.clone();
        self.clear_current_page();
        Ok(())
    }

    #[cfg(feature = "execute")]
    pub(crate) async fn has_driver_server(
        &self,
        ctx: &ExecutionContext,
    ) -> flow_like_types::Result<bool> {
        Ok(self
            .wrapper(ctx)
            .await?
            .browser
            .read()
            .await
            .server
            .is_some())
    }

    /// Ties the WebDriver server to this session so it is stopped only after the browser session ends.
    #[cfg(feature = "execute")]
    pub(crate) async fn set_driver_server(
        &self,
        ctx: &ExecutionContext,
        server: DriverServer,
    ) -> flow_like_types::Result<()> {
        let wrapper = self.wrapper(ctx).await?;
        let mut state = wrapper.browser.write().await;
        if state.server.is_some() {
            return Err(flow_like_types::anyhow!(
                "Stop this session's WebDriver before starting another"
            ));
        }
        state.server = Some(Arc::new(server));
        Ok(())
    }

    #[cfg(feature = "execute")]
    pub(crate) async fn stop_driver_server(
        &self,
        ctx: &ExecutionContext,
    ) -> flow_like_types::Result<()> {
        self.wrapper(ctx).await?.browser.write().await.server = None;
        Ok(())
    }

    #[cfg(feature = "execute")]
    pub async fn get_browser_driver(
        &self,
        ctx: &ExecutionContext,
    ) -> flow_like_types::Result<BrowserOperationGuard> {
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
        let driver = wrapper
            .browser
            .read()
            .await
            .driver
            .clone()
            .ok_or_else(|| flow_like_types::anyhow!("No browser attached to this session"))?;
        Ok(BrowserOperationGuard {
            driver,
            _guard: guard,
        })
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
    pub async fn set_current_page(
        &mut self,
        ctx: &mut ExecutionContext,
        window_handle: thirtyfour::WindowHandle,
    ) -> flow_like_types::Result<()> {
        self.ensure_active(ctx).await?;
        self.browser_frame_selectors.clear();
        self.current_page_ref = Some(create_id());
        self.current_window_handle = Some(window_handle.to_string());
        Ok(())
    }

    #[cfg(feature = "execute")]
    pub async fn get_browser_driver_and_switch(
        &self,
        ctx: &ExecutionContext,
    ) -> flow_like_types::Result<BrowserOperationGuard> {
        let driver = self.get_browser_driver(ctx).await?;
        let handle = self
            .current_window_handle
            .as_ref()
            .ok_or_else(|| flow_like_types::anyhow!("Select or open a browser page first"))?;
        driver
            .switch_to_window(thirtyfour::WindowHandle::from(handle.clone()))
            .await?;
        driver.enter_default_frame().await?;
        for selector in &self.browser_frame_selectors {
            crate::browser::selector::find(&driver, selector)
                .await?
                .enter_frame()
                .await?;
        }
        Ok(driver)
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
        take_and_end_browser(&wrapper).await
    }

    /// Ends the browser session while its WebDriver server is still running, then
    /// drops the session's cached resources, which stops the server.
    #[cfg(feature = "execute")]
    pub async fn close(&self, ctx: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let wrapper = self.wrapper(ctx).await?;
        wrapper
            .active
            .store(false, std::sync::atomic::Ordering::Release);
        let ended = take_and_end_browser(&wrapper).await;
        let server = wrapper.browser.write().await.server.take();
        let prefixes = [
            "automation:auth:",
            "automation:network:",
            "automation:download:",
            "automation:debugger:",
            "automation:refs:",
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
        drop(server);
        ended
    }
}

#[cfg(feature = "execute")]
async fn take_and_end_browser(wrapper: &AutomationSessionWrapper) -> flow_like_types::Result<()> {
    let _guard = wrapper.browser_lock.lock().await;
    let (driver, owned) = {
        let mut state = wrapper.browser.write().await;
        (state.driver.take(), state.owned)
    };
    match driver {
        Some(driver) => end_driver_session(&driver, owned).await,
        None => Ok(()),
    }
}

#[cfg(feature = "execute")]
async fn end_driver_session(
    driver: &thirtyfour::WebDriver,
    owned: bool,
) -> flow_like_types::Result<()> {
    if owned {
        ignore_closed_driver(driver.clone().quit().await)
    } else {
        // Chromium's remote-debugging backend deletes the driver session without closing the attached browser.
        ignore_closed_driver(
            driver
                .handle
                .cmd(thirtyfour::common::command::Command::DeleteSession)
                .await
                .map(|_| ()),
        )
    }
}

#[cfg(feature = "execute")]
fn ignore_closed_driver(
    result: thirtyfour::error::WebDriverResult<()>,
) -> flow_like_types::Result<()> {
    match result {
        Ok(()) => Ok(()),
        Err(error)
            if matches!(
                error.as_inner(),
                thirtyfour::error::WebDriverErrorInner::InvalidSessionId(_)
            ) =>
        {
            Ok(())
        }
        Err(error) => Err(error.into()),
    }
}
