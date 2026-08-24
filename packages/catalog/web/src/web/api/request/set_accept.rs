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
pub struct SetAcceptNode {}

impl SetAcceptNode {
    pub fn new() -> Self {
        SetAcceptNode {}
    }
}

#[async_trait]
impl NodeLogic for SetAcceptNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "http_set_accept",
            "Set Accept",
            "Sets the Accept header of a http request",
            "Web/API/Request",
        );
        node.set_flowscript_name("http", "setAccept");
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

        node.add_input_pin(
            "accept",
            "Accept",
            "The accept header value",
            VariableType::String,
        )
        .set_default_value(Some(json!("application/json")));

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
        let accept: String = context.evaluate_pin("accept").await?;

        request.set_header("Accept".to_string(), accept);

        context.set_pin_value("request_out", json!(request)).await?;

        Ok(())
    }
}
