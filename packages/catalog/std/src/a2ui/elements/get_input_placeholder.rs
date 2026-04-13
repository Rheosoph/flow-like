use flow_like::a2ui::components::TextFieldProps;
use flow_like::flow::{
    board::Board,
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_types::{Value, async_trait};
use std::sync::Arc;

use super::element_utils::{extract_element_id_from_pin, find_element};

/// Gets the placeholder text of an input element.
#[crate::register_node]
#[derive(Default)]
pub struct GetInputPlaceholder;

impl GetInputPlaceholder {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for GetInputPlaceholder {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "a2ui_get_input_placeholder",
            "Get Input Placeholder",
            "Gets the placeholder text of an input element",
            "UI/Elements/Input",
        );
        node.add_icon("/flow/icons/a2ui.svg");

        node.add_input_pin(
            "element_ref",
            "Input",
            "Reference to the input element",
            VariableType::Struct,
        )
        .set_schema::<TextFieldProps>()
        .set_options(PinOptions::new().set_enforce_schema(false).build());

        node.add_output_pin(
            "placeholder",
            "Placeholder",
            "The input's placeholder text",
            VariableType::String,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let element_value: Value = context.evaluate_pin("element_ref").await?;
        let element_id = extract_element_id_from_pin(element_value)
            .ok_or_else(|| flow_like_types::anyhow!("Invalid element reference"))?;

        let elements = context.get_frontend_elements().await?;
        let element = elements.as_ref().and_then(|e| find_element(e, &element_id));

        let placeholder = element
            .map(|(_, el)| el)
            .and_then(|el| el.get("component"))
            .and_then(|c| c.get("placeholder"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        context
            .get_pin_by_name("placeholder")
            .await?
            .set_value(Value::String(placeholder))
            .await;

        Ok(())
    }

    async fn on_update(&self, node: &mut Node, _board: &Board) {
        node.error = None;
    }
}
