//! Node for general model information
//!
//! Returns generic information about any ML model type.

use crate::ml::NodeMLModel;
use flow_like::flow::{
    execution::{LogLevel, context::ExecutionContext},
    node::{Node, NodeLogic},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_types::{Result, async_trait, json::json};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub struct ModelInfo {
    /// Type of the model (e.g., "KMeans", "LinearRegression", "SVMMultiClass")
    pub model_type: String,
    /// Human-readable description
    pub description: String,
    /// Number of classes (for classifiers) or clusters (for clustering)
    pub n_classes: Option<usize>,
    /// Class names if available
    pub class_names: Option<Vec<String>>,
}

#[crate::register_node]
#[derive(Default)]
pub struct GetModelInfoNode {}

impl GetModelInfoNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for GetModelInfoNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "ml_model_info",
            "Model Info",
            "Get general information about any ML model",
            "AI/ML/Model Info",
        );
        node.set_flowscript_name("ml", "info");
        node.set_receiver("model");
        node.add_icon("/flow/icons/chart-network.svg");

        node.add_input_pin(
            "exec_in",
            "Input",
            "Execution trigger",
            VariableType::Execution,
        );

        node.add_input_pin(
            "model",
            "Model",
            "Any trained ML model",
            VariableType::Struct,
        )
        .set_schema::<NodeMLModel>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin(
            "exec_out",
            "Done",
            "Activated when info is extracted",
            VariableType::Execution,
        );

        node.add_output_pin(
            "info",
            "Info",
            "Model information structure",
            VariableType::Struct,
        )
        .set_schema::<ModelInfo>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin(
            "model_type",
            "Type",
            "Model type as string",
            VariableType::String,
        );

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let node_model: NodeMLModel = context.evaluate_pin("model").await?;
        let model_arc = node_model.get_model(context).await?;
        let model = model_arc.lock().await;

        let n_classes = model.cardinality();
        let info = ModelInfo {
            model_type: model.kind().to_string(),
            description: format!(
                "{}{}",
                model.label(),
                n_classes
                    .map(|n| format!(" with {} classes", n))
                    .unwrap_or_default()
            ),
            n_classes,
            class_names: model.class_names(),
        };

        context.log_message(&format!("Model info: {:?}", info), LogLevel::Debug);

        let model_type = info.model_type.clone();
        context.set_pin_value("info", json!(info)).await?;
        context
            .set_pin_value("model_type", json!(model_type))
            .await?;
        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> Result<()> {
        Err(flow_like_types::anyhow!(
            "ML execution requires the 'execute' feature"
        ))
    }
}
