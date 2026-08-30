use super::element_utils::extract_element_id;
use flow_like::a2ui::components::TextFieldProps;
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_types::{Value, async_trait, json::json};

/// Sets the error state/message of a text field element.
#[crate::register_node]
#[derive(Default)]
pub struct SetTextFieldError;

impl SetTextFieldError {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for SetTextFieldError {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "a2ui_set_textfield_error",
            "Set TextField Error",
            "Sets the error state or message of a text field",
            "UI/Elements/Input",
        );
        node.set_flowscript_name("ui", "setTextfieldError");
        node.add_icon("/flow/icons/a2ui.svg");

        node.add_input_pin("exec_in", "▶", "Execution input", VariableType::Execution);

        node.add_input_pin(
            "element_ref",
            "TextField",
            "Reference to the text field element",
            VariableType::Struct,
        )
        .set_schema::<TextFieldProps>()
        .set_options(PinOptions::new().set_enforce_schema(false).build());

        node.add_input_pin(
            "error",
            "Error",
            "Error message (empty string clears error)",
            VariableType::String,
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

        let error: String = context.evaluate_pin("error").await?;

        context
            .upsert_element(
                &element_id,
                json!({
                    "type": "setError",
                    "error": if error.is_empty() { Value::Null } else { Value::String(error) }
                }),
            )
            .await?;
        context.activate_exec_pin("exec_out").await?;

        Ok(())
    }
}
