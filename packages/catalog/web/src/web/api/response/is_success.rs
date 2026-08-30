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
pub struct IsSuccessNode {}

impl IsSuccessNode {
    pub fn new() -> Self {
        IsSuccessNode {}
    }
}

#[async_trait]
impl NodeLogic for IsSuccessNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "http_response_is_success",
            "Is Success",
            "Checks if the status code of a http response is a success",
            "Web/API/Response",
        );
        node.set_flowscript_name("http", "isSuccess");
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
            "is_success",
            "Is Success",
            "True if the status code is a success",
            VariableType::Boolean,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let response: HttpResponse = context.evaluate_pin("response").await?;

        let is_success = response.is_success();

        context
            .set_pin_value("is_success", json!(is_success))
            .await?;

        Ok(())
    }
}
