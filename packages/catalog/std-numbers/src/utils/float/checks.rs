use crate::utils::pure_scores;
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    variable::VariableType,
};
use flow_like_types::{Value, async_trait, json::json};

fn check_node(id: &str, label: &str, description: &str) -> Node {
    let mut node = Node::new(id, label, description, "Math/Float/Comparison");
    node.add_icon("/flow/icons/sigma.svg");
    node.set_scores(pure_scores());

    node.add_input_pin("float", "Float", "Input Float", VariableType::Float);
    node.add_output_pin("result", "Result", description, VariableType::Boolean);

    node
}

/// Reads the pin as raw JSON: results that left the real numbers cannot be
/// carried as JSON numbers and arrive as null.
async fn optional_float(context: &mut ExecutionContext) -> flow_like_types::Result<Option<f64>> {
    let value: Value = context.evaluate_pin("float").await?;
    Ok(value.as_f64())
}

#[crate::register_node]
#[derive(Default)]
pub struct FloatIsNanNode {}

impl FloatIsNanNode {
    pub fn new() -> Self {
        FloatIsNanNode {}
    }
}

#[async_trait]
impl NodeLogic for FloatIsNanNode {
    fn get_node(&self) -> Node {
        let mut node = check_node(
            "float_is_nan",
            "Is Not A Number",
            "True when the value is missing or not a real number",
        );
        node.set_flowscript_name("float", "isNan");
        node.set_receiver("float");
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let value = optional_float(context).await?;
        let result = value.is_none_or(|value| value.is_nan());
        context.set_pin_value("result", json!(result)).await?;
        Ok(())
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct FloatIsFiniteNode {}

impl FloatIsFiniteNode {
    pub fn new() -> Self {
        FloatIsFiniteNode {}
    }
}

#[async_trait]
impl NodeLogic for FloatIsFiniteNode {
    fn get_node(&self) -> Node {
        let mut node = check_node(
            "float_is_finite",
            "Is Finite",
            "True when the value is a real, finite number",
        );
        node.set_flowscript_name("float", "isFinite");
        node.set_receiver("float");
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let value = optional_float(context).await?;
        let result = value.is_some_and(|value| value.is_finite());
        context.set_pin_value("result", json!(result)).await?;
        Ok(())
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct FloatIsInfiniteNode {}

impl FloatIsInfiniteNode {
    pub fn new() -> Self {
        FloatIsInfiniteNode {}
    }
}

#[async_trait]
impl NodeLogic for FloatIsInfiniteNode {
    fn get_node(&self) -> Node {
        let mut node = check_node(
            "float_is_infinite",
            "Is Infinite",
            "True when the value is positive or negative infinity",
        );
        node.set_flowscript_name("float", "isInfinite");
        node.set_receiver("float");
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let value = optional_float(context).await?;
        let result = value.is_some_and(|value| value.is_infinite());
        context.set_pin_value("result", json!(result)).await?;
        Ok(())
    }
}
