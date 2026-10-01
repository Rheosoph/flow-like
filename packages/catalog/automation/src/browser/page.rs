#[cfg(feature = "execute")]
use super::driver::PageContext;
use crate::types::handles::AutomationSession;
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    variable::VariableType,
};
#[cfg(feature = "execute")]
use flow_like_browser::{Browser, BrowserError, ClosePageOutcome, Page, types::TargetId};
use flow_like_types::async_trait;
#[cfg(feature = "execute")]
use flow_like_types::json::json;

#[crate::register_node]
#[derive(Default)]
pub struct BrowserNewPageNode {}

impl BrowserNewPageNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for BrowserNewPageNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "browser_new_page",
            "New Page",
            "Creates a new browser page/tab in the given context",
            "Automation/Browser",
        );
        node.set_flowscript_name("browser", "newPage");
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
            "Automation session with browser attached",
            VariableType::Struct,
        )
        .set_schema::<AutomationSession>();

        node.add_output_pin("exec_out", "▶", "Continue", VariableType::Execution);

        node.add_output_pin(
            "session_out",
            "Session",
            "Automation session with new page set as current",
            VariableType::Struct,
        )
        .set_schema::<AutomationSession>();

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let mut session: AutomationSession = context.evaluate_pin("session").await?;
        let target = open_tab(&session.browser_guard(context).await?.browser).await?;
        session.set_current_page_target(context, &target).await?;

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

#[cfg(feature = "execute")]
async fn open_tab(browser: &Browser) -> flow_like_types::Result<TargetId> {
    let page = browser
        .new_page()
        .await
        .map_err(|e| flow_like_types::anyhow!("Failed to create new tab: {}", e))?;
    let target = page.target_id().clone();
    browser.activate(&target).await?;
    Ok(target)
}

#[cfg(feature = "execute")]
async fn close_tab(page: &PageContext) -> flow_like_types::Result<ClosePageOutcome> {
    refuse_open_dialog(&page.page)?;
    let outcome = page.browser.close_page(page.page.target_id()).await?;
    if let ClosePageOutcome::Remaining { next } = &outcome {
        page.browser.activate(next).await?;
    }
    Ok(outcome)
}

/// A tab with an open JavaScript dialog stays open, as in chromedriver.
#[cfg(feature = "execute")]
fn refuse_open_dialog(page: &Page) -> flow_like_types::Result<()> {
    match page.pending_dialog() {
        Some(dialog) => Err(BrowserError::DialogOpen {
            kind: dialog.kind,
            message: dialog.message,
        }
        .into()),
        None => Ok(()),
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct BrowserClosePageNode {}

impl BrowserClosePageNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for BrowserClosePageNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "browser_close_page",
            "Close Page",
            "Closes a browser page/tab",
            "Automation/Browser",
        );
        node.set_version(1);
        node.set_flowscript_name("browser", "closePage");
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
            "Automation session with page to close",
            VariableType::Struct,
        )
        .set_schema::<AutomationSession>();

        node.add_output_pin("exec_out", "▶", "Continue", VariableType::Execution);

        node.add_output_pin(
            "session_out",
            "Session",
            "Session selecting a remaining tab when available",
            VariableType::Struct,
        )
        .set_schema::<AutomationSession>();
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let mut session: AutomationSession = context.evaluate_pin("session").await?;
        session.browser_frame_selectors.clear();
        let outcome = close_tab(&*session.browser_top_page(context).await?).await?;
        match outcome {
            ClosePageOutcome::Remaining { next } => {
                session.set_current_page_target(context, &next).await?;
            }
            ClosePageOutcome::LastPageClosed { .. } => {
                session.detach_browser(context).await?;
                super::protocol::clear_listeners(context, &session).await;
            }
        }
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

#[cfg(all(test, feature = "execute"))]
pub(super) mod tests {
    use super::*;
    use flow_like_browser::testing::PageHarness;
    use std::time::Duration;

    pub(in crate::browser) async fn show_dialog(harness: &PageHarness, kind: &str, message: &str) {
        open_dialog(harness, kind, message, "").await;
    }

    pub(in crate::browser) async fn show_prompt(
        harness: &PageHarness,
        message: &str,
        default_prompt: &str,
    ) {
        open_dialog(harness, "prompt", message, default_prompt).await;
    }

    async fn open_dialog(harness: &PageHarness, kind: &str, message: &str, default_prompt: &str) {
        harness.emit(
            "Page.javascriptDialogOpening",
            json!({"url": "http://127.0.0.1/", "frameId": "T1", "message": message, "type": kind, "defaultPrompt": default_prompt}),
        );
        wait_for_dialog(harness, true).await;
    }

    pub(in crate::browser) async fn dismiss_dialog(harness: &PageHarness) {
        harness.emit("Page.javascriptDialogClosed", json!({"result": false}));
        wait_for_dialog(harness, false).await;
    }

    async fn wait_for_dialog(harness: &PageHarness, open: bool) {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
        while harness.page.pending_dialog().is_some() != open {
            assert!(
                tokio::time::Instant::now() < deadline,
                "the dialog event never reached the page"
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    #[tokio::test]
    async fn a_tab_with_an_open_dialog_is_not_closed() {
        let harness = PageHarness::new().await;
        assert!(refuse_open_dialog(&harness.page).is_ok());
        show_dialog(&harness, "alert", "Unsaved changes").await;
        assert_eq!(
            refuse_open_dialog(&harness.page)
                .expect_err("an open dialog blocks Close Page")
                .to_string(),
            "unexpected alert open: {Alert text : Unsaved changes}"
        );
        dismiss_dialog(&harness).await;
        assert!(refuse_open_dialog(&harness.page).is_ok());
    }
}
