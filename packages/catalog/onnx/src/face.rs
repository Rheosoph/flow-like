/// # ONNX Face Detection and Analysis Nodes
/// Face detection, landmark detection, and face recognition
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
    ndarray::Array4,
    ort::{inputs, value::Value},
};
#[cfg(feature = "execute")]
use flow_like_types::image::{GenericImageView, imageops::FilterType};
use flow_like_types::{Result, anyhow, async_trait, json::json};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Detected face with bounding box and confidence
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub struct DetectedFace {
    /// Bounding box [x, y, width, height]
    pub bbox: [f32; 4],
    /// Detection confidence
    pub confidence: f32,
    /// Face landmarks if available
    pub landmarks: Option<FaceLandmarks>,
}

/// Face landmark points (5 or 68 points)
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub struct FaceLandmarks {
    /// Landmark points [[x, y], ...]
    pub points: Vec<[f32; 2]>,
    /// Landmark type (5-point or 68-point)
    pub landmark_type: LandmarkType,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, Default)]
pub enum LandmarkType {
    /// 5 points: left eye, right eye, nose, left mouth, right mouth
    #[default]
    FivePoint,
    /// 68 point face landmark model
    SixtyEightPoint,
}

/// Face embedding for recognition/comparison
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub struct FaceEmbedding {
    /// Embedding vector
    pub embedding: Vec<f32>,
    /// Embedding dimension
    pub dimension: usize,
}

impl FaceEmbedding {
    /// Cosine similarity between two embeddings
    pub fn cosine_similarity(&self, other: &FaceEmbedding) -> f32 {
        if self.embedding.len() != other.embedding.len() {
            return 0.0;
        }

        let dot: f32 = self
            .embedding
            .iter()
            .zip(other.embedding.iter())
            .map(|(a, b)| a * b)
            .sum();

        let norm_a: f32 = self.embedding.iter().map(|x| x * x).sum::<f32>().sqrt();
        let norm_b: f32 = other.embedding.iter().map(|x| x * x).sum::<f32>().sqrt();

        if norm_a > 0.0 && norm_b > 0.0 {
            dot / (norm_a * norm_b)
        } else {
            0.0
        }
    }

    /// Euclidean distance between two embeddings
    pub fn euclidean_distance(&self, other: &FaceEmbedding) -> f32 {
        if self.embedding.len() != other.embedding.len() {
            return f32::MAX;
        }

        self.embedding
            .iter()
            .zip(other.embedding.iter())
            .map(|(a, b)| (a - b).powi(2))
            .sum::<f32>()
            .sqrt()
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct FaceDetectionNode {}

impl FaceDetectionNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for FaceDetectionNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "onnx_face_detection",
            "Face Detection",
            "Detect faces in images. Download models from: UltraFace (https://github.com/onnx/models/tree/main/validated/vision/body_analysis/ultraface), RetinaFace (https://huggingface.co/arnabdhar/retinaface-onnx), SCRFD (https://huggingface.co/onnx-community/scrfd_10g_bnkps)",
            "AI/ML/ONNX/Face",
        );
        node.set_version(1);

        node.add_icon("/flow/icons/face.svg");

        node.add_input_pin(
            "exec_in",
            "Input",
            "Initiate Execution",
            VariableType::Execution,
        );

        node.add_input_pin(
            "model",
            "Model",
            "ONNX Face Detection Model",
            VariableType::Struct,
        )
        .set_schema::<NodeOnnxSession>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin("image", "Image", "Input Image", VariableType::Struct)
            .set_schema::<NodeImage>()
            .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin(
            "threshold",
            "Threshold",
            "Detection confidence threshold",
            VariableType::Float,
        )
        .set_default_value(Some(json!(0.5)));

        node.add_input_pin(
            "nms_threshold",
            "NMS Threshold",
            "Non-maximum suppression threshold",
            VariableType::Float,
        )
        .set_default_value(Some(json!(0.4)));

        node.add_input_pin(
            "input_size",
            "Input Size",
            "Model input size",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(640)));

        node.add_output_pin("exec_out", "Output", "Done", VariableType::Execution);

        node.add_output_pin("faces", "Faces", "Detected faces", VariableType::Generic);

        node.add_output_pin(
            "count",
            "Count",
            "Number of detected faces",
            VariableType::Integer,
        );

        node
    }

    #[allow(unused_variables)]
    async fn run(&self, context: &mut ExecutionContext) -> Result<()> {
        #[cfg(feature = "execute")]
        {
            context.deactivate_exec_pin("exec_out").await?;

            let model_ref: NodeOnnxSession = context.evaluate_pin("model").await?;
            let image: NodeImage = context.evaluate_pin("image").await?;
            let threshold: f64 = context.evaluate_pin("threshold").await.unwrap_or(0.5);
            let nms_threshold: f64 = context.evaluate_pin("nms_threshold").await.unwrap_or(0.4);
            let input_size: i64 = context.evaluate_pin("input_size").await.unwrap_or(640);
            let input_size = input_size as u32;

            let session_wrapper = model_ref.get_session(context).await?;
            let mut session_guard = session_wrapper.lock().await;
            let session = &mut session_guard.session;

            let img_wrapper = image.get_image(context).await?;
            let dyn_image = img_wrapper.lock().await;
            let (orig_w, orig_h) = dyn_image.dimensions();

            let resized = dyn_image.resize_exact(input_size, input_size, FilterType::Triangle);
            let rgb = resized.to_rgb8();

            let mut input = Array4::<f32>::zeros((1, 3, input_size as usize, input_size as usize));
            for y in 0..input_size {
                for x in 0..input_size {
                    let pixel = rgb.get_pixel(x, y);
                    // BGR order and mean subtraction for most face detectors
                    input[[0, 0, y as usize, x as usize]] = pixel[2] as f32 - 104.0;
                    input[[0, 1, y as usize, x as usize]] = pixel[1] as f32 - 117.0;
                    input[[0, 2, y as usize, x as usize]] = pixel[0] as f32 - 123.0;
                }
            }

            let input_value = Value::from_array(input)?;
            let outputs = session.run(inputs![input_value])?;

            let scale_x = orig_w as f32 / input_size as f32;
            let scale_y = orig_h as f32 / input_size as f32;

            let mut detections: Vec<(f32, [f32; 4], Option<Vec<[f32; 2]>>)> = Vec::new();

            // Try different output formats
            let output_vec: Vec<_> = outputs.iter().collect();

            if output_vec.len() >= 2 {
                // RetinaFace/SCRFD style: boxes + scores (+ landmarks)
                let (_, boxes_tensor) = &output_vec[0];
                let (_, scores_tensor) = &output_vec[1];

                if let (Ok(boxes), Ok(scores)) = (
                    boxes_tensor.try_extract_array::<f32>(),
                    scores_tensor.try_extract_array::<f32>(),
                ) {
                    let num_boxes = if boxes.shape().len() >= 2 {
                        boxes.shape()[1]
                    } else {
                        0
                    };

                    for i in 0..num_boxes {
                        let score = if scores.shape().len() == 3 {
                            scores[[0, i, 1]] // Often [batch, anchors, 2] with [bg, fg]
                        } else if scores.shape().len() == 2 {
                            scores[[0, i]]
                        } else {
                            continue;
                        };

                        if score > threshold as f32 {
                            let x1 = boxes[[0, i, 0]] * scale_x;
                            let y1 = boxes[[0, i, 1]] * scale_y;
                            let x2 = boxes[[0, i, 2]] * scale_x;
                            let y2 = boxes[[0, i, 3]] * scale_y;

                            // Check for landmarks in third output
                            let landmarks = if output_vec.len() >= 3 {
                                if let Ok(lm) = output_vec[2].1.try_extract_array::<f32>() {
                                    if lm.shape().len() >= 2 && lm.shape()[2] >= 10 {
                                        let mut points = Vec::new();
                                        for j in 0..5 {
                                            let lx = lm[[0, i, j * 2]] * scale_x;
                                            let ly = lm[[0, i, j * 2 + 1]] * scale_y;
                                            points.push([lx, ly]);
                                        }
                                        Some(points)
                                    } else {
                                        None
                                    }
                                } else {
                                    None
                                }
                            } else {
                                None
                            };

                            detections.push((score, [x1, y1, x2 - x1, y2 - y1], landmarks));
                        }
                    }
                }
            } else if let Some((_, tensor)) = output_vec.first() {
                // Single output format (YOLO-like)
                if let Ok(data) = tensor.try_extract_array::<f32>() {
                    let shape = data.shape();

                    if shape.len() >= 2 {
                        let num_det = shape[1];

                        for i in 0..num_det {
                            let score = if shape.len() == 3 {
                                data[[0, i, 4]]
                            } else {
                                data[[i, 4]]
                            };
                            if score > threshold as f32 {
                                let cx = if shape.len() == 3 {
                                    data[[0, i, 0]]
                                } else {
                                    data[[i, 0]]
                                };
                                let cy = if shape.len() == 3 {
                                    data[[0, i, 1]]
                                } else {
                                    data[[i, 1]]
                                };
                                let w = if shape.len() == 3 {
                                    data[[0, i, 2]]
                                } else {
                                    data[[i, 2]]
                                };
                                let h = if shape.len() == 3 {
                                    data[[0, i, 3]]
                                } else {
                                    data[[i, 3]]
                                };

                                let x = (cx - w / 2.0) * scale_x;
                                let y = (cy - h / 2.0) * scale_y;

                                detections.push((score, [x, y, w * scale_x, h * scale_y], None));
                            }
                        }
                    }
                }
            }

            // Apply NMS
            detections.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));

            let mut faces: Vec<DetectedFace> = Vec::new();
            let mut suppressed = vec![false; detections.len()];

            for i in 0..detections.len() {
                if suppressed[i] {
                    continue;
                }

                let (score, bbox, landmarks) = &detections[i];
                faces.push(DetectedFace {
                    bbox: *bbox,
                    confidence: *score,
                    landmarks: landmarks.as_ref().map(|pts| FaceLandmarks {
                        points: pts.clone(),
                        landmark_type: LandmarkType::FivePoint,
                    }),
                });

                // Suppress overlapping detections
                for j in (i + 1)..detections.len() {
                    if suppressed[j] {
                        continue;
                    }
                    let iou = calculate_iou(bbox, &detections[j].1);
                    if iou > nms_threshold as f32 {
                        suppressed[j] = true;
                    }
                }
            }

            let count = faces.len() as i64;
            context.set_pin_value("faces", json!(faces)).await?;
            context.set_pin_value("count", json!(count)).await?;
            context.activate_exec_pin("exec_out").await?;
            Ok(())
        }

        #[cfg(not(feature = "execute"))]
        Err(anyhow!("Execute feature not enabled"))
    }
}

#[cfg(feature = "execute")]
fn calculate_iou(a: &[f32; 4], b: &[f32; 4]) -> f32 {
    let x1 = a[0].max(b[0]);
    let y1 = a[1].max(b[1]);
    let x2 = (a[0] + a[2]).min(b[0] + b[2]);
    let y2 = (a[1] + a[3]).min(b[1] + b[3]);

    let inter_w = (x2 - x1).max(0.0);
    let inter_h = (y2 - y1).max(0.0);
    let inter_area = inter_w * inter_h;

    let area_a = a[2] * a[3];
    let area_b = b[2] * b[3];
    let union_area = area_a + area_b - inter_area;

    if union_area > 0.0 {
        inter_area / union_area
    } else {
        0.0
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct FaceEmbeddingNode {}

impl FaceEmbeddingNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for FaceEmbeddingNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "onnx_face_embedding",
            "Face Embedding",
            "Extract face embedding for recognition. Download models from: ArcFace (https://huggingface.co/onnx-community/arcface_torch/tree/main), FaceNet (https://huggingface.co/rocca/facenet-onnx)",
            "AI/ML/ONNX/Face",
        );
        node.set_version(1);

        node.add_icon("/flow/icons/fingerprint.svg");

        node.add_input_pin(
            "exec_in",
            "Input",
            "Initiate Execution",
            VariableType::Execution,
        );

        node.add_input_pin(
            "model",
            "Model",
            "ONNX Face Embedding Model",
            VariableType::Struct,
        )
        .set_schema::<NodeOnnxSession>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin("image", "Image", "Aligned face image", VariableType::Struct)
            .set_schema::<NodeImage>()
            .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin(
            "input_size",
            "Input Size",
            "Model input size (typically 112 or 160)",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(112)));

        node.add_output_pin("exec_out", "Output", "Done", VariableType::Execution);

        node.add_output_pin(
            "embedding",
            "Embedding",
            "Face embedding vector",
            VariableType::Struct,
        )
        .set_schema::<FaceEmbedding>();

        node
    }

    #[allow(unused_variables)]
    async fn run(&self, context: &mut ExecutionContext) -> Result<()> {
        #[cfg(feature = "execute")]
        {
            context.deactivate_exec_pin("exec_out").await?;

            let model_ref: NodeOnnxSession = context.evaluate_pin("model").await?;
            let image: NodeImage = context.evaluate_pin("image").await?;
            let input_size: i64 = context.evaluate_pin("input_size").await.unwrap_or(112);
            let input_size = input_size as u32;

            let session_wrapper = model_ref.get_session(context).await?;
            let mut session_guard = session_wrapper.lock().await;
            let session = &mut session_guard.session;

            let img_wrapper = image.get_image(context).await?;
            let dyn_image = img_wrapper.lock().await;
            let resized = dyn_image.resize_exact(input_size, input_size, FilterType::Triangle);
            let rgb = resized.to_rgb8();

            // ArcFace normalization
            let mut input = Array4::<f32>::zeros((1, 3, input_size as usize, input_size as usize));
            for y in 0..input_size {
                for x in 0..input_size {
                    let pixel = rgb.get_pixel(x, y);
                    input[[0, 0, y as usize, x as usize]] = (pixel[0] as f32 - 127.5) / 127.5;
                    input[[0, 1, y as usize, x as usize]] = (pixel[1] as f32 - 127.5) / 127.5;
                    input[[0, 2, y as usize, x as usize]] = (pixel[2] as f32 - 127.5) / 127.5;
                }
            }

            let input_value = Value::from_array(input)?;
            let outputs = session.run(inputs![input_value])?;

            let output = outputs
                .iter()
                .next()
                .ok_or_else(|| anyhow!("No output from model"))?;
            let (_, tensor) = output;
            let emb_arr = tensor.try_extract_array::<f32>()?;

            let embedding: Vec<f32> = emb_arr.iter().copied().collect();
            let dimension = embedding.len();

            // L2 normalize
            let norm: f32 = embedding.iter().map(|x| x * x).sum::<f32>().sqrt();
            let embedding: Vec<f32> = if norm > 0.0 {
                embedding.iter().map(|x| x / norm).collect()
            } else {
                embedding
            };

            let result = FaceEmbedding {
                embedding,
                dimension,
            };

            context.set_pin_value("embedding", json!(result)).await?;
            context.activate_exec_pin("exec_out").await?;
            Ok(())
        }

        #[cfg(not(feature = "execute"))]
        Err(anyhow!("Execute feature not enabled"))
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct CompareFacesNode {}

#[async_trait]
impl NodeLogic for CompareFacesNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "compare_faces",
            "Compare Faces",
            "Compare two face embeddings for similarity",
            "AI/ML/ONNX/Face",
        );

        node.add_icon("/flow/icons/compare.svg");

        node.add_input_pin(
            "exec_in",
            "Input",
            "Initiate Execution",
            VariableType::Execution,
        );

        node.add_input_pin(
            "embedding_a",
            "Embedding A",
            "First face embedding",
            VariableType::Struct,
        )
        .set_schema::<FaceEmbedding>();

        node.add_input_pin(
            "embedding_b",
            "Embedding B",
            "Second face embedding",
            VariableType::Struct,
        )
        .set_schema::<FaceEmbedding>();

        node.add_input_pin(
            "threshold",
            "Threshold",
            "Match threshold (cosine similarity)",
            VariableType::Float,
        )
        .set_default_value(Some(json!(0.5)));

        node.add_output_pin("exec_out", "Output", "Done", VariableType::Execution);

        node.add_output_pin(
            "is_match",
            "Is Match",
            "Whether faces match",
            VariableType::Boolean,
        );

        node.add_output_pin(
            "similarity",
            "Similarity",
            "Cosine similarity score",
            VariableType::Float,
        );

        node.add_output_pin(
            "distance",
            "Distance",
            "Euclidean distance",
            VariableType::Float,
        );

        node
    }

    #[allow(unused_variables)]
    async fn run(&self, context: &mut ExecutionContext) -> Result<()> {
        #[cfg(feature = "execute")]
        {
            context.deactivate_exec_pin("exec_out").await?;

            let emb_a: FaceEmbedding = context.evaluate_pin("embedding_a").await?;
            let emb_b: FaceEmbedding = context.evaluate_pin("embedding_b").await?;
            let threshold: f64 = context.evaluate_pin("threshold").await.unwrap_or(0.5);

            let similarity = emb_a.cosine_similarity(&emb_b);
            let distance = emb_a.euclidean_distance(&emb_b);
            let is_match = similarity >= threshold as f32;

            context.set_pin_value("is_match", json!(is_match)).await?;
            context
                .set_pin_value("similarity", json!(similarity as f64))
                .await?;
            context
                .set_pin_value("distance", json!(distance as f64))
                .await?;
            context.activate_exec_pin("exec_out").await?;
            Ok(())
        }

        #[cfg(not(feature = "execute"))]
        Err(anyhow!("Execute feature not enabled"))
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct CropFacesNode {}

#[async_trait]
impl NodeLogic for CropFacesNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "crop_faces",
            "Crop Faces",
            "Crop detected faces from image",
            "AI/ML/ONNX/Face",
        );

        node.add_icon("/flow/icons/crop.svg");

        node.add_input_pin(
            "exec_in",
            "Input",
            "Initiate Execution",
            VariableType::Execution,
        );

        node.add_input_pin("image", "Image", "Source image", VariableType::Struct)
            .set_schema::<NodeImage>()
            .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin("faces", "Faces", "Detected faces", VariableType::Generic);

        node.add_input_pin(
            "margin",
            "Margin",
            "Margin around face (fraction)",
            VariableType::Float,
        )
        .set_default_value(Some(json!(0.2)));

        node.add_output_pin("exec_out", "Output", "Done", VariableType::Execution);

        node.add_output_pin(
            "crops",
            "Crops",
            "Cropped face images",
            VariableType::Generic,
        );

        node
    }

    #[allow(unused_variables)]
    async fn run(&self, context: &mut ExecutionContext) -> Result<()> {
        #[cfg(feature = "execute")]
        {
            context.deactivate_exec_pin("exec_out").await?;

            let image: NodeImage = context.evaluate_pin("image").await?;
            let faces: Vec<DetectedFace> = context.evaluate_pin("faces").await?;
            let margin: f64 = context.evaluate_pin("margin").await.unwrap_or(0.2);

            let img_wrapper = image.get_image(context).await?;
            let dyn_image = img_wrapper.lock().await;
            let (img_w, img_h) = dyn_image.dimensions();

            let mut crops: Vec<NodeImage> = Vec::new();

            for face in faces {
                let margin_x = (face.bbox[2] * margin as f32) as i32;
                let margin_y = (face.bbox[3] * margin as f32) as i32;

                let x = (face.bbox[0] as i32 - margin_x).max(0) as u32;
                let y = (face.bbox[1] as i32 - margin_y).max(0) as u32;
                let w = ((face.bbox[2] as i32 + 2 * margin_x) as u32).min(img_w - x);
                let h = ((face.bbox[3] as i32 + 2 * margin_y) as u32).min(img_h - y);

                if w > 0 && h > 0 {
                    let cropped = dyn_image.crop_imm(x, y, w, h);
                    let node_img = NodeImage::new(context, cropped).await;
                    crops.push(node_img);
                }
            }

            context.set_pin_value("crops", json!(crops)).await?;
            context.activate_exec_pin("exec_out").await?;
            Ok(())
        }

        #[cfg(not(feature = "execute"))]
        Err(anyhow!("Execute feature not enabled"))
    }
}
