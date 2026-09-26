use crate::types::handles::AutomationSession;
#[cfg(feature = "execute")]
use crate::types::handles::{BrowserContextOptions, BrowserType};
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json};
#[cfg(feature = "execute")]
use std::time::Duration;
#[cfg(feature = "execute")]
use thirtyfour::{
    Capabilities, DesiredCapabilities, common::capabilities::chromium::ChromiumLikeCapabilities,
};

#[crate::register_node]
#[derive(Default)]
pub struct BrowserOpenNode {}

impl BrowserOpenNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for BrowserOpenNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "browser_open",
            "Open Browser",
            "Connects to a WebDriver server and opens a new browser session",
            "Automation/Browser",
        );
        node.set_version(1);
        node.set_flowscript_name("browser", "open");
        node.add_icon("/flow/icons/browser.svg");

        node.set_scores(
            flow_like::flow::node::NodeScores::new()
                .set_privacy(4)
                .set_security(5)
                .set_performance(8)
                .set_governance(6)
                .set_reliability(8)
                .set_cost(9)
                .build(),
        );
        node.set_only_offline(true);

        node.add_input_pin("exec_in", "▶", "Trigger", VariableType::Execution);

        node.add_input_pin(
            "session",
            "Session",
            "Automation session to attach browser to",
            VariableType::Struct,
        )
        .set_schema::<AutomationSession>();

        node.add_input_pin(
            "webdriver_url",
            "WebDriver URL",
            "URL of the WebDriver server (e.g., http://localhost:9515 for ChromeDriver)",
            VariableType::String,
        )
        .set_default_value(Some(json!("http://localhost:9515")));

        node.add_input_pin(
            "browser_type",
            "Browser Type",
            "Browser to use (Chrome, Firefox, Edge, Safari)",
            VariableType::String,
        )
        .set_options(
            flow_like::flow::pin::PinOptions::new()
                .set_valid_values(vec![
                    "Chrome".to_string(),
                    "Firefox".to_string(),
                    "Edge".to_string(),
                    "Safari".to_string(),
                ])
                .build(),
        )
        .set_default_value(Some(json!("Chrome")));

        node.add_input_pin(
            "headless",
            "Headless",
            "Run browser in headless mode (no visible window)",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(true)));

        node.add_input_pin(
            "viewport_width",
            "Viewport Width",
            "Browser viewport width in pixels",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(1920)));

        node.add_input_pin(
            "viewport_height",
            "Viewport Height",
            "Browser viewport height in pixels",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(1080)));

        node.add_input_pin(
            "user_agent",
            "User Agent",
            "Custom user agent string (optional)",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_input_pin(
            "page_load_timeout",
            "Page Load Timeout (s)",
            "Timeout for page loads in seconds",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(30)));

        node.add_output_pin("exec_out", "▶", "Continue", VariableType::Execution);
        node.add_output_pin(
            "debugger_address",
            "Debugger Address",
            "Chrome or Edge debugger endpoint, when available",
            VariableType::String,
        );

        node.add_output_pin(
            "session_out",
            "Session",
            "Automation session with browser attached",
            VariableType::Struct,
        )
        .set_schema::<AutomationSession>();

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let mut session: AutomationSession = context.evaluate_pin("session").await?;
        let webdriver_url: String = context.evaluate_pin("webdriver_url").await?;
        let browser_type_str: String = context.evaluate_pin("browser_type").await?;
        let headless: bool = context.evaluate_pin("headless").await?;
        let viewport_width: i64 = context.evaluate_pin("viewport_width").await?;
        let viewport_height: i64 = context.evaluate_pin("viewport_height").await?;
        let user_agent: String = context.evaluate_pin("user_agent").await?;
        let page_load_timeout: i64 = context.evaluate_pin("page_load_timeout").await?;
        session.ensure_active(context).await?;
        if session.has_browser() {
            return Err(flow_like_types::anyhow!(
                "Close the attached browser before replacing it"
            ));
        }

        let browser_type = match browser_type_str.as_str() {
            "Firefox" => BrowserType::Firefox,
            "Edge" => BrowserType::Edge,
            "Safari" => BrowserType::Safari,
            "Chrome" => BrowserType::Chrome,
            _ => return Err(flow_like_types::anyhow!("Unknown browser type")),
        };

        let caps = match browser_type {
            BrowserType::Chrome => {
                let mut chrome_caps = DesiredCapabilities::chrome();
                if headless {
                    chrome_caps
                        .set_headless()
                        .map_err(|e| flow_like_types::anyhow!("Failed to set headless: {}", e))?;
                }
                chrome_caps
                    .add_arg(&format!(
                        "--window-size={},{}",
                        viewport_width, viewport_height
                    ))
                    .map_err(|e| flow_like_types::anyhow!("Failed to set window size: {}", e))?;
                if !user_agent.is_empty() {
                    chrome_caps
                        .add_arg(&format!("--user-agent={}", user_agent))
                        .map_err(|e| flow_like_types::anyhow!("Failed to set user agent: {}", e))?;
                }
                Capabilities::from(chrome_caps)
            }
            BrowserType::Firefox => {
                let mut firefox_caps = DesiredCapabilities::firefox();
                if headless {
                    firefox_caps
                        .set_headless()
                        .map_err(|e| flow_like_types::anyhow!("Failed to set headless: {}", e))?;
                }
                if !user_agent.is_empty() {
                    let mut prefs =
                        thirtyfour::common::capabilities::firefox::FirefoxPreferences::new();
                    prefs.set("general.useragent.override", &user_agent)?;
                    firefox_caps.set_preferences(prefs)?;
                }
                Capabilities::from(firefox_caps)
            }
            BrowserType::Edge => {
                let mut edge_caps = DesiredCapabilities::edge();
                if headless {
                    edge_caps
                        .set_headless()
                        .map_err(|e| flow_like_types::anyhow!("Failed to set headless: {}", e))?;
                }
                if !user_agent.is_empty() {
                    edge_caps.add_arg(&format!("--user-agent={user_agent}"))?;
                }
                Capabilities::from(edge_caps)
            }
            BrowserType::Safari => {
                if headless || !user_agent.is_empty() {
                    return Err(flow_like_types::anyhow!(
                        "Safari WebDriver does not support headless mode or a custom user agent"
                    ));
                }
                let safari_caps = DesiredCapabilities::safari();
                Capabilities::from(safari_caps)
            }
        };

        if viewport_width < 1
            || viewport_height < 1
            || viewport_width > u32::MAX as i64
            || viewport_height > u32::MAX as i64
            || page_load_timeout < 0
        {
            return Err(flow_like_types::anyhow!(
                "Viewport dimensions must be positive and page timeout nonnegative"
            ));
        }
        let (driver, debugger_address) = super::protocol::connect_webdriver(&webdriver_url, caps)
            .await
            .map_err(|e| {
                flow_like_types::anyhow!(
                    "Failed to connect to WebDriver at {}: {}",
                    webdriver_url,
                    e
                )
            })?;

        let setup: flow_like_types::Result<thirtyfour::WindowHandle> = async {
            driver
                .set_page_load_timeout(Duration::from_secs(page_load_timeout as u64))
                .await
                .map_err(|e| flow_like_types::anyhow!("Failed to set page load timeout: {}", e))?;

            driver
                .set_window_rect(0, 0, viewport_width as u32, viewport_height as u32)
                .await
                .map_err(|e| flow_like_types::anyhow!("Failed to set window size: {}", e))?;
            Ok(driver.window().await?)
        }
        .await;
        let initial_window = match setup {
            Ok(handle) => handle,
            Err(error) => {
                let _ = driver.quit().await;
                return Err(error);
            }
        };

        let options = BrowserContextOptions {
            browser_type,
            headless,
            viewport_width: Some(viewport_width as u32),
            viewport_height: Some(viewport_height as u32),
            webdriver_url: Some(webdriver_url),
            user_agent: if user_agent.is_empty() {
                None
            } else {
                Some(user_agent)
            },
            ..Default::default()
        };

        if let Err(error) = session
            .attach_browser(context, driver.clone(), &options)
            .await
        {
            let _ = driver.quit().await;
            return Err(error);
        }
        session.set_current_page(context, initial_window).await?;
        if let Some(address) = &debugger_address {
            super::protocol::remember_debugger_address(context, &session, address).await;
        }
        super::selector::optional_output(
            context,
            "debugger_address",
            json!(debugger_address.unwrap_or_default()),
        )
        .await?;

        super::selector::optional_output(context, "session_out", json!(session)).await?;
        context.activate_exec_pin("exec_out").await?;

        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "Browser automation requires the 'execute' feature"
        ))
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct BrowserCloseNode {}

impl BrowserCloseNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for BrowserCloseNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "browser_close",
            "Close Browser",
            "Closes a browser started by this session, or disconnects from an existing browser",
            "Automation/Browser",
        );
        node.set_version(1);
        node.set_flowscript_name("browser", "close");
        node.add_icon("/flow/icons/browser.svg");

        node.set_scores(
            flow_like::flow::node::NodeScores::new()
                .set_privacy(4)
                .set_security(5)
                .set_performance(8)
                .set_governance(6)
                .set_reliability(9)
                .set_cost(10)
                .build(),
        );
        node.set_only_offline(true);

        node.add_input_pin("exec_in", "▶", "Trigger", VariableType::Execution);

        node.add_input_pin(
            "session",
            "Session",
            "Automation session with browser to close",
            VariableType::Struct,
        )
        .set_schema::<AutomationSession>();

        node.add_output_pin("exec_out", "▶", "Continue", VariableType::Execution);

        node.add_output_pin(
            "session_out",
            "Session",
            "Session with browser detached",
            VariableType::Struct,
        )
        .set_schema::<AutomationSession>();
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let mut session: AutomationSession = context.evaluate_pin("session").await?;
        let detached = session.detach_browser(context).await;
        super::protocol::clear_listeners(context, &session).await;
        context
            .cache
            .write()
            .await
            .remove(&format!("automation:debugger:{}", session.session_ref));
        detached?;
        super::selector::optional_output(context, "session_out", json!(session)).await?;

        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "Browser automation requires the 'execute' feature"
        ))
    }
}
