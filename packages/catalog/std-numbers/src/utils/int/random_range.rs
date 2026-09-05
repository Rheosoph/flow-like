use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    variable::VariableType,
};
use flow_like_types::{
    anyhow, async_trait,
    json::json,
    rand::{self, Rng},
};

#[crate::register_node]
#[derive(Default)]
pub struct RandomIntegerInRangeNode {}

impl RandomIntegerInRangeNode {
    pub fn new() -> Self {
        RandomIntegerInRangeNode {}
    }
}

#[async_trait]
impl NodeLogic for RandomIntegerInRangeNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "int_random_in_range",
            "Random Integer in Range",
            "Generates a random integer within a specified range",
            "Math/Int/Random",
        );
        node.set_flowscript_name("int", "randomInRange");
        node.set_receiver("");
        node.add_icon("/flow/icons/random.svg");

        node.add_input_pin("min", "Min", "Minimum Value", VariableType::Integer);
        node.add_input_pin("max", "Max", "Maximum Value", VariableType::Integer);

        node.add_output_pin(
            "random_integer",
            "Random Integer",
            "The generated random integer",
            VariableType::Integer,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let min: i64 = context.evaluate_pin("min").await?;
        let max: i64 = context.evaluate_pin("max").await?;

        if min >= max {
            return Err(anyhow!("min must be less than max"));
        }

        let random_integer = {
            let mut rng = rand::rng();
            rng.random_range(min..=max) // Inclusive range for integers
        };

        context
            .set_pin_value("random_integer", json!(random_integer))
            .await?;
        Ok(())
    }
}
