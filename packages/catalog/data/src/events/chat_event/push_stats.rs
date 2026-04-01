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
pub struct PushStatsNode {}

impl PushStatsNode {
    pub fn new() -> Self {
        PushStatsNode {}
    }
}

#[async_trait]
impl NodeLogic for PushStatsNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "events_chat_push_stats",
            "Push Stats",
            "Pushes multiple LLM usage stats to the chat at once",
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
            "step_name",
            "Step Name",
            "Label for this batch of stats (e.g. 'Agent Execution', 'Pipeline')",
            VariableType::String,
        );

        node.add_input_pin(
            "input_stats",
            "Stats",
            "Array of LLM usage statistics",
            VariableType::Struct,
        )
        .set_schema::<LLMUsageStats>()
        .set_value_type(flow_like::flow::pin::ValueType::Array)
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
        let stats: Vec<LLMUsageStats> = context.evaluate_pin("input_stats").await?;

        for stat in stats {
            let event = ChatUsageStat {
                step_name: step_name.clone(),
                stats: stat,
            };
            context.stream_response("chat_usage_stat", event).await?;
        }

        context.activate_exec_pin("exec_out").await?;

        Ok(())
    }
}
