use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::{PinOptions, ValueType},
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json};

use crate::web::api::HttpRequest;

#[crate::register_node]
#[derive(Default)]
pub struct SetHeadersNode {}

impl SetHeadersNode {
    pub fn new() -> Self {
        SetHeadersNode {}
    }
}

#[async_trait]
impl NodeLogic for SetHeadersNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "http_set_headers",
            "Set Headers",
            "Sets the headers of a http request",
            "Web/API/Request",
        );
        node.set_flowscript_name("http", "setHeaders");
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
            "headers",
            "Headers",
            "The headers of the request",
            VariableType::String,
        )
        .set_value_type(ValueType::HashMap);

        node.add_input_pin(
            "merge",
            "Merge",
            "Merge with existing headers instead of replacing",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(true)));

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
        let headers: std::collections::HashMap<String, String> =
            context.evaluate_pin("headers").await?;
        let merge: bool = context.evaluate_pin("merge").await?;

        if merge {
            if request.headers.is_none() {
                request.headers = Some(std::collections::HashMap::new());
            }

            if let Some(existing) = request.headers.as_mut() {
                existing.extend(headers);
            }
        } else {
            request.headers = Some(headers);
        }

        context.set_pin_value("request_out", json!(request)).await?;

        Ok(())
    }
}
