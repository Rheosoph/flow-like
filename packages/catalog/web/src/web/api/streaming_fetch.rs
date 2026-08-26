use ahash::AHashSet;
use flow_like::flow::{
    execution::{
        LogLevel, context::ExecutionContext, internal_node::InternalNode, log::LogMessage,
    },
    node::{Node, NodeLogic},
    pin::{PinOptions, ValueType},
    variable::VariableType,
};
use flow_like_types::{
    async_trait,
    json::json,
    sync::{DashMap, Mutex},
};
use std::sync::Arc;

use super::{GuardedHttpClient, HttpRequest, HttpResponse, StreamingCallback};

#[crate::register_node]
#[derive(Default)]
pub struct StreamingHttpFetchNode {}

impl StreamingHttpFetchNode {
    pub fn new() -> Self {
        StreamingHttpFetchNode {}
    }
}

#[async_trait]
impl NodeLogic for StreamingHttpFetchNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "streaming_http_fetch",
            "Streaming API Call",
            "Performs an HTTP request",
            "Web/API",
        );
        node.set_flowscript_name("http", "fetchStreaming");
        node.set_receiver("request");

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
            "streaming_exec",
            "On Stream",
            "Intermediate Result",
            VariableType::Execution,
        );
        node.add_output_pin(
            "streaming_response",
            "Stream Response",
            "The HTTP response",
            VariableType::Byte,
        )
        .set_value_type(ValueType::Array);

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

        let streaming_pin = context.get_pin_by_name("streaming_exec").await?;
        let streaming_response_pin = context.get_pin_by_name("streaming_response").await?;
        context.activate_exec_pin_ref(&streaming_pin).await?;

        let request: HttpRequest = context.evaluate_pin("request").await?;
        let client = GuardedHttpClient::new(context.execution_environment())?;
        let connected_nodes = Arc::new(DashMap::new());
        let connected = streaming_pin.get_connected_nodes();
        for node in connected {
            let context = Arc::new(Mutex::new(context.create_sub_context(&node).await));
            connected_nodes.insert(node.node.lock().await.id.clone(), context);
        }

        let parent_node_id = context.node.node.lock().await.id.clone();
        let callback: StreamingCallback = Arc::new(move |response: flow_like_types::Bytes| {
            let response = response.to_vec();
            let streaming_response_pin = streaming_response_pin.clone();
            let connected_nodes = connected_nodes.clone();
            let parent_node_id = parent_node_id.clone();

            Box::pin(async move {
                let mut recursion_guard = AHashSet::new();
                recursion_guard.insert(parent_node_id.clone());
                streaming_response_pin.set_value(json!(response)).await;
                for entry in connected_nodes.iter() {
                    let (_id, context) = entry.pair();
                    let mut context = context.lock().await;
                    let mut message =
                        LogMessage::new("Streaming Intermediate Response", LogLevel::Debug, None);
                    let run = InternalNode::trigger(
                        &mut context,
                        &mut Some(recursion_guard.clone()),
                        true,
                    )
                    .await;
                    message.end();
                    context.log(message);
                    context.end_trace();
                    match run {
                        Ok(_) => {}
                        Err(err) => {
                            println!("Error running stream node {:?}", err);
                        }
                    }
                }
                Ok(())
            })
        });

        let response = request.streaming_trigger(&client, Some(callback)).await?;
        context.deactivate_exec_pin_ref(&streaming_pin).await?;

        context.set_pin_value("response", json!(response)).await?;
        let success = response.is_success();

        if success {
            context.deactivate_exec_pin("exec_error").await?;
            context.activate_exec_pin("exec_success").await?;
        }

        Ok(())
    }
}
