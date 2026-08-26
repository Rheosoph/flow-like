use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic, NodeScores},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_model_provider::response::{Choice, Response, ResponseMessage};
use flow_like_types::{async_trait, json::json};

#[crate::register_node]
#[derive(Default)]
pub struct ResponseFromStringNode {}

impl ResponseFromStringNode {
    pub fn new() -> Self {
        ResponseFromStringNode {}
    }
}

#[async_trait]
impl NodeLogic for ResponseFromStringNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "ai_generative_llm_response_from_string",
            "Response From String",
            "Wraps a plain string into a synthetic LLM response object for downstream tooling.",
            "AI/Generative/Response",
        );
        node.set_flowscript_name("ai.response", "fromString");
        node.add_icon("/flow/icons/history.svg");

        // Local synthetic helper, no external calls: very good privacy, security and reliability, low cost.
        node.set_scores(
            NodeScores::new()
                .set_privacy(10)
                .set_security(10)
                .set_performance(9)
                .set_governance(10)
                .set_reliability(10)
                .set_cost(10)
                .build(),
        );

        node.add_input_pin(
            "content",
            "Content",
            "Plain assistant text that should be wrapped into a Response object.",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_output_pin(
            "response",
            "Response",
            "LLM-style Response struct containing the provided content as a single assistant message.",
            VariableType::Struct,
        )
            .set_schema::<Response>()
            .set_options(PinOptions::new().set_enforce_schema(true).build());

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let mut response = Response::new();
        let message: String = context.evaluate_pin("content").await?;

        response.choices.push(Choice {
            finish_reason: String::from("artificial"),
            index: 0,
            logprobs: None,
            message: ResponseMessage {
                role: "assistant".to_string(),
                content: Some(message),
                ..Default::default()
            },
        });

        context.set_pin_value("response", json!(response)).await?;

        Ok(())
    }
}
