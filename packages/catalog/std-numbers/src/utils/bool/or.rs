use flow_like::flow::{
    execution::{LogLevel, context::ExecutionContext},
    node::{Node, NodeLogic},
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json};

#[crate::register_node]
#[derive(Default)]
pub struct BoolOr {}

impl BoolOr {
    pub fn new() -> Self {
        BoolOr {}
    }
}

#[async_trait]
impl NodeLogic for BoolOr {
    fn get_node(&self) -> Node {
        let mut node = Node::new("bool_or", "Or", "Boolean Or operation", "Utils/Bool");
        node.set_flowscript_name("bool", "or");
        node.set_receiver("boolean");
        node.add_icon("/flow/icons/bool.svg");

        node.add_input_pin(
            "boolean",
            "Boolean",
            "Input Pin for OR Operation",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(false)));

        node.add_input_pin(
            "boolean",
            "Boolean",
            "Input Pin for OR Operation",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(false)));

        node.add_output_pin(
            "result",
            "Result",
            "OR operation between all boolean inputs",
            VariableType::Boolean,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let mut output_value = false;

        let boolean_pins = context.get_pins_by_name("boolean").await?;

        for pin in boolean_pins {
            let pin = context.evaluate_pin_ref(pin).await?;

            output_value = output_value || pin;
            if output_value {
                break;
            }
        }

        let result = context.get_pin_by_name("result").await?;

        context.log_message(
            &format!("OR Operation Result: {}", output_value),
            LogLevel::Debug,
        );
        context
            .set_pin_ref_value(&result, json!(output_value))
            .await?;

        return Ok(());
    }
}
