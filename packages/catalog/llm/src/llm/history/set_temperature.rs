use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic, NodeScores},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_model_provider::history::History;
use flow_like_types::{async_trait, json::json};

#[crate::register_node]
#[derive(Default)]
pub struct SetHistoryTemperatureNode {}

impl SetHistoryTemperatureNode {
    pub fn new() -> Self {
        SetHistoryTemperatureNode {}
    }
}

#[async_trait]
impl NodeLogic for SetHistoryTemperatureNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "ai_generative_set_history_temperature",
            "Set History Temperature",
            "Stores the sampling temperature used for later LLM invocations",
            "AI/Generative/History",
        );
        node.add_icon("/flow/icons/history.svg");
        node.set_version(1);
        node.set_scores(
            NodeScores::new()
                .set_privacy(10)
                .set_security(10)
                .set_performance(9)
                .set_reliability(10)
                .set_governance(9)
                .set_cost(10)
                .build(),
        );

        node.add_input_pin(
            "exec_in",
            "Input",
            "Initiate Execution",
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
            "temperature",
            "Temperature",
            "Sampling temperature (0-2)",
            VariableType::Float,
        )
        .set_options(PinOptions::new().set_range((0.0, 2.0)).build());

        node.add_output_pin(
            "exec_out",
            "Output",
            "Signals completion once temperature is stored",
            VariableType::Execution,
        );

        node.add_output_pin(
            "history_out",
            "History",
            "History including the temperature setting",
            VariableType::Struct,
        )
        .set_schema::<History>();

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let mut history: History = context.evaluate_pin("history").await?;
        let temperature: f64 = context.evaluate_pin("temperature").await?;

        history.temperature = Some(temperature as f32);

        context.set_pin_value("history_out", json!(history)).await?;
        context.activate_exec_pin("exec_out").await?;

        Ok(())
    }
}
