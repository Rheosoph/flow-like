use super::element_utils::extract_element_id;
use flow_like::a2ui::components::IconProps;
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_types::{Value, async_trait, json::json};

/// Sets the icon name of an icon element.
#[crate::register_node]
#[derive(Default)]
pub struct SetIcon;

impl SetIcon {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for SetIcon {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "a2ui_set_icon",
            "Set Icon",
            "Sets the icon name of an icon element",
            "UI/Elements/Display",
        );
        node.set_flowscript_name("ui", "setIcon");
        node.add_icon("/flow/icons/a2ui.svg");

        node.add_input_pin("exec_in", "▶", "Execution input", VariableType::Execution);

        node.add_input_pin(
            "element_ref",
            "Icon",
            "Reference to the icon element",
            VariableType::Struct,
        )
        .set_schema::<IconProps>()
        .set_options(PinOptions::new().set_enforce_schema(false).build());

        node.add_input_pin(
            "name",
            "Name",
            "The icon name (e.g., 'check', 'x', 'star')",
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

        let name: String = context.evaluate_pin("name").await?;

        context
            .upsert_element(
                &element_id,
                json!({
                    "type": "setIcon",
                    "name": name
                }),
            )
            .await?;
        context.activate_exec_pin("exec_out").await?;

        Ok(())
    }
}
