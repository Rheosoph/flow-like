use super::element_utils::extract_element_id;
use flow_like::a2ui::components::SliderProps;
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_types::{Value, async_trait, json::json};

/// Sets the value of a slider element.
#[crate::register_node]
#[derive(Default)]
pub struct SetSliderValue;

impl SetSliderValue {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for SetSliderValue {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "a2ui_set_slider_value",
            "Set Slider Value",
            "Sets the value of a slider element",
            "UI/Elements/Slider",
        );
        node.set_flowscript_name("ui", "setSliderValue");
        node.add_icon("/flow/icons/a2ui.svg");

        node.add_input_pin("exec_in", "▶", "Execution input", VariableType::Execution);

        node.add_input_pin(
            "element_ref",
            "Slider",
            "Reference to the slider element",
            VariableType::Struct,
        )
        .set_schema::<SliderProps>()
        .set_options(PinOptions::new().set_enforce_schema(false).build());

        node.add_input_pin(
            "value",
            "Value",
            "The new slider value",
            VariableType::Float,
        );

        node.add_output_pin("exec_out", "▶", "Execution output", VariableType::Execution);

        node.set_long_running(true);

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let element_value: Value = context.evaluate_pin("element_ref").await?;
        let element_id = extract_element_id(&element_value)
            .ok_or_else(|| flow_like_types::anyhow!("Invalid element reference"))?;

        let value: f64 = context.evaluate_pin("value").await?;

        context
            .upsert_element(
                &element_id,
                json!({
                    "type": "setValue",
                    "value": value
                }),
            )
            .await?;
        context.activate_exec_pin("exec_out").await?;

        Ok(())
    }
}
