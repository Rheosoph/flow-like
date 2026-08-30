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

/// Gets the loading state of a button element.
#[crate::register_node]
#[derive(Default)]
pub struct GetButtonLoading;

impl GetButtonLoading {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for GetButtonLoading {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "a2ui_get_button_loading",
            "Get Button Loading",
            "Gets whether a button element is in loading state",
            "UI/Elements/Button",
        );
        node.set_flowscript_name("ui", "getButtonLoading");
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
            "loading",
            "Loading",
            "Whether the button is loading",
            VariableType::Boolean,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let element_value: Value = context.evaluate_pin("element_ref").await?;
        let element_id = extract_element_id_from_pin(element_value)
            .ok_or_else(|| flow_like_types::anyhow!("Invalid element reference"))?;

        let element = context.read_element(&element_id).await?;

        let loading = element
            .as_ref()
            .map(|(_, el)| el)
            .and_then(|el| el.get("component"))
            .and_then(|c| c.get("loading"))
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        context
            .get_pin_by_name("loading")
            .await?
            .set_value(Value::Bool(loading))
            .await;

        Ok(())
    }

    async fn on_update(&self, node: &mut Node, _board: &Board) {
        node.error = None;
    }
}
