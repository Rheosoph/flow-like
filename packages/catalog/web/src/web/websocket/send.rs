use flow_like::flow::{
    node::{Node, NodeLogic},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json};

#[cfg(not(feature = "execute"))]
use flow_like::flow::execution::context::ExecutionContext;
#[cfg(feature = "execute")]
use flow_like::flow::execution::{LogLevel, context::ExecutionContext};

use super::WebSocketSession;

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug, schemars::JsonSchema)]
pub enum WebSocketMessageType {
    Text,
    Binary,
}

#[crate::register_node]
#[derive(Default)]
pub struct WebSocketSendNode {}

impl WebSocketSendNode {
    pub fn new() -> Self {
        WebSocketSendNode {}
    }
}

#[async_trait]
impl NodeLogic for WebSocketSendNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "websocket_send",
            "WebSocket Send",
            "Sends a message through an open WebSocket connection",
            "Web/WebSocket",
        );
        node.add_icon("/flow/icons/web.svg");
        node.scores = Some(
            flow_like::flow::node::NodeScores::new()
                .set_privacy(7)
                .set_security(6)
                .set_performance(8)
                .set_governance(5)
                .set_reliability(7)
                .set_cost(9)
                .build(),
        );

        node.add_input_pin(
            "exec_in",
            "Execute",
            "Trigger the send",
            VariableType::Execution,
        );
        node.add_input_pin(
            "session",
            "Session",
            "WebSocket session reference",
            VariableType::Struct,
        )
        .set_schema::<WebSocketSession>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());
        node.add_input_pin(
            "message_type",
            "Message Type",
            "Whether to send as text or binary",
            VariableType::String,
        )
        .set_options(
            PinOptions::new()
                .set_valid_values(vec!["Text".to_string(), "Binary".to_string()])
                .build(),
        )
        .set_default_value(Some(json!("Text")));
        node.add_input_pin(
            "payload",
            "Payload",
            "The message content to send (string for Text, byte array for Binary)",
            VariableType::String,
        );

        node.add_output_pin(
            "exec_out",
            "Done",
            "Fires after the message is sent",
            VariableType::Execution,
        );

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        use futures::SinkExt;
        use tokio_tungstenite::tungstenite::Message;

        context.deactivate_exec_pin("exec_out").await?;

        let session: WebSocketSession = context.evaluate_pin("session").await?;
        let message_type: String = context.evaluate_pin("message_type").await?;

        let conn = super::get_ws_connection(context, &session.ref_id).await?;
        let mut sink = conn.sink.lock().await;

        let msg = match message_type.as_str() {
            "Binary" => {
                let data: Vec<u8> = context.evaluate_pin("payload").await?;
                Message::Binary(data.into())
            }
            _ => {
                let text: String = context.evaluate_pin("payload").await?;
                Message::Text(text.into())
            }
        };

        sink.send(msg).await.map_err(|e| {
            context.log_message(&format!("WebSocket send error: {}", e), LogLevel::Error);
            flow_like_types::anyhow!("WebSocket send failed: {}", e)
        })?;

        context.activate_exec_pin("exec_out").await?;

        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "WebSocket requires the 'execute' feature"
        ))
    }
}
