use super::element_utils::extract_element_id;
use flow_like::a2ui::components::DiffViewProps;
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_types::{Value, async_trait, json::json};

/// Sets the original and modified content of a diff view element in one update.
#[crate::register_node]
#[derive(Default)]
pub struct SetDiffContent;

impl SetDiffContent {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for SetDiffContent {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "a2ui_set_diff_content",
            "Set Diff Content",
            "Sets the original and modified content of a diff view element",
            "UI/Elements/Display",
        );
        node.add_icon("/flow/icons/a2ui.svg");

        node.add_input_pin("exec_in", "▶", "Execution input", VariableType::Execution);

        node.add_input_pin(
            "element_ref",
            "Diff View",
            "Reference to the diff view element",
            VariableType::Struct,
        )
        .set_schema::<DiffViewProps>()
        .set_options(PinOptions::new().set_enforce_schema(false).build());

        node.add_input_pin(
            "original",
            "Original",
            "Left / old content (text or document URL)",
            VariableType::String,
        );

        node.add_input_pin(
            "modified",
            "Modified",
            "Right / new content (text or document URL)",
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

        let original: String = context.evaluate_pin("original").await?;
        let modified: String = context.evaluate_pin("modified").await?;

        let update_value = json!({
            "type": "setProps",
            "props": {
                "original": original,
                "modified": modified
            }
        });

        context.upsert_element(&element_id, update_value).await?;
        context.activate_exec_pin("exec_out").await?;

        Ok(())
    }
}
