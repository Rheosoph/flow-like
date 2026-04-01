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
}

#[cfg(feature = "execute")]
pub struct AutomationSessionWrapper {
    pub autogui: Arc<flow_like_types::sync::Mutex<rustautogui::RustAutoGui>>,
    pub browser_driver: Option<Arc<thirtyfour::WebDriver>>,
    pub current_window_handle: Option<thirtyfour::WindowHandle>,
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

        let autogui = rustautogui::RustAutoGui::new(debug_mode)
            .map_err(|e| flow_like_types::anyhow!("Failed to create RustAutoGui: {}", e))?;

        let wrapper = AutomationSessionWrapper {
            autogui: Arc::new(flow_like_types::sync::Mutex::new(autogui)),
            browser_driver: None,
            current_window_handle: None,
        };
        ctx.cache
            .write()
            .await
            .insert(id.clone(), Arc::new(wrapper));

        Ok(AutomationSession {
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
        })
    }

    /// Creates enigo instance for keyboard/mouse control
    #[cfg(feature = "execute")]
    pub fn create_enigo(&self) -> flow_like_types::Result<enigo::Enigo> {
        use enigo::{Enigo, Settings};
        let settings = Settings::default();
        Enigo::new(&settings).map_err(|e| flow_like_types::anyhow!("Failed to create Enigo: {}", e))
    }

    /// Get rustautogui for template matching
    #[cfg(feature = "execute")]
    pub async fn get_autogui(
        &self,
        ctx: &ExecutionContext,
    ) -> flow_like_types::Result<Arc<flow_like_types::sync::Mutex<rustautogui::RustAutoGui>>> {
        let cache = ctx.cache.read().await;
        let wrapper = cache
            .get(&self.session_ref)
            .ok_or_else(|| flow_like_types::anyhow!("Automation session not found in cache"))?;
        let wrapper = wrapper
            .as_any()
            .downcast_ref::<AutomationSessionWrapper>()
            .ok_or_else(|| {
                flow_like_types::anyhow!("Could not downcast to AutomationSessionWrapper")
            })?;
        Ok(wrapper.autogui.clone())
    }

    /// Attach a browser to this session
    #[cfg(feature = "execute")]
    pub async fn attach_browser(
        &mut self,
        ctx: &mut ExecutionContext,
        driver: thirtyfour::WebDriver,
        options: &BrowserContextOptions,
    ) -> flow_like_types::Result<()> {
        let driver_arc = Arc::new(driver);

        {
            let mut cache = ctx.cache.write().await;
            if let Some(wrapper) = cache.get_mut(&self.session_ref) {
                if let Some(wrapper) = Arc::get_mut(wrapper) {
                    if let Some(auto_wrapper) = wrapper
                        .as_any_mut()
                        .downcast_mut::<AutomationSessionWrapper>()
                    {
                        auto_wrapper.browser_driver = Some(driver_arc);
                    }
                } else {
                    return Err(flow_like_types::anyhow!(
                        "Cannot attach browser: session wrapper has multiple references"
                    ));
                }
            }
        }

        self.browser_type = Some(options.browser_type.clone());
        self.browser_headless = Some(options.headless);
        self.browser_user_data_dir = options.user_data_dir.clone();

        Ok(())
    }

    /// Get the browser driver if attached
    #[cfg(feature = "execute")]
    pub async fn get_browser_driver(
        &self,
        ctx: &ExecutionContext,
    ) -> flow_like_types::Result<Arc<thirtyfour::WebDriver>> {
        let cache = ctx.cache.read().await;
        let wrapper = cache
            .get(&self.session_ref)
            .ok_or_else(|| flow_like_types::anyhow!("Automation session not found in cache"))?;
        let wrapper = wrapper
            .as_any()
            .downcast_ref::<AutomationSessionWrapper>()
            .ok_or_else(|| {
                flow_like_types::anyhow!("Could not downcast to AutomationSessionWrapper")
            })?;
        wrapper
            .browser_driver
            .clone()
            .ok_or_else(|| flow_like_types::anyhow!("No browser attached to this session"))
    }

    /// Check if browser is attached
    pub fn has_browser(&self) -> bool {
        self.browser_type.is_some()
    }

    /// Set the current page/window handle
    #[cfg(feature = "execute")]
    pub async fn set_current_page(
        &mut self,
        ctx: &mut ExecutionContext,
        window_handle: thirtyfour::WindowHandle,
    ) -> flow_like_types::Result<()> {
        let page_ref = create_id();
        let handle_str = window_handle.to_string();

        {
            let mut cache = ctx.cache.write().await;
            if let Some(wrapper) = cache.get_mut(&self.session_ref) {
                if let Some(wrapper) = Arc::get_mut(wrapper) {
                    if let Some(auto_wrapper) = wrapper
                        .as_any_mut()
                        .downcast_mut::<AutomationSessionWrapper>()
                    {
                        auto_wrapper.current_window_handle = Some(window_handle);
                    }
                } else {
                    return Err(flow_like_types::anyhow!(
                        "Cannot set current page: session wrapper has multiple references"
                    ));
                }
            }
        }

        self.current_page_ref = Some(page_ref);
        self.current_window_handle = Some(handle_str);

        Ok(())
    }

    /// Get browser driver and switch to current window
    #[cfg(feature = "execute")]
    pub async fn get_browser_driver_and_switch(
        &self,
        ctx: &ExecutionContext,
    ) -> flow_like_types::Result<Arc<thirtyfour::WebDriver>> {
        let cache = ctx.cache.read().await;
        let wrapper = cache
            .get(&self.session_ref)
            .ok_or_else(|| flow_like_types::anyhow!("Automation session not found in cache"))?;
        let wrapper = wrapper
            .as_any()
            .downcast_ref::<AutomationSessionWrapper>()
            .ok_or_else(|| {
                flow_like_types::anyhow!("Could not downcast to AutomationSessionWrapper")
            })?;

        let driver = wrapper
            .browser_driver
            .clone()
            .ok_or_else(|| flow_like_types::anyhow!("No browser attached to this session"))?;

        if let Some(window_handle) = &wrapper.current_window_handle {
            driver
                .switch_to_window(window_handle.clone())
                .await
                .map_err(|e| flow_like_types::anyhow!("Failed to switch window: {}", e))?;
        }

        Ok(driver)
    }

    /// Close the session and release resources
    #[cfg(feature = "execute")]
    pub async fn close(&self, ctx: &mut ExecutionContext) -> flow_like_types::Result<()> {
        // Close browser if attached
        {
            let cache = ctx.cache.read().await;
            if let Some(wrapper) = cache.get(&self.session_ref)
                && let Some(auto_wrapper) =
                    wrapper.as_any().downcast_ref::<AutomationSessionWrapper>()
                && let Some(driver) = &auto_wrapper.browser_driver
            {
                let driver_clone = (**driver).clone();
                let _ = driver_clone.quit().await;
            }
        }

        // Remove from cache
        ctx.cache.write().await.remove(&self.session_ref);

        Ok(())
    }
}
