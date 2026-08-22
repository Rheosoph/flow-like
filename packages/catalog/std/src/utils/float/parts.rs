use super::unary::{unary_node, unary_run};
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
};
use flow_like_types::async_trait;

#[crate::register_node]
#[derive(Default)]
pub struct FloatTruncNode {}

impl FloatTruncNode {
    pub fn new() -> Self {
        FloatTruncNode {}
    }
}

#[async_trait]
impl NodeLogic for FloatTruncNode {
    fn get_node(&self) -> Node {
        unary_node(
            "float_trunc",
            "Truncate",
            "Drops the fractional part of a float",
            "Math/Float",
        )
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        unary_run(context, "Truncate", f64::trunc).await
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct FloatFractNode {}

impl FloatFractNode {
    pub fn new() -> Self {
        FloatFractNode {}
    }
}

#[async_trait]
impl NodeLogic for FloatFractNode {
    fn get_node(&self) -> Node {
        unary_node(
            "float_fract",
            "Fraction",
            "Keeps only the fractional part of a float",
            "Math/Float",
        )
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        unary_run(context, "Fraction", f64::fract).await
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct FloatSignumNode {}

impl FloatSignumNode {
    pub fn new() -> Self {
        FloatSignumNode {}
    }
}

#[async_trait]
impl NodeLogic for FloatSignumNode {
    fn get_node(&self) -> Node {
        unary_node(
            "float_signum",
            "Sign (Float)",
            "Returns -1 or 1 depending on the sign of a float",
            "Math/Float",
        )
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        unary_run(context, "Sign", f64::signum).await
    }
}
