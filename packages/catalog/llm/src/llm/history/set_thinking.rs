use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic, NodeScores},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_model_provider::history::{History, HistoryThinking};
use flow_like_types::{async_trait, json::json};

#[crate::register_node]
#[derive(Default)]
pub struct SetHistoryThinkingNode {}

impl SetHistoryThinkingNode {
    pub fn new() -> Self {
        SetHistoryThinkingNode {}
    }
}

#[async_trait]
impl NodeLogic for SetHistoryThinkingNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "ai_generative_set_history_thinking",
            "Set History Thinking",
            "Stores the thinking level that downstream model invocations should use",
            "AI/Generative/History",
        );
        node.add_icon("/flow/icons/history.svg");
        node.set_scores(
            NodeScores::new()
                .set_privacy(10)
                .set_security(10)
                .set_performance(9)
                .set_reliability(10)
                .set_governance(9)
                .set_cost(8)
                .build(),
        );

        node.add_input_pin(
            "exec_in",
            "Input",
            "Initiate execution",
            VariableType::Execution,
        );

        node.add_input_pin(
            "history",
            "History",
            "Existing chat history to update",
            VariableType::Struct,
        )
        .set_schema::<History>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin(
            "thinking",
            "Thinking",
            "Reasoning effort for downstream models: off, low, mid, or high",
            VariableType::String,
        )
        .set_options(
            PinOptions::new()
                .set_valid_values(vec![
                    "off".into(),
                    "low".into(),
                    "mid".into(),
                    "high".into(),
                ])
                .build(),
        )
        .set_default_value(Some(json!("mid")));

        node.add_output_pin(
            "exec_out",
            "Output",
            "Signals completion after storing the thinking mode",
            VariableType::Execution,
        );

        node.add_output_pin(
            "history_out",
            "History",
            "History updated with the thinking mode",
            VariableType::Struct,
        )
        .set_schema::<History>();

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let mut history: History = context.evaluate_pin("history").await?;
        let thinking: String = context.evaluate_pin("thinking").await?;

        history.thinking = Some(
            thinking
                .parse::<HistoryThinking>()
                .map_err(|err| flow_like_types::anyhow!(err))?,
        );

        context.set_pin_value("history_out", json!(history)).await?;
        context.activate_exec_pin("exec_out").await?;

        Ok(())
    }
}
