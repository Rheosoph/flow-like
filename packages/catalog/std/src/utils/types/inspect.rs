use crate::utils::pure_scores;
use flow_like::flow::{
    board::Board,
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_types::{Value, async_trait, json::json};

const TYPES: [&str; 6] = ["null", "boolean", "number", "string", "array", "object"];

fn type_name(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct TypeOfNode {}

impl TypeOfNode {
    pub fn new() -> Self {
        TypeOfNode {}
    }
}

#[async_trait]
impl NodeLogic for TypeOfNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "utils_types_type_of",
            "Type Of",
            "Reports what a value actually is — useful for data coming back from an API or a model",
            "Utils/Types",
        );
        node.set_flowscript_name("types", "typeOf");
        node.set_receiver("value");
        node.add_icon("/flow/icons/type.svg");
        node.set_scores(pure_scores());

        node.add_input_pin("value", "Value", "Value to inspect", VariableType::Generic);

        node.add_output_pin(
            "type",
            "Type",
            "One of null, boolean, number, string, array or object",
            VariableType::String,
        )
        .set_options(
            PinOptions::new()
                .set_valid_values(TYPES.iter().map(|name| name.to_string()).collect())
                .build(),
        );
        node.add_output_pin(
            "is_null",
            "Is Null",
            "True when the value is missing",
            VariableType::Boolean,
        );
        node.add_output_pin(
            "size",
            "Size",
            "Elements for an array, fields for an object, characters for a string, otherwise 0",
            VariableType::Integer,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let value: Value = context.evaluate_pin("value").await?;

        let size = match &value {
            Value::Array(items) => items.len() as i64,
            Value::Object(map) => map.len() as i64,
            Value::String(text) => text.chars().count() as i64,
            _ => 0,
        };

        context
            .set_pin_value("type", json!(type_name(&value)))
            .await?;
        context
            .set_pin_value("is_null", json!(value.is_null()))
            .await?;
        context.set_pin_value("size", json!(size)).await?;
        Ok(())
    }

    async fn on_update(&self, node: &mut Node, board: &Board) {
        let _ = node.match_type("value", board, None, None);
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct IsEmptyValueNode {}

impl IsEmptyValueNode {
    pub fn new() -> Self {
        IsEmptyValueNode {}
    }
}

#[async_trait]
impl NodeLogic for IsEmptyValueNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "utils_types_is_empty",
            "Is Empty",
            "True for null, an empty string, an empty array and an empty struct",
            "Utils/Types",
        );
        node.set_flowscript_name("types", "isEmpty");
        node.add_icon("/flow/icons/type.svg");
        node.set_scores(pure_scores());

        node.add_input_pin("value", "Value", "Value to inspect", VariableType::Generic);
        node.add_input_pin(
            "trim",
            "Trim",
            "Treat whitespace-only text as empty",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(true)));

        node.add_output_pin(
            "is_empty",
            "Is Empty",
            "True when the value holds nothing",
            VariableType::Boolean,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let value: Value = context.evaluate_pin("value").await?;
        let trim: bool = context.evaluate_pin("trim").await?;

        let is_empty = match &value {
            Value::Null => true,
            Value::String(text) if trim => text.trim().is_empty(),
            Value::String(text) => text.is_empty(),
            Value::Array(items) => items.is_empty(),
            Value::Object(map) => map.is_empty(),
            _ => false,
        };

        context.set_pin_value("is_empty", json!(is_empty)).await?;
        Ok(())
    }

    async fn on_update(&self, node: &mut Node, board: &Board) {
        let _ = node.match_type("value", board, None, None);
    }
}
