use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_model_provider::response::LLMUsageStats;
use flow_like_types::async_trait;

use super::ChatUsageStat;

#[crate::register_node]
#[derive(Default)]
pub struct PushStatNode {}

impl PushStatNode {
    pub fn new() -> Self {
        PushStatNode {}
    }
}

#[async_trait]
impl NodeLogic for PushStatNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "events_chat_push_stat",
            "Push Stat",
            "Pushes a single LLM usage stat to the chat for transparent model usage display",
            "Events/Chat",
        );
        node.set_flowscript_name("chat", "pushStat");
        node.add_icon("/flow/icons/event.svg");
        node.set_event_callback(true);

        node.add_input_pin(
            "exec_in",
            "Input",
            "Initiate Execution",
            VariableType::Execution,
        );

        node.add_input_pin(
            "step_name",
            "Step Name",
            "Label for this step (e.g. 'Summarization', 'Tool Selection')",
            VariableType::String,
        );

        node.add_input_pin("stat", "Stat", "LLM usage statistics", VariableType::Struct)
            .set_schema::<LLMUsageStats>()
            .set_options(PinOptions::new().set_enforce_schema(true).build());

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

        let step_name: String = context.evaluate_pin("step_name").await?;
        let stat: LLMUsageStats = context.evaluate_pin("stat").await?;

        let event = ChatUsageStat {
            step_name,
            stats: stat,
        };

        context.stream_response("chat_usage_stat", event).await?;
        context.activate_exec_pin("exec_out").await?;

        Ok(())
    }
}
