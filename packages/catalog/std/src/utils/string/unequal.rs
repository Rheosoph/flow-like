use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json};

#[crate::register_node]
#[derive(Default)]
pub struct UnEqualStringNode {}

impl UnEqualStringNode {
    pub fn new() -> Self {
        UnEqualStringNode {}
    }
}

#[async_trait]
impl NodeLogic for UnEqualStringNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "not_equal_string",
            "!= (String)",
            "Compares two Strings",
            "Utils/String",
        );
        node.add_icon("/flow/icons/string.svg");

        node.add_input_pin("string", "String", "Input", VariableType::String);
        node.add_input_pin("string", "String", "Input", VariableType::String);

        node.add_output_pin(
            "unequal",
            "Is Unequal?",
            "Are the strings equal?",
            VariableType::Boolean,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let string_pins = context.get_pins_by_name("string").await?;
        let mut equal = true;
        let mut value = None;

        for pin in string_pins {
            let pin: String = context.evaluate_pin_ref(pin).await?;
            if let Some(value) = &value {
                if value != &pin {
                    equal = false;
                    break;
                }

                continue;
            }

            value = Some(pin);
        }

        context.set_pin_value("unequal", json!(!equal)).await?;
        Ok(())
    }
}
