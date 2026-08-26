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
pub struct GetUrlNode {}

impl GetUrlNode {
    pub fn new() -> Self {
        GetUrlNode {}
    }
}

#[async_trait]
impl NodeLogic for GetUrlNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "http_get_url",
            "Get Url",
            "Gets the url from a http request",
            "Web/API/Request",
        );
        node.set_flowscript_name("http", "getUrl");
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

        node.add_output_pin("url", "Url", "The url of the request", VariableType::String);

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let request: HttpRequest = context.evaluate_pin("request").await?;

        let url = request.url;

        context.set_pin_value("url", json!(url)).await?;

        Ok(())
    }
}
