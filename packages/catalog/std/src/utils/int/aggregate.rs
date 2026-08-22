use crate::utils::pure_scores;
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::ValueType,
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json};

fn aggregate_node(id: &str, label: &str, description: &str, output: VariableType) -> Node {
    let mut node = Node::new(id, label, description, "Math/Int/Aggregate");
    node.add_icon("/flow/icons/sigma.svg");
    node.set_scores(pure_scores());

    node.add_input_pin(
        "integers",
        "Integers",
        "Input Integers",
        VariableType::Integer,
    )
    .set_value_type(ValueType::Array);
    node.add_output_pin("result", "Result", description, output);
    node.add_output_pin(
        "empty",
        "Empty",
        "True when the input array held no values",
        VariableType::Boolean,
    );

    node
}

async fn integers(context: &mut ExecutionContext) -> flow_like_types::Result<Vec<i64>> {
    let values: Vec<i64> = context.evaluate_pin("integers").await?;
    context
        .set_pin_value("empty", json!(values.is_empty()))
        .await?;
    Ok(values)
}

#[crate::register_node]
#[derive(Default)]
pub struct IntSumNode {}

impl IntSumNode {
    pub fn new() -> Self {
        IntSumNode {}
    }
}

#[async_trait]
impl NodeLogic for IntSumNode {
    fn get_node(&self) -> Node {
        aggregate_node(
            "int_sum",
            "Sum (Int)",
            "Adds up every integer in an array",
            VariableType::Integer,
        )
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let values = integers(context).await?;
        let sum = values
            .iter()
            .fold(0i64, |acc, value| acc.saturating_add(*value));
        context.set_pin_value("result", json!(sum)).await?;
        Ok(())
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct IntProductNode {}

impl IntProductNode {
    pub fn new() -> Self {
        IntProductNode {}
    }
}

#[async_trait]
impl NodeLogic for IntProductNode {
    fn get_node(&self) -> Node {
        aggregate_node(
            "int_product",
            "Product (Int)",
            "Multiplies every integer in an array",
            VariableType::Integer,
        )
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let values = integers(context).await?;
        let product = values
            .iter()
            .fold(1i64, |acc, value| acc.saturating_mul(*value));
        context
            .set_pin_value("result", json!(if values.is_empty() { 0 } else { product }))
            .await?;
        Ok(())
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct IntAverageNode {}

impl IntAverageNode {
    pub fn new() -> Self {
        IntAverageNode {}
    }
}

#[async_trait]
impl NodeLogic for IntAverageNode {
    fn get_node(&self) -> Node {
        aggregate_node(
            "int_average",
            "Average (Int)",
            "Arithmetic mean of every integer in an array",
            VariableType::Float,
        )
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let values = integers(context).await?;
        let average = if values.is_empty() {
            0.0
        } else {
            values.iter().map(|value| *value as f64).sum::<f64>() / values.len() as f64
        };
        context.set_pin_value("result", json!(average)).await?;
        Ok(())
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct IntMinOfNode {}

impl IntMinOfNode {
    pub fn new() -> Self {
        IntMinOfNode {}
    }
}

#[async_trait]
impl NodeLogic for IntMinOfNode {
    fn get_node(&self) -> Node {
        aggregate_node(
            "int_min_of",
            "Min Of (Int)",
            "Smallest integer in an array",
            VariableType::Integer,
        )
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let values = integers(context).await?;
        context
            .set_pin_value("result", json!(values.iter().copied().min().unwrap_or(0)))
            .await?;
        Ok(())
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct IntMaxOfNode {}

impl IntMaxOfNode {
    pub fn new() -> Self {
        IntMaxOfNode {}
    }
}

#[async_trait]
impl NodeLogic for IntMaxOfNode {
    fn get_node(&self) -> Node {
        aggregate_node(
            "int_max_of",
            "Max Of (Int)",
            "Largest integer in an array",
            VariableType::Integer,
        )
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let values = integers(context).await?;
        context
            .set_pin_value("result", json!(values.iter().copied().max().unwrap_or(0)))
            .await?;
        Ok(())
    }
}
