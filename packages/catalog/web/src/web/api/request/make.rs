use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json};

use crate::web::api::{HttpRequest, Method};

#[crate::register_node]
#[derive(Default)]
pub struct MakeRequestNode {}

impl MakeRequestNode {
    pub fn new() -> Self {
        MakeRequestNode {}
    }
}

#[async_trait]
impl NodeLogic for MakeRequestNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "http_make_request",
            "Make Request",
            "Creates a http request",
            "Web/API/Request",
        );
        node.set_flowscript_name("http", "request");
        node.add_icon("/flow/icons/web.svg");

        node.add_input_pin(
            "method",
            "Method",
            "Http Method GET,POST etc.",
            VariableType::String,
        )
        .set_options(
            PinOptions::new()
                .set_valid_values(vec![
                    "GET".to_string(),
                    "POST".to_string(),
                    "PUT".to_string(),
                    "DELETE".to_string(),
                    "PATCH".to_string(),
                ])
                .build(),
        )
        .set_default_value(Some(json!("GET")));

        node.add_input_pin("url", "URL", "The request URL", VariableType::String);

        node.add_output_pin(
            "request",
            "Request",
            "The http request",
            VariableType::Struct,
        )
        .set_schema::<HttpRequest>();

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let method: String = context.evaluate_pin("method").await?;
        let url: String = context.evaluate_pin("url").await?;

        let method = match method.as_str() {
            "GET" => Method::GET,
            "POST" => Method::POST,
            "PUT" => Method::PUT,
            "DELETE" => Method::DELETE,
            "PATCH" => Method::PATCH,
            _ => Method::GET,
        };

        let request = HttpRequest::new(url, method);

        context.set_pin_value("request", json!(request)).await?;

        Ok(())
    }
}
