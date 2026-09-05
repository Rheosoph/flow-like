use super::element_utils::extract_element_id;
use flow_like::a2ui::components::ProgressProps;
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_types::{Value, async_trait, json::json};

/// Sets the progress value of a progress bar element.
#[crate::register_node]
#[derive(Default)]
pub struct SetProgress;

impl SetProgress {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for SetProgress {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "a2ui_set_progress",
            "Set Progress",
            "Sets the value of a progress bar (0-100)",
            "UI/Elements/Display",
        );
        node.set_flowscript_name("ui", "setProgress");
        node.add_icon("/flow/icons/a2ui.svg");

        node.add_input_pin("exec_in", "▶", "Execution input", VariableType::Execution);

        node.add_input_pin(
            "element_ref",
            "Progress Bar",
            "Reference to the progress bar element",
            VariableType::Struct,
        )
        .set_schema::<ProgressProps>()
        .set_options(PinOptions::new().set_enforce_schema(false).build());

        node.add_input_pin(
            "value",
            "Value",
            "Progress value (0-100)",
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
        let clamped_value = value.clamp(0.0, 100.0);

        let update_value = json!({
            "type": "setProgress",
            "value": clamped_value
        });

        context.upsert_element(&element_id, update_value).await?;
        context.activate_exec_pin("exec_out").await?;

        Ok(())
    }
}
