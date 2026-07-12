/// # ONNX Model Utility Nodes
/// Model inspection, caching, and utility operations
use crate::onnx::NodeOnnxSession;
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::{PinOptions, ValueType},
    variable::VariableType,
};
use flow_like_catalog_core::FlowPath;
use flow_like_types::{Result, async_trait, json::json};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Information about an ONNX model input/output
#[derive(Default, Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub struct TensorInfo {
    /// Name of the tensor
    pub name: String,
    /// Shape of the tensor (may include -1 for dynamic dimensions)
    pub shape: Vec<i64>,
    /// Data type (e.g., "Float32", "Int64")
    pub dtype: String,
}

/// Comprehensive ONNX model metadata
#[derive(Default, Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub struct ModelMetadata {
    /// List of model inputs
    pub inputs: Vec<TensorInfo>,
    /// List of model outputs
    pub outputs: Vec<TensorInfo>,
    /// Producer name (if available)
    pub producer: Option<String>,
    /// Model version (if available)
    pub version: Option<i64>,
    /// Total number of parameters (estimated)
    pub num_parameters: Option<u64>,
}

#[crate::register_node]
#[derive(Default)]
/// # Model Info Node
/// Inspect ONNX model metadata without loading for inference
pub struct ModelInfoNode {}

impl ModelInfoNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for ModelInfoNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "onnx_model_info",
            "Model Info",
            "Get ONNX model metadata (inputs, outputs, shapes)",
            "AI/ML/ONNX",
        );

        node.add_icon("/flow/icons/find_model.svg");

        node.add_input_pin(
            "exec_in",
            "Input",
            "Initiate Execution",
            VariableType::Execution,
        );

        node.add_input_pin("path", "Path", "Path to ONNX file", VariableType::Struct)
            .set_schema::<FlowPath>()
            .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin(
            "exec_out",
            "Output",
            "Done with the Execution",
            VariableType::Execution,
        );

        node.add_output_pin(
            "metadata",
            "Metadata",
            "Model metadata",
            VariableType::Struct,
        )
        .set_schema::<ModelMetadata>();

        node.add_output_pin(
            "inputs",
            "Inputs",
            "List of model inputs",
            VariableType::Struct,
        )
        .set_schema::<TensorInfo>()
        .set_value_type(ValueType::Array);

        node.add_output_pin(
            "outputs",
            "Outputs",
            "List of model outputs",
            VariableType::Struct,
        )
        .set_schema::<TensorInfo>()
        .set_value_type(ValueType::Array);

        node
    }

    #[allow(unused_variables)]
    async fn run(&self, context: &mut ExecutionContext) -> Result<()> {
        #[cfg(feature = "execute")]
        {
            crate::onnx::execution_providers::ensure_ort_initialized()?;
            context.deactivate_exec_pin("exec_out").await?;

            let path: FlowPath = context.evaluate_pin("path").await?;
            let bytes = path.get(context, false).await?;
            let session = {
                let mut session_builder =
                    crate::onnx::execution_providers::configured_session_builder()?;
                session_builder.commit_from_memory(&bytes)?
            };

            let inputs: Vec<TensorInfo> = session
                .inputs()
                .iter()
                .map(|i| {
                    let shape = i
                        .dtype()
                        .tensor_shape()
                        .map(|s| s.iter().copied().collect())
                        .unwrap_or_default();
                    TensorInfo {
                        name: i.name().to_string(),
                        shape,
                        dtype: format!("{:?}", i.dtype()),
                    }
                })
                .collect();

            let outputs: Vec<TensorInfo> = session
                .outputs()
                .iter()
                .map(|o| {
                    let shape = o
                        .dtype()
                        .tensor_shape()
                        .map(|s| s.iter().copied().collect())
                        .unwrap_or_default();
                    TensorInfo {
                        name: o.name().to_string(),
                        shape,
                        dtype: format!("{:?}", o.dtype()),
                    }
                })
                .collect();

            let metadata = ModelMetadata {
                inputs: inputs.clone(),
                outputs: outputs.clone(),
                producer: None,
                version: None,
                num_parameters: None,
            };

            context.set_pin_value("metadata", json!(metadata)).await?;
            context.set_pin_value("inputs", json!(inputs)).await?;
            context.set_pin_value("outputs", json!(outputs)).await?;
            context.activate_exec_pin("exec_out").await?;
            Ok(())
        }

        #[cfg(not(feature = "execute"))]
        {
            Err(flow_like_types::anyhow!(
                "ONNX execution requires the 'execute' feature. Rebuild with --features execute"
            ))
        }
    }
}

#[crate::register_node]
#[derive(Default)]
/// # Unload ONNX Node
/// Release an ONNX session from cache to free memory
pub struct UnloadOnnxNode {}

impl UnloadOnnxNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for UnloadOnnxNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "unload_onnx",
            "Unload ONNX",
            "Release ONNX model from cache to free memory",
            "AI/ML/ONNX",
        );
        node.set_version(1);

        node.add_icon("/flow/icons/find_model.svg");

        node.add_input_pin(
            "exec_in",
            "Input",
            "Initiate Execution",
            VariableType::Execution,
        );

        node.add_input_pin(
            "model",
            "Model",
            "ONNX Model Session to unload",
            VariableType::Struct,
        )
        .set_schema::<NodeOnnxSession>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin(
            "exec_out",
            "Output",
            "Done with the Execution",
            VariableType::Execution,
        );

        node.add_output_pin(
            "success",
            "Success",
            "Whether the model was successfully unloaded",
            VariableType::Boolean,
        );

        node
    }

    #[allow(unused_variables)]
    async fn run(&self, context: &mut ExecutionContext) -> Result<()> {
        #[cfg(feature = "execute")]
        {
            context.deactivate_exec_pin("exec_out").await?;

            let node_session: NodeOnnxSession = context.evaluate_pin("model").await?;

            // Remove from cache
            let removed = context
                .cache
                .write()
                .await
                .remove(&node_session.session_ref);
            let success = removed.is_some();

            context.set_pin_value("success", json!(success)).await?;
            context.activate_exec_pin("exec_out").await?;
            Ok(())
        }

        #[cfg(not(feature = "execute"))]
        {
            Err(flow_like_types::anyhow!(
                "ONNX execution requires the 'execute' feature. Rebuild with --features execute"
            ))
        }
    }
}

#[crate::register_node]
#[derive(Default)]
/// # Session Info Node
/// Get information about a loaded ONNX session
pub struct SessionInfoNode {}

impl SessionInfoNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for SessionInfoNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "onnx_session_info",
            "Session Info",
            "Get information about a loaded ONNX session",
            "AI/ML/ONNX",
        );
        node.set_version(1);

        node.add_icon("/flow/icons/find_model.svg");

        node.add_input_pin("model", "Model", "ONNX Model Session", VariableType::Struct)
            .set_schema::<NodeOnnxSession>()
            .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin(
            "inputs",
            "Inputs",
            "List of model inputs",
            VariableType::Struct,
        )
        .set_schema::<TensorInfo>()
        .set_value_type(ValueType::Array);

        node.add_output_pin(
            "outputs",
            "Outputs",
            "List of model outputs",
            VariableType::Struct,
        )
        .set_schema::<TensorInfo>()
        .set_value_type(ValueType::Array);

        node.add_output_pin(
            "input_names",
            "Input Names",
            "Comma-separated input names",
            VariableType::String,
        );

        node.add_output_pin(
            "output_names",
            "Output Names",
            "Comma-separated output names",
            VariableType::String,
        );

        node
    }

    #[allow(unused_variables)]
    async fn run(&self, context: &mut ExecutionContext) -> Result<()> {
        #[cfg(feature = "execute")]
        {
            let node_session: NodeOnnxSession = context.evaluate_pin("model").await?;
            let session = node_session.get_session(context).await?;
            let session_guard = session.lock().await;

            let inputs: Vec<TensorInfo> = session_guard
                .session
                .inputs()
                .iter()
                .map(|i| {
                    let shape = i
                        .dtype()
                        .tensor_shape()
                        .map(|s| s.iter().copied().collect())
                        .unwrap_or_default();
                    TensorInfo {
                        name: i.name().to_string(),
                        shape,
                        dtype: format!("{:?}", i.dtype()),
                    }
                })
                .collect();

            let outputs: Vec<TensorInfo> = session_guard
                .session
                .outputs()
                .iter()
                .map(|o| {
                    let shape = o
                        .dtype()
                        .tensor_shape()
                        .map(|s| s.iter().copied().collect())
                        .unwrap_or_default();
                    TensorInfo {
                        name: o.name().to_string(),
                        shape,
                        dtype: format!("{:?}", o.dtype()),
                    }
                })
                .collect();

            let input_names: String = inputs
                .iter()
                .map(|i| i.name.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            let output_names: String = outputs
                .iter()
                .map(|o| o.name.as_str())
                .collect::<Vec<_>>()
                .join(", ");

            context.set_pin_value("inputs", json!(inputs)).await?;
            context.set_pin_value("outputs", json!(outputs)).await?;
            context
                .set_pin_value("input_names", json!(input_names))
                .await?;
            context
                .set_pin_value("output_names", json!(output_names))
                .await?;
            Ok(())
        }

        #[cfg(not(feature = "execute"))]
        {
            Err(flow_like_types::anyhow!(
                "ONNX execution requires the 'execute' feature. Rebuild with --features execute"
            ))
        }
    }
}
