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
pub struct SetContentTypeNode {}

impl SetContentTypeNode {
    pub fn new() -> Self {
        SetContentTypeNode {}
    }
}

#[async_trait]
impl NodeLogic for SetContentTypeNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "http_set_content_type",
            "Set Content-Type",
            "Sets the Content-Type header of a http request",
            "Web/API/Request",
        );
        node.set_flowscript_name("http", "setContentType");
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
            "content_type",
            "Content-Type",
            "The content type value",
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
        let content_type: String = context.evaluate_pin("content_type").await?;

        request.set_header("Content-Type".to_string(), content_type);

        context.set_pin_value("request_out", json!(request)).await?;

        Ok(())
    }
}
