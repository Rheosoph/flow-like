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
pub struct GetMethodNode {}

impl GetMethodNode {
    pub fn new() -> Self {
        GetMethodNode {}
    }
}

#[async_trait]
impl NodeLogic for GetMethodNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "http_get_method",
            "Get Method",
            "Gets the method from a http request",
            "Web/API/Request",
        );
        node.set_flowscript_name("http", "getMethod");
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

        node.add_output_pin(
            "method",
            "Method",
            "The method of the request",
            VariableType::String,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let request: HttpRequest = context.evaluate_pin("request").await?;

        let method = request.method;

        context.set_pin_value("method", json!(method)).await?;

        Ok(())
    }
}
