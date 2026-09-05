use super::element_utils::extract_element_id;
use flow_like::a2ui::components::TextProps;
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_types::{Value, async_trait, json::json};

/// Sets the text content of a text-based element.
///
/// Streams a ui_update event to the frontend to update the element.
#[crate::register_node]
#[derive(Default)]
pub struct SetElementText;

impl SetElementText {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for SetElementText {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "a2ui_set_element_text",
            "Set Element Text",
            "Sets the text content of an element",
            "UI/Elements",
        );
        node.set_flowscript_name("ui", "setElementText");
        node.add_icon("/flow/icons/a2ui.svg");

        node.add_input_pin("exec_in", "▶", "Execution input", VariableType::Execution);

        node.add_input_pin(
            "element_ref",
            "Element",
            "Reference to the text element (ID string or element object from Get Element)",
            VariableType::Struct,
        )
        .set_schema::<TextProps>()
        .set_options(PinOptions::new().set_enforce_schema(false).build());

        node.add_input_pin("text", "Text", "The new text content", VariableType::String);

        node.add_output_pin("exec_out", "▶", "Execution output", VariableType::Execution);

        node.set_long_running(true);

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let element_value: Value = context.evaluate_pin("element_ref").await?;

        // Debug: Log what value we received
        context.log_message(
            &format!(
                "[SetElementText] Received element_ref value: {:?}",
                element_value
            ),
            flow_like::flow::execution::LogLevel::Debug,
        );

        // Handle null explicitly - this means get_element didn't find the element
        if element_value.is_null() {
            return Err(flow_like_types::anyhow!(
                "Element reference is null - the element was not found. Make sure the element ID exists and elements are being passed in the workflow payload."
            ));
        }

        let element_id = extract_element_id(&element_value)
            .ok_or_else(|| {
                // Log more details about why extraction failed
                let type_info = match &element_value {
                    Value::Null => "null",
                    Value::Bool(_) => "bool",
                    Value::Number(_) => "number",
                    Value::String(s) if s.is_empty() => "empty string",
                    Value::String(_) => "string",
                    Value::Array(_) => "array",
                    Value::Object(obj) => {
                        if obj.contains_key("__element_id") {
                            "object with __element_id"
                        } else if obj.contains_key("id") {
                            "object with id"
                        } else {
                            "object without id fields"
                        }
                    }
                };
                flow_like_types::anyhow!(
                    "Invalid element reference (type: {}) - expected string ID or element object with __element_id. Value: {:?}",
                    type_info,
                    element_value
                )
            })?;

        context.log_message(
            &format!("[SetElementText] Extracted element_id: {}", element_id),
            flow_like::flow::execution::LogLevel::Debug,
        );

        let text: String = context.evaluate_pin("text").await?;

        let update_value = json!({
            "type": "setText",
            "text": text
        });

        context.upsert_element(&element_id, update_value).await?;
        context.activate_exec_pin("exec_out").await?;

        Ok(())
    }
}
