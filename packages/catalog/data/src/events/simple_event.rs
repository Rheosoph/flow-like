use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    variable::VariableType,
};
use flow_like_types::async_trait;

#[crate::register_node]
#[derive(Default)]
pub struct SimpleEventNode {}

impl SimpleEventNode {
    pub fn new() -> Self {
        SimpleEventNode {}
    }
}

#[async_trait]
impl NodeLogic for SimpleEventNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "events_simple",
            "Simple Event",
            "A simple event without input or output",
            "Events",
        );
        node.set_flowscript_name("events", "simple");
        node.add_icon("/flow/icons/event.svg");
        node.set_start(true);
        node.set_can_be_referenced_by_fns(true);

        node.add_output_pin(
            "exec_out",
            "Output",
            "Starting an event",
            VariableType::Execution,
        );
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let exec_out_pin = context.get_pin_by_name("exec_out").await?;

        context.activate_exec_pin_ref(&exec_out_pin).await?;

        return Ok(());
    }
}
