use crate::types::handles::AutomationSession;
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json};

#[crate::register_node]
#[derive(Default)]
pub struct BrowserWaitForNode {}

impl BrowserWaitForNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for BrowserWaitForNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "browser_wait_for",
            "Wait For Selector",
            "Waits for an element matching the selector to appear in the DOM",
            "Automation/Browser/Wait",
        );
        node.set_flowscript_name("browser", "waitFor");
        node.add_icon("/flow/icons/browser.svg");

        node.set_scores(
            flow_like::flow::node::NodeScores::new()
                .set_privacy(4)
                .set_security(5)
                .set_performance(7)
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

        node.add_input_pin(
            "selector",
            "Selector",
            "CSS selector to wait for",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_input_pin(
            "timeout_ms",
            "Timeout (ms)",
            "Maximum time to wait",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(30000)));

        node.add_output_pin("exec_out", "▶", "Continue", VariableType::Execution);

        node.add_output_pin(
            "session_out",
            "Session",
            "Automation session (pass-through)",
            VariableType::Struct,
        )
        .set_schema::<AutomationSession>();

        node.add_output_pin(
            "found",
            "Found",
            "Whether the element was found within timeout",
            VariableType::Boolean,
        );

        super::selector::add_locator_pin(&mut node);
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        use std::time::Duration;

        context.deactivate_exec_pin("exec_out").await?;

        let session: AutomationSession = context.evaluate_pin("session").await?;
        let selector: String = context.evaluate_pin("selector").await?;
        let locator = super::selector::evaluate_locator(context, &selector).await?;
        let timeout_ms: i64 = context.evaluate_pin("timeout_ms").await?;
        if timeout_ms < 0 {
            return Err(flow_like_types::anyhow!("Wait timeout must be nonnegative"));
        }

        let driver = session.get_browser_driver_and_switch(context).await?;

        let token = context.get_cancellation_token();
        let operation = tokio::time::timeout(Duration::from_millis(timeout_ms as u64), async {
            loop {
                context.check_cancelled()?;
                match super::selector::find(&driver, &locator).await {
                    Ok(_) => return Ok::<(), flow_like_types::Error>(()),
                    Err(error) if error.downcast_ref::<thirtyfour::error::WebDriverError>().is_some_and(|error| matches!(error.as_inner(), thirtyfour::error::WebDriverErrorInner::NoSuchElement(_) | thirtyfour::error::WebDriverErrorInner::StaleElementReference(_))) => {}
                    Err(error) => return Err(error),
                }
                crate::rpa::branch::delay(context, Duration::from_millis(100)).await?;
            }
        });
        let result = tokio::select! {
            biased;
            _ = async { if let Some(token) = token { token.cancelled().await } else { std::future::pending::<()>().await } } => return Err(flow_like_types::anyhow!("Execution was cancelled")),
            result = operation => result,
        };

        let found = match result {
            Ok(result) => {
                result?;
                true
            }
            Err(_) => false,
        };

        context.set_pin_value("session_out", json!(session)).await?;
        context.set_pin_value("found", json!(found)).await?;
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
pub struct BrowserWaitForDelayNode {}

impl BrowserWaitForDelayNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for BrowserWaitForDelayNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "browser_wait_delay",
            "Wait Delay",
            "Waits for a specified amount of time",
            "Automation/Browser/Wait",
        );
        node.set_flowscript_name("browser", "waitDelay");
        node.add_icon("/flow/icons/browser.svg");

        node.set_scores(
            flow_like::flow::node::NodeScores::new()
                .set_privacy(10)
                .set_security(10)
                .set_performance(5)
                .set_governance(10)
                .set_reliability(10)
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

        node.add_input_pin(
            "delay_ms",
            "Delay (ms)",
            "Time to wait in milliseconds",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(1000)));

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
        use std::time::Duration;

        context.deactivate_exec_pin("exec_out").await?;

        let session: AutomationSession = context.evaluate_pin("session").await?;
        let delay_ms: i64 = context.evaluate_pin("delay_ms").await?;
        if delay_ms < 0 {
            return Err(flow_like_types::anyhow!("Delay must be nonnegative"));
        }
        session.ensure_active(context).await?;
        crate::rpa::branch::delay(context, Duration::from_millis(delay_ms as u64)).await?;

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
