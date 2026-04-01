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

use super::WebSocketSession;

#[crate::register_node]
#[derive(Default)]
pub struct WebSocketCloseNode {}

impl WebSocketCloseNode {
    pub fn new() -> Self {
        WebSocketCloseNode {}
    }
}

#[async_trait]
impl NodeLogic for WebSocketCloseNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "websocket_close",
            "WebSocket Close",
            "Closes an open WebSocket connection gracefully",
            "Web/WebSocket",
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
            "WebSocket session to close",
            VariableType::Struct,
        )
        .set_schema::<WebSocketSession>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin(
            "exec_out",
            "Done",
            "Fires after the connection is closed",
            VariableType::Execution,
        );

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        use futures::SinkExt;

        context.deactivate_exec_pin("exec_out").await?;

        let session: WebSocketSession = context.evaluate_pin("session").await?;

        let close_err = {
            let cache = context.cache.read().await;
            if let Some(conn) = cache.get(&session.ref_id) {
                if let Some(conn) = conn
                    .as_any()
                    .downcast_ref::<super::CachedWebSocketConnection>()
                {
                    let mut sink = conn.sink.lock().await;
                    sink.close().await.err()
                } else {
                    None
                }
            } else {
                None
            }
        };

        if let Some(e) = close_err {
            context.log_message(
                &format!("WebSocket close error (non-fatal): {}", e),
                LogLevel::Warn,
            );
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
            "WebSocket requires the 'execute' feature"
        ))
    }
}
