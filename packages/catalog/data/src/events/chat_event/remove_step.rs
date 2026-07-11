use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    variable::VariableType,
};
use flow_like_types::async_trait;

use super::{CachedChatResponse, ChatStreamingResponse};

#[crate::register_node]
#[derive(Default)]
pub struct RemoveStepNode {}

impl RemoveStepNode {
    pub fn new() -> Self {
        RemoveStepNode {}
    }
}

#[async_trait]
impl NodeLogic for RemoveStepNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "events_chat_remove_step",
            "Remove Step",
            "Removes a step from the plan by its ID",
            "Events/Chat",
        );
        node.add_icon("/flow/icons/event.svg");
        node.set_event_callback(true);

        node.add_input_pin(
            "exec_in",
            "Input",
            "Initiate Execution",
            VariableType::Execution,
        );

        node.add_input_pin(
            "step_id",
            "Step ID",
            "ID of the step to remove",
            VariableType::Integer,
        );

        node.add_output_pin(
            "exec_out",
            "Output",
            "Done with the Execution",
            VariableType::Execution,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let step_id: u32 = context.evaluate_pin("step_id").await?;

        let cached_response = CachedChatResponse::load(context).await?;
        {
            let mut mutable_reasoning = cached_response.reasoning.lock().await;

            // Remove step from plan
            mutable_reasoning.plan.retain(|(id, _)| *id != step_id);
        }

        let reasoning_ref = cached_response.reasoning.lock().await;
        let streaming_response = ChatStreamingResponse {
            actions: vec![],
            attachments: vec![],
            chunk: None,
            plan: Some(reasoning_ref.clone()),
            widgets: vec![],
        };
        drop(reasoning_ref);

        context
            .stream_response("chat_stream_partial", streaming_response)
            .await?;
        context.activate_exec_pin("exec_out").await?;

        return Ok(());
    }
}
