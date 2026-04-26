use flow_like::a2ui::components::SelectProps;
use flow_like::flow::{
    board::Board,
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_types::{Value, async_trait};

use super::element_utils::{extract_element_id_from_pin, find_element};

/// Gets the selected value of a select element.
#[crate::register_node]
#[derive(Default)]
pub struct GetSelectValue;

impl GetSelectValue {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for GetSelectValue {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "a2ui_get_select_value",
            "Get Select Value",
            "Gets the selected value of a select element",
            "UI/Elements/Select",
        );
        node.add_icon("/flow/icons/a2ui.svg");

        node.add_input_pin(
            "element_ref",
            "Select",
            "Reference to the select element",
            VariableType::Struct,
        )
        .set_schema::<SelectProps>()
        .set_options(PinOptions::new().set_enforce_schema(false).build());

        node.add_output_pin(
            "value",
            "Value",
            "The currently selected value",
            VariableType::String,
        );
        node.add_output_pin(
            "has_selection",
            "Has Selection",
            "Whether a value is selected",
            VariableType::Boolean,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let element_value: Value = context.evaluate_pin("element_ref").await?;
        let element_id = extract_element_id_from_pin(element_value)
            .ok_or_else(|| flow_like_types::anyhow!("Invalid element reference"))?;

        let elements = context.get_frontend_elements().await?;
        let element = elements.as_ref().and_then(|e| find_element(e, &element_id));

        let value = element
            .map(|(_, el)| el)
            .and_then(|el| el.get("component"))
            .and_then(|c| c.get("value").or_else(|| c.get("defaultValue")))
            .and_then(|v| v.as_str())
            .map(String::from);

        if let Some(v) = value {
            context
                .get_pin_by_name("value")
                .await?
                .set_value(Value::String(v))
                .await;
            context
                .get_pin_by_name("has_selection")
                .await?
                .set_value(Value::Bool(true))
                .await;
        } else {
            context
                .get_pin_by_name("value")
                .await?
                .set_value(Value::String(String::new()))
                .await;
            context
                .get_pin_by_name("has_selection")
                .await?
                .set_value(Value::Bool(false))
                .await;
        }

        Ok(())
    }

    async fn on_update(&self, node: &mut Node, _board: &Board) {
        node.error = None;
    }
}
