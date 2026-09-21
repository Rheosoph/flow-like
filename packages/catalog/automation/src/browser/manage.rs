use crate::types::handles::AutomationSession;
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::ValueType,
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub(crate) fn base_node(id: &str, title: &str, description: &str) -> Node {
    let mut node = Node::new(id, title, description, "Automation/Browser");
    node.set_version(1);
    node.add_icon("/flow/icons/browser.svg");
    node.set_only_offline(true);
    node.add_input_pin("exec_in", "▶", "Trigger", VariableType::Execution);
    node.add_input_pin(
        "session",
        "Session",
        "Automation session",
        VariableType::Struct,
    )
    .set_schema::<AutomationSession>();
    node.add_output_pin("exec_out", "▶", "Continue", VariableType::Execution);
    node.add_output_pin(
        "session_out",
        "Session",
        "Updated automation session",
        VariableType::Struct,
    )
    .set_schema::<AutomationSession>();
    node
}
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub struct BrowserTab {
    pub handle: String,
    pub title: String,
    pub url: String,
}

#[crate::register_node]
#[derive(Default)]
pub struct BrowserListTabsNode {}
impl BrowserListTabsNode {
    pub fn new() -> Self {
        Self {}
    }
}
#[async_trait]
impl NodeLogic for BrowserListTabsNode {
    fn get_node(&self) -> Node {
        let mut node = base_node(
            "browser_list_tabs",
            "List Tabs",
            "Lists the browser tabs and their URLs.",
        );
        node.set_flowscript_name("browser", "listTabs");
        node.add_output_pin("tabs", "Tabs", "Open browser tabs", VariableType::Struct)
            .set_schema::<BrowserTab>()
            .set_value_type(ValueType::Array);
        node
    }
    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.check_cancelled()?;
        context.deactivate_exec_pin("exec_out").await?;
        let session: AutomationSession = context.evaluate_pin("session").await?;
        let driver = session.get_browser_driver_and_switch(context).await?;
        let current = driver.window().await?;
        let result: flow_like_types::Result<Vec<BrowserTab>> = async {
            let mut tabs = Vec::new();
            for handle in driver.windows().await? {
                context.check_cancelled()?;
                driver.switch_to_window(handle.clone()).await?;
                tabs.push(BrowserTab {
                    handle: handle.to_string(),
                    title: driver.title().await?,
                    url: driver.current_url().await?.to_string(),
                });
            }
            Ok(tabs)
        }
        .await;
        let restored = driver.switch_to_window(current).await;
        let tabs = result?;
        restored?;
        context.set_pin_value("tabs", json!(tabs)).await?;
        context.set_pin_value("session_out", json!(session)).await?;
        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }
    #[cfg(not(feature = "execute"))]
    async fn run(&self, _: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "Browser automation requires the execute feature"
        ))
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct BrowserSelectTabNode {}
impl BrowserSelectTabNode {
    pub fn new() -> Self {
        Self {}
    }
}
#[async_trait]
impl NodeLogic for BrowserSelectTabNode {
    fn get_node(&self) -> Node {
        let mut node = base_node(
            "browser_select_tab",
            "Select Tab",
            "Selects a tab by its explicit browser handle.",
        );
        node.set_flowscript_name("browser", "selectTab");
        node.add_input_pin(
            "target_id",
            "Browser Target ID",
            "Exact CDP target ID of an attached Chrome or Edge tab",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));
        node.add_input_pin(
            "handle",
            "Handle",
            "Handle returned by List Tabs",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));
        node.add_input_pin(
            "url",
            "URL",
            "Exact URL when no handle is supplied",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));
        node
    }
    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.check_cancelled()?;
        context.deactivate_exec_pin("exec_out").await?;
        let mut session: AutomationSession = context.evaluate_pin("session").await?;
        let handle: String = context.evaluate_pin("handle").await?;
        let target_id: String = context.evaluate_pin("target_id").await?;
        let driver = session.get_browser_driver(context).await?;
        let handle = if !target_id.is_empty() {
            let expected = format!("CDwindow-{target_id}");
            driver
                .windows()
                .await?
                .into_iter()
                .find(|window| window.to_string() == expected || window.to_string() == target_id)
                .ok_or_else(|| flow_like_types::anyhow!("Recorded browser tab is no longer open"))?
        } else if handle.is_empty() {
            let url: String = context.evaluate_pin("url").await?;
            if url.is_empty() {
                return Err(flow_like_types::anyhow!(
                    "Provide a tab handle or exact URL"
                ));
            }
            let original = driver.window().await?;
            let result: flow_like_types::Result<Vec<thirtyfour::WindowHandle>> = async {
                let mut matches = Vec::new();
                for handle in driver.windows().await? {
                    driver.switch_to_window(handle.clone()).await?;
                    if driver.current_url().await?.as_str() == url {
                        matches.push(handle);
                    }
                }
                Ok(matches)
            }
            .await;
            driver.switch_to_window(original).await?;
            let matches = result?;
            if matches.len() != 1 {
                return Err(flow_like_types::anyhow!(
                    "Expected one tab with this URL; found {}",
                    matches.len()
                ));
            }
            matches[0].clone()
        } else {
            thirtyfour::WindowHandle::from(handle)
        };
        driver.switch_to_window(handle.clone()).await?;
        session.set_current_page(context, handle).await?;
        context.set_pin_value("session_out", json!(session)).await?;
        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }
    #[cfg(not(feature = "execute"))]
    async fn run(&self, _: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "Browser automation requires the execute feature"
        ))
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct BrowserEnterFrameNode {}
impl BrowserEnterFrameNode {
    pub fn new() -> Self {
        Self {}
    }
}
#[async_trait]
impl NodeLogic for BrowserEnterFrameNode {
    fn get_node(&self) -> Node {
        let mut node = base_node(
            "browser_enter_frame",
            "Enter Frame",
            "Selects an iframe for subsequent actions on this session wire.",
        );
        node.set_flowscript_name("browser", "enterFrame");
        super::selector::add_locator_pin(&mut node);
        node.add_input_pin(
            "selector",
            "CSS Selector",
            "Frame CSS selector",
            VariableType::String,
        )
        .set_default_value(Some(json!("iframe")));
        node
    }
    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.check_cancelled()?;
        context.deactivate_exec_pin("exec_out").await?;
        let mut session: AutomationSession = context.evaluate_pin("session").await?;
        let selector: String = context.evaluate_pin("selector").await?;
        let locator = super::selector::evaluate_locator(context, &selector).await?;
        let driver = session.get_browser_driver_and_switch(context).await?;
        super::selector::find(&driver, &locator)
            .await?
            .enter_frame()
            .await?;
        session.browser_frame_selectors.push(locator);
        context.set_pin_value("session_out", json!(session)).await?;
        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }
    #[cfg(not(feature = "execute"))]
    async fn run(&self, _: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "Browser automation requires the execute feature"
        ))
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct BrowserLeaveFrameNode {}
impl BrowserLeaveFrameNode {
    pub fn new() -> Self {
        Self {}
    }
}
#[async_trait]
impl NodeLogic for BrowserLeaveFrameNode {
    fn get_node(&self) -> Node {
        let mut node = base_node(
            "browser_leave_frame",
            "Leave Frame",
            "Returns to the parent frame or the top-level document.",
        );
        node.set_flowscript_name("browser", "leaveFrame");
        node.add_input_pin(
            "top_level",
            "Top Level",
            "Return to the top-level document",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(false)));
        node
    }
    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.check_cancelled()?;
        context.deactivate_exec_pin("exec_out").await?;
        let mut session: AutomationSession = context.evaluate_pin("session").await?;
        let top_level: bool = context.evaluate_pin("top_level").await?;
        let driver = session.get_browser_driver_and_switch(context).await?;
        if top_level {
            driver.enter_default_frame().await?;
            session.browser_frame_selectors.clear();
        } else {
            driver.enter_parent_frame().await?;
            session.browser_frame_selectors.pop();
        }
        context.set_pin_value("session_out", json!(session)).await?;
        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }
    #[cfg(not(feature = "execute"))]
    async fn run(&self, _: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "Browser automation requires the execute feature"
        ))
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct BrowserHandleDialogNode {}
impl BrowserHandleDialogNode {
    pub fn new() -> Self {
        Self {}
    }
}
#[async_trait]
impl NodeLogic for BrowserHandleDialogNode {
    fn get_node(&self) -> Node {
        let mut node = base_node(
            "browser_handle_dialog",
            "Handle Browser Dialog",
            "Reads, accepts, or dismisses a JavaScript alert, confirm, or prompt.",
        );
        node.set_flowscript_name("browser", "handleDialog");
        node.add_input_pin(
            "action",
            "Action",
            "read, accept, or dismiss",
            VariableType::String,
        )
        .set_default_value(Some(json!("read")));
        node.add_input_pin(
            "text",
            "Prompt Text",
            "Text for a prompt before accepting",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));
        node.add_output_pin(
            "dialog_text",
            "Dialog Text",
            "Text shown in the dialog",
            VariableType::String,
        );
        node
    }
    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.check_cancelled()?;
        context.deactivate_exec_pin("exec_out").await?;
        let session: AutomationSession = context.evaluate_pin("session").await?;
        let action: String = context.evaluate_pin("action").await?;
        let text: String = context.evaluate_pin("text").await?;
        // Switching windows can be blocked by the dialog itself.
        let driver = session.get_browser_driver(context).await?;
        let expected = session.current_window_handle.as_ref().ok_or_else(|| {
            flow_like_types::anyhow!("Select a browser page before handling its dialog")
        })?;
        if driver.window().await?.to_string() != *expected {
            driver
                .switch_to_window(thirtyfour::WindowHandle::from(expected.clone()))
                .await?;
        }
        let message = driver.get_alert_text().await?;
        match action.as_str() {
            "read" => {}
            "accept" => {
                if !text.is_empty() {
                    driver.send_alert_text(text).await?;
                }
                driver.accept_alert().await?;
            }
            "dismiss" => driver.dismiss_alert().await?,
            _ => {
                return Err(flow_like_types::anyhow!(
                    "Dialog action must be read, accept, or dismiss"
                ));
            }
        }
        context.set_pin_value("dialog_text", json!(message)).await?;
        context.set_pin_value("session_out", json!(session)).await?;
        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }
    #[cfg(not(feature = "execute"))]
    async fn run(&self, _: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "Browser automation requires the execute feature"
        ))
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct BrowserAttachNode {}
impl BrowserAttachNode {
    pub fn new() -> Self {
        Self {}
    }
}
#[async_trait]
impl NodeLogic for BrowserAttachNode {
    fn get_node(&self) -> Node {
        let mut node = base_node(
            "browser_attach",
            "Attach to Browser",
            "Attaches ChromeDriver or EdgeDriver to an existing debugging-enabled browser.",
        );
        node.set_flowscript_name("browser", "attach");
        node.add_input_pin(
            "webdriver_url",
            "WebDriver URL",
            "Running ChromeDriver or EdgeDriver URL",
            VariableType::String,
        )
        .set_default_value(Some(json!("http://127.0.0.1:9515")));
        node.add_input_pin(
            "debugger_address",
            "Debugger Address",
            "Existing browser debugging host:port",
            VariableType::String,
        )
        .set_default_value(Some(json!("127.0.0.1:9222")));
        node.add_input_pin(
            "browser_type",
            "Browser",
            "Chrome or Edge",
            VariableType::String,
        )
        .set_default_value(Some(json!("Chrome")));
        node
    }
    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.check_cancelled()?;
        context.deactivate_exec_pin("exec_out").await?;
        let mut session: AutomationSession = context.evaluate_pin("session").await?;
        use thirtyfour::common::capabilities::chromium::ChromiumLikeCapabilities;
        let url: String = context.evaluate_pin("webdriver_url").await?;
        let address: String = context.evaluate_pin("debugger_address").await?;
        let browser: String = context.evaluate_pin("browser_type").await?;
        let (caps, browser_type) = match browser.as_str() {
            "Chrome" => {
                let mut caps = thirtyfour::DesiredCapabilities::chrome();
                caps.set_debugger_address(&address)?;
                (
                    thirtyfour::Capabilities::from(caps),
                    crate::types::handles::BrowserType::Chrome,
                )
            }
            "Edge" => {
                let mut caps = thirtyfour::DesiredCapabilities::edge();
                caps.set_debugger_address(&address)?;
                (
                    thirtyfour::Capabilities::from(caps),
                    crate::types::handles::BrowserType::Edge,
                )
            }
            _ => {
                return Err(flow_like_types::anyhow!(
                    "Attachment supports Chrome and Edge"
                ));
            }
        };
        session.ensure_active(context).await?;
        if session.has_browser() {
            return Err(flow_like_types::anyhow!(
                "Close the attached browser before replacing it"
            ));
        }
        let (driver, _) = super::protocol::connect_webdriver(&url, caps).await?;
        let handle = driver.window().await?;
        let options = crate::types::handles::BrowserContextOptions {
            browser_type,
            headless: false,
            webdriver_url: Some(url),
            ..Default::default()
        };
        session
            .attach_existing_browser(context, driver, &options)
            .await?;
        session.set_current_page(context, handle).await?;
        super::protocol::remember_debugger_address(context, &session, &address).await;
        context.set_pin_value("session_out", json!(session)).await?;
        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }
    #[cfg(not(feature = "execute"))]
    async fn run(&self, _: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "Browser automation requires the execute feature"
        ))
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct BrowserStartDriverNode {}
impl BrowserStartDriverNode {
    pub fn new() -> Self {
        Self {}
    }
}
#[async_trait]
impl NodeLogic for BrowserStartDriverNode {
    fn get_node(&self) -> Node {
        let mut node = base_node(
            "browser_start_driver",
            "Start WebDriver",
            "Starts an installed WebDriver executable and waits until it is ready.",
        );
        node.set_flowscript_name("browser", "startDriver");
        node.add_input_pin(
            "executable",
            "Executable",
            "Path to chromedriver, geckodriver, or msedgedriver",
            VariableType::String,
        )
        .set_default_value(Some(json!("chromedriver")));
        node.add_input_pin(
            "port",
            "Port",
            "Local WebDriver port",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(9515)));
        node.add_output_pin(
            "webdriver_url",
            "WebDriver URL",
            "Ready local WebDriver endpoint",
            VariableType::String,
        );
        node
    }
    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.check_cancelled()?;
        context.deactivate_exec_pin("exec_out").await?;
        let session: AutomationSession = context.evaluate_pin("session").await?;
        let executable: String = context.evaluate_pin("executable").await?;
        let port: i64 = context.evaluate_pin("port").await?;
        session.ensure_active(context).await?;
        if context
            .cache
            .read()
            .await
            .contains_key(&format!("automation:driver:{}", session.session_ref))
        {
            return Err(flow_like_types::anyhow!(
                "Stop this session's WebDriver before starting another"
            ));
        }
        if !(1..=65535).contains(&port) {
            return Err(flow_like_types::anyhow!("Port must be between 1 and 65535"));
        }
        let reservation = std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, port as u16))
            .map_err(|_| flow_like_types::anyhow!("WebDriver port is already in use"))?;
        drop(reservation);
        let mut child = tokio::process::Command::new(executable)
            .arg(format!("--port={port}"))
            .kill_on_drop(true)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()?;
        let url = format!("http://127.0.0.1:{port}");
        let client = flow_like_types::reqwest::Client::builder()
            .timeout(std::time::Duration::from_millis(500))
            .build()?;
        let start = std::time::Instant::now();
        loop {
            context.check_cancelled()?;
            if child.try_wait()?.is_some() {
                return Err(flow_like_types::anyhow!(
                    "WebDriver exited before becoming ready"
                ));
            }
            if let Ok(response) = client.get(format!("{url}/status")).send().await {
                if response.status().is_success() {
                    if let Ok(status) = response.json::<flow_like_types::Value>().await {
                        if status["value"]["ready"] == true {
                            break;
                        }
                    }
                }
            }
            if start.elapsed() > std::time::Duration::from_secs(30) {
                return Err(flow_like_types::anyhow!(
                    "WebDriver did not become ready within 30 seconds"
                ));
            }
            crate::rpa::branch::delay(context, std::time::Duration::from_millis(100)).await?;
        }
        context.cache.write().await.insert(
            format!("automation:driver:{}", session.session_ref),
            std::sync::Arc::new(DriverProcess { child }),
        );
        context.set_pin_value("webdriver_url", json!(url)).await?;
        context.set_pin_value("session_out", json!(session)).await?;
        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }
    #[cfg(not(feature = "execute"))]
    async fn run(&self, _: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "Browser automation requires the execute feature"
        ))
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct BrowserStopDriverNode {}
impl BrowserStopDriverNode {
    pub fn new() -> Self {
        Self {}
    }
}
#[async_trait]
impl NodeLogic for BrowserStopDriverNode {
    fn get_node(&self) -> Node {
        let mut node = base_node(
            "browser_stop_driver",
            "Stop WebDriver",
            "Stops the WebDriver process started for this automation session.",
        );
        node.set_flowscript_name("browser", "stopDriver");

        node
    }
    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.check_cancelled()?;
        context.deactivate_exec_pin("exec_out").await?;
        let mut session: AutomationSession = context.evaluate_pin("session").await?;
        let detached = session.detach_browser(context).await;
        super::protocol::clear_listeners(context, &session).await;
        context
            .cache
            .write()
            .await
            .remove(&format!("automation:driver:{}", session.session_ref));
        context
            .cache
            .write()
            .await
            .remove(&format!("automation:debugger:{}", session.session_ref));
        detached?;
        context.set_pin_value("session_out", json!(session)).await?;
        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }
    #[cfg(not(feature = "execute"))]
    async fn run(&self, _: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "Browser automation requires the execute feature"
        ))
    }
}

#[cfg(feature = "execute")]
struct DriverProcess {
    child: tokio::process::Child,
}
#[cfg(feature = "execute")]
impl flow_like_types::Cacheable for DriverProcess {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}
#[cfg(feature = "execute")]
impl Drop for DriverProcess {
    fn drop(&mut self) {
        let _ = self.child.start_kill();
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct BrowserWaitForUrlNode {}
impl BrowserWaitForUrlNode {
    pub fn new() -> Self {
        Self {}
    }
}
#[async_trait]
impl NodeLogic for BrowserWaitForUrlNode {
    fn get_node(&self) -> Node {
        let mut node = base_node(
            "browser_wait_for_url",
            "Wait For URL",
            "Waits for the current tab to reach an expected URL without navigating again.",
        );
        node.set_flowscript_name("browser", "waitForUrl");
        node.add_input_pin(
            "expected_url",
            "Expected URL",
            "Exact URL after navigation",
            VariableType::String,
        );
        node.add_input_pin(
            "timeout_ms",
            "Timeout",
            "Maximum wait in milliseconds",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(30000)));
        node.add_output_pin(
            "found",
            "Found",
            "Expected URL was reached",
            VariableType::Boolean,
        );
        node.add_output_pin(
            "exec_timeout",
            "Timeout",
            "URL did not match before the deadline",
            VariableType::Execution,
        );
        node
    }
    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.check_cancelled()?;
        context.deactivate_exec_pin("exec_out").await?;
        let mut session: AutomationSession = context.evaluate_pin("session").await?;
        session.browser_frame_selectors.clear();
        let expected: String = context.evaluate_pin("expected_url").await?;
        let timeout: i64 = context.evaluate_pin("timeout_ms").await?;
        if timeout < 0 {
            return Err(flow_like_types::anyhow!("Timeout must be nonnegative"));
        }
        context.deactivate_exec_pin("exec_timeout").await?;
        let driver = session.get_browser_driver_and_switch(context).await?;
        let start = std::time::Instant::now();
        loop {
            context.check_cancelled()?;
            if driver.current_url().await?.as_str() == expected {
                break;
            }
            if start.elapsed() >= std::time::Duration::from_millis(timeout as u64) {
                context.set_pin_value("found", json!(false)).await?;
                context.set_pin_value("session_out", json!(session)).await?;
                context.activate_exec_pin("exec_timeout").await?;
                return Ok(());
            }
            crate::rpa::branch::delay(context, std::time::Duration::from_millis(100)).await?;
        }
        context.set_pin_value("found", json!(true)).await?;
        context.set_pin_value("session_out", json!(session)).await?;
        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }
    #[cfg(not(feature = "execute"))]
    async fn run(&self, _: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "Browser automation requires the execute feature"
        ))
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct BrowserStartConsoleObserverNode {}
impl BrowserStartConsoleObserverNode {
    pub fn new() -> Self {
        Self {}
    }
}
#[async_trait]
impl NodeLogic for BrowserStartConsoleObserverNode {
    fn get_node(&self) -> Node {
        let mut node = base_node(
            "browser_start_console_observer",
            "Start Console Observer",
            "Captures browser console messages before navigation or actions.",
        );
        node.set_flowscript_name("browser", "startConsoleObserver");
        node.add_input_pin(
            "debugger_address",
            "Debugger Address",
            "Chrome or Edge debugging address (host:port)",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));
        node
    }
    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.check_cancelled()?;
        context.deactivate_exec_pin("exec_out").await?;
        let session: AutomationSession = context.evaluate_pin("session").await?;
        let address: String = context.evaluate_pin("debugger_address").await?;
        let driver = session.get_browser_driver_and_switch(context).await?;
        super::protocol::start_listener(context, &session, &driver, &address, None).await?;
        context.set_pin_value("session_out", json!(session)).await?;
        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }
    #[cfg(not(feature = "execute"))]
    async fn run(&self, _: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "Browser automation requires the execute feature"
        ))
    }
}
