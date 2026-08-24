use flow_like::a2ui::components::SelectProps;
use flow_like::flow::{
    execution::{LogLevel, context::ExecutionContext},
    node::{Node, NodeLogic},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_types::{Value, async_trait, json::json};

use super::element_utils::extract_element_id;

/// Sets the selected value of a select element.
#[crate::register_node]
#[derive(Default)]
pub struct SetSelectValue;

impl SetSelectValue {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for SetSelectValue {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "a2ui_set_select_value",
            "Set Select Value",
            "Sets the selected value of a select element",
            "UI/Elements/Select",
        );
        node.set_flowscript_name("ui", "setSelectValue");
        node.add_icon("/flow/icons/a2ui.svg");

        node.add_input_pin("exec_in", "▶", "Execution input", VariableType::Execution);

        node.add_input_pin(
            "element_ref",
            "Select",
            "Element ID string or element object from Get Element",
            VariableType::Struct,
        )
        .set_schema::<SelectProps>()
        .set_options(PinOptions::new().set_enforce_schema(false).build());
        node.add_input_pin(
            "value",
            "Value",
            "The value to select",
            VariableType::String,
        );

        node.add_output_pin("exec_out", "▶", "Execution output", VariableType::Execution);

        node.set_long_running(true);

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.activate_exec_pin("exec_out").await?;

        let element_value: Value = context.evaluate_pin("element_ref").await?;
        let element_id = extract_element_id(&element_value).ok_or_else(|| {
            flow_like_types::anyhow!(
                "Invalid element reference - expected string ID or element object"
            )
        })?;
        let value: String = context.evaluate_pin("value").await?;

        context
            .upsert_element(
                &element_id,
                json!({
                    "type": "setValue",
                    "value": value
                }),
            )
            .await?;

        context.log_message(
            &format!("Set select value: {} = {}", element_id, value),
            LogLevel::Debug,
        );

        Ok(())
    }
}
