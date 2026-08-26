use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::{PinOptions, ValueType},
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json};

use crate::web::api::HttpResponse;

#[crate::register_node]
#[derive(Default)]
pub struct ToBytesNode {}

impl ToBytesNode {
    pub fn new() -> Self {
        ToBytesNode {}
    }
}

#[async_trait]
impl NodeLogic for ToBytesNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "http_response_to_bytes",
            "To Bytes",
            "Gets the body of a http response as bytes",
            "Web/API/Response",
        );
        node.set_flowscript_name("http", "toBytes");
        node.set_receiver("response");
        node.add_icon("/flow/icons/web.svg");

        node.add_input_pin("exec_in", "", "", VariableType::Execution);

        node.add_input_pin(
            "response",
            "Response",
            "The http response",
            VariableType::Struct,
        )
        .set_schema::<HttpResponse>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin(
            "exec_out",
            "Exec Out",
            "Called when the node is finished",
            VariableType::Execution,
        );

        node.add_output_pin(
            "bytes",
            "Bytes",
            "The body of the response as bytes",
            VariableType::Byte,
        )
        .set_value_type(ValueType::Array);

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        let response: HttpResponse = context.evaluate_pin("response").await?;

        let bytes = response.to_bytes();

        context.set_pin_value("bytes", json!(bytes)).await?;

        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }
}
