use flow_like::flow::{execution::context::ExecutionContext, node::NodeLogic};
use flow_like_types::{Result, anyhow};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize, JsonSchema)]
pub struct PromptSam2Request {
    pub source_id: String,
    /// A CHW image or NCHW batch resized to 1024 and normalized with ImageNet mean and standard deviation.
    pub images: flow_like_ml_core::TensorData,
    pub prompts: Vec<flow_like_ml_burn::Sam2Prompt>,
    #[serde(default)]
    pub compute: flow_like_ml_core::ComputeConfig,
}

#[derive(Clone, Serialize, Deserialize, JsonSchema)]
pub struct FineTuneSam2Request {
    pub source_id: String,
    pub config: flow_like_ml_burn::Sam2TrainingConfig,
    pub data: flow_like_ml_burn::Sam2TrainingDataset,
    #[serde(default)]
    pub compute: flow_like_ml_core::ComputeConfig,
    #[serde(default = "default_artifact_limit")]
    pub maximum_bytes: u64,
}
fn default_artifact_limit() -> u64 {
    512 * 1024 * 1024
}

#[crate::register_node]
#[derive(Default)]
pub struct PromptSam2Node;
#[flow_like_types::async_trait]
impl NodeLogic for PromptSam2Node {
    fn get_node(&self) -> flow_like::flow::node::Node {
        super::operation_node::<PromptSam2Request, flow_like_ml_burn::Sam2Prediction>(
            "ml_prompt_sam2",
            "Segment with SAM 2",
            "Predict image masks from foreground points, background points, a box or a previous mask",
            "promptSam2",
            "AI/ML/Vision",
        )
    }
    async fn run(&self, context: &mut ExecutionContext) -> Result<()> {
        #[cfg(feature = "training")]
        {
            context.deactivate_exec_pin("exec_out").await?;
            let request: PromptSam2Request = context.evaluate_pin("request").await?;
            let (repo, project) = super::lifecycle::repository(context)?;
            if context.is_cancelled() {
                return Err(anyhow!("SAM prediction cancelled"));
            }
            let result = tokio::task::spawn_blocking(move || {
                flow_like_ml_runtime::engines::predict_sam2_source(
                    &repo,
                    &project,
                    &request.source_id,
                    &request.images,
                    &request.prompts,
                    &request.compute,
                )
            })
            .await??;
            if context.is_cancelled() {
                return Err(anyhow!("SAM prediction cancelled"));
            }
            context
                .set_pin_value("result", flow_like_types::json::json!(result))
                .await?;
            context.activate_exec_pin("exec_out").await?;
            Ok(())
        }
        #[cfg(not(feature = "training"))]
        {
            let _ = context;
            Err(anyhow!(
                "Native SAM prediction requires a local training build"
            ))
        }
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct FineTuneSam2Node;
#[flow_like_types::async_trait]
impl NodeLogic for FineTuneSam2Node {
    fn get_node(&self) -> flow_like::flow::node::Node {
        super::operation_node::<FineTuneSam2Request, flow_like_ml_runtime::Sam2FineTuneResult>(
            "ml_fine_tune_sam2",
            "Fine-tune SAM 2",
            "Fine-tune a native SAM image model from prompts and reviewed masks, then publish new project weights",
            "fineTuneSam2",
            "AI/ML/Training",
        )
    }
    async fn run(&self, context: &mut ExecutionContext) -> Result<()> {
        #[cfg(feature = "training")]
        {
            context.deactivate_exec_pin("exec_out").await?;
            let request: FineTuneSam2Request = context.evaluate_pin("request").await?;
            if request.maximum_bytes == 0 || request.maximum_bytes > 2 * 1024 * 1024 * 1024 {
                return Err(anyhow!(
                    "SAM artifact byte limit must be between 1 and 2 GiB"
                ));
            }
            if context.is_cancelled() {
                return Err(anyhow!("SAM fine-tuning cancelled"));
            }
            let (repo, project) = super::lifecycle::repository(context)?;
            let cancellation = flow_like_ml_burn::CancellationToken::new();
            let worker_cancellation = cancellation.clone();
            let mut work = tokio::task::spawn_blocking(move || {
                flow_like_ml_runtime::engines::fine_tune_sam2_source(
                    &repo,
                    &project,
                    &request.source_id,
                    &request.config,
                    &request.data,
                    &request.compute,
                    request.maximum_bytes,
                    &worker_cancellation,
                    |_| {},
                    flow_like_types::json::json!({"fine_tuned_from": request.source_id}),
                    flow_like_ml_runtime::now_ms(),
                )
            });
            let result = if let Some(token) = context.get_cancellation_token() {
                tokio::select! {
                    result = &mut work => result?,
                    _ = token.cancelled() => {
                        cancellation.cancel();
                        work.await?
                    }
                }
            } else {
                work.await?
            }?;
            context
                .set_pin_value("result", flow_like_types::json::json!(result))
                .await?;
            context.activate_exec_pin("exec_out").await?;
            Ok(())
        }
        #[cfg(not(feature = "training"))]
        {
            let _ = context;
            Err(anyhow!("SAM fine-tuning requires a local training build"))
        }
    }
}
