use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_catalog_core::FlowPath;
use flow_like_types::async_trait;

use super::{GuardedHttpClient, HttpRequest};

#[crate::register_node]
#[derive(Default)]
pub struct HttpDownloadNode {}

impl HttpDownloadNode {
    pub fn new() -> Self {
        HttpDownloadNode {}
    }
}

#[async_trait]
impl NodeLogic for HttpDownloadNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "http_download",
            "HTTP Download",
            "Downloads a file from a url",
            "Web",
        );

        node.set_long_running(true);
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
        node.add_input_pin(
            "flow_path",
            "Path",
            "The path to save the file to",
            VariableType::Struct,
        )
        .set_schema::<FlowPath>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin(
            "exec_success",
            "Success",
            "Execution if the request succeeds",
            VariableType::Execution,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_success").await?;

        let request: HttpRequest = context.evaluate_pin("request").await?;
        let flow_path: FlowPath = context.evaluate_pin("flow_path").await?;

        let client = GuardedHttpClient::new(context.execution_environment())?;
        request
            .download_to_path(&client, &flow_path, context)
            .await?;

        context.activate_exec_pin("exec_success").await?;

        Ok(())
    }
}
