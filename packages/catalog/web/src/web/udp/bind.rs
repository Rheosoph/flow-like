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
use flow_like_types::Cacheable;
#[cfg(feature = "execute")]
use flow_like_types::json::json;
#[cfg(feature = "execute")]
use std::sync::Arc;

use super::{UdpConfig, UdpSession};

#[cfg(feature = "execute")]
use super::CachedUdpSocket;

#[crate::register_node]
#[derive(Default)]
pub struct UdpBindNode {}

impl UdpBindNode {
    pub fn new() -> Self {
        UdpBindNode {}
    }
}

#[async_trait]
impl NodeLogic for UdpBindNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "udp_bind",
            "UDP Bind",
            "Binds a UDP socket to a local address and port",
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
            "Bind the UDP socket",
            VariableType::Execution,
        );
        node.add_input_pin(
            "config",
            "Config",
            "UDP bind configuration (host and port)",
            VariableType::Struct,
        )
        .set_schema::<UdpConfig>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin(
            "exec_out",
            "Done",
            "Fires after the socket is bound",
            VariableType::Execution,
        );
        node.add_output_pin(
            "session",
            "Session",
            "UDP session reference for use with SendTo/Receive/Close nodes",
            VariableType::Struct,
        )
        .set_schema::<UdpSession>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());
        node.add_output_pin(
            "exec_error",
            "Error",
            "Fires if the bind fails",
            VariableType::Execution,
        );

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        context.activate_exec_pin("exec_error").await?;

        let config: UdpConfig = context.evaluate_pin("config").await?;
        let addr = format!("{}:{}", config.host, config.port);

        let socket = match tokio::net::UdpSocket::bind(&addr).await {
            Ok(s) => s,
            Err(e) => {
                context.log_message(&format!("UDP bind failed: {}", e), LogLevel::Error);
                return Ok(());
            }
        };

        let local_addr = socket.local_addr()?.to_string();
        let ref_id = format!("udp_{}", flow_like_types::create_id());
        let close_notify = Arc::new(tokio::sync::Notify::new());

        let cached = CachedUdpSocket {
            socket: Arc::new(socket),
            close_notify: close_notify.clone(),
        };
        let cacheable: Arc<dyn Cacheable> = Arc::new(cached);
        context.set_cache(&ref_id, cacheable).await;

        let session = UdpSession { ref_id, local_addr };
        context.set_pin_value("session", json!(session)).await?;

        context.deactivate_exec_pin("exec_error").await?;
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
