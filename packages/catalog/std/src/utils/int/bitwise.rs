use crate::utils::pure_scores;
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json};

fn binary_node(id: &str, label: &str, description: &str) -> Node {
    let mut node = Node::new(id, label, description, "Math/Int/Bitwise");
    node.add_icon("/flow/icons/sigma.svg");
    node.set_scores(pure_scores());

    node.add_input_pin(
        "integer1",
        "Integer 1",
        "Input Integer",
        VariableType::Integer,
    );
    node.add_input_pin(
        "integer2",
        "Integer 2",
        "Input Integer",
        VariableType::Integer,
    );
    node.add_output_pin("result", "Result", description, VariableType::Integer);

    node
}

async fn binary_run(
    context: &mut ExecutionContext,
    operation: impl Fn(i64, i64) -> i64,
) -> flow_like_types::Result<()> {
    let integer1: i64 = context.evaluate_pin("integer1").await?;
    let integer2: i64 = context.evaluate_pin("integer2").await?;
    context
        .set_pin_value("result", json!(operation(integer1, integer2)))
        .await?;
    Ok(())
}

fn shift_node(id: &str, label: &str, description: &str) -> Node {
    let mut node = Node::new(id, label, description, "Math/Int/Bitwise");
    node.add_icon("/flow/icons/sigma.svg");
    node.set_scores(pure_scores());

    node.add_input_pin("integer", "Integer", "Input Integer", VariableType::Integer);
    node.add_input_pin(
        "shift",
        "Shift",
        "Number of bit positions to shift by",
        VariableType::Integer,
    )
    .set_default_value(Some(json!(1)));
    node.add_output_pin("result", "Result", description, VariableType::Integer);

    node
}

async fn shift_run(
    context: &mut ExecutionContext,
    operation: impl Fn(i64, u32) -> i64,
) -> flow_like_types::Result<()> {
    let integer: i64 = context.evaluate_pin("integer").await?;
    let shift: i64 = context.evaluate_pin("shift").await?;
    context
        .set_pin_value(
            "result",
            json!(operation(integer, shift.clamp(0, 63) as u32)),
        )
        .await?;
    Ok(())
}

#[crate::register_node]
#[derive(Default)]
pub struct IntBitAndNode {}

impl IntBitAndNode {
    pub fn new() -> Self {
        IntBitAndNode {}
    }
}

#[async_trait]
impl NodeLogic for IntBitAndNode {
    fn get_node(&self) -> Node {
        binary_node("int_bitand", "& (Int)", "Bitwise AND of two integers")
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        binary_run(context, |a, b| a & b).await
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct IntBitOrNode {}

impl IntBitOrNode {
    pub fn new() -> Self {
        IntBitOrNode {}
    }
}

#[async_trait]
impl NodeLogic for IntBitOrNode {
    fn get_node(&self) -> Node {
        binary_node("int_bitor", "| (Int)", "Bitwise OR of two integers")
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        binary_run(context, |a, b| a | b).await
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct IntBitXorNode {}

impl IntBitXorNode {
    pub fn new() -> Self {
        IntBitXorNode {}
    }
}

#[async_trait]
impl NodeLogic for IntBitXorNode {
    fn get_node(&self) -> Node {
        binary_node("int_bitxor", "^ (Int)", "Bitwise XOR of two integers")
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        binary_run(context, |a, b| a ^ b).await
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct IntShiftLeftNode {}

impl IntShiftLeftNode {
    pub fn new() -> Self {
        IntShiftLeftNode {}
    }
}

#[async_trait]
impl NodeLogic for IntShiftLeftNode {
    fn get_node(&self) -> Node {
        shift_node(
            "int_shl",
            "<< (Int)",
            "Shifts the bits of an integer to the left",
        )
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        shift_run(context, |value, shift| {
            value.checked_shl(shift).unwrap_or(0)
        })
        .await
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct IntShiftRightNode {}

impl IntShiftRightNode {
    pub fn new() -> Self {
        IntShiftRightNode {}
    }
}

#[async_trait]
impl NodeLogic for IntShiftRightNode {
    fn get_node(&self) -> Node {
        shift_node(
            "int_shr",
            ">> (Int)",
            "Shifts the bits of an integer to the right",
        )
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        shift_run(context, |value, shift| {
            value.checked_shr(shift).unwrap_or(0)
        })
        .await
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct IntBitNotNode {}

impl IntBitNotNode {
    pub fn new() -> Self {
        IntBitNotNode {}
    }
}

#[async_trait]
impl NodeLogic for IntBitNotNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "int_bitnot",
            "~ (Int)",
            "Inverts every bit of an integer",
            "Math/Int/Bitwise",
        );
        node.add_icon("/flow/icons/sigma.svg");
        node.set_scores(pure_scores());

        node.add_input_pin("integer", "Integer", "Input Integer", VariableType::Integer);
        node.add_output_pin(
            "result",
            "Result",
            "The integer with all bits inverted",
            VariableType::Integer,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let integer: i64 = context.evaluate_pin("integer").await?;
        context.set_pin_value("result", json!(!integer)).await?;
        Ok(())
    }
}
