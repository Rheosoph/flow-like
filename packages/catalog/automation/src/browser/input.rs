use crate::types::handles::AutomationSession;
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json};

#[crate::register_node]
#[derive(Default)]
pub struct BrowserTypeTextNode {}

impl BrowserTypeTextNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for BrowserTypeTextNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "browser_type_text",
            "Type Text",
            "Types text into an element matching the selector",
            "Automation/Browser/Input",
        );
        node.add_icon("/flow/icons/browser.svg");

        node.set_scores(
            flow_like::flow::node::NodeScores::new()
                .set_privacy(3)
                .set_security(4)
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

        node.add_input_pin(
            "selector",
            "Selector",
            "CSS selector of input element",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_input_pin(
            "text",
            "Text",
            "Text to type into the element",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_input_pin(
            "clear_first",
            "Clear First",
            "Clear existing text before typing",
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

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        use thirtyfour::prelude::*;

        context.deactivate_exec_pin("exec_out").await?;

        let session: AutomationSession = context.evaluate_pin("session").await?;
        let selector: String = context.evaluate_pin("selector").await?;
        let text: String = context.evaluate_pin("text").await?;
        let clear_first: bool = context.evaluate_pin("clear_first").await?;

        let driver = session.get_browser_driver_and_switch(context).await?;

        let element = driver.find(By::Css(&selector)).await.map_err(|e| {
            flow_like_types::anyhow!("Failed to find element '{}': {}", selector, e)
        })?;

        if clear_first {
            element
                .clear()
                .await
                .map_err(|e| flow_like_types::anyhow!("Failed to clear element: {}", e))?;
        }

        element
            .send_keys(&text)
            .await
            .map_err(|e| flow_like_types::anyhow!("Failed to type text: {}", e))?;

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
pub struct BrowserPressKeyNode {}

impl BrowserPressKeyNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for BrowserPressKeyNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "browser_press_key",
            "Press Key",
            "Presses a keyboard key (Enter, Tab, Escape, etc.)",
            "Automation/Browser/Input",
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
            "CSS selector of element (optional, press on active element if empty)",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_input_pin("key", "Key", "Key to press", VariableType::String)
            .set_options(
                flow_like::flow::pin::PinOptions::new()
                    .set_valid_values(vec![
                        "Enter".to_string(),
                        "Tab".to_string(),
                        "Escape".to_string(),
                        "Backspace".to_string(),
                        "Delete".to_string(),
                        "ArrowUp".to_string(),
                        "ArrowDown".to_string(),
                        "ArrowLeft".to_string(),
                        "ArrowRight".to_string(),
                        "Home".to_string(),
                        "End".to_string(),
                        "PageUp".to_string(),
                        "PageDown".to_string(),
                    ])
                    .build(),
            )
            .set_default_value(Some(json!("Enter")));

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
        use thirtyfour::prelude::*;

        context.deactivate_exec_pin("exec_out").await?;

        let session: AutomationSession = context.evaluate_pin("session").await?;
        let selector: String = context.evaluate_pin("selector").await?;
        let key: String = context.evaluate_pin("key").await?;

        let driver = session.get_browser_driver_and_switch(context).await?;

        let key_code = match key.as_str() {
            "Enter" => Key::Enter,
            "Tab" => Key::Tab,
            "Escape" => Key::Escape,
            "Backspace" => Key::Backspace,
            "Delete" => Key::Delete,
            "ArrowUp" => Key::Up,
            "ArrowDown" => Key::Down,
            "ArrowLeft" => Key::Left,
            "ArrowRight" => Key::Right,
            "Home" => Key::Home,
            "End" => Key::End,
            "PageUp" => Key::PageUp,
            "PageDown" => Key::PageDown,
            _ => {
                return Err(flow_like_types::anyhow!("Unknown key: {}", key));
            }
        };

        if selector.is_empty() {
            driver
                .action_chain()
                .send_keys(key_code)
                .perform()
                .await
                .map_err(|e| flow_like_types::anyhow!("Failed to press key: {}", e))?;
        } else {
            let element = driver.find(By::Css(&selector)).await.map_err(|e| {
                flow_like_types::anyhow!("Failed to find element '{}': {}", selector, e)
            })?;
            element
                .send_keys(key_code)
                .await
                .map_err(|e| flow_like_types::anyhow!("Failed to press key on element: {}", e))?;
        }

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
pub struct BrowserSelectOptionNode {}

impl BrowserSelectOptionNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for BrowserSelectOptionNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "browser_select_option",
            "Select Option",
            "Selects an option in a dropdown/select element",
            "Automation/Browser/Input",
        );
        node.add_icon("/flow/icons/browser.svg");

        node.set_scores(
            flow_like::flow::node::NodeScores::new()
                .set_privacy(4)
                .set_security(5)
                .set_performance(8)
                .set_governance(6)
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
            "selector",
            "Selector",
            "CSS selector of select element",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_input_pin(
            "value",
            "Value",
            "Option value to select",
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

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        use thirtyfour::components::SelectElement;
        use thirtyfour::prelude::*;

        context.deactivate_exec_pin("exec_out").await?;

        let session: AutomationSession = context.evaluate_pin("session").await?;
        let selector: String = context.evaluate_pin("selector").await?;
        let value: String = context.evaluate_pin("value").await?;

        let driver = session.get_browser_driver_and_switch(context).await?;

        let element = driver.find(By::Css(&selector)).await.map_err(|e| {
            flow_like_types::anyhow!("Failed to find select element '{}': {}", selector, e)
        })?;

        let select = SelectElement::new(&element)
            .await
            .map_err(|e| flow_like_types::anyhow!("Element is not a select: {}", e))?;

        select
            .select_by_value(&value)
            .await
            .map_err(|e| flow_like_types::anyhow!("Failed to select option: {}", e))?;

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
