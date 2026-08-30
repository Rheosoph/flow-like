use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::{PinOptions, ValueType},
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json};

use crate::web::api::{HttpBody, HttpRequest};

#[crate::register_node]
#[derive(Default)]
pub struct SetBytesBodyNode {}

impl SetBytesBodyNode {
    pub fn new() -> Self {
        SetBytesBodyNode {}
    }
}

#[async_trait]
impl NodeLogic for SetBytesBodyNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "http_set_bytes_body",
            "Set Bytes Body",
            "Sets the body of a http request",
            "Web/API/Request",
        );
        node.set_flowscript_name("http", "setBytesBody");
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
            "body",
            "Body",
            "The body of the request",
            VariableType::Byte,
        )
        .set_value_type(ValueType::Array);

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
        let body: Vec<u8> = context.evaluate_pin("body").await?;

        request.body = Some(HttpBody::Bytes(body));

        context.set_pin_value("request_out", json!(request)).await?;

        Ok(())
    }
}
