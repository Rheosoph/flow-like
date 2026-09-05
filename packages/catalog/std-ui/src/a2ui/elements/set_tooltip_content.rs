use super::element_utils::extract_element_id;
use flow_like::a2ui::components::TooltipProps;
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_types::{Value, async_trait, json::json};

#[crate::register_node]
#[derive(Default)]
pub struct SetTooltipContent;

impl SetTooltipContent {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for SetTooltipContent {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "a2ui_set_tooltip_content",
            "Set Tooltip Content",
            "Sets the content text of a tooltip element",
            "UI/Elements/Set",
        );
        node.set_flowscript_name("ui", "setTooltipContent");
        node.add_icon("/flow/icons/a2ui.svg");

        node.add_input_pin("exec_in", "▶", "Execution input", VariableType::Execution);

        node.add_input_pin(
            "element_ref",
            "Tooltip",
            "Reference to the tooltip element",
            VariableType::Struct,
        )
        .set_schema::<TooltipProps>()
        .set_options(PinOptions::new().set_enforce_schema(false).build());

        node.add_input_pin(
            "content",
            "Content",
            "The content text to set",
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
        let content: String = context.evaluate_pin("content").await?;

        let update_value = json!({
            "type": "setProps",
            "props": { "content": content }
        });

        context.upsert_element(&element_id, update_value).await?;
        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }
}
