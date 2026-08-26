use crate::utils::pure_scores;
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json};

#[crate::register_node]
#[derive(Default)]
pub struct BoolToIntNode {}

impl BoolToIntNode {
    pub fn new() -> Self {
        BoolToIntNode {}
    }
}

#[async_trait]
impl NodeLogic for BoolToIntNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "bool_to_int",
            "To Integer",
            "Converts a boolean into 1 or 0",
            "Utils/Bool",
        );
        node.set_flowscript_name("bool", "toInt");
        node.set_receiver("boolean");
        node.add_icon("/flow/icons/convert.svg");
        node.set_scores(pure_scores());

        node.add_input_pin("boolean", "Boolean", "Input Boolean", VariableType::Boolean)
            .set_default_value(Some(json!(false)));
        node.add_output_pin(
            "integer",
            "Integer",
            "1 when true, 0 when false",
            VariableType::Integer,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let boolean: bool = context.evaluate_pin("boolean").await?;
        context
            .set_pin_value("integer", json!(i64::from(boolean)))
            .await?;
        Ok(())
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct BoolToStringNode {}

impl BoolToStringNode {
    pub fn new() -> Self {
        BoolToStringNode {}
    }
}

#[async_trait]
impl NodeLogic for BoolToStringNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "bool_to_string",
            "To String",
            "Converts a boolean into text",
            "Utils/Bool",
        );
        node.set_flowscript_name("bool", "toString");
        node.set_receiver("boolean");
        node.add_icon("/flow/icons/convert.svg");
        node.set_scores(pure_scores());

        node.add_input_pin("boolean", "Boolean", "Input Boolean", VariableType::Boolean)
            .set_default_value(Some(json!(false)));
        node.add_input_pin(
            "true_text",
            "True Text",
            "Text used when the boolean is true",
            VariableType::String,
        )
        .set_default_value(Some(json!("true")));
        node.add_input_pin(
            "false_text",
            "False Text",
            "Text used when the boolean is false",
            VariableType::String,
        )
        .set_default_value(Some(json!("false")));

        node.add_output_pin("string", "String", "The text", VariableType::String);

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let boolean: bool = context.evaluate_pin("boolean").await?;
        let true_text: String = context.evaluate_pin("true_text").await?;
        let false_text: String = context.evaluate_pin("false_text").await?;

        let string = if boolean { true_text } else { false_text };
        context.set_pin_value("string", json!(string)).await?;
        Ok(())
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct IntToBoolNode {}

impl IntToBoolNode {
    pub fn new() -> Self {
        IntToBoolNode {}
    }
}

#[async_trait]
impl NodeLogic for IntToBoolNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "int_to_bool",
            "From Integer",
            "Converts an integer into a boolean, zero is false",
            "Utils/Bool",
        );
        node.set_flowscript_name("bool", "fromInt");
        node.add_icon("/flow/icons/convert.svg");
        node.set_scores(pure_scores());

        node.add_input_pin("integer", "Integer", "Input Integer", VariableType::Integer)
            .set_default_value(Some(json!(0)));
        node.add_output_pin(
            "boolean",
            "Boolean",
            "False when the integer was zero",
            VariableType::Boolean,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let integer: i64 = context.evaluate_pin("integer").await?;
        context
            .set_pin_value("boolean", json!(integer != 0))
            .await?;
        Ok(())
    }
}
