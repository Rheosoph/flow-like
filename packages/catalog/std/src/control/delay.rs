use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    variable::VariableType,
};
use flow_like_types::{async_trait, tokio::time};

#[crate::register_node]
#[derive(Default)]
pub struct DelayNode {}

impl DelayNode {
    pub fn new() -> Self {
        DelayNode {}
    }
}

#[async_trait]
impl NodeLogic for DelayNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "delay",
            "Delay",
            "Delays execution for a specified amount of time",
            "Control",
        );
        node.set_flowscript_name("control", "delay");

        node.set_long_running(true);
        node.add_icon("/flow/icons/clock.svg");

        node.add_input_pin("exec_in", "Execute", "Execution", VariableType::Execution);
        node.add_input_pin(
            "time",
            "Time (ms)",
            "Delay time in milliseconds",
            VariableType::Float,
        )
        .set_default_value(Some(flow_like_types::json::json!(1000.0)));

        node.add_output_pin("exec_out", "Done", "Execution", VariableType::Execution);

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let delay_time: f64 = context.evaluate_pin("time").await?;

        time::sleep(time::Duration::from_millis(delay_time as u64)).await;

        context.activate_exec_pin("exec_out").await?;

        return Ok(());
    }
}
