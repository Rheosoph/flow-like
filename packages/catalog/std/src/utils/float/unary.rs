use crate::utils::pure_scores;
use flow_like::flow::{execution::context::ExecutionContext, node::Node, variable::VariableType};
use flow_like_types::json::json;

pub fn unary_node(id: &str, label: &str, description: &str, category: &str) -> Node {
    let mut node = Node::new(id, label, description, category);
    node.add_icon("/flow/icons/sigma.svg");
    node.set_scores(pure_scores());

    node.add_input_pin("float", "Float", "Input Float", VariableType::Float);
    node.add_output_pin("result", "Result", description, VariableType::Float);

    node
}

/// Applies a unary float operation and fails loudly when the result leaves the
/// real numbers — JSON cannot carry NaN or infinity.
pub async fn unary_run(
    context: &mut ExecutionContext,
    label: &str,
    operation: impl Fn(f64) -> f64,
) -> flow_like_types::Result<()> {
    let float: f64 = context.evaluate_pin("float").await?;
    let result = operation(float);

    if !result.is_finite() {
        return Err(flow_like_types::anyhow!(
            "{label} is not defined for {float}"
        ));
    }

    context.set_pin_value("result", json!(result)).await?;
    Ok(())
}

pub fn binary_node(id: &str, label: &str, description: &str, category: &str) -> Node {
    let mut node = Node::new(id, label, description, category);
    node.add_icon("/flow/icons/sigma.svg");
    node.set_scores(pure_scores());

    node.add_input_pin("float1", "Float 1", "Input Float", VariableType::Float);
    node.add_input_pin("float2", "Float 2", "Input Float", VariableType::Float);
    node.add_output_pin("result", "Result", description, VariableType::Float);

    node
}

pub async fn binary_run(
    context: &mut ExecutionContext,
    label: &str,
    operation: impl Fn(f64, f64) -> f64,
) -> flow_like_types::Result<()> {
    let float1: f64 = context.evaluate_pin("float1").await?;
    let float2: f64 = context.evaluate_pin("float2").await?;
    let result = operation(float1, float2);

    if !result.is_finite() {
        return Err(flow_like_types::anyhow!(
            "{label} is not defined for {float1} and {float2}"
        ));
    }

    context.set_pin_value("result", json!(result)).await?;
    Ok(())
}
