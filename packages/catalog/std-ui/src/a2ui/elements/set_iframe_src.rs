use super::element_utils::extract_element_id;
use flow_like::a2ui::components::IframeProps;
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_types::{Value, async_trait, json::json};

#[crate::register_node]
#[derive(Default)]
pub struct SetIframeSrc;

impl SetIframeSrc {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for SetIframeSrc {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "a2ui_set_iframe_src",
            "Set Iframe Src",
            "Sets the source URL of an iframe element",
            "UI/Elements/Media",
        );
        node.set_flowscript_name("ui", "setIframeSrc");
        node.add_icon("/flow/icons/a2ui.svg");

        node.add_input_pin("exec_in", "▶", "Execution input", VariableType::Execution);

        node.add_input_pin(
            "element_ref",
            "Iframe",
            "Reference to the iframe element",
            VariableType::Struct,
        )
        .set_schema::<IframeProps>()
        .set_options(PinOptions::new().set_enforce_schema(false).build());

        node.add_input_pin(
            "src",
            "URL",
            "The URL to load in the iframe",
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

        let src: String = context.evaluate_pin("src").await?;

        context
            .upsert_element(
                &element_id,
                json!({
                    "type": "setIframeSrc",
                    "src": src
                }),
            )
            .await?;
        context.activate_exec_pin("exec_out").await?;

        Ok(())
    }
}
