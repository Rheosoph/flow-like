use crate::utils::pure_scores;
use flow_like::flow::{
    board::Board,
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::{PinOptions, ValueType},
    variable::VariableType,
};
use flow_like_types::{Value, async_trait, json::json};

const ALPHABETS: [(&str, &str); 5] = [
    (
        "Alphanumeric",
        "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789",
    ),
    ("Lowercase", "abcdefghijklmnopqrstuvwxyz"),
    ("Digits", "0123456789"),
    ("Hex", "0123456789abcdef"),
    (
        "Unambiguous",
        "abcdefghijkmnopqrstuvwxyzABCDEFGHJKLMNPQRSTUVWXYZ23456789",
    ),
];

#[crate::register_node]
#[derive(Default)]
pub struct RandomStringNode {}

impl RandomStringNode {
    pub fn new() -> Self {
        RandomStringNode {}
    }
}

#[async_trait]
impl NodeLogic for RandomStringNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "random_string",
            "Random String",
            "Generates a random string, for example a token or a short code",
            "Utils/Random",
        );
        node.set_flowscript_name("random", "string");
        node.add_icon("/flow/icons/random.svg");
        node.set_scores(pure_scores());

        node.add_input_pin("exec_in", "In", "Trigger Pin", VariableType::Execution);
        node.add_input_pin(
            "length",
            "Length",
            "How many characters to generate",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(16)));
        node.add_input_pin(
            "alphabet",
            "Alphabet",
            "Characters to draw from. Unambiguous leaves out l, I, 1, O and 0",
            VariableType::String,
        )
        .set_default_value(Some(json!("Alphanumeric")))
        .set_options(
            PinOptions::new()
                .set_valid_values(ALPHABETS.iter().map(|(name, _)| name.to_string()).collect())
                .build(),
        );
        node.add_input_pin(
            "custom_alphabet",
            "Custom Alphabet",
            "Use exactly these characters instead, when set",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_output_pin("exec_out", "Out", "", VariableType::Execution);
        node.add_output_pin(
            "result",
            "Result",
            "The generated string",
            VariableType::String,
        );

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        use flow_like_types::rand::{Rng, rng};

        context.deactivate_exec_pin("exec_out").await?;

        let length: i64 = context.evaluate_pin("length").await?;
        let alphabet: String = context.evaluate_pin("alphabet").await?;
        let custom: String = context.evaluate_pin("custom_alphabet").await?;

        let characters: Vec<char> = if custom.is_empty() {
            ALPHABETS
                .iter()
                .find(|(name, _)| *name == alphabet)
                .map(|(_, set)| set.chars().collect())
                .unwrap_or_default()
        } else {
            custom.chars().collect()
        };

        if characters.is_empty() {
            return Err(flow_like_types::anyhow!("The alphabet is empty"));
        }

        // The thread generator is not `Send`, so it must not be alive across an
        // await — build the whole string first, then write the pins.
        let result: String = {
            let mut generator = rng();
            (0..length.clamp(0, 4096))
                .map(|_| characters[generator.random_range(0..characters.len())])
                .collect()
        };

        context.set_pin_value("result", json!(result)).await?;
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
pub struct RandomChoiceNode {}

impl RandomChoiceNode {
    pub fn new() -> Self {
        RandomChoiceNode {}
    }
}

#[async_trait]
impl NodeLogic for RandomChoiceNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "random_choice",
            "Random Choice",
            "Picks elements out of an array at random",
            "Utils/Random",
        );
        node.set_flowscript_name("random", "choice");
        node.add_icon("/flow/icons/random.svg");
        node.set_scores(pure_scores());

        node.add_input_pin("exec_in", "In", "Trigger Pin", VariableType::Execution);
        node.add_input_pin("array_in", "Array", "Your Array", VariableType::Generic)
            .set_value_type(ValueType::Array)
            .set_options(
                PinOptions::new()
                    .set_enforce_generic_value_type(true)
                    .build(),
            );
        node.add_input_pin(
            "count",
            "Count",
            "How many elements to draw",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(1)));
        node.add_input_pin(
            "allow_repeats",
            "Allow Repeats",
            "Draw with replacement, so the same element can come up twice",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(false)));

        node.add_output_pin("exec_out", "Out", "", VariableType::Execution);
        node.add_output_pin(
            "element",
            "Element",
            "The first drawn element",
            VariableType::Generic,
        );
        node.add_output_pin(
            "elements",
            "Elements",
            "Every drawn element",
            VariableType::Generic,
        )
        .set_value_type(ValueType::Array)
        .set_options(
            PinOptions::new()
                .set_enforce_generic_value_type(true)
                .build(),
        );

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        use flow_like_types::rand::{Rng, rng};

        context.deactivate_exec_pin("exec_out").await?;

        let array: Vec<Value> = context.evaluate_pin("array_in").await?;
        let count: i64 = context.evaluate_pin("count").await?;
        let allow_repeats: bool = context.evaluate_pin("allow_repeats").await?;

        // The thread generator is not `Send`, so it must not be alive across an
        // await — draw everything first, then write the pins.
        let drawn: Vec<Value> = {
            let mut pool: Vec<Value> = array;
            let mut generator = rng();
            let mut drawn: Vec<Value> = Vec::new();

            for _ in 0..count.max(0) {
                if pool.is_empty() {
                    break;
                }
                let index = generator.random_range(0..pool.len());
                if allow_repeats {
                    drawn.push(pool[index].clone());
                } else {
                    drawn.push(pool.swap_remove(index));
                }
            }
            drawn
        };

        context
            .set_pin_value("element", drawn.first().cloned().unwrap_or(Value::Null))
            .await?;
        context.set_pin_value("elements", json!(drawn)).await?;
        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "This node requires the 'execute' feature"
        ))
    }

    async fn on_update(&self, node: &mut Node, board: &Board) {
        let _ = node.match_type("array_in", board, Some(ValueType::Array), None);
        let _ = node.match_type("element", board, Some(ValueType::Normal), None);
        let _ = node.match_type("elements", board, Some(ValueType::Array), None);
        node.harmonize_type(vec!["array_in", "element", "elements"], true);
    }
}
