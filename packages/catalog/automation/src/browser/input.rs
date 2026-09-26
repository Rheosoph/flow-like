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
        node.set_flowscript_name("browser", "typeText");
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

        super::selector::add_locator_pin(&mut node);
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let session: AutomationSession = context.evaluate_pin("session").await?;
        let selector: String = context.evaluate_pin("selector").await?;
        let locator = super::selector::evaluate_locator(context, &selector).await?;
        let text: String = context.evaluate_pin("text").await?;
        let clear_first: bool = context.evaluate_pin("clear_first").await?;

        let driver = session.get_browser_driver_and_switch(context).await?;

        let element = super::selector::find(&driver, &locator)
            .await
            .map_err(|e| {
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
        node.set_flowscript_name("browser", "pressKey");
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
            .set_default_value(Some(json!("Enter")));

        node.add_output_pin("exec_out", "▶", "Continue", VariableType::Execution);

        node.add_output_pin(
            "session_out",
            "Session",
            "Automation session (pass-through)",
            VariableType::Struct,
        )
        .set_schema::<AutomationSession>();

        node.add_input_pin(
            "modifiers",
            "Modifiers",
            "Control, Shift, Alt, or Meta",
            VariableType::String,
        )
        .set_value_type(flow_like::flow::pin::ValueType::Array)
        .set_default_value(Some(json!([])));
        super::selector::add_locator_pin(&mut node);
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let session: AutomationSession = context.evaluate_pin("session").await?;
        let selector: String = context.evaluate_pin("selector").await?;
        let locator = super::selector::evaluate_locator(context, &selector).await?;
        let key: String = context.evaluate_pin("key").await?;

        let driver = session.get_browser_driver_and_switch(context).await?;

        let modifiers: Vec<String> =
            super::selector::optional_input(context, "modifiers", Vec::new()).await?;
        if !locator.value.is_empty() {
            super::selector::find(&driver, &locator)
                .await?
                .focus()
                .await?;
        }
        super::actions::key_chord(&driver, &key, &modifiers).await?;

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
        node.set_flowscript_name("browser", "selectOption");
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

        super::selector::add_locator_pin(&mut node);
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        use thirtyfour::components::SelectElement;

        context.deactivate_exec_pin("exec_out").await?;

        let session: AutomationSession = context.evaluate_pin("session").await?;
        let selector: String = context.evaluate_pin("selector").await?;
        let locator = super::selector::evaluate_locator(context, &selector).await?;
        let value: String = context.evaluate_pin("value").await?;

        let driver = session.get_browser_driver_and_switch(context).await?;

        let element = super::selector::find(&driver, &locator)
            .await
            .map_err(|e| {
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
