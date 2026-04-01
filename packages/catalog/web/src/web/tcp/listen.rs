#[cfg(not(feature = "execute"))]
use flow_like::flow::execution::context::ExecutionContext;
#[cfg(feature = "execute")]
use flow_like::flow::execution::{
    LogLevel, context::ExecutionContext, internal_node::InternalNode, log::LogMessage,
};

use flow_like::flow::{
    node::{Node, NodeLogic},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_types::async_trait;
#[cfg(feature = "execute")]
use flow_like_types::json::json;

#[cfg(feature = "execute")]
use std::sync::Arc;

#[cfg(feature = "execute")]
use flow_like_types::Cacheable;

use super::TcpListenConfig;

#[cfg(feature = "execute")]
use super::{CachedTcpConnection, TcpSession};

#[crate::register_node]
#[derive(Default)]
pub struct TcpListenNode {}

impl TcpListenNode {
    pub fn new() -> Self {
        TcpListenNode {}
    }
}

#[async_trait]
impl NodeLogic for TcpListenNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "tcp_listen",
            "TCP Listen",
            "Binds a TCP listener on a port. Fires on_listening, then accepts incoming \
             connections and invokes the handler for each. Holds execution until closed \
             or timed out, then triggers on_close.",
            "Web/TCP",
        );
        node.add_icon("/flow/icons/web.svg");
        node.set_long_running(true);
        node.set_can_reference_fns(true);
        node.scores = Some(
            flow_like::flow::node::NodeScores::new()
                .set_privacy(6)
                .set_security(5)
                .set_performance(7)
                .set_governance(5)
                .set_reliability(7)
                .set_cost(10)
                .build(),
        );

        node.add_input_pin(
            "exec_in",
            "Execute",
            "Start the TCP listener",
            VariableType::Execution,
        );
        node.add_input_pin(
            "config",
            "Config",
            "TCP listener configuration (host, port, optional timeout, max connections)",
            VariableType::Struct,
        )
        .set_schema::<TcpListenConfig>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin(
            "on_listening",
            "On Listening",
            "Fires after the listener is bound and ready",
            VariableType::Execution,
        );

        node.add_output_pin(
            "on_close",
            "On Close",
            "Fires when the listener stops",
            VariableType::Execution,
        );

        node.add_output_pin(
            "exec_error",
            "Error",
            "Fires if the listener fails to bind",
            VariableType::Execution,
        );

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        use flow_like::flow::pin::PinType;

        context.deactivate_exec_pin("on_listening").await?;
        context.deactivate_exec_pin("on_close").await?;
        context.activate_exec_pin("exec_error").await?;

        let config: TcpListenConfig = context.evaluate_pin("config").await?;
        let referenced_fns = context.get_referenced_functions().await?;
        let handler = referenced_fns
            .first()
            .ok_or_else(|| {
                flow_like_types::anyhow!("No on-connection handler function referenced")
            })?
            .clone();

        let addr = format!("{}:{}", config.host, config.port);
        let listener = match tokio::net::TcpListener::bind(&addr).await {
            Ok(l) => l,
            Err(e) => {
                context.log_message(
                    &format!("TCP bind failed on {}: {}", addr, e),
                    LogLevel::Error,
                );
                return Ok(());
            }
        };

        context.deactivate_exec_pin("exec_error").await?;
        context.activate_exec_pin("on_listening").await?;

        let on_listening_pin = context.get_pin_by_name("on_listening").await?;
        let connected_on_listening = on_listening_pin.get_connected_nodes();
        for node in connected_on_listening {
            let mut sub = context.create_sub_context(&node).await;
            sub.delegated = true;
            let mut message = LogMessage::new("TCP on_listening", LogLevel::Debug, None);
            let _ = InternalNode::trigger(&mut sub, &mut None, true).await;
            message.end();
            sub.log(message);
            sub.end_trace();
            context.push_sub_context(&mut sub);
        }

        let close_notify = Arc::new(tokio::sync::Notify::new());
        let close_fut = close_notify.notified();
        tokio::pin!(close_fut);

        let reference_function = handler;

        loop {
            tokio::select! {
                _ = &mut close_fut => break,
                result = listener.accept() => {
                    let (stream, addr) = match result {
                        Ok(pair) => pair,
                        Err(e) => {
                            context.log_message(
                                &format!("TCP accept error: {}", e),
                                LogLevel::Error,
                            );
                            continue;
                        }
                    };

                    let remote_addr = addr.to_string();
                    let (reader, writer) = tokio::io::split(stream);

                    let ref_id = format!("tcp_{}", flow_like_types::create_id());
                    let conn_close = Arc::new(tokio::sync::Notify::new());

                    let cached = CachedTcpConnection {
                        reader: Arc::new(tokio::sync::Mutex::new(reader)),
                        writer: Arc::new(tokio::sync::Mutex::new(writer)),
                        close_notify: conn_close,
                    };
                    let cacheable: Arc<dyn Cacheable> = Arc::new(cached);
                    context.set_cache(&ref_id, cacheable).await;

                    let session = TcpSession {
                        ref_id,
                        remote_addr,
                    };

                    let mut sub = context.create_sub_context(&reference_function).await;
                    sub.delegated = true;

                    for (_, pin) in sub.node.pins.iter() {
                        if pin.pin_type == PinType::Output
                            && pin.data_type != VariableType::Execution
                        {
                            pin.set_value(json!(session)).await;
                            break;
                        }
                    }

                    let mut message = LogMessage::new("TCP on_connection", LogLevel::Debug, None);
                    let run = InternalNode::trigger(&mut sub, &mut None, true).await;
                    message.end();
                    sub.log(message);
                    sub.end_trace();
                    context.push_sub_context(&mut sub);

                    if let Err(e) = run {
                        context.log_message(
                            &format!("TCP on_connection handler error: {:?}", e),
                            LogLevel::Warn,
                        );
                    }
                }
            }

            let timeout = config.timeout_seconds;
            if timeout > 0 {
                tokio::select! {
                    _ = &mut close_fut => break,
                    _ = tokio::time::sleep(std::time::Duration::from_secs(timeout)) => {
                        context.log_message("TCP listener timed out", LogLevel::Warn);
                        break;
                    }
                }
            }
        }

        context.deactivate_exec_pin("on_listening").await?;
        context.activate_exec_pin("on_close").await?;

        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "TCP requires the 'execute' feature"
        ))
    }
}
