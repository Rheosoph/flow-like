use super::element_utils::extract_element_id;
use flow_like::a2ui::components::MarkdownProps;
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_types::{Value, async_trait, json::json};

/// Sets the content of a markdown element.
#[crate::register_node]
#[derive(Default)]
pub struct SetMarkdownContent;

impl SetMarkdownContent {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for SetMarkdownContent {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "a2ui_set_markdown_content",
            "Set Markdown Content",
            "Sets the markdown content of a markdown element",
            "UI/Elements/Display",
        );
        node.set_flowscript_name("ui", "setMarkdownContent");
        node.add_icon("/flow/icons/a2ui.svg");

        node.add_input_pin("exec_in", "▶", "Execution input", VariableType::Execution);

        node.add_input_pin(
            "element_ref",
            "Markdown",
            "Reference to the markdown element",
            VariableType::Struct,
        )
        .set_schema::<MarkdownProps>()
        .set_options(PinOptions::new().set_enforce_schema(false).build());

        node.add_input_pin(
            "content",
            "Content",
            "The markdown content",
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

        context
            .upsert_element(
                &element_id,
                json!({
                    "type": "setText",
                    "text": content
                }),
            )
            .await?;
        context.activate_exec_pin("exec_out").await?;

        Ok(())
    }
}
