use crate::utils::pure_scores;
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::ValueType,
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json};

fn aggregate_node(id: &str, label: &str, description: &str) -> Node {
    let mut node = Node::new(id, label, description, "Math/Float/Aggregate");
    node.add_icon("/flow/icons/sigma.svg");
    node.set_scores(pure_scores());

    node.add_input_pin("floats", "Floats", "Input Floats", VariableType::Float)
        .set_value_type(ValueType::Array);
    node.add_output_pin("result", "Result", description, VariableType::Float);
    node.add_output_pin(
        "empty",
        "Empty",
        "True when the input array held no values",
        VariableType::Boolean,
    );

    node
}

async fn floats(context: &mut ExecutionContext) -> flow_like_types::Result<Vec<f64>> {
    let values: Vec<f64> = context.evaluate_pin("floats").await?;
    context
        .set_pin_value("empty", json!(values.is_empty()))
        .await?;
    Ok(values)
}

fn sorted(values: &[f64]) -> Vec<f64> {
    let mut sorted = values.to_vec();
    sorted.sort_by(|a, b| a.total_cmp(b));
    sorted
}

fn mean(values: &[f64]) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    values.iter().sum::<f64>() / values.len() as f64
}

fn variance(values: &[f64]) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    let mean = mean(values);
    values
        .iter()
        .map(|value| (value - mean).powi(2))
        .sum::<f64>()
        / values.len() as f64
}

/// Linear interpolation between the neighbouring ranks, matching the common
/// "inclusive" percentile definition.
fn percentile(values: &[f64], percentile: f64) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    let sorted = sorted(values);
    let rank = percentile.clamp(0.0, 100.0) / 100.0 * (sorted.len() - 1) as f64;
    let lower = rank.floor() as usize;
    let upper = rank.ceil() as usize;

    if lower == upper {
        return sorted[lower];
    }
    sorted[lower] + (sorted[upper] - sorted[lower]) * (rank - lower as f64)
}

#[crate::register_node]
#[derive(Default)]
pub struct FloatSumNode {}

impl FloatSumNode {
    pub fn new() -> Self {
        FloatSumNode {}
    }
}

#[async_trait]
impl NodeLogic for FloatSumNode {
    fn get_node(&self) -> Node {
        let mut node = aggregate_node(
            "float_sum",
            "Sum (Float)",
            "Adds up every float in an array",
        );
        node.set_flowscript_name("float", "sum");
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let values = floats(context).await?;
        context
            .set_pin_value("result", json!(values.iter().sum::<f64>()))
            .await?;
        Ok(())
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct FloatAverageNode {}

impl FloatAverageNode {
    pub fn new() -> Self {
        FloatAverageNode {}
    }
}

#[async_trait]
impl NodeLogic for FloatAverageNode {
    fn get_node(&self) -> Node {
        let mut node = aggregate_node(
            "float_average",
            "Average (Float)",
            "Arithmetic mean of every float in an array",
        );
        node.set_flowscript_name("float", "average");
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let values = floats(context).await?;
        context
            .set_pin_value("result", json!(mean(&values)))
            .await?;
        Ok(())
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct FloatMinOfNode {}

impl FloatMinOfNode {
    pub fn new() -> Self {
        FloatMinOfNode {}
    }
}

#[async_trait]
impl NodeLogic for FloatMinOfNode {
    fn get_node(&self) -> Node {
        let mut node = aggregate_node(
            "float_min_of",
            "Min Of (Float)",
            "Smallest float in an array",
        );
        node.set_flowscript_name("float", "minOf");
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let values = floats(context).await?;
        let result = values.iter().copied().fold(f64::INFINITY, f64::min);
        context
            .set_pin_value(
                "result",
                json!(if values.is_empty() { 0.0 } else { result }),
            )
            .await?;
        Ok(())
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct FloatMaxOfNode {}

impl FloatMaxOfNode {
    pub fn new() -> Self {
        FloatMaxOfNode {}
    }
}

#[async_trait]
impl NodeLogic for FloatMaxOfNode {
    fn get_node(&self) -> Node {
        let mut node = aggregate_node(
            "float_max_of",
            "Max Of (Float)",
            "Largest float in an array",
        );
        node.set_flowscript_name("float", "maxOf");
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let values = floats(context).await?;
        let result = values.iter().copied().fold(f64::NEG_INFINITY, f64::max);
        context
            .set_pin_value(
                "result",
                json!(if values.is_empty() { 0.0 } else { result }),
            )
            .await?;
        Ok(())
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct FloatMedianNode {}

impl FloatMedianNode {
    pub fn new() -> Self {
        FloatMedianNode {}
    }
}

#[async_trait]
impl NodeLogic for FloatMedianNode {
    fn get_node(&self) -> Node {
        let mut node = aggregate_node(
            "float_median",
            "Median",
            "Middle value of an array, averaging the two middle values for even counts",
        );
        node.set_flowscript_name("float", "median");
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let values = floats(context).await?;
        context
            .set_pin_value("result", json!(percentile(&values, 50.0)))
            .await?;
        Ok(())
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct FloatVarianceNode {}

impl FloatVarianceNode {
    pub fn new() -> Self {
        FloatVarianceNode {}
    }
}

#[async_trait]
impl NodeLogic for FloatVarianceNode {
    fn get_node(&self) -> Node {
        let mut node = aggregate_node(
            "float_variance",
            "Variance",
            "Population variance of every float in an array",
        );
        node.set_flowscript_name("float", "variance");
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let values = floats(context).await?;
        context
            .set_pin_value("result", json!(variance(&values)))
            .await?;
        Ok(())
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct FloatStdDevNode {}

impl FloatStdDevNode {
    pub fn new() -> Self {
        FloatStdDevNode {}
    }
}

#[async_trait]
impl NodeLogic for FloatStdDevNode {
    fn get_node(&self) -> Node {
        let mut node = aggregate_node(
            "float_std_dev",
            "Standard Deviation",
            "Population standard deviation of every float in an array",
        );
        node.set_flowscript_name("float", "stdDev");
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let values = floats(context).await?;
        context
            .set_pin_value("result", json!(variance(&values).sqrt()))
            .await?;
        Ok(())
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct FloatPercentileNode {}

impl FloatPercentileNode {
    pub fn new() -> Self {
        FloatPercentileNode {}
    }
}

#[async_trait]
impl NodeLogic for FloatPercentileNode {
    fn get_node(&self) -> Node {
        let mut node = aggregate_node(
            "float_percentile",
            "Percentile",
            "Value at a percentile of an array, interpolating between neighbours",
        );
        node.set_flowscript_name("float", "percentile");
        node.add_input_pin(
            "percentile",
            "Percentile",
            "Percentile between 0 and 100",
            VariableType::Float,
        )
        .set_default_value(Some(json!(95.0)));

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let requested: f64 = context.evaluate_pin("percentile").await?;
        let values = floats(context).await?;
        context
            .set_pin_value("result", json!(percentile(&values, requested)))
            .await?;
        Ok(())
    }
}
