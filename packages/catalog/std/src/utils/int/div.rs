use crate::utils::pure_scores;
use flow_like::flow::{
    execution::{LogLevel, context::ExecutionContext},
    node::{Node, NodeLogic},
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json};

fn division_node(id: &str, label: &str, description: &str, output: &str) -> Node {
    let mut node = Node::new(id, label, description, "Math/Int");
    node.add_icon("/flow/icons/sigma.svg");
    node.set_scores(pure_scores());

    node.add_input_pin("integer1", "Integer 1", "Dividend", VariableType::Integer);
    node.add_input_pin("integer2", "Integer 2", "Divisor", VariableType::Integer);
    node.add_output_pin("result", "Result", output, VariableType::Integer);
    node.add_output_pin(
        "success",
        "Success",
        "False when the divisor was zero",
        VariableType::Boolean,
    );

    node
}

async fn division_run(
    context: &mut ExecutionContext,
    operation: impl Fn(i64, i64) -> Option<i64>,
) -> flow_like_types::Result<()> {
    let integer1: i64 = context.evaluate_pin("integer1").await?;
    let integer2: i64 = context.evaluate_pin("integer2").await?;

    let result = operation(integer1, integer2);
    if result.is_none() {
        context.log_message("Divided by Zero", LogLevel::Error);
    }

    context
        .set_pin_value("result", json!(result.unwrap_or(0)))
        .await?;
    context
        .set_pin_value("success", json!(result.is_some()))
        .await?;
    Ok(())
}

#[crate::register_node]
#[derive(Default)]
pub struct IntDivNode {}

impl IntDivNode {
    pub fn new() -> Self {
        IntDivNode {}
    }
}

#[async_trait]
impl NodeLogic for IntDivNode {
    fn get_node(&self) -> Node {
        let mut node = division_node(
            "int_div",
            "// (Int)",
            "Divides two integers and truncates towards zero",
            "Truncated quotient",
        );
        node.set_flowscript_name("int", "div");
        node.set_receiver("integer1");
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        division_run(context, |a, b| a.checked_div(b)).await
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct IntDivEuclidNode {}

impl IntDivEuclidNode {
    pub fn new() -> Self {
        IntDivEuclidNode {}
    }
}

#[async_trait]
impl NodeLogic for IntDivEuclidNode {
    fn get_node(&self) -> Node {
        let mut node = division_node(
            "int_div_euclid",
            "Floor Divide (Int)",
            "Divides two integers and rounds towards negative infinity",
            "Euclidean quotient",
        );
        node.set_flowscript_name("int", "divEuclid");
        node.set_receiver("integer1");
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        division_run(context, |a, b| a.checked_div_euclid(b)).await
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct IntRemEuclidNode {}

impl IntRemEuclidNode {
    pub fn new() -> Self {
        IntRemEuclidNode {}
    }
}

#[async_trait]
impl NodeLogic for IntRemEuclidNode {
    fn get_node(&self) -> Node {
        let mut node = division_node(
            "int_rem_euclid",
            "Modulo (Int)",
            "Remainder that is always positive, unlike the % operator",
            "Non-negative remainder",
        );
        node.set_flowscript_name("int", "remEuclid");
        node.set_receiver("integer1");
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        division_run(context, |a, b| a.checked_rem_euclid(b)).await
    }
}
