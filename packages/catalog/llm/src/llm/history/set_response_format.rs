use flow_like::flow::{
    board::Board,
    execution::context::ExecutionContext,
    node::{Node, NodeLogic, NodeScores},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_model_provider::history::{History, ResponseFormat};
use flow_like_types::{Value, async_trait, json::json};

#[crate::register_node]
#[derive(Default)]
pub struct SetHistoryResponseFormatNode {}

impl SetHistoryResponseFormatNode {
    pub fn new() -> Self {
        SetHistoryResponseFormatNode {}
    }
}

#[async_trait]
impl NodeLogic for SetHistoryResponseFormatNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "ai_generative_set_history_response_format",
            "Set Response Format",
            "Configures the structured response format expected from later LLM calls",
            "AI/Generative/History",
        );
        node.set_flowscript_name("history", "setResponseFormat");
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
            "response_format",
            "Response Format",
            "JSON schema or `string` that shapes responses",
            VariableType::Generic,
        )
        .set_options(
            PinOptions::new()
                .set_enforce_generic_value_type(true)
                .build(),
        );

        node.add_output_pin(
            "exec_out",
            "Output",
            "Signals completion after storing the format",
            VariableType::Execution,
        );

        node.add_output_pin(
            "history_out",
            "History",
            "History updated with the response format",
            VariableType::Struct,
        )
        .set_schema::<History>();

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let mut history: History = context.evaluate_pin("history").await?;
        let response_format: Value = context.evaluate_pin("response_format").await?;

        match response_format {
            Value::String(string) => {
                history.response_format = Some(ResponseFormat::String(string));
            }
            _ => {
                history.response_format = Some(ResponseFormat::Object(response_format));
            }
        }

        context.set_pin_value("history_out", json!(history)).await?;
        context.activate_exec_pin("exec_out").await?;

        Ok(())
    }

    async fn on_update(&self, node: &mut Node, board: &Board) {
        let match_type = node
            .match_type("response_format", board, None, None)
            .unwrap_or(VariableType::Generic);
        if match_type != VariableType::String
            && match_type != VariableType::Struct
            && let Some(pin) = node.get_pin_mut_by_name("response_format")
        {
            pin.depends_on.clear();
        }
    }
}
