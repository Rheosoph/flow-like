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

use super::TcpSession;

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug, schemars::JsonSchema)]
pub enum TcpMessageType {
    Text,
    Binary,
}

#[crate::register_node]
#[derive(Default)]
pub struct TcpSendNode {}

impl TcpSendNode {
    pub fn new() -> Self {
        TcpSendNode {}
    }
}

#[async_trait]
impl NodeLogic for TcpSendNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "tcp_send",
            "TCP Send",
            "Sends data through an open TCP connection",
            "Web/TCP",
        );
        node.add_icon("/flow/icons/web.svg");
        node.scores = Some(
            flow_like::flow::node::NodeScores::new()
                .set_privacy(7)
                .set_security(6)
                .set_performance(8)
                .set_governance(5)
                .set_reliability(7)
                .set_cost(10)
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
            "TCP session reference",
            VariableType::Struct,
        )
        .set_schema::<TcpSession>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());
        node.add_input_pin(
            "message_type",
            "Message Type",
            "Whether to send as text (UTF-8) or binary",
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
            "The data to send (string for Text, byte array for Binary)",
            VariableType::String,
        );

        node.add_output_pin(
            "exec_out",
            "Done",
            "Fires after the data is sent",
            VariableType::Execution,
        );

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        use tokio::io::AsyncWriteExt;

        context.deactivate_exec_pin("exec_out").await?;

        let session: TcpSession = context.evaluate_pin("session").await?;
        let message_type: String = context.evaluate_pin("message_type").await?;

        let conn = super::get_tcp_connection(context, &session.ref_id).await?;
        let mut writer = conn.writer.lock().await;

        match message_type.as_str() {
            "Binary" => {
                let data: Vec<u8> = context.evaluate_pin("payload").await?;
                writer.write_all(&data).await.map_err(|e| {
                    context.log_message(&format!("TCP send error: {}", e), LogLevel::Error);
                    flow_like_types::anyhow!("TCP send failed: {}", e)
                })?;
            }
            _ => {
                let text: String = context.evaluate_pin("payload").await?;
                writer.write_all(text.as_bytes()).await.map_err(|e| {
                    context.log_message(&format!("TCP send error: {}", e), LogLevel::Error);
                    flow_like_types::anyhow!("TCP send failed: {}", e)
                })?;
            }
        }

        context.activate_exec_pin("exec_out").await?;

        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "TCP requires the 'execute' feature"
        ))
    }
}
