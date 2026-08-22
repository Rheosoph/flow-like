use crate::utils::pure_scores;
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json};

const OPERATIONS: [&str; 4] = ["Add", "Subtract", "Multiply", "Divide"];

fn operation_node(id: &str, label: &str, description: &str) -> Node {
    let mut node = Node::new(id, label, description, "Math/Int/Overflow");
    node.add_icon("/flow/icons/sigma.svg");
    node.set_scores(pure_scores());

    node.add_input_pin(
        "integer1",
        "Integer 1",
        "Left hand side",
        VariableType::Integer,
    );
    node.add_input_pin(
        "integer2",
        "Integer 2",
        "Right hand side",
        VariableType::Integer,
    );
    node.add_input_pin(
        "operation",
        "Operation",
        "Arithmetic operation to apply",
        VariableType::String,
    )
    .set_default_value(Some(json!("Add")))
    .set_options(
        PinOptions::new()
            .set_valid_values(OPERATIONS.iter().map(|value| value.to_string()).collect())
            .build(),
    );

    node.add_output_pin("result", "Result", description, VariableType::Integer);

    node
}

async fn operands(context: &mut ExecutionContext) -> flow_like_types::Result<(i64, i64, String)> {
    let integer1: i64 = context.evaluate_pin("integer1").await?;
    let integer2: i64 = context.evaluate_pin("integer2").await?;
    let operation: String = context.evaluate_pin("operation").await?;
    Ok((integer1, integer2, operation))
}

#[crate::register_node]
#[derive(Default)]
pub struct IntCheckedOpNode {}

impl IntCheckedOpNode {
    pub fn new() -> Self {
        IntCheckedOpNode {}
    }
}

#[async_trait]
impl NodeLogic for IntCheckedOpNode {
    fn get_node(&self) -> Node {
        let mut node = operation_node(
            "int_checked_op",
            "Checked Arithmetic",
            "Arithmetic that reports overflow and division by zero instead of failing",
        );
        node.add_output_pin(
            "success",
            "Success",
            "False on overflow or division by zero",
            VariableType::Boolean,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let (integer1, integer2, operation) = operands(context).await?;

        let result = match operation.as_str() {
            "Subtract" => integer1.checked_sub(integer2),
            "Multiply" => integer1.checked_mul(integer2),
            "Divide" => integer1.checked_div(integer2),
            _ => integer1.checked_add(integer2),
        };

        context
            .set_pin_value("result", json!(result.unwrap_or(0)))
            .await?;
        context
            .set_pin_value("success", json!(result.is_some()))
            .await?;
        Ok(())
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct IntSaturatingOpNode {}

impl IntSaturatingOpNode {
    pub fn new() -> Self {
        IntSaturatingOpNode {}
    }
}

#[async_trait]
impl NodeLogic for IntSaturatingOpNode {
    fn get_node(&self) -> Node {
        operation_node(
            "int_saturating_op",
            "Saturating Arithmetic",
            "Arithmetic that clamps to the integer limits instead of overflowing",
        )
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let (integer1, integer2, operation) = operands(context).await?;

        let result = match operation.as_str() {
            "Subtract" => integer1.saturating_sub(integer2),
            "Multiply" => integer1.saturating_mul(integer2),
            "Divide" => integer1.checked_div(integer2).unwrap_or(0),
            _ => integer1.saturating_add(integer2),
        };

        context.set_pin_value("result", json!(result)).await?;
        Ok(())
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct IntWrappingOpNode {}

impl IntWrappingOpNode {
    pub fn new() -> Self {
        IntWrappingOpNode {}
    }
}

#[async_trait]
impl NodeLogic for IntWrappingOpNode {
    fn get_node(&self) -> Node {
        operation_node(
            "int_wrapping_op",
            "Wrapping Arithmetic",
            "Arithmetic that wraps around the integer limits",
        )
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let (integer1, integer2, operation) = operands(context).await?;

        let result = match operation.as_str() {
            "Subtract" => integer1.wrapping_sub(integer2),
            "Multiply" => integer1.wrapping_mul(integer2),
            "Divide" => integer1.checked_div(integer2).unwrap_or(0),
            _ => integer1.wrapping_add(integer2),
        };

        context.set_pin_value("result", json!(result)).await?;
        Ok(())
    }
}
