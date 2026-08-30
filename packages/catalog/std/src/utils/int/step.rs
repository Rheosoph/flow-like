use crate::utils::pure_scores;
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json};

fn step_node(id: &str, label: &str, description: &str) -> Node {
    let mut node = Node::new(id, label, description, "Math/Int");
    node.add_icon("/flow/icons/sigma.svg");
    node.set_scores(pure_scores());

    node.add_input_pin("integer", "Integer", "Input Integer", VariableType::Integer);
    node.add_input_pin("step", "Step", "Step width", VariableType::Integer)
        .set_default_value(Some(json!(1)));
    node.add_output_pin("result", "Result", description, VariableType::Integer);

    node
}

#[crate::register_node]
#[derive(Default)]
pub struct IntIncrementNode {}

impl IntIncrementNode {
    pub fn new() -> Self {
        IntIncrementNode {}
    }
}

#[async_trait]
impl NodeLogic for IntIncrementNode {
    fn get_node(&self) -> Node {
        let mut node = step_node(
            "int_increment",
            "Increment",
            "Increases an integer by a step",
        );
        node.set_flowscript_name("int", "increment");
        node.set_receiver("integer");
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let integer: i64 = context.evaluate_pin("integer").await?;
        let step: i64 = context.evaluate_pin("step").await?;
        context
            .set_pin_value("result", json!(integer.saturating_add(step)))
            .await?;
        Ok(())
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct IntDecrementNode {}

impl IntDecrementNode {
    pub fn new() -> Self {
        IntDecrementNode {}
    }
}

#[async_trait]
impl NodeLogic for IntDecrementNode {
    fn get_node(&self) -> Node {
        let mut node = step_node(
            "int_decrement",
            "Decrement",
            "Decreases an integer by a step",
        );
        node.set_flowscript_name("int", "decrement");
        node.set_receiver("integer");
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let integer: i64 = context.evaluate_pin("integer").await?;
        let step: i64 = context.evaluate_pin("step").await?;
        context
            .set_pin_value("result", json!(integer.saturating_sub(step)))
            .await?;
        Ok(())
    }
}
