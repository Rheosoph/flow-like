use crate::types::handles::AutomationSession;
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json};

#[crate::register_node]
#[derive(Default)]
pub struct BrowserGotoNode {}

impl BrowserGotoNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for BrowserGotoNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "browser_goto",
            "Go To URL",
            "Navigates the page to a URL. file:, javascript:, chrome:, edge:, devtools: and view-source: URLs are blocked unless Set Navigation Policy allows them; a session navigation policy is enforced before loading and again on the final URL after redirects.",
            "Automation/Browser/Navigation",
        );
        node.set_version(2);
        node.set_flowscript_name("browser", "goto");
        node.add_icon("/flow/icons/browser.svg");

        node.set_scores(
            flow_like::flow::node::NodeScores::new()
                .set_privacy(4)
                .set_security(5)
                .set_performance(7)
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
            "Automation session",
            VariableType::Struct,
        )
        .set_schema::<AutomationSession>();

        node.add_input_pin("url", "URL", "URL to navigate to", VariableType::String)
            .set_default_value(Some(json!("")));

        node.add_output_pin("exec_out", "▶", "Continue", VariableType::Execution);

        node.add_output_pin(
            "session_out",
            "Session",
            "Automation session (pass-through)",
            VariableType::Struct,
        )
        .set_schema::<AutomationSession>();

        node.add_output_pin(
            "final_url",
            "Final URL",
            "The actual URL after navigation (may differ due to redirects)",
            VariableType::String,
        );

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let mut session: AutomationSession = context.evaluate_pin("session").await?;
        session.browser_frame_selectors.clear();
        let url: String = context.evaluate_pin("url").await?;
        let policy = super::policy::session_policy(
            context,
            &session,
            super::policy::NavigationPolicy::goto_default(),
        )
        .await;
        let target = policy.check(&url).await?;

        let driver = session.get_browser_driver_and_switch(context).await?;

        driver
            .goto(target.as_str())
            .await
            .map_err(|e| flow_like_types::anyhow!("Failed to navigate to '{}': {}", url, e))?;

        let final_url = super::policy::verify_landing(&driver, &policy, &url).await?;

        context.set_pin_value("session_out", json!(session)).await?;
        context
            .set_pin_value("final_url", json!(final_url))
            .await?;
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
pub struct BrowserBackNode {}

impl BrowserBackNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for BrowserBackNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "browser_back",
            "Go Back",
            "Navigates back in browser history",
            "Automation/Browser/Navigation",
        );
        node.set_flowscript_name("browser", "back");
        node.add_icon("/flow/icons/browser.svg");

        node.set_scores(
            flow_like::flow::node::NodeScores::new()
                .set_privacy(4)
                .set_security(5)
                .set_performance(8)
                .set_governance(6)
                .set_reliability(8)
                .set_cost(10)
                .build(),
        );
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
            "Automation session (pass-through)",
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

        driver
            .back()
            .await
            .map_err(|e| flow_like_types::anyhow!("Failed to go back: {}", e))?;

        context.set_pin_value("session_out", json!(session)).await?;
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
pub struct BrowserForwardNode {}

impl BrowserForwardNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for BrowserForwardNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "browser_forward",
            "Go Forward",
            "Navigates forward in browser history",
            "Automation/Browser/Navigation",
        );
        node.set_flowscript_name("browser", "forward");
        node.add_icon("/flow/icons/browser.svg");

        node.set_scores(
            flow_like::flow::node::NodeScores::new()
                .set_privacy(4)
                .set_security(5)
                .set_performance(8)
                .set_governance(6)
                .set_reliability(8)
                .set_cost(10)
                .build(),
        );
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
            "Automation session (pass-through)",
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

        driver
            .forward()
            .await
            .map_err(|e| flow_like_types::anyhow!("Failed to go forward: {}", e))?;

        context.set_pin_value("session_out", json!(session)).await?;
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
pub struct BrowserReloadNode {}

impl BrowserReloadNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for BrowserReloadNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "browser_reload",
            "Reload",
            "Reloads the current page",
            "Automation/Browser/Navigation",
        );
        node.set_flowscript_name("browser", "reload");
        node.add_icon("/flow/icons/browser.svg");

        node.set_scores(
            flow_like::flow::node::NodeScores::new()
                .set_privacy(4)
                .set_security(5)
                .set_performance(8)
                .set_governance(6)
                .set_reliability(8)
                .set_cost(10)
                .build(),
        );
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
            "Automation session (pass-through)",
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

        driver
            .refresh()
            .await
            .map_err(|e| flow_like_types::anyhow!("Failed to reload page: {}", e))?;

        context.set_pin_value("session_out", json!(session)).await?;
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
