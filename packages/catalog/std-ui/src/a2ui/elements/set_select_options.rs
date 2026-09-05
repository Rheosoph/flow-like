use super::element_utils::extract_element_id;
use flow_like::a2ui::components::SelectProps;
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_types::{Value, async_trait, json::json};

/// Sets the options of a select element.
#[crate::register_node]
#[derive(Default)]
pub struct SetSelectOptions;

impl SetSelectOptions {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for SetSelectOptions {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "a2ui_set_select_options",
            "Set Select Options",
            "Sets the available options in a select element",
            "UI/Elements/Select",
        );
        node.set_flowscript_name("ui", "setSelectOptions");
        node.add_icon("/flow/icons/a2ui.svg");

        node.add_input_pin("exec_in", "▶", "Execution input", VariableType::Execution);

        node.add_input_pin(
            "element_ref",
            "Select",
            "Reference to the select element",
            VariableType::Struct,
        )
        .set_schema::<SelectProps>()
        .set_options(PinOptions::new().set_enforce_schema(false).build());

        node.add_input_pin(
            "options",
            "Options",
            "Array of options [{value, label}] or simple strings",
            VariableType::Generic,
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

        let options: Value = context.evaluate_pin("options").await?;

        context
            .upsert_element(
                &element_id,
                json!({
                    "type": "setSelectOptions",
                    "options": options
                }),
            )
            .await?;
        context.activate_exec_pin("exec_out").await?;

        Ok(())
    }
}
