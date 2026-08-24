use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json};

use crate::web::api::HttpRequest;

#[crate::register_node]
#[derive(Default)]
pub struct SetBearerAuthNode {}

impl SetBearerAuthNode {
    pub fn new() -> Self {
        SetBearerAuthNode {}
    }
}

#[async_trait]
impl NodeLogic for SetBearerAuthNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "http_set_bearer_auth",
            "Set Bearer Auth",
            "Sets the Authorization header using a Bearer token",
            "Web/API/Request",
        );
        node.set_flowscript_name("http", "setBearerAuth");
        node.set_receiver("request");
        node.add_icon("/flow/icons/web.svg");

        node.add_input_pin(
            "request",
            "Request",
            "The http request",
            VariableType::Struct,
        )
        .set_schema::<HttpRequest>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin("token", "Token", "Bearer token", VariableType::String);

        node.add_output_pin(
            "request_out",
            "Request",
            "The http request",
            VariableType::Struct,
        )
        .set_schema::<HttpRequest>();

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let mut request: HttpRequest = context.evaluate_pin("request").await?;
        let token: String = context.evaluate_pin("token").await?;

        request.set_header("Authorization".to_string(), format!("Bearer {}", token));

        context.set_pin_value("request_out", json!(request)).await?;

        Ok(())
    }
}
