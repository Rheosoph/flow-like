/// # ONNX Pose Estimation Nodes
use crate::onnx::NodeOnnxSession;
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::{PinOptions, ValueType},
    variable::VariableType,
};
use flow_like_catalog_core::{
    COCO_KEYPOINT_NAMES, COCO_SKELETON_CONNECTIONS, Keypoint, NodeImage, PoseDetection,
    SkeletonConnection,
};
#[cfg(feature = "execute")]
use flow_like_model_provider::ml::{
    ndarray::{Array3, Array4, Axis, s},
    ort::{
        inputs,
        session::{Session, SessionInputValue, SessionOutputs},
        value::Value,
    },
};
#[cfg(feature = "execute")]
use flow_like_types::{
    Error,
    image::{DynamicImage, GenericImageView, imageops::FilterType},
};
use flow_like_types::{Result, async_trait, json::json};
#[cfg(feature = "execute")]
use std::borrow::Cow;

#[cfg(feature = "execute")]
/// Pose Estimation Trait for Common Behavior
pub trait PoseEstimation {
    /// Preprocess image for pose estimation model
    fn make_inputs(
        &self,
        img: &DynamicImage,
    ) -> Result<Vec<(Cow<'_, str>, SessionInputValue<'_>)>, Error>;

    /// Postprocess model outputs to pose detections
    fn make_results(
        &self,
        outputs: SessionOutputs<'_>,
        original_width: u32,
        original_height: u32,
        conf_threshold: f32,
    ) -> Result<Vec<PoseDetection>, Error>;

    /// End-to-End Inference
    fn run(
        &self,
        session: &mut Session,
        img: &DynamicImage,
        conf_threshold: f32,
    ) -> Result<Vec<PoseDetection>, Error>;
}

/// YOLO-Pose model provider (YOLOv8-pose format)
pub struct YoloPoseLike {
    pub input_width: u32,
    pub input_height: u32,
    pub num_keypoints: u32,
}

impl Default for YoloPoseLike {
    fn default() -> Self {
        Self {
            input_width: 640,
            input_height: 640,
            num_keypoints: 17, // COCO keypoints
        }
    }
}

#[cfg(feature = "execute")]
impl PoseEstimation for YoloPoseLike {
    fn make_inputs(
        &self,
        img: &DynamicImage,
    ) -> Result<Vec<(Cow<'_, str>, SessionInputValue<'_>)>, Error> {
        let arr = img_to_arr(img, self.input_width, self.input_height)?;
        let value = Value::from_array(arr)?;
        let session_inputs = inputs![
            "images" => value
        ];
        Ok(session_inputs)
    }

    fn make_results(
        &self,
        outputs: SessionOutputs<'_>,
        original_width: u32,
        original_height: u32,
        conf_threshold: f32,
    ) -> Result<Vec<PoseDetection>, Error> {
        // YOLOv8-pose output: [1, num_predictions, 56] where:
        // - 0:4 = box (cx, cy, w, h)
        // - 4 = confidence
        // - 5:56 = 17 keypoints * 3 (x, y, conf)
        let output = outputs["output0"].try_extract_array::<f32>()?;
        let output = output.slice(s![0, .., ..]);

        let scale_x = original_width as f32 / self.input_width as f32;
        let scale_y = original_height as f32 / self.input_height as f32;

        let mut poses = Vec::new();

        // Transpose if needed: shape could be [56, N] or [N, 56]
        let (num_preds, feat_dim) = (output.shape()[0], output.shape()[1]);
        let transposed = if feat_dim > num_preds {
            // Already [N, 56] format
            false
        } else {
            // [56, N] format, need to transpose
            true
        };

        let num_detections = if transposed { feat_dim } else { num_preds };

        for i in 0..num_detections {
            let get_val = |idx: usize| -> f32 {
                if transposed {
                    output[[idx, i]]
                } else {
                    output[[i, idx]]
                }
            };

            let conf = get_val(4);
            if conf < conf_threshold {
                continue;
            }

            // Parse bounding box
            let cx = get_val(0) * scale_x;
            let cy = get_val(1) * scale_y;
            let w = get_val(2) * scale_x;
            let h = get_val(3) * scale_y;
            let x1 = cx - w / 2.0;
            let y1 = cy - h / 2.0;
            let x2 = cx + w / 2.0;
            let y2 = cy + h / 2.0;

            // Parse keypoints
            let mut keypoints = Vec::with_capacity(self.num_keypoints as usize);
            for k in 0..self.num_keypoints as usize {
                let base_idx = 5 + k * 3;
                let kp_x = get_val(base_idx) * scale_x;
                let kp_y = get_val(base_idx + 1) * scale_y;
                let kp_conf = get_val(base_idx + 2);

                keypoints.push(Keypoint {
                    index: k as u32,
                    x: kp_x,
                    y: kp_y,
                    confidence: kp_conf,
                    name: COCO_KEYPOINT_NAMES.get(k).map(|s| s.to_string()),
                });
            }

            // Build skeleton connections
            let connections: Vec<SkeletonConnection> = COCO_SKELETON_CONNECTIONS
                .iter()
                .map(|&(from, to)| SkeletonConnection {
                    from_idx: from,
                    to_idx: to,
                })
                .collect();

            poses.push(PoseDetection {
                keypoints,
                score: conf,
                bbox: Some((x1, y1, x2, y2)),
                connections,
            });
        }

        // Sort by confidence
        poses.sort_unstable_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        Ok(poses)
    }

    fn run(
        &self,
        session: &mut Session,
        img: &DynamicImage,
        conf_threshold: f32,
    ) -> Result<Vec<PoseDetection>, Error> {
        let (original_width, original_height) = img.dimensions();
        let inputs = self.make_inputs(img)?;
        let outputs = session.run(inputs)?;
        self.make_results(outputs, original_width, original_height, conf_threshold)
    }
}

/// MoveNet-like model provider
pub struct MoveNetLike {
    pub input_width: u32,
    pub input_height: u32,
}

impl Default for MoveNetLike {
    fn default() -> Self {
        Self {
            input_width: 256,
            input_height: 256,
        }
    }
}

#[cfg(feature = "execute")]
impl PoseEstimation for MoveNetLike {
    fn make_inputs(
        &self,
        img: &DynamicImage,
    ) -> Result<Vec<(Cow<'_, str>, SessionInputValue<'_>)>, Error> {
        // MoveNet expects int32 input in range [0, 255]
        let (img_width, img_height) = img.dimensions();
        let resized = if (img_width == self.input_width) && (img_height == self.input_height) {
            img.to_rgb8()
        } else {
            img.resize_exact(self.input_width, self.input_height, FilterType::Triangle)
                .to_rgb8()
        };

        let buf_i32: Vec<i32> = resized.into_raw().into_iter().map(|v| v as i32).collect();
        let arr = ndarray::Array4::from_shape_vec(
            (1, self.input_height as usize, self.input_width as usize, 3),
            buf_i32,
        )?;

        let value = Value::from_array(arr)?;
        let session_inputs = inputs![
            "input" => value
        ];
        Ok(session_inputs)
    }

    fn make_results(
        &self,
        outputs: SessionOutputs<'_>,
        original_width: u32,
        original_height: u32,
        conf_threshold: f32,
    ) -> Result<Vec<PoseDetection>, Error> {
        // MoveNet output: [1, 1, 17, 3] for single pose
        // or [1, 6, 56] for multi-pose
        let output = outputs["output_0"].try_extract_array::<f32>()?;
        let shape = output.shape();

        let mut poses = Vec::new();

        if shape.len() == 4 && shape[2] == 17 {
            // Single pose: [1, 1, 17, 3]
            let mut keypoints = Vec::with_capacity(17);
            for k in 0..17 {
                let y = output[[0, 0, k, 0]] * original_height as f32;
                let x = output[[0, 0, k, 1]] * original_width as f32;
                let conf = output[[0, 0, k, 2]];

                keypoints.push(Keypoint {
                    index: k as u32,
                    x,
                    y,
                    confidence: conf,
                    name: COCO_KEYPOINT_NAMES.get(k).map(|s| s.to_string()),
                });
            }

            let avg_conf: f32 = keypoints
                .iter()
                .filter(|k| k.confidence >= conf_threshold)
                .map(|k| k.confidence)
                .sum::<f32>()
                / 17.0;

            if avg_conf > 0.0 {
                let connections: Vec<SkeletonConnection> = COCO_SKELETON_CONNECTIONS
                    .iter()
                    .map(|&(from, to)| SkeletonConnection {
                        from_idx: from,
                        to_idx: to,
                    })
                    .collect();

                poses.push(PoseDetection {
                    keypoints,
                    score: avg_conf,
                    bbox: None,
                    connections,
                });
            }
        }

        Ok(poses)
    }

    fn run(
        &self,
        session: &mut Session,
        img: &DynamicImage,
        conf_threshold: f32,
    ) -> Result<Vec<PoseDetection>, Error> {
        let (original_width, original_height) = img.dimensions();
        let inputs = self.make_inputs(img)?;
        let outputs = session.run(inputs)?;
        self.make_results(outputs, original_width, original_height, conf_threshold)
    }
}

// ## Pose Estimation Utilities

#[cfg(feature = "execute")]
/// Load DynamicImage as Array4 normalized to 0-1
fn img_to_arr(img: &DynamicImage, width: u32, height: u32) -> Result<Array4<f32>, Error> {
    let (img_width, img_height) = img.dimensions();
    let buf_u8 = if (img_width == width) && (img_height == height) {
        img.to_rgb8().into_raw()
    } else {
        img.resize_exact(width, height, FilterType::Triangle)
            .into_rgb8()
            .into_raw()
    };

    let buf_f32: Vec<f32> = buf_u8.into_iter().map(|v| (v as f32) / 255.0).collect();
    let arr4 = Array3::from_shape_vec((height as usize, width as usize, 3), buf_f32)?
        .permuted_axes([2, 0, 1])
        .insert_axis(Axis(0));
    Ok(arr4)
}

#[crate::register_node]
#[derive(Default)]
/// # Pose Estimation Node
/// Detect human poses and keypoints using ONNX models
pub struct PoseEstimationNode {}

impl PoseEstimationNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for PoseEstimationNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "pose_estimation",
            "Pose Estimation",
            "Detect human poses and keypoints using ONNX models. Download models from: YOLOv8-Pose (https://docs.ultralytics.com/models/yolov8/), MoveNet (https://tfhub.dev/google/movenet/), HRNet (https://github.com/OAID/TengineKit)",
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
            "conf",
            "Confidence",
            "Minimum keypoint confidence threshold",
            VariableType::Float,
        )
        .set_options(PinOptions::new().set_range((0., 1.)).build())
        .set_default_value(Some(json!(0.3)));

        node.add_input_pin(
            "max_poses",
            "Max Poses",
            "Maximum number of poses to detect",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(10)));

        node.add_output_pin(
            "exec_out",
            "Output",
            "Done with the Execution",
            VariableType::Execution,
        );

        node.add_output_pin(
            "poses",
            "Poses",
            "Detected poses with keypoints",
            VariableType::Struct,
        )
        .set_schema::<PoseDetection>()
        .set_value_type(ValueType::Array);

        node
    }

    #[allow(unused_variables)]
    async fn run(&self, context: &mut ExecutionContext) -> Result<()> {
        #[cfg(feature = "execute")]
        {
            context.deactivate_exec_pin("exec_out").await?;

            let node_session: NodeOnnxSession = context.evaluate_pin("model").await?;
            let node_img: NodeImage = context.evaluate_pin("image_in").await?;
            let conf_threshold: f32 = context.evaluate_pin("conf").await.unwrap_or(0.3);
            let max_poses: usize = context.evaluate_pin("max_poses").await.unwrap_or(10);

            let mut poses = {
                let img = node_img.get_image(context).await?;
                let img_guard = img.lock().await;
                let session = node_session.get_session(context).await?;
                let mut session_guard = session.lock().await;

                // Determine input shape from session
                let (input_width, input_height) =
                    if let Some(input) = session_guard.session.inputs.first() {
                        if let Some(dims) = input.input_type.tensor_shape() {
                            let d = dims.len();
                            if d >= 2 {
                                (dims[d - 1] as u32, dims[d - 2] as u32)
                            } else {
                                (640, 640)
                            }
                        } else {
                            (640, 640)
                        }
                    } else {
                        (640, 640)
                    };

                // Use YoloPoseLike as default provider
                let provider = YoloPoseLike {
                    input_width,
                    input_height,
                    num_keypoints: 17,
                };

                provider.run(&mut session_guard.session, &img_guard, conf_threshold)?
            };

            poses.truncate(max_poses);
            context.set_pin_value("poses", json!(poses)).await?;
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
/// # Extract Keypoint Node
/// Extract a specific keypoint from a pose detection
pub struct ExtractKeypointNode {}

impl ExtractKeypointNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for ExtractKeypointNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "extract_keypoint",
            "Extract Keypoint",
            "Extract a specific keypoint from a pose by index or name",
            "AI/ML/ONNX",
        );

        node.add_icon("/flow/icons/find_model.svg");

        node.add_input_pin(
            "pose",
            "Pose",
            "Pose detection to extract keypoint from",
            VariableType::Struct,
        )
        .set_schema::<PoseDetection>();

        node.add_input_pin(
            "keypoint_idx",
            "Index",
            "Keypoint index (0-based)",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(0)));

        node.add_output_pin("x", "X", "Keypoint X coordinate", VariableType::Float);
        node.add_output_pin("y", "Y", "Keypoint Y coordinate", VariableType::Float);
        node.add_output_pin(
            "confidence",
            "Confidence",
            "Keypoint confidence score",
            VariableType::Float,
        );
        node.add_output_pin(
            "name",
            "Name",
            "Keypoint name (if available)",
            VariableType::String,
        );
        node.add_output_pin(
            "found",
            "Found",
            "Whether the keypoint was found",
            VariableType::Boolean,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> Result<()> {
        let pose: PoseDetection = context.evaluate_pin("pose").await?;
        let idx: i64 = context.evaluate_pin("keypoint_idx").await.unwrap_or(0);

        if let Some(kp) = pose.get_keypoint(idx as u32) {
            context.set_pin_value("x", json!(kp.x)).await?;
            context.set_pin_value("y", json!(kp.y)).await?;
            context
                .set_pin_value("confidence", json!(kp.confidence))
                .await?;
            context
                .set_pin_value("name", json!(kp.name.clone().unwrap_or_default()))
                .await?;
            context.set_pin_value("found", json!(true)).await?;
        } else {
            context.set_pin_value("x", json!(0.0)).await?;
            context.set_pin_value("y", json!(0.0)).await?;
            context.set_pin_value("confidence", json!(0.0)).await?;
            context.set_pin_value("name", json!("")).await?;
            context.set_pin_value("found", json!(false)).await?;
        }

        Ok(())
    }
}
