use crate::types::handles::AutomationSession;
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json};

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
        let driver = session.get_browser_driver(context).await?;

        let window_handle = driver
            .new_tab()
            .await
            .map_err(|e| flow_like_types::anyhow!("Failed to create new tab: {}", e))?;

        session.set_current_page(context, window_handle).await?;

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
        let driver = session.get_browser_driver_and_switch(context).await?;
        let remaining: Vec<thirtyfour::WindowHandle> = driver
            .handle
            .cmd(thirtyfour::common::command::Command::CloseWindow)
            .await?
            .value()?;
        if let Some(handle) = remaining.first() {
            driver.switch_to_window(handle.clone()).await?;
            session.set_current_page(context, handle.clone()).await?;
        } else {
            drop(driver);
            session.detach_browser(context).await?;
            super::protocol::clear_listeners(context, &session).await;
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
