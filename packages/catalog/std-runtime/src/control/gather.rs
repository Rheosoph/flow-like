use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    variable::VariableType,
};
use flow_like_types::async_trait;

#[crate::register_node]
#[derive(Default)]
pub struct GatherExecutionNode {}

impl GatherExecutionNode {
    pub fn new() -> Self {
        GatherExecutionNode {}
    }
}

#[async_trait]
impl NodeLogic for GatherExecutionNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "control_gather",
            "Gather",
            "Gather all execution states",
            "Control/Parallel",
        );
        node.set_flowscript_name("control", "gather");
        node.add_icon("/flow/icons/par_execution.svg");

        node.add_input_pin("exec_in", "Input", "Input Pin", VariableType::Execution);
        node.add_input_pin("exec_in", "Input", "Input Pin", VariableType::Execution);

        node.add_output_pin(
            "exec_done",
            "In Sync",
            "Executes when all inputs are in Sync",
            VariableType::Execution,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_done").await?;
        let input_pins = context.get_pins_by_name("exec_in").await?;

        for pin in &input_pins {
            let value: bool = match context.evaluate_pin_ref(pin.to_owned()).await {
                Ok(value) => value,
                Err(val) => {
                    // This means the pin is not set.
                    context.log_message(
                        &format!("Failed to evaluate pin {}", val),
                        flow_like::flow::execution::LogLevel::Debug,
                    );
                    return Ok(());
                }
            };

            if !value {
                return Ok(());
            }
        }

        context.activate_exec_pin("exec_done").await?;
        for pin in input_pins {
            context
                .set_pin_ref_value(&pin, flow_like_types::Value::Bool(false))
                .await?;
        }

        return Ok(());
    }
}
