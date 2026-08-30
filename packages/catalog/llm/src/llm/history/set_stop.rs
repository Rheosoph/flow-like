use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic, NodeScores},
    pin::{PinOptions, ValueType},
    variable::VariableType,
};
use flow_like_model_provider::history::History;
use flow_like_types::{async_trait, json::json};
#[crate::register_node]
#[derive(Default)]
pub struct SetHistoryStopWordsNode {}

impl SetHistoryStopWordsNode {
    pub fn new() -> Self {
        SetHistoryStopWordsNode {}
    }
}

#[async_trait]
impl NodeLogic for SetHistoryStopWordsNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "ai_generative_set_history_stop_words",
            "Set Stop Words",
            "Stores one or more stop sequences to truncate future completions",
            "AI/Generative/History",
        );
        node.set_flowscript_name("history", "setStopWords");
        node.set_receiver("history");
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
            "stop_words",
            "Stop Words",
            "Strings that should stop generation",
            VariableType::String,
        )
        .set_value_type(ValueType::Array);

        node.add_output_pin(
            "exec_out",
            "Output",
            "Signals completion after storing stop words",
            VariableType::Execution,
        );

        node.add_output_pin(
            "history_out",
            "History",
            "History updated with stop sequences",
            VariableType::Struct,
        )
        .set_schema::<History>();

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let mut history: History = context.evaluate_pin("history").await?;
        let stop_words: Vec<String> = context.evaluate_pin("stop_words").await?;

        history.stop = Some(stop_words);

        context.set_pin_value("history_out", json!(history)).await?;
        context.activate_exec_pin("exec_out").await?;

        Ok(())
    }
}
