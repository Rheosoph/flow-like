use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    variable::VariableType,
};
use flow_like_types::async_trait;

#[crate::register_node]
#[derive(Default)]
pub struct ServiceReadyNode {}
impl ServiceReadyNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for ServiceReadyNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "service_ready",
            "Service Ready",
            "Marks a supervised daemon ready after its initialization succeeds. Continue into the daemon's long-running work; returning from the workflow stops the service.",
            "Web/Services",
        );
        node.set_flowscript_name("web", "serviceReady");
        node.add_input_pin(
            "exec_in",
            "Execute",
            "Initialization has succeeded",
            VariableType::Execution,
        );
        node.add_output_pin(
            "exec_out",
            "Ready",
            "Continue the service",
            VariableType::Execution,
        );
        node
    }
    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context
            .signal_service_ready(flow_like::flow::execution::service::ServiceReadyKind::Daemon)
            .await?;
        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }
    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "Service Ready requires the 'execute' feature"
        ))
    }
}
