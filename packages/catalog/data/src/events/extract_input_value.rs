use flow_like::a2ui::widget::InputValuesPayload;
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json, Value};

/// Extracts a single component value from an InputValuesPayload by component ID.
#[crate::register_node]
#[derive(Default)]
pub struct ExtractInputValue;

impl ExtractInputValue {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for ExtractInputValue {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "events_extract_input_value",
            "Extract Input Value",
            "Extracts a component's current value from the input values payload by component ID",
            "Events/Widget",
        );
        node.add_icon("/flow/icons/event.svg");

        node.add_input_pin(
            "input_values",
            "Input Values",
            "The input values payload from a Widget Action Event",
            VariableType::Struct,
        )
        .set_schema::<InputValuesPayload>();

        node.add_input_pin(
            "component_id",
            "Component ID",
            "The ID of the component whose value to extract",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_output_pin(
            "value",
            "Value",
            "The current value of the component (null if not found)",
            VariableType::Generic,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let payload: Value = context.evaluate_pin("input_values").await?;
        let component_id: String = context.evaluate_pin("component_id").await?;

        let value = payload.get(&component_id).cloned().unwrap_or(Value::Null);

        context.set_pin_value("value", json!(value)).await?;
        Ok(())
    }
}
