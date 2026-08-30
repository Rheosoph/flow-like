use super::element_utils::extract_element_id_from_pin;
use flow_like::a2ui::components::IframeProps;
use flow_like::flow::{
    execution::{LogLevel, context::ExecutionContext},
    node::{Node, NodeLogic},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_types::{Value, async_trait};

#[crate::register_node]
#[derive(Default)]
pub struct GetIframeSrc;

impl GetIframeSrc {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for GetIframeSrc {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "a2ui_get_iframe_src",
            "Get Iframe Src",
            "Gets the src URL of an iframe element",
            "UI/Elements/Get",
        );
        node.set_flowscript_name("ui", "getIframeSrc");
        node.add_icon("/flow/icons/a2ui.svg");

        node.add_input_pin(
            "element_ref",
            "Iframe",
            "Reference to the iframe element",
            VariableType::Struct,
        )
        .set_schema::<IframeProps>()
        .set_options(PinOptions::new().set_enforce_schema(false).build());

        node.add_output_pin(
            "src",
            "Src",
            "The iframe's source URL",
            VariableType::String,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let element_value: Value = context.evaluate_pin("element_ref").await?;
        let element_id = extract_element_id_from_pin(element_value)
            .ok_or_else(|| flow_like_types::anyhow!("Invalid element reference"))?;

        let Some((_, element)) = context.read_element(&element_id).await? else {
            context.log_message(
                &format!("Element not found: {}", element_id),
                LogLevel::Warn,
            );
            context
                .get_pin_by_name("src")
                .await?
                .set_value(Value::Null)
                .await;
            return Ok(());
        };

        let src = element
            .get("component")
            .and_then(|c| c.get("props"))
            .and_then(|p| p.get("src"))
            .and_then(|s| s.as_str())
            .unwrap_or_default();

        context
            .get_pin_by_name("src")
            .await?
            .set_value(Value::String(src.to_string()))
            .await;
        Ok(())
    }
}
