use crate::types::handles::AutomationSession;
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    variable::VariableType,
};
use flow_like_types::{Value, async_trait, json::json};

#[crate::register_node]
#[derive(Default)]
pub struct BrowserGetTextNode {}

impl BrowserGetTextNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for BrowserGetTextNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "browser_get_text",
            "Get Text",
            "Gets the text content of an element",
            "Automation/Browser/Extract",
        );
        node.add_icon("/flow/icons/browser.svg");

        node.set_scores(
            flow_like::flow::node::NodeScores::new()
                .set_privacy(4)
                .set_security(5)
                .set_performance(9)
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
            "CSS selector of element",
            VariableType::String,
        )
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
            "text",
            "Text",
            "Text content of the element",
            VariableType::String,
        );

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        use thirtyfour::prelude::*;

        context.deactivate_exec_pin("exec_out").await?;

        let session: AutomationSession = context.evaluate_pin("session").await?;
        let selector: String = context.evaluate_pin("selector").await?;

        let driver = session.get_browser_driver_and_switch(context).await?;

        let element = driver.find(By::Css(&selector)).await.map_err(|e| {
            flow_like_types::anyhow!("Failed to find element '{}': {}", selector, e)
        })?;

        let text = element
            .text()
            .await
            .map_err(|e| flow_like_types::anyhow!("Failed to get element text: {}", e))?;

        context.set_pin_value("session_out", json!(session)).await?;
        context.set_pin_value("text", json!(text)).await?;
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
pub struct BrowserGetAttributeNode {}

impl BrowserGetAttributeNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for BrowserGetAttributeNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "browser_get_attribute",
            "Get Attribute",
            "Gets an attribute value of an element",
            "Automation/Browser/Extract",
        );
        node.add_icon("/flow/icons/browser.svg");

        node.set_scores(
            flow_like::flow::node::NodeScores::new()
                .set_privacy(4)
                .set_security(5)
                .set_performance(9)
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
            "CSS selector of element",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_input_pin(
            "attribute",
            "Attribute",
            "Name of attribute to get",
            VariableType::String,
        )
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
            "value",
            "Value",
            "Attribute value (empty if not found)",
            VariableType::String,
        );

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        use thirtyfour::prelude::*;

        context.deactivate_exec_pin("exec_out").await?;

        let session: AutomationSession = context.evaluate_pin("session").await?;
        let selector: String = context.evaluate_pin("selector").await?;
        let attribute: String = context.evaluate_pin("attribute").await?;

        let driver = session.get_browser_driver_and_switch(context).await?;

        let element = driver.find(By::Css(&selector)).await.map_err(|e| {
            flow_like_types::anyhow!("Failed to find element '{}': {}", selector, e)
        })?;

        let value = element.attr(&attribute).await.map_err(|e| {
            flow_like_types::anyhow!("Failed to get attribute '{}': {}", attribute, e)
        })?;

        context.set_pin_value("session_out", json!(session)).await?;
        context
            .set_pin_value("value", json!(value.unwrap_or_default()))
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
pub struct BrowserGetHtmlNode {}

impl BrowserGetHtmlNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for BrowserGetHtmlNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "browser_get_html",
            "Get HTML",
            "Gets the HTML content of an element or the entire page",
            "Automation/Browser/Extract",
        );
        node.add_icon("/flow/icons/browser.svg");

        node.set_scores(
            flow_like::flow::node::NodeScores::new()
                .set_privacy(3)
                .set_security(4)
                .set_performance(8)
                .set_governance(5)
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
            "CSS selector of element (empty for entire page)",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_input_pin(
            "outer_html",
            "Outer HTML",
            "Include element's own tags (vs just inner content)",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(true)));

        node.add_output_pin("exec_out", "▶", "Continue", VariableType::Execution);

        node.add_output_pin(
            "session_out",
            "Session",
            "Automation session (pass-through)",
            VariableType::Struct,
        )
        .set_schema::<AutomationSession>();

        node.add_output_pin("html", "HTML", "HTML content", VariableType::String);

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        use thirtyfour::prelude::*;

        context.deactivate_exec_pin("exec_out").await?;

        let session: AutomationSession = context.evaluate_pin("session").await?;
        let selector: String = context.evaluate_pin("selector").await?;
        let outer_html: bool = context.evaluate_pin("outer_html").await?;

        let driver = session.get_browser_driver_and_switch(context).await?;

        let html = if selector.is_empty() {
            driver
                .source()
                .await
                .map_err(|e| flow_like_types::anyhow!("Failed to get page source: {}", e))?
        } else {
            let element = driver.find(By::Css(&selector)).await.map_err(|e| {
                flow_like_types::anyhow!("Failed to find element '{}': {}", selector, e)
            })?;

            if outer_html {
                element
                    .outer_html()
                    .await
                    .map_err(|e| flow_like_types::anyhow!("Failed to get outer HTML: {}", e))?
            } else {
                element
                    .inner_html()
                    .await
                    .map_err(|e| flow_like_types::anyhow!("Failed to get inner HTML: {}", e))?
            }
        };

        context.set_pin_value("session_out", json!(session)).await?;
        context.set_pin_value("html", json!(html)).await?;
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
pub struct BrowserExecuteJsNode {}

impl BrowserExecuteJsNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for BrowserExecuteJsNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "browser_execute_js",
            "Execute JavaScript",
            "Executes JavaScript code in the browser and returns the result",
            "Automation/Browser/Extract",
        );
        node.add_icon("/flow/icons/browser.svg");

        node.set_scores(
            flow_like::flow::node::NodeScores::new()
                .set_privacy(2)
                .set_security(3)
                .set_performance(8)
                .set_governance(4)
                .set_reliability(7)
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
            "script",
            "Script",
            "JavaScript code to execute (use 'return' to return a value)",
            VariableType::String,
        )
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
            "result",
            "Result",
            "Return value from JavaScript (as JSON)",
            VariableType::Generic,
        )
        .set_schema::<Value>();

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let session: AutomationSession = context.evaluate_pin("session").await?;
        let script: String = context.evaluate_pin("script").await?;

        let driver = session.get_browser_driver_and_switch(context).await?;

        let result = driver
            .execute(&script, vec![])
            .await
            .map_err(|e| flow_like_types::anyhow!("Failed to execute JavaScript: {}", e))?;

        let json_result = result.json();

        context.set_pin_value("session_out", json!(session)).await?;
        context.set_pin_value("result", json_result.clone()).await?;
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
