use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic, NodeScores},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_model_provider::response::ResponseMessage;
use flow_like_types::{async_trait, json::json};

#[crate::register_node]
#[derive(Default)]
pub struct GetRoleNode {}

impl GetRoleNode {
    pub fn new() -> Self {
        GetRoleNode {}
    }
}

#[async_trait]
impl NodeLogic for GetRoleNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "ai_generative_llm_response_message_get_role",
            "Get Role",
            "Extracts the author role string from a response message",
            "AI/Generative/Response/Message",
        );
        node.set_flowscript_name("ai.response", "getRole");
        node.set_receiver("message");
        node.add_icon("/flow/icons/history.svg");
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
            "message",
            "Message",
            "Message to extract the role from",
            VariableType::Struct,
        )
        .set_schema::<ResponseMessage>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin(
            "role",
            "Role",
            "Role string from the message",
            VariableType::String,
        )
        .set_options(
            PinOptions::new()
                .set_valid_values(vec![
                    "system".to_string(),
                    "user".to_string(),
                    "assistant".to_string(),
                ])
                .build(),
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let message: ResponseMessage = context.evaluate_pin("message").await?;

        context.set_pin_value("role", json!(message.role)).await?;
        Ok(())
    }
}
