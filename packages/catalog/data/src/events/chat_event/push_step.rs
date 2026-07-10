use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json};

use super::{CachedChatResponse, ChatStreamingResponse};

#[crate::register_node]
#[derive(Default)]
pub struct PushStepNode {}

impl PushStepNode {
    pub fn new() -> Self {
        PushStepNode {}
    }
}

#[async_trait]
impl NodeLogic for PushStepNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "events_chat_push_step",
            "Push Step",
            "Starts a new plan step with title and description",
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

        node.add_input_pin("title", "Title", "Step title", VariableType::String);

        node.add_input_pin(
            "description",
            "Description",
            "Step description (optional)",
            VariableType::String,
        );

        node.add_output_pin(
            "exec_out",
            "Output",
            "Done with the Execution",
            VariableType::Execution,
        );

        node.add_output_pin(
            "step_id",
            "Step ID",
            "The ID of the created step",
            VariableType::Integer,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let title: String = context.evaluate_pin("title").await?;
        let description: Option<String> = context.evaluate_pin("description").await.ok();

        let cached_response = CachedChatResponse::load(context).await?;
        let step_id = {
            let mut mutable_reasoning = cached_response.reasoning.lock().await;

            // Increment step counter
            mutable_reasoning.current_step += 1;
            let step_id = mutable_reasoning.current_step;

            // Add new step to plan
            let step_description = description.unwrap_or_default();
            mutable_reasoning
                .plan
                .push((step_id, format!("{}: {}", title, step_description)));

            // Clear current message for new step
            mutable_reasoning.current_message.clear();

            step_id
        };

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

        context.set_pin_value("step_id", json!(step_id)).await?;
        context.activate_exec_pin("exec_out").await?;

        return Ok(());
    }
}
