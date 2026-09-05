use super::element_utils::extract_element_id_from_pin;
use flow_like::a2ui::components::TooltipProps;
use flow_like::flow::{
    execution::{LogLevel, context::ExecutionContext},
    node::{Node, NodeLogic},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_types::{Value, async_trait};

#[crate::register_node]
#[derive(Default)]
pub struct GetTooltipContent;

impl GetTooltipContent {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for GetTooltipContent {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "a2ui_get_tooltip_content",
            "Get Tooltip Content",
            "Gets the content text of a tooltip element",
            "UI/Elements/Get",
        );
        node.set_flowscript_name("ui", "getTooltipContent");
        node.add_icon("/flow/icons/a2ui.svg");

        node.add_input_pin(
            "element_ref",
            "Tooltip",
            "Reference to the tooltip element",
            VariableType::Struct,
        )
        .set_schema::<TooltipProps>()
        .set_options(PinOptions::new().set_enforce_schema(false).build());

        node.add_output_pin(
            "content",
            "Content",
            "The tooltip's content text",
            VariableType::String,
        );

        node.add_output_pin(
            "side",
            "Side",
            "The tooltip's side position (top, bottom, left, right)",
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
                .get_pin_by_name("content")
                .await?
                .set_value(Value::Null)
                .await;
            context
                .get_pin_by_name("side")
                .await?
                .set_value(Value::Null)
                .await;
            return Ok(());
        };

        let props = element.get("component").and_then(|c| c.get("props"));

        let content = props
            .and_then(|p| p.get("content"))
            .and_then(|c| c.as_str())
            .unwrap_or_default();

        let side = props
            .and_then(|p| p.get("side"))
            .and_then(|s| s.as_str())
            .unwrap_or("top");

        context
            .get_pin_by_name("content")
            .await?
            .set_value(Value::String(content.to_string()))
            .await;
        context
            .get_pin_by_name("side")
            .await?
            .set_value(Value::String(side.to_string()))
            .await;
        Ok(())
    }
}
