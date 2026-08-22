use crate::utils::pure_scores;
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json};

fn bits_node(id: &str, label: &str, description: &str) -> Node {
    let mut node = Node::new(id, label, description, "Math/Int/Bitwise");
    node.add_icon("/flow/icons/sigma.svg");
    node.set_scores(pure_scores());

    node.add_input_pin("integer", "Integer", "Input Integer", VariableType::Integer);
    node.add_output_pin("result", "Result", description, VariableType::Integer);

    node
}

async fn bits_run(
    context: &mut ExecutionContext,
    operation: impl Fn(i64) -> u32,
) -> flow_like_types::Result<()> {
    let integer: i64 = context.evaluate_pin("integer").await?;
    context
        .set_pin_value("result", json!(operation(integer) as i64))
        .await?;
    Ok(())
}

#[crate::register_node]
#[derive(Default)]
pub struct IntCountOnesNode {}

impl IntCountOnesNode {
    pub fn new() -> Self {
        IntCountOnesNode {}
    }
}

#[async_trait]
impl NodeLogic for IntCountOnesNode {
    fn get_node(&self) -> Node {
        bits_node(
            "int_count_ones",
            "Count Ones",
            "Number of bits that are set to one",
        )
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        bits_run(context, |value| value.count_ones()).await
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct IntLeadingZerosNode {}

impl IntLeadingZerosNode {
    pub fn new() -> Self {
        IntLeadingZerosNode {}
    }
}

#[async_trait]
impl NodeLogic for IntLeadingZerosNode {
    fn get_node(&self) -> Node {
        bits_node(
            "int_leading_zeros",
            "Leading Zeros",
            "Number of zero bits before the highest set bit",
        )
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        bits_run(context, |value| value.leading_zeros()).await
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct IntTrailingZerosNode {}

impl IntTrailingZerosNode {
    pub fn new() -> Self {
        IntTrailingZerosNode {}
    }
}

#[async_trait]
impl NodeLogic for IntTrailingZerosNode {
    fn get_node(&self) -> Node {
        bits_node(
            "int_trailing_zeros",
            "Trailing Zeros",
            "Number of zero bits after the lowest set bit",
        )
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        bits_run(context, |value| value.trailing_zeros()).await
    }
}
