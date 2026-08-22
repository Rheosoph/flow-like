use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    variable::VariableType,
};
use flow_like_types::async_trait;
use std::collections::HashMap;

#[crate::register_node]
#[derive(Default)]
pub struct ListStructFields {}

impl ListStructFields {
    pub fn new() -> Self {
        ListStructFields {}
    }
}

#[async_trait]
impl NodeLogic for ListStructFields {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "struct_get_fields",
            "Get Fields",
            "Fetches fields from a struct",
            "Structs/Fields",
        );
        node.add_icon("/flow/icons/struct.svg");

        node.add_input_pin("struct", "Struct", "Struct Output", VariableType::Struct)
        .set_open_schema();

        node.add_output_pin("field_names", "Field Names", "Fields", VariableType::String)
            .set_value_type(flow_like::flow::pin::ValueType::Array)
            .set_default_value(Some(flow_like_types::json::json!([])));

        node.add_output_pin("fields", "Fields", "Fields", VariableType::Generic)
            .set_value_type(flow_like::flow::pin::ValueType::Array)
            .set_default_value(Some(flow_like_types::json::json!([])));

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let struct_value = context
            .evaluate_pin::<HashMap<String, flow_like_types::Value>>("struct")
            .await?;

        let mut sorted_entries: Vec<_> = struct_value.iter().collect();
        sorted_entries.sort_by_key(|(key, _)| *key);

        let field_names: Vec<String> = sorted_entries
            .iter()
            .map(|(key, _)| (*key).clone())
            .collect();
        let field_values: Vec<flow_like_types::Value> = sorted_entries
            .iter()
            .map(|(_, value)| (*value).clone())
            .collect();

        context
            .set_pin_value("field_names", flow_like_types::json::json!(field_names))
            .await?;
        context
            .set_pin_value("fields", flow_like_types::json::json!(field_values))
            .await?;

        return Ok(());
    }
}
