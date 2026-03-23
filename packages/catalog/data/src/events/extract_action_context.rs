use flow_like::a2ui::widget::ActionContextPayload;
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json, Value};

/// Extracts a single field from an ActionContextPayload by name.
#[crate::register_node]
#[derive(Default)]
pub struct ExtractActionContextField;

impl ExtractActionContextField {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for ExtractActionContextField {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "events_extract_action_context",
            "Extract Action Context Field",
            "Extracts a field value from an action context payload by field name",
            "Events/Widget",
        );
        node.add_icon("/flow/icons/event.svg");

        node.add_input_pin(
            "action_context",
            "Action Context",
            "The action context payload from a Widget Action Event",
            VariableType::Struct,
        )
        .set_schema::<ActionContextPayload>();

        node.add_input_pin(
            "field_name",
            "Field Name",
            "The name of the field to extract",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_output_pin(
            "value",
            "Value",
            "The extracted field value (null if field does not exist)",
            VariableType::Generic,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let payload: Value = context.evaluate_pin("action_context").await?;
        let field_name: String = context.evaluate_pin("field_name").await?;

        let value = payload.get(&field_name).cloned().unwrap_or(Value::Null);

        context.set_pin_value("value", json!(value)).await?;
        Ok(())
    }
}
