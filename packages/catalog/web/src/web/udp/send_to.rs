use flow_like::flow::{
    node::{Node, NodeLogic},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_types::async_trait;

#[cfg(not(feature = "execute"))]
use flow_like::flow::execution::context::ExecutionContext;
#[cfg(feature = "execute")]
use flow_like::flow::execution::{LogLevel, context::ExecutionContext};

#[cfg(feature = "execute")]
use flow_like_types::json::json;

use super::UdpSession;

#[crate::register_node]
#[derive(Default)]
pub struct UdpSendToNode {}

impl UdpSendToNode {
    pub fn new() -> Self {
        UdpSendToNode {}
    }
}

#[async_trait]
impl NodeLogic for UdpSendToNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "udp_send_to",
            "UDP Send To",
            "Sends a datagram to a target address through a bound UDP socket",
            "Web/UDP",
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
            "UDP session reference",
            VariableType::Struct,
        )
        .set_schema::<UdpSession>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());
        node.add_input_pin(
            "target_host",
            "Target Host",
            "Destination host address",
            VariableType::String,
        );
        node.add_input_pin(
            "target_port",
            "Target Port",
            "Destination port number",
            VariableType::Integer,
        );
        node.add_input_pin(
            "payload",
            "Payload",
            "The message content to send",
            VariableType::String,
        );

        node.add_output_pin(
            "exec_out",
            "Done",
            "Fires after the datagram is sent",
            VariableType::Execution,
        );
        node.add_output_pin(
            "bytes_sent",
            "Bytes Sent",
            "Number of bytes sent",
            VariableType::Integer,
        );

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let session: UdpSession = context.evaluate_pin("session").await?;
        let target_host: String = context.evaluate_pin("target_host").await?;
        let target_port: i64 = context.evaluate_pin("target_port").await?;
        let payload: String = context.evaluate_pin("payload").await?;

        let cached = super::get_udp_socket(context, &session.ref_id).await?;
        let target_addr = format!("{}:{}", target_host, target_port);

        let bytes_sent = cached
            .socket
            .send_to(payload.as_bytes(), &target_addr)
            .await
            .map_err(|e| {
                context.log_message(&format!("UDP send_to error: {}", e), LogLevel::Error);
                flow_like_types::anyhow!("UDP send_to failed: {}", e)
            })?;

        context
            .set_pin_value("bytes_sent", json!(bytes_sent as i64))
            .await?;
        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "UDP requires the 'execute' feature"
        ))
    }
}
