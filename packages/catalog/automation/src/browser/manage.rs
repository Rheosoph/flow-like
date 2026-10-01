use crate::types::handles::AutomationSession;
#[cfg(feature = "execute")]
use crate::types::handles::{BrowserContextOptions, BrowserType, UnclaimedBrowser};
#[cfg(feature = "execute")]
use flow_like::flow::execution::LogLevel;
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::ValueType,
    variable::VariableType,
};
#[cfg(feature = "execute")]
use flow_like_browser::{
    Browser, BrowserError, Dialog, DialogAction, Page,
    attach::AttachEndpoint,
    launch::BrowserKind,
    types::{DialogType, TargetId},
};
#[cfg(feature = "execute")]
use flow_like_types::tokio_util::sync::CancellationToken;
use flow_like_types::{async_trait, json::json};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
#[cfg(feature = "execute")]
use std::time::Duration;

#[cfg(feature = "execute")]
const TAB_TITLE_BUDGET: Duration = Duration::from_secs(2);
#[cfg(feature = "execute")]
const TAB_URL_WAIT: Duration = Duration::from_secs(10);
#[cfg(feature = "execute")]
const POLL_INTERVAL: Duration = Duration::from_millis(100);
#[cfg(feature = "execute")]
const ATTACHED_PAGE_LOAD_TIMEOUT: Duration = Duration::from_secs(300);

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
        let page = session.browser_top_page(context).await?;
        let tabs: Vec<BrowserTab> = page
            .browser
            .list_pages(TAB_TITLE_BUDGET)
            .await
            .into_iter()
            .map(|info| BrowserTab {
                handle: info.target_id.to_string(),
                title: info.title,
                url: info.url,
            })
            .collect();
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
        let target = {
            let guard = session.browser_guard(context).await?;
            let target = if !target_id.is_empty() {
                open_tab(&guard.browser, &target_id).ok_or_else(|| {
                    flow_like_types::anyhow!("Recorded browser tab is no longer open")
                })?
            } else if handle.is_empty() {
                let url: String = context.evaluate_pin("url").await?;
                tab_with_url(context, &guard.browser, &url).await?
            } else {
                open_tab(&guard.browser, &handle).ok_or_else(|| {
                    flow_like_types::anyhow!(
                        "Browser tab '{handle}' is no longer open; use List Tabs for the current handles"
                    )
                })?
            };
            guard.browser.activate(&target).await?;
            target
        };
        session.set_current_page_target(context, &target).await?;
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
fn open_tab(browser: &Browser, id: &str) -> Option<TargetId> {
    let id = id.strip_prefix("CDwindow-").unwrap_or(id);
    browser
        .pages()
        .into_iter()
        .find(|page| page.target_id.as_str() == id)
        .map(|page| page.target_id)
}

/// A popup still shows about:blank or its pre-redirect URL right after the click that opened it.
#[cfg(feature = "execute")]
async fn tab_with_url(
    context: &ExecutionContext,
    browser: &Browser,
    url: &str,
) -> flow_like_types::Result<TargetId> {
    if url.is_empty() {
        return Err(flow_like_types::anyhow!(
            "Provide a tab handle or exact URL"
        ));
    }
    let deadline =
        tokio::time::Instant::now() + browser.settings().page_load_timeout().min(TAB_URL_WAIT);
    loop {
        let matches: Vec<TargetId> = browser
            .pages()
            .into_iter()
            .filter(|page| page.url == url)
            .map(|page| page.target_id)
            .collect();
        if let [target] = matches.as_slice() {
            return Ok(target.clone());
        }
        if !matches.is_empty() || tokio::time::Instant::now() >= deadline {
            return Err(flow_like_types::anyhow!(
                "Expected one tab with this URL; found {}",
                matches.len()
            ));
        }
        crate::rpa::branch::delay(context, POLL_INTERVAL).await?;
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
        let page = session.browser_page(context).await?;
        let owner = super::selector::find_element(&page, &locator).await?;
        owner.frame().child_frame(&owner).await?;
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
        drop(session.browser_page(context).await?);
        if top_level {
            session.browser_frame_selectors.clear();
        } else {
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
        let page = session.browser_top_page(context).await?;
        let dialog = match action.as_str() {
            "read" => page.page.pending_dialog().ok_or(BrowserError::NoDialog)?,
            "accept" => accept_dialog(&page.page, text).await?,
            "dismiss" => page.page.handle_dialog(DialogAction::Dismiss).await?,
            _ => {
                return Err(flow_like_types::anyhow!(
                    "Dialog action must be read, accept, or dismiss"
                ));
            }
        };
        context
            .set_pin_value("dialog_text", json!(dialog.message))
            .await?;
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

/// Without text a prompt is accepted with its default value, as chromedriver does.
#[cfg(feature = "execute")]
async fn accept_dialog(page: &Page, text: String) -> flow_like_types::Result<Dialog> {
    let prompt_text = if text.is_empty() {
        page.pending_dialog()
            .filter(|dialog| matches!(dialog.kind, DialogType::Prompt))
            .map(|dialog| dialog.default_prompt)
    } else {
        require_prompt(page)?;
        Some(text)
    };
    Ok(page
        .handle_dialog(DialogAction::Accept { prompt_text })
        .await?)
}

/// Prompt text on an alert or confirm fails and leaves the dialog open, as in chromedriver.
#[cfg(feature = "execute")]
fn require_prompt(page: &Page) -> flow_like_types::Result<()> {
    match page.pending_dialog() {
        None => Err(BrowserError::NoDialog.into()),
        Some(dialog) if matches!(dialog.kind, DialogType::Prompt) => Ok(()),
        Some(_) => Err(BrowserError::NotInteractable {
            message: "User dialog does not have a text box input field.".to_owned(),
        }
        .into()),
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
            "Attaches to a running Chrome or Edge over the DevTools protocol: a dedicated debugging browser (host:port or ws:// URL), or your everyday browser after you allow remote debugging (leave Debugger Address empty).",
        );
        node.set_flowscript_name("browser", "attach");
        node.add_input_pin(
            "webdriver_url",
            "WebDriver URL",
            "Legacy WebDriver URL; ignored for loopback addresses",
            VariableType::String,
        )
        .set_default_value(Some(json!("http://127.0.0.1:9515")));
        node.add_input_pin(
            "debugger_address",
            "Debugger Address",
            "DevTools address of a browser started with --remote-debugging-port (host:port, http:// or ws:// URL). Leave empty to attach to your everyday Chrome or Edge via chrome://inspect/#remote-debugging",
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
        let url: String = context.evaluate_pin("webdriver_url").await?;
        let address: String = context.evaluate_pin("debugger_address").await?;
        let browser_name: String = context.evaluate_pin("browser_type").await?;
        let (kind, browser_type) = match browser_name.as_str() {
            "Chrome" => (BrowserKind::Chrome, BrowserType::Chrome),
            "Edge" => (BrowserKind::Edge, BrowserType::Edge),
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
        super::context::check_webdriver_url(&url).await?;
        session.prepare_browser_slot(context).await?;
        let endpoint = flow_like_browser::attach::resolve_endpoint(&address, kind).await?;
        if endpoint.needs_approval() {
            context.log_message(
                &format!(
                    "Waiting for you to click Allow in {browser_name} (remote debugging consent)…"
                ),
                LogLevel::Info,
            );
        }
        let browser = attach_until_cancelled(context, endpoint).await?;
        browser
            .settings()
            .set_page_load_timeout(ATTACHED_PAGE_LOAD_TIMEOUT);
        let options = BrowserContextOptions {
            browser_type,
            headless: false,
            webdriver_url: Some(url),
            ..Default::default()
        };
        session
            .attach_cdp_browser(context, browser, &options)
            .await?;
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
async fn attach_until_cancelled(
    context: &ExecutionContext,
    endpoint: AttachEndpoint,
) -> flow_like_types::Result<Browser> {
    until_attached(context.get_cancellation_token(), Browser::attach(endpoint)).await
}

/// Stop does not wait for the remote debugging consent. The attach keeps running on its own
/// task, because dropping it half way would leave the DevTools connection open. The task hands
/// out an unclaimed browser: when the node was cancelled, or dropped by the run loop or a branch
/// timeout, nobody claims it and the browser is disconnected as soon as the attach finishes.
#[cfg(feature = "execute")]
async fn until_attached(
    token: Option<CancellationToken>,
    attach: impl Future<Output = flow_like_browser::Result<Browser>> + Send + 'static,
) -> flow_like_types::Result<Browser> {
    let mut attaching = tokio::spawn(async move { attach.await.map(UnclaimedBrowser::new) });
    let Some(token) = token else {
        return Ok(attaching.await??.claim());
    };
    tokio::select! {
        biased;
        attached = &mut attaching => Ok(attached??.claim()),
        _ = token.cancelled() => Err(flow_like_types::anyhow!("Execution was cancelled")),
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
            "Legacy node kept for existing boards: browsers are now launched directly, so no WebDriver is started. Outputs a local URL for compatibility.",
        );
        node.set_flowscript_name("browser", "startDriver");
        node.add_input_pin(
            "executable",
            "Executable",
            "Ignored (legacy)",
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
            "Local compatibility URL (ignored by Open Browser)",
            VariableType::String,
        );
        node
    }
    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.check_cancelled()?;
        context.deactivate_exec_pin("exec_out").await?;
        let session: AutomationSession = context.evaluate_pin("session").await?;
        let port: i64 = context.evaluate_pin("port").await?;
        session.ensure_active(context).await?;
        if !(1..=65535).contains(&port) {
            return Err(flow_like_types::anyhow!("Port must be between 1 and 65535"));
        }
        context.log_message(
            "No WebDriver is needed any more; this node is kept for existing boards",
            LogLevel::Info,
        );
        context
            .set_pin_value("webdriver_url", json!(format!("http://127.0.0.1:{port}")))
            .await?;
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
            "Legacy node kept for existing boards: closes the session's browser like Close Browser.",
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
        let page = session.browser_top_page(context).await?;
        let start = std::time::Instant::now();
        loop {
            context.check_cancelled()?;
            if url_matches(&page, &expected).await? {
                break;
            }
            if start.elapsed() >= Duration::from_millis(timeout as u64) {
                context.set_pin_value("found", json!(false)).await?;
                context.set_pin_value("session_out", json!(session)).await?;
                context.activate_exec_pin("exec_timeout").await?;
                return Ok(());
            }
            crate::rpa::branch::delay(context, POLL_INTERVAL).await?;
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

#[cfg(feature = "execute")]
async fn url_matches(
    page: &super::driver::PageContext,
    expected: &str,
) -> flow_like_types::Result<bool> {
    match page.page.url().await {
        Ok(url) => Ok(url == expected),
        Err(error) => {
            let error = flow_like_types::Error::from(error);
            if super::driver::is_navigation_interrupted(&error) {
                Ok(false)
            } else {
                Err(error)
            }
        }
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
            "Ignored (legacy); the observer uses the session's own browser connection",
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
        context.evaluate_pin::<String>("debugger_address").await?;
        let page = session.browser_page(context).await?;
        super::protocol::start_observer(context, &session, &page).await?;
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

#[cfg(all(test, feature = "execute"))]
mod tests {
    use super::*;
    use crate::browser::page::tests::{dismiss_dialog, show_dialog, show_prompt};
    use crate::types::handles::tests::{Scripted, scripted};
    use flow_like_browser::testing::{PageHarness, default_auto_reply};
    use tokio::sync::oneshot;

    const DISCONNECT_WAIT: Duration = Duration::from_secs(5);

    async fn answering_harness() -> PageHarness {
        let harness = PageHarness::new().await;
        harness.control.set_auto_reply(|command| {
            (command.method == "Page.handleJavaScriptDialog")
                .then(|| json!({}))
                .or_else(|| default_auto_reply(command))
        });
        harness
    }

    async fn accept_error(harness: &PageHarness) -> String {
        accept_dialog(&harness.page, "Ada".to_owned())
            .await
            .expect_err("prompt text needs an open prompt")
            .to_string()
    }

    fn answered(harness: &PageHarness) -> Vec<flow_like_types::Value> {
        harness
            .control
            .commands_seen()
            .into_iter()
            .filter(|command| command.method == "Page.handleJavaScriptDialog")
            .map(|command| command.params)
            .collect()
    }

    #[tokio::test]
    async fn prompt_text_is_only_sent_to_an_open_prompt() {
        let harness = answering_harness().await;
        assert_eq!(
            accept_error(&harness).await,
            "no such alert: no dialog is open on this page"
        );
        for kind in ["alert", "confirm"] {
            show_dialog(&harness, kind, "Continue?").await;
            assert_eq!(
                accept_error(&harness).await,
                "element not interactable: User dialog does not have a text box input field.",
                "{kind}"
            );
            assert!(harness.page.pending_dialog().is_some(), "{kind} stays open");
            dismiss_dialog(&harness).await;
        }
        assert!(answered(&harness).is_empty());

        show_dialog(&harness, "alert", "Saved").await;
        let alert = accept_dialog(&harness.page, String::new()).await.unwrap();
        assert_eq!(alert.message, "Saved");
        show_dialog(&harness, "prompt", "Your name?").await;
        let prompt = accept_dialog(&harness.page, "Ada".to_owned())
            .await
            .unwrap();
        assert_eq!(prompt.message, "Your name?");
        assert!(harness.page.pending_dialog().is_none());
        assert_eq!(
            answered(&harness),
            vec![
                json!({"accept": true}),
                json!({"accept": true, "promptText": "Ada"})
            ]
        );
    }

    #[tokio::test]
    async fn accepting_a_prompt_without_text_submits_its_default() {
        let harness = answering_harness().await;
        show_prompt(&harness, "Save as:", "report.csv").await;
        let prompt = accept_dialog(&harness.page, String::new()).await.unwrap();
        assert_eq!(prompt.default_prompt, "report.csv");
        show_dialog(&harness, "confirm", "Overwrite?").await;
        accept_dialog(&harness.page, String::new()).await.unwrap();
        assert_eq!(
            answered(&harness),
            vec![
                json!({"accept": true, "promptText": "report.csv"}),
                json!({"accept": true})
            ]
        );
    }

    /// An attach that connects once `gate` opens and reports the scripted browser it made.
    async fn gated_attach(
        gate: oneshot::Receiver<()>,
        report: oneshot::Sender<Scripted>,
    ) -> flow_like_browser::Result<Browser> {
        gate.await.expect("the test opens the gate");
        let attached = scripted().await;
        let browser = attached.browser.clone();
        report
            .send(attached)
            .unwrap_or_else(|_| panic!("the test waits for the attached browser"));
        Ok(browser)
    }

    async fn disconnects(browser: &Browser) -> bool {
        tokio::time::timeout(DISCONNECT_WAIT, browser.connection().closed())
            .await
            .is_ok()
    }

    #[tokio::test]
    async fn a_browser_attached_after_the_node_was_dropped_is_disconnected() {
        let (release, gate) = oneshot::channel();
        let (report, attached) = oneshot::channel();
        let mut node = Box::pin(until_attached(
            Some(CancellationToken::new()),
            gated_attach(gate, report),
        ));
        assert!(futures::poll!(node.as_mut()).is_pending());
        drop(node);
        release.send(()).unwrap();
        let late = attached.await.expect("the attach outlives the node");
        assert!(
            disconnects(&late.browser).await,
            "no node owns the late browser, so its DevTools connection must close"
        );
    }

    #[tokio::test]
    async fn a_browser_attached_after_the_cancel_is_disconnected() {
        let (release, gate) = oneshot::channel();
        let (report, attached) = oneshot::channel();
        let token = CancellationToken::new();
        let node = until_attached(Some(token.clone()), gated_attach(gate, report));
        token.cancel();
        assert_eq!(
            node.await.err().expect("Stop ends the node").to_string(),
            "Execution was cancelled"
        );
        release.send(()).unwrap();
        let late = attached.await.expect("the attach outlives the node");
        assert!(disconnects(&late.browser).await);
    }

    #[tokio::test]
    async fn an_attached_browser_is_handed_to_the_node_connected() {
        for token in [None, Some(CancellationToken::new())] {
            let (release, gate) = oneshot::channel();
            let (report, attached) = oneshot::channel();
            release.send(()).unwrap();
            let browser = until_attached(token, gated_attach(gate, report))
                .await
                .expect("the attach succeeds");
            let _attached = attached.await.expect("the attach reported its browser");
            tokio::task::yield_now().await;
            assert!(browser.is_alive());
        }
    }
}
