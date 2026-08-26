/// # ONNX Batch Inference Nodes
/// Efficient batch processing for ONNX models
use crate::onnx::NodeOnnxSession;
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::PinOptions,
    variable::VariableType,
};
#[cfg(feature = "execute")]
use flow_like_catalog_core::NodeImage;
#[cfg(feature = "execute")]
use flow_like_model_provider::ml::{
    ndarray::Array4,
    ort::{inputs, value::Value as OrtValue},
};
#[cfg(feature = "execute")]
use flow_like_types::image::imageops::FilterType;
use flow_like_types::{Result, Value, async_trait, json::json};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Batch inference result
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub struct BatchResult {
    /// Number of items processed
    pub count: usize,
    /// Individual results (as JSON values)
    pub results: Vec<Value>,
    /// Processing time per item (ms)
    pub avg_time_ms: f32,
}

#[crate::register_node]
#[derive(Default)]
pub struct BatchImageInferenceNode {}

impl BatchImageInferenceNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for BatchImageInferenceNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "onnx_batch_image_inference",
            "Batch Image Inference",
            "Run ONNX inference on multiple images in batches",
            "AI/ML/ONNX/Batch",
        );
        node.set_flowscript_name("onnx", "batchImageInference");
        node.set_version(1);

        node.add_icon("/flow/icons/layers.svg");

        node.add_input_pin(
            "exec_in",
            "Input",
            "Initiate Execution",
            VariableType::Execution,
        );

        node.add_input_pin("model", "Model", "ONNX Model Session", VariableType::Struct)
            .set_schema::<NodeOnnxSession>()
            .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin(
            "images",
            "Images",
            "List of images to process",
            VariableType::Generic,
        );

        node.add_input_pin(
            "batch_size",
            "Batch Size",
            "Number of images per batch",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(8)));

        node.add_input_pin(
            "input_size",
            "Input Size",
            "Model input size",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(224)));

        node.add_input_pin(
            "normalize",
            "Normalize",
            "Apply ImageNet normalization",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(true)));

        node.add_output_pin("exec_out", "Output", "Done", VariableType::Execution);

        node.add_output_pin(
            "results",
            "Results",
            "Raw output tensors per image",
            VariableType::Generic,
        );

        node.add_output_pin(
            "batch_result",
            "Batch Result",
            "Batch processing summary",
            VariableType::Struct,
        )
        .set_schema::<BatchResult>();

        node
    }

    #[allow(unused_variables)]
    async fn run(&self, context: &mut ExecutionContext) -> Result<()> {
        #[cfg(feature = "execute")]
        {
            use std::time::Instant;

            context.deactivate_exec_pin("exec_out").await?;

            let model_ref: NodeOnnxSession = context.evaluate_pin("model").await?;
            let images: Vec<NodeImage> = context.evaluate_pin("images").await?;
            let batch_size: i64 = context.evaluate_pin("batch_size").await.unwrap_or(8);
            let input_size: i64 = context.evaluate_pin("input_size").await.unwrap_or(224);
            let normalize: bool = context.evaluate_pin("normalize").await.unwrap_or(true);

            let batch_size = batch_size as usize;
            let input_size = input_size as u32;

            let session_wrapper = model_ref.get_session(context).await?;
            let mut session_guard = session_wrapper.lock().await;
            let session = &mut session_guard.session;

            let total_count = images.len();
            let mut all_results: Vec<Value> = Vec::new();
            let start_time = Instant::now();

            // Process in batches
            for chunk in images.chunks(batch_size) {
                let current_batch_size = chunk.len();

                // Create batch tensor
                let mut batch_input = Array4::<f32>::zeros((
                    current_batch_size,
                    3,
                    input_size as usize,
                    input_size as usize,
                ));

                for (idx, img) in chunk.iter().enumerate() {
                    let img_wrapper = img.get_image(context).await?;
                    let dyn_image = img_wrapper.lock().await;
                    let resized =
                        dyn_image.resize_exact(input_size, input_size, FilterType::Triangle);
                    let rgb = resized.to_rgb8();

                    for y in 0..input_size {
                        for x in 0..input_size {
                            let pixel = rgb.get_pixel(x, y);
                            if normalize {
                                let mean = [0.485, 0.456, 0.406];
                                let std = [0.229, 0.224, 0.225];
                                batch_input[[idx, 0, y as usize, x as usize]] =
                                    (pixel[0] as f32 / 255.0 - mean[0]) / std[0];
                                batch_input[[idx, 1, y as usize, x as usize]] =
                                    (pixel[1] as f32 / 255.0 - mean[1]) / std[1];
                                batch_input[[idx, 2, y as usize, x as usize]] =
                                    (pixel[2] as f32 / 255.0 - mean[2]) / std[2];
                            } else {
                                batch_input[[idx, 0, y as usize, x as usize]] =
                                    pixel[0] as f32 / 255.0;
                                batch_input[[idx, 1, y as usize, x as usize]] =
                                    pixel[1] as f32 / 255.0;
                                batch_input[[idx, 2, y as usize, x as usize]] =
                                    pixel[2] as f32 / 255.0;
                            }
                        }
                    }
                }

                // Run inference
                let input_value = OrtValue::from_array(batch_input)?;
                let outputs = session.run(inputs![input_value])?;

                // Extract results
                if let Some((_, tensor)) = outputs.iter().next()
                    && let Ok(data) = tensor.try_extract_array::<f32>()
                {
                    let shape = data.shape();

                    // Split batch results
                    for i in 0..current_batch_size {
                        let result: Vec<f32> = if shape.len() >= 2 {
                            (0..shape[1]).map(|j| data[[i, j]]).collect()
                        } else {
                            data.iter().copied().collect()
                        };
                        all_results.push(json!(result));
                    }
                }
            }

            let elapsed = start_time.elapsed();
            let avg_time_ms = if total_count > 0 {
                elapsed.as_secs_f32() * 1000.0 / total_count as f32
            } else {
                0.0
            };

            let batch_result = BatchResult {
                count: total_count,
                results: all_results.clone(),
                avg_time_ms,
            };

            context.set_pin_value("results", json!(all_results)).await?;
            context
                .set_pin_value("batch_result", json!(batch_result))
                .await?;
            context.activate_exec_pin("exec_out").await?;
            Ok(())
        }

        #[cfg(not(feature = "execute"))]
        Err(flow_like_types::anyhow!("Execute feature not enabled"))
    }
}
