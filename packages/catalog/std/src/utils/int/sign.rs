use crate::utils::pure_scores;
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json};

fn unary_node(id: &str, label: &str, description: &str, output: VariableType) -> Node {
    let mut node = Node::new(id, label, description, "Math/Int");
    node.add_icon("/flow/icons/sigma.svg");
    node.set_scores(pure_scores());

    node.add_input_pin("integer", "Integer", "Input Integer", VariableType::Integer);
    node.add_output_pin("result", "Result", description, output);

    node
}

async fn unary_int(
    context: &mut ExecutionContext,
    operation: impl Fn(i64) -> i64,
) -> flow_like_types::Result<()> {
    let integer: i64 = context.evaluate_pin("integer").await?;
    context
        .set_pin_value("result", json!(operation(integer)))
        .await?;
    Ok(())
}

async fn unary_predicate(
    context: &mut ExecutionContext,
    predicate: impl Fn(i64) -> bool,
) -> flow_like_types::Result<()> {
    let integer: i64 = context.evaluate_pin("integer").await?;
    context
        .set_pin_value("result", json!(predicate(integer)))
        .await?;
    Ok(())
}

#[crate::register_node]
#[derive(Default)]
pub struct IntNegateNode {}

impl IntNegateNode {
    pub fn new() -> Self {
        IntNegateNode {}
    }
}

#[async_trait]
impl NodeLogic for IntNegateNode {
    fn get_node(&self) -> Node {
        unary_node(
            "int_negate",
            "Negate (Int)",
            "Flips the sign of an integer",
            VariableType::Integer,
        )
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        unary_int(context, |value| value.saturating_neg()).await
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct IntSignumNode {}

impl IntSignumNode {
    pub fn new() -> Self {
        IntSignumNode {}
    }
}

#[async_trait]
impl NodeLogic for IntSignumNode {
    fn get_node(&self) -> Node {
        unary_node(
            "int_signum",
            "Sign (Int)",
            "Returns -1, 0 or 1 depending on the sign of an integer",
            VariableType::Integer,
        )
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        unary_int(context, |value| value.signum()).await
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct IntIsEvenNode {}

impl IntIsEvenNode {
    pub fn new() -> Self {
        IntIsEvenNode {}
    }
}

#[async_trait]
impl NodeLogic for IntIsEvenNode {
    fn get_node(&self) -> Node {
        unary_node(
            "int_is_even",
            "Is Even",
            "Checks whether an integer is divisible by two",
            VariableType::Boolean,
        )
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        unary_predicate(context, |value| value % 2 == 0).await
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct IntIsOddNode {}

impl IntIsOddNode {
    pub fn new() -> Self {
        IntIsOddNode {}
    }
}

#[async_trait]
impl NodeLogic for IntIsOddNode {
    fn get_node(&self) -> Node {
        unary_node(
            "int_is_odd",
            "Is Odd",
            "Checks whether an integer is not divisible by two",
            VariableType::Boolean,
        )
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        unary_predicate(context, |value| value % 2 != 0).await
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct IntIsPositiveNode {}

impl IntIsPositiveNode {
    pub fn new() -> Self {
        IntIsPositiveNode {}
    }
}

#[async_trait]
impl NodeLogic for IntIsPositiveNode {
    fn get_node(&self) -> Node {
        unary_node(
            "int_is_positive",
            "Is Positive",
            "Checks whether an integer is greater than zero",
            VariableType::Boolean,
        )
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        unary_predicate(context, |value| value.is_positive()).await
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct IntIsNegativeNode {}

impl IntIsNegativeNode {
    pub fn new() -> Self {
        IntIsNegativeNode {}
    }
}

#[async_trait]
impl NodeLogic for IntIsNegativeNode {
    fn get_node(&self) -> Node {
        unary_node(
            "int_is_negative",
            "Is Negative",
            "Checks whether an integer is less than zero",
            VariableType::Boolean,
        )
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        unary_predicate(context, |value| value.is_negative()).await
    }
}
