use flow_like::flow::{
    node::{Node, NodeLogic},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_types::async_trait;

#[cfg(feature = "execute")]
use flow_like::flow::execution::context::ExecutionContext;
#[cfg(not(feature = "execute"))]
use flow_like::flow::execution::context::ExecutionContext;

use super::UdpSession;

#[crate::register_node]
#[derive(Default)]
pub struct UdpCloseNode {}

impl UdpCloseNode {
    pub fn new() -> Self {
        UdpCloseNode {}
    }
}

#[async_trait]
impl NodeLogic for UdpCloseNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "udp_close",
            "UDP Close",
            "Closes a bound UDP socket and releases resources",
            "Web/UDP",
        );
        node.add_icon("/flow/icons/web.svg");
        node.scores = Some(
            flow_like::flow::node::NodeScores::new()
                .set_privacy(8)
                .set_security(7)
                .set_performance(9)
                .set_governance(6)
                .set_reliability(8)
                .set_cost(10)
                .build(),
        );

        node.add_input_pin(
            "exec_in",
            "Execute",
            "Trigger the close",
            VariableType::Execution,
        );
        node.add_input_pin(
            "session",
            "Session",
            "UDP session to close",
            VariableType::Struct,
        )
        .set_schema::<UdpSession>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin(
            "exec_out",
            "Done",
            "Fires after the socket is closed",
            VariableType::Execution,
        );

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let session: UdpSession = context.evaluate_pin("session").await?;

        {
            let cache = context.cache.read().await;
            if let Some(conn) = cache.get(&session.ref_id)
                && let Some(conn) = conn.as_any().downcast_ref::<super::CachedUdpSocket>()
            {
                conn.close_notify.notify_waiters();
            }
        }

        {
            let mut cache = context.cache.write().await;
            cache.remove(&session.ref_id);
        }

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
