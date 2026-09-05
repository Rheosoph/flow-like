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
pub struct RandomFloatInRangeNode {}

impl RandomFloatInRangeNode {
    pub fn new() -> Self {
        RandomFloatInRangeNode {}
    }
}

#[async_trait]
impl NodeLogic for RandomFloatInRangeNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "float_random_in_range",
            "Random Float in Range",
            "Generates a random float within a specified range",
            "Math/Float/Random",
        );
        node.set_flowscript_name("float", "randomInRange");
        node.set_receiver("");
        node.add_icon("/flow/icons/random.svg");

        node.add_input_pin("min", "Min", "Minimum Value", VariableType::Float);
        node.add_input_pin("max", "Max", "Maximum Value", VariableType::Float);

        node.add_output_pin(
            "random_float",
            "Random Float",
            "The generated random float",
            VariableType::Float,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let min: f64 = context.evaluate_pin("min").await?;
        let max: f64 = context.evaluate_pin("max").await?;

        if min >= max {
            return Err(anyhow!("min must be less than max"));
        }

        let random_float = {
            let mut rng = rand::rng();
            rng.random_range(min..max)
        };

        context
            .set_pin_value("random_float", json!(random_float))
            .await?;
        Ok(())
    }
}
