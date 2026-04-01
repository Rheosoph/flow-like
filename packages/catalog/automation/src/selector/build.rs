use crate::types::selectors::{Selector, SelectorKind, SelectorSet};
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json};

#[crate::register_node]
#[derive(Default)]
pub struct BuildSelectorNode {}

impl BuildSelectorNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for BuildSelectorNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "selector_build",
            "Build Selector",
            "Creates a selector from a value and kind",
            "Automation/Selector",
        );
        node.add_icon("/flow/icons/selector.svg");

        node.add_input_pin("exec_in", "▶", "Trigger", VariableType::Execution);

        node.add_input_pin("kind", "Kind", "Type of selector", VariableType::String)
            .set_options(
                PinOptions::new()
                    .set_valid_values(vec![
                        "Css".to_string(),
                        "Xpath".to_string(),
                        "Text".to_string(),
                        "TextExact".to_string(),
                        "Role".to_string(),
                        "TestId".to_string(),
                        "AriaLabel".to_string(),
                        "Placeholder".to_string(),
                        "AltText".to_string(),
                        "Title".to_string(),
                    ])
                    .build(),
            )
            .set_default_value(Some(json!("Css")));

        node.add_input_pin(
            "value",
            "Value",
            "Selector value (CSS selector, XPath, text, etc.)",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_input_pin(
            "confidence",
            "Confidence",
            "Confidence level (0.0-1.0)",
            VariableType::Float,
        )
        .set_default_value(Some(json!(1.0)));

        node.add_input_pin(
            "scope",
            "Scope",
            "Optional scope selector to narrow search",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_output_pin("exec_out", "▶", "Continue", VariableType::Execution);

        node.add_output_pin(
            "selector",
            "Selector",
            "The built selector",
            VariableType::Struct,
        )
        .set_schema::<Selector>();

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let kind_str: String = context.evaluate_pin("kind").await?;
        let value: String = context.evaluate_pin("value").await?;
        let confidence: f64 = context.evaluate_pin("confidence").await?;
        let scope: String = context.evaluate_pin("scope").await?;

        let kind = match kind_str.as_str() {
            "Xpath" => SelectorKind::Xpath,
            "Text" => SelectorKind::Text,
            "TextExact" => SelectorKind::TextExact,
            "Role" => SelectorKind::Role,
            "TestId" => SelectorKind::TestId,
            "AriaLabel" => SelectorKind::AriaLabel,
            "Placeholder" => SelectorKind::Placeholder,
            "AltText" => SelectorKind::AltText,
            "Title" => SelectorKind::Title,
            _ => SelectorKind::Css,
        };

        let mut selector = Selector {
            kind,
            value,
            confidence: Some(confidence),
            scope: None,
        };

        if !scope.is_empty() {
            selector.scope = Some(scope);
        }

        context.set_pin_value("selector", json!(selector)).await?;
        context.activate_exec_pin("exec_out").await?;

        Ok(())
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct CreateSelectorSetNode {}

impl CreateSelectorSetNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for CreateSelectorSetNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "selector_create_set",
            "Create Selector Set",
            "Creates a new empty selector set",
            "Automation/Selector",
        );
        node.add_icon("/flow/icons/selector.svg");

        node.add_input_pin("exec_in", "▶", "Trigger", VariableType::Execution);

        node.add_output_pin("exec_out", "▶", "Continue", VariableType::Execution);

        node.add_output_pin(
            "selector_set",
            "Selector Set",
            "Empty selector set",
            VariableType::Struct,
        )
        .set_schema::<SelectorSet>();

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let selector_set = SelectorSet::default();

        context
            .set_pin_value("selector_set", json!(selector_set))
            .await?;
        context.activate_exec_pin("exec_out").await?;

        Ok(())
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct AddToSelectorSetNode {}

impl AddToSelectorSetNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for AddToSelectorSetNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "selector_add_to_set",
            "Add To Selector Set",
            "Adds a selector to an existing selector set",
            "Automation/Selector",
        );
        node.add_icon("/flow/icons/selector.svg");

        node.add_input_pin("exec_in", "▶", "Trigger", VariableType::Execution);

        node.add_input_pin(
            "selector_set",
            "Selector Set",
            "Existing selector set",
            VariableType::Struct,
        )
        .set_schema::<SelectorSet>();

        node.add_input_pin(
            "selector",
            "Selector",
            "Selector to add",
            VariableType::Struct,
        )
        .set_schema::<Selector>();

        node.add_output_pin("exec_out", "▶", "Continue", VariableType::Execution);

        node.add_output_pin(
            "updated_set",
            "Updated Set",
            "Selector set with new selector added",
            VariableType::Struct,
        )
        .set_schema::<SelectorSet>();

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let mut selector_set: SelectorSet = context.evaluate_pin("selector_set").await?;
        let selector: Selector = context.evaluate_pin("selector").await?;

        let idx = selector_set.selectors.len();
        selector_set.selectors.push(selector);
        selector_set.fallback_order.push(idx);

        context
            .set_pin_value("updated_set", json!(selector_set))
            .await?;
        context.activate_exec_pin("exec_out").await?;

        Ok(())
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct GetPrimarySelectorNode {}

impl GetPrimarySelectorNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for GetPrimarySelectorNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "selector_get_primary",
            "Get Primary Selector",
            "Gets the primary (first) selector from a selector set",
            "Automation/Selector",
        );
        node.add_icon("/flow/icons/selector.svg");

        node.add_input_pin("exec_in", "▶", "Trigger", VariableType::Execution);

        node.add_input_pin(
            "selector_set",
            "Selector Set",
            "Selector set to get primary from",
            VariableType::Struct,
        )
        .set_schema::<SelectorSet>();

        node.add_output_pin(
            "exec_found",
            "Found",
            "Primary selector exists",
            VariableType::Execution,
        );
        node.add_output_pin(
            "exec_empty",
            "Empty",
            "Selector set is empty",
            VariableType::Execution,
        );

        node.add_output_pin(
            "selector",
            "Selector",
            "Primary selector",
            VariableType::Struct,
        )
        .set_schema::<Selector>();

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_found").await?;
        context.deactivate_exec_pin("exec_empty").await?;

        let selector_set: SelectorSet = context.evaluate_pin("selector_set").await?;

        if let Some(&idx) = selector_set.fallback_order.first()
            && let Some(selector) = selector_set.selectors.get(idx)
        {
            context.set_pin_value("selector", json!(selector)).await?;
            context.activate_exec_pin("exec_found").await?;
            return Ok(());
        }

        context.activate_exec_pin("exec_empty").await?;

        Ok(())
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct SelectorToStringNode {}

impl SelectorToStringNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for SelectorToStringNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "selector_to_string",
            "Selector To String",
            "Converts a selector to its string representation",
            "Automation/Selector",
        );
        node.add_icon("/flow/icons/selector.svg");

        node.add_input_pin("exec_in", "▶", "Trigger", VariableType::Execution);

        node.add_input_pin(
            "selector",
            "Selector",
            "Selector to convert",
            VariableType::Struct,
        )
        .set_schema::<Selector>();

        node.add_output_pin("exec_out", "▶", "Continue", VariableType::Execution);

        node.add_output_pin("kind", "Kind", "Selector kind", VariableType::String);
        node.add_output_pin("value", "Value", "Selector value", VariableType::String);
        node.add_output_pin(
            "confidence",
            "Confidence",
            "Selector confidence",
            VariableType::Float,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let selector: Selector = context.evaluate_pin("selector").await?;

        let kind = format!("{:?}", selector.kind);
        context.set_pin_value("kind", json!(kind)).await?;
        context
            .set_pin_value("value", json!(selector.value))
            .await?;
        context
            .set_pin_value("confidence", json!(selector.confidence.unwrap_or(1.0)))
            .await?;
        context.activate_exec_pin("exec_out").await?;

        Ok(())
    }
}
