use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json};

#[cfg(feature = "execute")]
use fake::Fake;

#[crate::register_node]
#[derive(Default)]
pub struct FakeInteger;

#[async_trait]
impl NodeLogic for FakeInteger {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "faker_integer",
            "Fake Integer",
            "Generates a random integer in a specified range for mocking data",
            "Utils/Faker/Number",
        );
        node.set_flowscript_name("faker.number", "integer");
        node.add_icon("/flow/icons/random.svg");

        node.add_input_pin("exec_in", "Input", "Trigger", VariableType::Execution);
        node.add_input_pin(
            "min",
            "Min",
            "Minimum value (inclusive)",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(0)));
        node.add_input_pin(
            "max",
            "Max",
            "Maximum value (exclusive)",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(100)));
        node.add_output_pin("exec_out", "Output", "Continue", VariableType::Execution);
        node.add_output_pin(
            "number",
            "Number",
            "Generated integer",
            VariableType::Integer,
        );

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let min: i64 = context.evaluate_pin("min").await?;
        let max: i64 = context.evaluate_pin("max").await?;
        let number: i64 = (min..max).fake();
        context.set_pin_value("number", json!(number)).await?;
        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "This node requires the 'execute' feature"
        ))
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct FakeFloat;

#[async_trait]
impl NodeLogic for FakeFloat {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "faker_float",
            "Fake Float",
            "Generates a random float in a specified range for mocking data",
            "Utils/Faker/Number",
        );
        node.set_flowscript_name("faker.number", "float");
        node.add_icon("/flow/icons/random.svg");

        node.add_input_pin("exec_in", "Input", "Trigger", VariableType::Execution);
        node.add_input_pin(
            "min",
            "Min",
            "Minimum value (inclusive)",
            VariableType::Float,
        )
        .set_default_value(Some(json!(0.0)));
        node.add_input_pin(
            "max",
            "Max",
            "Maximum value (exclusive)",
            VariableType::Float,
        )
        .set_default_value(Some(json!(100.0)));
        node.add_output_pin("exec_out", "Output", "Continue", VariableType::Execution);
        node.add_output_pin("number", "Number", "Generated float", VariableType::Float);

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let min: f64 = context.evaluate_pin("min").await?;
        let max: f64 = context.evaluate_pin("max").await?;
        use rand::Rng;
        let number: f64 = {
            let mut rng = rand::rng();
            rng.random_range(min..max)
        };
        context.set_pin_value("number", json!(number)).await?;
        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "This node requires the 'execute' feature"
        ))
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct FakeBoolean;

#[async_trait]
impl NodeLogic for FakeBoolean {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "faker_boolean",
            "Fake Boolean",
            "Generates a random boolean for mocking data",
            "Utils/Faker/Number",
        );
        node.set_flowscript_name("faker.number", "boolean");
        node.add_icon("/flow/icons/random.svg");

        node.add_input_pin("exec_in", "Input", "Trigger", VariableType::Execution);
        node.add_input_pin(
            "probability",
            "True Probability",
            "Probability of true (0.0 to 1.0)",
            VariableType::Float,
        )
        .set_default_value(Some(json!(0.5)));
        node.add_output_pin("exec_out", "Output", "Continue", VariableType::Execution);
        node.add_output_pin("value", "Value", "Generated boolean", VariableType::Boolean);

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let prob: f64 = context.evaluate_pin("probability").await?;
        use rand::Rng;
        let value: bool = {
            let mut rng = rand::rng();
            rng.random::<f64>() < prob
        };
        context.set_pin_value("value", json!(value)).await?;
        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "This node requires the 'execute' feature"
        ))
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct FakeDigit;

#[async_trait]
impl NodeLogic for FakeDigit {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "faker_digit",
            "Fake Digit",
            "Generates a random digit (0-9) for mocking data",
            "Utils/Faker/Number",
        );
        node.set_flowscript_name("faker.number", "digit");
        node.add_icon("/flow/icons/random.svg");

        node.add_input_pin("exec_in", "Input", "Trigger", VariableType::Execution);
        node.add_output_pin("exec_out", "Output", "Continue", VariableType::Execution);
        node.add_output_pin("digit", "Digit", "Generated digit", VariableType::Integer);

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let digit: i64 = (0i64..10i64).fake();
        context.set_pin_value("digit", json!(digit)).await?;
        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "This node requires the 'execute' feature"
        ))
    }
}
