use super::path_utils::get_value_by_path;
use flow_like::flow::{
    board::Board,
    execution::{LogLevel, context::ExecutionContext},
    node::{Node, NodeLogic},
    variable::VariableType,
};
use flow_like_types::async_trait;

#[crate::register_node]
#[derive(Default)]
pub struct GetStructFieldNode {}

impl GetStructFieldNode {
    pub fn new() -> Self {
        GetStructFieldNode {}
    }
}

#[async_trait]
impl NodeLogic for GetStructFieldNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "struct_get",
            "Get Field",
            "Fetches a field from a struct (supports dot notation and array access)",
            "Structs/Fields",
        );
        node.add_icon("/flow/icons/struct.svg");

        node.add_output_pin(
            "value",
            "Value",
            "Value of the Struct",
            VariableType::Generic,
        );
        node.add_output_pin(
            "found",
            "Found?",
            "Indicates if the value was found",
            VariableType::Boolean,
        );

        node.add_input_pin("struct", "Struct", "Struct Output", VariableType::Struct)
            .set_open_schema();

        node.add_input_pin(
            "field",
            "Field",
            "Field selector (e.g., 'message.content' or 'items[0].name')",
            VariableType::String,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let struct_value = context
            .evaluate_pin::<flow_like_types::Value>("struct")
            .await?;
        context.log_message(&format!("Got Value: {:?}", struct_value), LogLevel::Debug);
        let field = context.evaluate_pin::<String>("field").await?;

        let value = get_value_by_path(&struct_value, &field);
        context
            .set_pin_value("found", flow_like_types::json::json!(value.is_some()))
            .await?;

        if let Some(value) = value {
            context.set_pin_value("value", value).await?;
            return Ok(());
        }

        context
            .set_pin_value("value", flow_like_types::Value::Null)
            .await?;

        return Ok(());
    }

    async fn on_update(&self, node: &mut Node, board: &Board) {
        let _ = node.match_type("value", board, None, None);
    }
}
