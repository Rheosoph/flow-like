use crate::utils::pure_scores;
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json};

fn greatest_common_divisor(a: i64, b: i64) -> i64 {
    let mut a = a.saturating_abs();
    let mut b = b.saturating_abs();
    while b != 0 {
        let remainder = a % b;
        a = b;
        b = remainder;
    }
    a
}

fn pair_node(id: &str, label: &str, description: &str) -> Node {
    let mut node = Node::new(id, label, description, "Math/Int");
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

#[crate::register_node]
#[derive(Default)]
pub struct IntGcdNode {}

impl IntGcdNode {
    pub fn new() -> Self {
        IntGcdNode {}
    }
}

#[async_trait]
impl NodeLogic for IntGcdNode {
    fn get_node(&self) -> Node {
        pair_node(
            "int_gcd",
            "Greatest Common Divisor",
            "Largest integer that divides both inputs",
        )
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let integer1: i64 = context.evaluate_pin("integer1").await?;
        let integer2: i64 = context.evaluate_pin("integer2").await?;
        context
            .set_pin_value("result", json!(greatest_common_divisor(integer1, integer2)))
            .await?;
        Ok(())
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct IntLcmNode {}

impl IntLcmNode {
    pub fn new() -> Self {
        IntLcmNode {}
    }
}

#[async_trait]
impl NodeLogic for IntLcmNode {
    fn get_node(&self) -> Node {
        pair_node(
            "int_lcm",
            "Least Common Multiple",
            "Smallest positive integer that both inputs divide",
        )
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let integer1: i64 = context.evaluate_pin("integer1").await?;
        let integer2: i64 = context.evaluate_pin("integer2").await?;

        let divisor = greatest_common_divisor(integer1, integer2);
        let result = if divisor == 0 {
            0
        } else {
            (integer1 / divisor)
                .saturating_mul(integer2)
                .saturating_abs()
        };

        context.set_pin_value("result", json!(result)).await?;
        Ok(())
    }
}
