use flow_like::a2ui::components::ButtonProps;
use flow_like::flow::{
    board::Board,
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_types::{Value, async_trait};

use super::element_utils::extract_element_id_from_pin;

/// Gets the label text of a button element.
#[crate::register_node]
#[derive(Default)]
pub struct GetButtonLabel;

impl GetButtonLabel {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for GetButtonLabel {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "a2ui_get_button_label",
            "Get Button Label",
            "Gets the label text of a button element",
            "UI/Elements/Button",
        );
        node.set_flowscript_name("ui", "getButtonLabel");
        node.add_icon("/flow/icons/a2ui.svg");

        node.add_input_pin(
            "element_ref",
            "Button",
            "Reference to the button element",
            VariableType::Struct,
        )
        .set_schema::<ButtonProps>()
        .set_options(PinOptions::new().set_enforce_schema(false).build());

        node.add_output_pin(
            "label",
            "Label",
            "The button's label text",
            VariableType::String,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let element_value: Value = context.evaluate_pin("element_ref").await?;
        let element_id = extract_element_id_from_pin(element_value)
            .ok_or_else(|| flow_like_types::anyhow!("Invalid element reference"))?;

        let element = context.read_element(&element_id).await?;

        let label = element
            .as_ref()
            .map(|(_, el)| el)
            .and_then(|el| el.get("component"))
            .and_then(|c| {
                c.get("label")
                    .or_else(|| c.get("text"))
                    .or_else(|| c.get("content"))
            })
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        context
            .get_pin_by_name("label")
            .await?
            .set_value(Value::String(label))
            .await;

        Ok(())
    }

    async fn on_update(&self, node: &mut Node, _board: &Board) {
        node.error = None;
    }
}
