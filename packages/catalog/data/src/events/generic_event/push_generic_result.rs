use std::sync::Arc;

use flow_like::flow::board::Board;
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    variable::VariableType,
};
use flow_like_types::async_trait;
#[crate::register_node]
#[derive(Default)]
pub struct ReturnGenericResultNode {}

impl ReturnGenericResultNode {
    pub fn new() -> Self {
        ReturnGenericResultNode {}
    }
}

#[async_trait]
impl NodeLogic for ReturnGenericResultNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "events_generic_return_result",
            "Return Generic Result",
            "Return a result",
            "Events/Generic",
        );
        node.add_icon("/flow/icons/event.svg");
        node.set_event_callback(true);

        node.add_input_pin(
            "exec_in",
            "Input",
            "Initiate Execution",
            VariableType::Execution,
        );

        node.add_input_pin("response", "Result", "Chat Response", VariableType::Generic);

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let result: flow_like_types::Value = context.evaluate_pin("response").await?;
        context
            .stream_response("generic_result", result.clone())
            .await?;
        context.set_result(result);

        return Ok(());
    }

    async fn on_update(&self, node: &mut Node, board: &Board) {
        let _ = node.match_type("response", board, None, None);
    }
}
