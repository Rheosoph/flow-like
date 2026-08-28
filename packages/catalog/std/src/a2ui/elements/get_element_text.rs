use super::element_utils::extract_element_id_from_pin;
use flow_like::a2ui::components::TextProps;
use flow_like::flow::{
    execution::{LogLevel, context::ExecutionContext},
    node::{Node, NodeLogic},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_types::{Value, async_trait};

/// Gets the text content of an element.
///
/// Works with text, button labels, headings, and other text-based elements.
#[crate::register_node]
#[derive(Default)]
pub struct GetElementText;

impl GetElementText {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for GetElementText {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "a2ui_get_element_text",
            "Get Element Text",
            "Gets the text content of an element",
            "UI/Elements",
        );
        node.set_flowscript_name("ui", "getElementText");
        node.add_icon("/flow/icons/a2ui.svg");

        node.add_input_pin(
            "element_ref",
            "Element",
            "Reference to the text element",
            VariableType::Struct,
        )
        .set_schema::<TextProps>()
        .set_options(PinOptions::new().set_enforce_schema(false).build());

        node.add_output_pin(
            "text",
            "Text",
            "The text content of the element",
            VariableType::String,
        );

        node.add_output_pin(
            "exists",
            "Exists",
            "Whether the element exists",
            VariableType::Boolean,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let element_value: Value = context.evaluate_pin("element_ref").await?;
        let element_id = extract_element_id_from_pin(element_value)
            .ok_or_else(|| flow_like_types::anyhow!("Invalid element reference"))?;

        context.log_message(
            &format!("[GetElementText] Looking for element_id: {}", element_id),
            LogLevel::Debug,
        );

        let element = context.read_element(&element_id).await?;

        let text_pin = context.get_pin_by_name("text").await?;
        let exists_pin = context.get_pin_by_name("exists").await?;

        if let Some((found_id, element_value)) = element {
            let text = element_value
                .get("component")
                .and_then(|c| {
                    c.get("content")
                        .or_else(|| c.get("text"))
                        .or_else(|| c.get("label"))
                        .or_else(|| c.get("title"))
                        .or_else(|| c.get("placeholder"))
                })
                .and_then(|v| v.as_str())
                .unwrap_or("");

            text_pin.set_value(Value::String(text.to_string())).await;
            exists_pin.set_value(Value::Bool(true)).await;

            context.log_message(
                &format!(
                    "Got text from element {} ({}): {}",
                    element_id, found_id, text
                ),
                LogLevel::Debug,
            );
        } else {
            text_pin.set_value(Value::String(String::new())).await;
            exists_pin.set_value(Value::Bool(false)).await;

            context.log_message(
                &format!("Element not found: {}", element_id),
                LogLevel::Warn,
            );
        }

        Ok(())
    }
}
