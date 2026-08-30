use crate::utils::pure_scores;
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::ValueType,
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json};

fn aggregate_node(id: &str, label: &str, description: &str) -> Node {
    let mut node = Node::new(id, label, description, "Utils/Bool");
    node.add_icon("/flow/icons/bool.svg");
    node.set_scores(pure_scores());

    node.add_input_pin(
        "booleans",
        "Booleans",
        "Input Booleans",
        VariableType::Boolean,
    )
    .set_value_type(ValueType::Array);
    node.add_output_pin("result", "Result", description, VariableType::Boolean);

    node
}

#[crate::register_node]
#[derive(Default)]
pub struct BoolAllNode {}

impl BoolAllNode {
    pub fn new() -> Self {
        BoolAllNode {}
    }
}

#[async_trait]
impl NodeLogic for BoolAllNode {
    fn get_node(&self) -> Node {
        let mut node = aggregate_node(
            "bool_all",
            "All",
            "True when every boolean in the array is true",
        );
        node.set_flowscript_name("bool", "all");
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let booleans: Vec<bool> = context.evaluate_pin("booleans").await?;
        context
            .set_pin_value("result", json!(booleans.iter().all(|value| *value)))
            .await?;
        Ok(())
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct BoolAnyNode {}

impl BoolAnyNode {
    pub fn new() -> Self {
        BoolAnyNode {}
    }
}

#[async_trait]
impl NodeLogic for BoolAnyNode {
    fn get_node(&self) -> Node {
        let mut node = aggregate_node(
            "bool_any",
            "Any",
            "True when at least one boolean in the array is true",
        );
        node.set_flowscript_name("bool", "any");
        node.add_output_pin(
            "count",
            "Count",
            "How many values were true",
            VariableType::Integer,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let booleans: Vec<bool> = context.evaluate_pin("booleans").await?;
        let count = booleans.iter().filter(|value| **value).count() as i64;

        context.set_pin_value("result", json!(count > 0)).await?;
        context.set_pin_value("count", json!(count)).await?;
        Ok(())
    }
}
