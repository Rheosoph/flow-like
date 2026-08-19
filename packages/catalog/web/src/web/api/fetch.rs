use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json};

use super::{GuardedHttpClient, HttpRequest, HttpResponse};

#[crate::register_node]
#[derive(Default)]
pub struct HttpFetchNode {}

impl HttpFetchNode {
    pub fn new() -> Self {
        HttpFetchNode {}
    }
}

#[async_trait]
impl NodeLogic for HttpFetchNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "http_fetch",
            "API Call",
            "Performs an HTTP request",
            "Web/API",
        );

        node.add_icon("/flow/icons/web.svg");

        node.add_input_pin(
            "exec_in",
            "Execute",
            "Initiate the HTTP request",
            VariableType::Execution,
        );
        node.add_input_pin(
            "request",
            "Request",
            "The HTTP request to perform",
            VariableType::Struct,
        )
        .set_schema::<HttpRequest>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin(
            "exec_success",
            "Success",
            "Execution if the request succeeds",
            VariableType::Execution,
        );
        node.add_output_pin(
            "response",
            "Response",
            "The HTTP response",
            VariableType::Struct,
        )
        .set_schema::<HttpResponse>();

        node.add_output_pin(
            "exec_error",
            "Error",
            "Execution if the request fails",
            VariableType::Execution,
        );
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_success").await?;
        context.activate_exec_pin("exec_error").await?;

        let request: HttpRequest = context.evaluate_pin("request").await?;
        let client = GuardedHttpClient::new(context.execution_environment())?;
        let response = request.trigger(&client).await?;

        context.set_pin_value("response", json!(response)).await?;
        let success = response.is_success();

        if success {
            context.deactivate_exec_pin("exec_error").await?;
            context.activate_exec_pin("exec_success").await?;
        }

        Ok(())
    }
}
