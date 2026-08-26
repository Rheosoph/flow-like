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
pub struct SetIframeSrcdoc;

impl SetIframeSrcdoc {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for SetIframeSrcdoc {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "a2ui_set_iframe_srcdoc",
            "Set Iframe HTML",
            "Sets raw HTML content of an iframe element for previewing generated HTML",
            "UI/Elements/Media",
        );
        node.set_flowscript_name("ui", "setIframeSrcdoc");
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
            "html",
            "HTML",
            "Raw HTML content to render inside the iframe",
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

        let html: String = context.evaluate_pin("html").await?;

        context
            .upsert_element(
                &element_id,
                json!({
                    "type": "setIframeSrcdoc",
                    "srcdoc": html
                }),
            )
            .await?;
        context.activate_exec_pin("exec_out").await?;

        Ok(())
    }
}
