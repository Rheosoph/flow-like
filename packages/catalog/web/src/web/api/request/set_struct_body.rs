use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_types::{Value, async_trait, json::json};

use crate::web::api::{HttpBody, HttpRequest};

#[crate::register_node]
#[derive(Default)]
pub struct SetStructBodyNode {}

impl SetStructBodyNode {
    pub fn new() -> Self {
        SetStructBodyNode {}
    }
}

#[async_trait]
impl NodeLogic for SetStructBodyNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "http_set_struct_body",
            "Set Struct Body",
            "Sets the body of a http request",
            "Web/API/Request",
        );
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
            VariableType::Struct,
        )
        .set_open_schema();

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
        let body: Value = context.evaluate_pin("body").await?;

        request.body = Some(HttpBody::Json(body));

        context.set_pin_value("request_out", json!(request)).await?;

        Ok(())
    }
}
