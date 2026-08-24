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
pub struct GetHeaderNode {}

impl GetHeaderNode {
    pub fn new() -> Self {
        GetHeaderNode {}
    }
}

#[async_trait]
impl NodeLogic for GetHeaderNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "http_get_header",
            "Get Header",
            "Gets a header from a http request",
            "Web/API/Request",
        );
        node.set_flowscript_name("http", "getHeader");
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
            "header",
            "Header",
            "The header to get",
            VariableType::String,
        );

        node.add_output_pin(
            "found",
            "Found",
            "True if the header was found",
            VariableType::Boolean,
        );

        node.add_output_pin(
            "value",
            "Value",
            "The value of the header",
            VariableType::String,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let request: HttpRequest = context.evaluate_pin("request").await?;
        let header: String = context.evaluate_pin("header").await?;

        let value = request
            .headers
            .and_then(|headers| headers.get(&header).cloned());

        context
            .set_pin_value("found", json!(value.is_some()))
            .await?;
        context
            .set_pin_value("value", json!(value.unwrap_or_default()))
            .await?;

        Ok(())
    }
}
