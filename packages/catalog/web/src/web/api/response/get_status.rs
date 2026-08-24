use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json};

use crate::web::api::HttpResponse;

#[crate::register_node]
#[derive(Default)]
pub struct GetStatusNode {}

impl GetStatusNode {
    pub fn new() -> Self {
        GetStatusNode {}
    }
}

#[async_trait]
impl NodeLogic for GetStatusNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "http_response_get_status",
            "Get Status Code",
            "Gets the status code from a http response",
            "Web/API/Response",
        );
        node.set_flowscript_name("http", "status");
        node.set_receiver("response");
        node.add_icon("/flow/icons/web.svg");

        node.add_input_pin(
            "response",
            "Response",
            "The http response",
            VariableType::Struct,
        )
        .set_schema::<HttpResponse>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin(
            "status_code",
            "Status Code",
            "The status code of the response",
            VariableType::Integer,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let response: HttpResponse = context.evaluate_pin("response").await?;

        let status_code = response.get_status_code();

        context
            .set_pin_value("status_code", json!(status_code))
            .await?;

        Ok(())
    }
}
