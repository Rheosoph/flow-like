/// # ONNX Feature Extraction Nodes
/// Generic feature extraction for various model types
use crate::onnx::NodeOnnxSession;
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_catalog_core::NodeImage;
#[cfg(feature = "execute")]
use flow_like_model_provider::ml::{
    ndarray::{Array3, Axis},
    ort::{inputs, session::Session, value::Value},
};
#[cfg(feature = "execute")]
use flow_like_types::{
    Error,
    image::{DynamicImage, GenericImageView, imageops::FilterType},
};
use flow_like_types::{Result, async_trait, json::json};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Feature vector output from a model
#[derive(Default, Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub struct FeatureVector {
    /// The extracted feature values
    pub values: Vec<f32>,
    /// Dimensionality of the feature vector
    pub dimensions: usize,
}

impl FeatureVector {
    /// Create a new feature vector
    pub fn new(values: Vec<f32>) -> Self {
        let dimensions = values.len();
        Self { values, dimensions }
    }

    /// Compute cosine similarity with another feature vector
    pub fn cosine_similarity(&self, other: &FeatureVector) -> f32 {
        if self.dimensions != other.dimensions || self.dimensions == 0 {
            return 0.0;
        }

        let dot: f32 = self
            .values
            .iter()
            .zip(&other.values)
            .map(|(a, b)| a * b)
            .sum();
        let norm_a: f32 = self.values.iter().map(|x| x * x).sum::<f32>().sqrt();
        let norm_b: f32 = other.values.iter().map(|x| x * x).sum::<f32>().sqrt();

        if norm_a == 0.0 || norm_b == 0.0 {
            0.0
        } else {
            dot / (norm_a * norm_b)
        }
    }

    /// Compute L2 (Euclidean) distance with another feature vector
    pub fn l2_distance(&self, other: &FeatureVector) -> f32 {
        if self.dimensions != other.dimensions {
            return f32::MAX;
        }

        self.values
            .iter()
            .zip(&other.values)
            .map(|(a, b)| (a - b).powi(2))
            .sum::<f32>()
            .sqrt()
    }

    /// Normalize the feature vector to unit length
    pub fn normalize(&mut self) {
        let norm: f32 = self.values.iter().map(|x| x * x).sum::<f32>().sqrt();
        if norm > 0.0 {
            self.values.iter_mut().for_each(|x| *x /= norm);
        }
    }
}

#[cfg(feature = "execute")]
/// Extract features from image using ONNX model
fn extract_image_features(
    session: &mut Session,
    img: &DynamicImage,
    input_width: u32,
    input_height: u32,
    input_name: &str,
    output_name: &str,
) -> Result<FeatureVector, Error> {
    // Preprocess image
    let (img_w, img_h) = img.dimensions();
    let buf_u8 = if img_w == input_width && img_h == input_height {
        img.to_rgb8().into_raw()
    } else {
        img.resize_exact(input_width, input_height, FilterType::Triangle)
            .to_rgb8()
            .into_raw()
    };

    let buf_f32: Vec<f32> = buf_u8.into_iter().map(|v| (v as f32) / 255.0).collect();
    let arr = Array3::from_shape_vec((input_height as usize, input_width as usize, 3), buf_f32)?
        .permuted_axes([2, 0, 1])
        .insert_axis(Axis(0));
    let value = Value::from_array(arr)?;
    let session_inputs = inputs![input_name => value];

    // Run inference
    let outputs = session.run(session_inputs)?;
    let output = outputs[output_name].try_extract_array::<f32>()?;

    // Flatten to 1D feature vector
    let values: Vec<f32> = output.iter().copied().collect();
    Ok(FeatureVector::new(values))
}

#[crate::register_node]
#[derive(Default)]
/// # Generic Feature Extraction Node
/// Extract feature vectors from images using ONNX models
pub struct FeatureExtractionNode {}

impl FeatureExtractionNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for FeatureExtractionNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "feature_extraction",
            "Feature Extraction",
            "Extract feature vectors from images using ONNX models",
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

        node.add_input_pin("model", "Model", "ONNX Model Session", VariableType::Struct)
            .set_schema::<NodeOnnxSession>()
            .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin("image_in", "Image", "Image Object", VariableType::Struct)
            .set_schema::<NodeImage>()
            .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin(
            "normalize",
            "Normalize",
            "Normalize output to unit length",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(true)));

        node.add_output_pin(
            "exec_out",
            "Output",
            "Done with the Execution",
            VariableType::Execution,
        );

        node.add_output_pin(
            "features",
            "Features",
            "Extracted feature vector",
            VariableType::Struct,
        )
        .set_schema::<FeatureVector>();

        node.add_output_pin(
            "dimensions",
            "Dimensions",
            "Feature vector dimensionality",
            VariableType::Integer,
        );

        node
    }

    #[allow(unused_variables)]
    async fn run(&self, context: &mut ExecutionContext) -> Result<()> {
        #[cfg(feature = "execute")]
        {
            context.deactivate_exec_pin("exec_out").await?;

            let node_session: NodeOnnxSession = context.evaluate_pin("model").await?;
            let node_img: NodeImage = context.evaluate_pin("image_in").await?;
            let normalize: bool = context.evaluate_pin("normalize").await.unwrap_or(true);

            let mut features = {
                let img = node_img.get_image(context).await?;
                let img_guard = img.lock().await;
                let session = node_session.get_session(context).await?;
                let mut session_guard = session.lock().await;

                // Determine input shape from session
                let (input_width, input_height) =
                    if let Some(input) = session_guard.session.inputs().first() {
                        if let Some(dims) = input.dtype().tensor_shape() {
                            let d = dims.len();
                            if d >= 2 {
                                (dims[d - 1] as u32, dims[d - 2] as u32)
                            } else {
                                (224, 224)
                            }
                        } else {
                            (224, 224)
                        }
                    } else {
                        (224, 224)
                    };

                let input_name = session_guard
                    .session
                    .inputs()
                    .first()
                    .map(|i| i.name().to_string())
                    .unwrap_or_else(|| "input".to_string());
                let output_name = session_guard
                    .session
                    .outputs()
                    .first()
                    .map(|o| o.name().to_string())
                    .unwrap_or_else(|| "output".to_string());

                extract_image_features(
                    &mut session_guard.session,
                    &img_guard,
                    input_width,
                    input_height,
                    &input_name,
                    &output_name,
                )?
            };

            if normalize {
                features.normalize();
            }

            let dimensions = features.dimensions;
            context.set_pin_value("features", json!(features)).await?;
            context
                .set_pin_value("dimensions", json!(dimensions))
                .await?;
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
/// # Feature Similarity Node
/// Compare two feature vectors for similarity
pub struct FeatureSimilarityNode {}

impl FeatureSimilarityNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for FeatureSimilarityNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "feature_similarity",
            "Feature Similarity",
            "Compare two feature vectors using cosine similarity or L2 distance",
            "AI/ML/ONNX",
        );

        node.add_icon("/flow/icons/find_model.svg");

        node.add_input_pin(
            "features_a",
            "Features A",
            "First feature vector",
            VariableType::Struct,
        )
        .set_schema::<FeatureVector>();

        node.add_input_pin(
            "features_b",
            "Features B",
            "Second feature vector",
            VariableType::Struct,
        )
        .set_schema::<FeatureVector>();

        node.add_output_pin(
            "cosine_similarity",
            "Cosine Similarity",
            "Cosine similarity (-1 to 1, higher is more similar)",
            VariableType::Float,
        );

        node.add_output_pin(
            "l2_distance",
            "L2 Distance",
            "Euclidean distance (lower is more similar)",
            VariableType::Float,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> Result<()> {
        let features_a: FeatureVector = context.evaluate_pin("features_a").await?;
        let features_b: FeatureVector = context.evaluate_pin("features_b").await?;

        let cosine = features_a.cosine_similarity(&features_b);
        let l2 = features_a.l2_distance(&features_b);

        context
            .set_pin_value("cosine_similarity", json!(cosine))
            .await?;
        context.set_pin_value("l2_distance", json!(l2)).await?;
        Ok(())
    }
}
