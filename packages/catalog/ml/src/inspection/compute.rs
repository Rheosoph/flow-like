use flow_like::flow::{execution::context::ExecutionContext, node::NodeLogic};
use flow_like_ml_burn::{BackendChoice, BackendProbe};
use flow_like_types::Result;
#[cfg(not(feature = "training"))]
use flow_like_types::anyhow;

#[crate::register_node]
#[derive(Default)]
pub struct ProbeTrainingDeviceNode;
#[flow_like_types::async_trait]
impl NodeLogic for ProbeTrainingDeviceNode {
    fn get_node(&self) -> flow_like::flow::node::Node {
        super::operation_node::<BackendChoice, BackendProbe>(
            "ml_probe_training_device",
            "Probe Training Device",
            "Resolve automatic device selection or test a specific CPU or GPU backend with a forward and backward calculation",
            "probeDevice",
            "AI/ML/Compute",
        )
    }
    async fn run(&self, context: &mut ExecutionContext) -> Result<()> {
        #[cfg(feature = "training")]
        {
            context.deactivate_exec_pin("exec_out").await?;
            let backend: BackendChoice = context.evaluate_pin("request").await?;
            let output =
                tokio::task::spawn_blocking(move || flow_like_ml_burn::probe_backend(&backend))
                    .await??;
            context
                .set_pin_value("result", flow_like_types::json::json!(output))
                .await?;
            context.activate_exec_pin("exec_out").await?;
            Ok(())
        }
        #[cfg(not(feature = "training"))]
        {
            let _ = context;
            Err(anyhow!("Device probing requires the training feature"))
        }
    }
}
