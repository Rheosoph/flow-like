/// # ONNX Depth Estimation Nodes
/// Monocular depth estimation from single images (MiDaS, DPT, Depth Anything)
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
use flow_like_types::image::{DynamicImage, GenericImageView, Rgb, imageops::FilterType};
use flow_like_types::{Result, anyhow, async_trait, json::json};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Depth map output from depth estimation models
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub struct DepthMap {
    /// Depth values normalized to 0-1 range (closer = lower values)
    pub values: Vec<f32>,
    /// Width of the depth map
    pub width: u32,
    /// Height of the depth map
    pub height: u32,
    /// Minimum depth value (before normalization)
    pub min_depth: f32,
    /// Maximum depth value (before normalization)
    pub max_depth: f32,
}

impl DepthMap {
    /// Get depth at a specific pixel
    pub fn get_depth(&self, x: u32, y: u32) -> Option<f32> {
        if x >= self.width || y >= self.height {
            return None;
        }
        let idx = (y * self.width + x) as usize;
        self.values.get(idx).copied()
    }

    /// Convert to grayscale image (closer = darker)
    #[cfg(feature = "execute")]
    pub fn to_image(&self) -> DynamicImage {
        use flow_like_types::image::{GrayImage, Luma};
        let mut img = GrayImage::new(self.width, self.height);
        for y in 0..self.height {
            for x in 0..self.width {
                let idx = (y * self.width + x) as usize;
                let depth = self.values.get(idx).copied().unwrap_or(0.0);
                let pixel = (depth * 255.0).clamp(0.0, 255.0) as u8;
                img.put_pixel(x, y, Luma([pixel]));
            }
        }
        DynamicImage::ImageLuma8(img)
    }

    /// Convert to colored depth visualization (rainbow colormap)
    #[cfg(feature = "execute")]
    pub fn to_colored_image(&self) -> DynamicImage {
        use flow_like_types::image::RgbImage;
        let mut img = RgbImage::new(self.width, self.height);
        for y in 0..self.height {
            for x in 0..self.width {
                let idx = (y * self.width + x) as usize;
                let depth = self.values.get(idx).copied().unwrap_or(0.0);
                let rgb = depth_to_rainbow(depth);
                img.put_pixel(x, y, rgb);
            }
        }
        DynamicImage::ImageRgb8(img)
    }
}

#[cfg(feature = "execute")]
fn depth_to_rainbow(depth: f32) -> Rgb<u8> {
    // Turbo colormap approximation (depth 0 = purple/blue, 1 = red/yellow)
    let t = depth.clamp(0.0, 1.0);
    let r = (34.61
        + t * (1172.33 - t * (10793.56 - t * (33300.12 - t * (38394.49 - t * 14825.05)))))
        .clamp(0.0, 255.0) as u8;
    let g = (23.31 + t * (557.33 + t * (1225.33 - t * (3574.96 - t * (1073.77 + t * 707.56)))))
        .clamp(0.0, 255.0) as u8;
    let b = (27.2 + t * (3211.1 - t * (15327.97 - t * (27814.0 - t * (22569.18 - t * 6838.66)))))
        .clamp(0.0, 255.0) as u8;
    Rgb([r, g, b])
}

/// Depth estimation model providers
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, Default)]
pub enum DepthProvider {
    /// MiDaS models (small, base, large)
    #[default]
    MiDaSLike,
    /// DPT (Dense Prediction Transformer) models
    DPTLike,
    /// Depth Anything models
    DepthAnythingLike,
    /// Generic depth model (single input, single output)
    Generic,
}

#[crate::register_node]
#[derive(Default)]
pub struct DepthEstimationNode {}

impl DepthEstimationNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for DepthEstimationNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "onnx_depth_estimation",
            "Depth Estimation",
            "Estimate depth from a single image using ONNX models. Download models from: MiDaS (https://github.com/isl-org/MiDaS/releases), DPT (https://huggingface.co/Intel/dpt-large/tree/main), Depth Anything (https://huggingface.co/depth-anything/Depth-Anything-V2-Small/tree/main)",
            "AI/ML/ONNX/Vision",
        );
        node.set_flowscript_name("onnx", "depthEstimation");
        node.set_version(1);

        node.add_icon("/flow/icons/depth.svg");

        node.add_input_pin(
            "exec_in",
            "Input",
            "Initiate Execution",
            VariableType::Execution,
        );

        node.add_input_pin(
            "model",
            "Model",
            "ONNX Depth Model Session",
            VariableType::Struct,
        )
        .set_schema::<NodeOnnxSession>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin("image", "Image", "Input Image", VariableType::Struct)
            .set_schema::<NodeImage>()
            .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin(
            "provider",
            "Provider",
            "Model provider type",
            VariableType::Struct,
        )
        .set_schema::<DepthProvider>()
        .set_options(
            PinOptions::new()
                .set_valid_values(vec![
                    "MiDaSLike".to_string(),
                    "DPTLike".to_string(),
                    "DepthAnythingLike".to_string(),
                    "Generic".to_string(),
                ])
                .build(),
        )
        .set_default_value(Some(json!(DepthProvider::MiDaSLike)));

        node.add_input_pin(
            "input_size",
            "Input Size",
            "Model input size (default 384 for MiDaS)",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(384)));

        node.add_output_pin("exec_out", "Output", "Done", VariableType::Execution);

        node.add_output_pin(
            "depth_map",
            "Depth Map",
            "Estimated depth map",
            VariableType::Struct,
        )
        .set_schema::<DepthMap>();

        node.add_output_pin(
            "depth_image",
            "Depth Image",
            "Grayscale depth visualization",
            VariableType::Struct,
        )
        .set_schema::<NodeImage>();

        node
    }

    #[allow(unused_variables)]
    async fn run(&self, context: &mut ExecutionContext) -> Result<()> {
        #[cfg(feature = "execute")]
        {
            context.deactivate_exec_pin("exec_out").await?;

            let model_ref: NodeOnnxSession = context.evaluate_pin("model").await?;
            let image: NodeImage = context.evaluate_pin("image").await?;
            let provider: DepthProvider =
                context.evaluate_pin("provider").await.unwrap_or_default();
            let input_size: i64 = context.evaluate_pin("input_size").await.unwrap_or(384);
            let input_size = input_size as u32;

            let session_wrapper = model_ref.get_session(context).await?;
            let mut session_guard = session_wrapper.lock().await;
            let session = &mut session_guard.session;

            let img_wrapper = image.get_image(context).await?;
            let dyn_image = img_wrapper.lock().await;
            let (orig_width, orig_height) = dyn_image.dimensions();

            // Preprocess: resize and normalize
            let resized = dyn_image.resize_exact(input_size, input_size, FilterType::Triangle);
            let rgb = resized.to_rgb8();

            // Create input tensor [1, 3, H, W] normalized to [0, 1] or [-1, 1] depending on model
            let mut input = Array4::<f32>::zeros((1, 3, input_size as usize, input_size as usize));
            for y in 0..input_size {
                for x in 0..input_size {
                    let pixel = rgb.get_pixel(x, y);
                    // MiDaS expects [0, 1] normalized with ImageNet mean/std
                    let mean = [0.485, 0.456, 0.406];
                    let std = [0.229, 0.224, 0.225];
                    input[[0, 0, y as usize, x as usize]] =
                        (pixel[0] as f32 / 255.0 - mean[0]) / std[0];
                    input[[0, 1, y as usize, x as usize]] =
                        (pixel[1] as f32 / 255.0 - mean[1]) / std[1];
                    input[[0, 2, y as usize, x as usize]] =
                        (pixel[2] as f32 / 255.0 - mean[2]) / std[2];
                }
            }

            let input_value = Value::from_array(input)?;
            let outputs = session.run(inputs![input_value])?;
            let output = outputs
                .iter()
                .next()
                .ok_or_else(|| anyhow!("No output from model"))?;
            let (_, tensor) = output;
            let depth_arr = tensor.try_extract_array::<f32>()?;

            // Extract depth values and normalize
            let shape = depth_arr.shape();
            let (out_h, out_w) = if shape.len() == 4 {
                (shape[2], shape[3])
            } else if shape.len() == 3 {
                (shape[1], shape[2])
            } else {
                (input_size as usize, input_size as usize)
            };

            let mut values: Vec<f32> = depth_arr.iter().copied().collect();
            let min_depth = values.iter().copied().fold(f32::INFINITY, f32::min);
            let max_depth = values.iter().copied().fold(f32::NEG_INFINITY, f32::max);

            // Normalize to [0, 1]
            let range = max_depth - min_depth;
            if range > 1e-6 {
                for v in &mut values {
                    *v = (*v - min_depth) / range;
                }
            }

            let depth_map = DepthMap {
                values,
                width: out_w as u32,
                height: out_h as u32,
                min_depth,
                max_depth,
            };

            // Create visualization
            let depth_dyn = depth_map.to_image();
            // Resize back to original size
            let depth_resized =
                depth_dyn.resize_exact(orig_width, orig_height, FilterType::Triangle);
            let depth_image = NodeImage::new(context, depth_resized).await;

            context.set_pin_value("depth_map", json!(depth_map)).await?;
            context
                .set_pin_value("depth_image", json!(depth_image))
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
pub struct DepthToPointCloudNode {}

#[async_trait]
impl NodeLogic for DepthToPointCloudNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "depth_to_point_cloud",
            "Depth to Point Cloud",
            "Convert depth map to 3D point cloud coordinates",
            "AI/ML/ONNX/Vision",
        );
        node.set_flowscript_name("onnx", "depthToPointCloud");

        node.add_icon("/flow/icons/3d.svg");

        node.add_input_pin(
            "exec_in",
            "Input",
            "Initiate Execution",
            VariableType::Execution,
        );

        node.add_input_pin(
            "depth_map",
            "Depth Map",
            "Input depth map",
            VariableType::Struct,
        )
        .set_schema::<DepthMap>();

        node.add_input_pin(
            "focal_length",
            "Focal Length",
            "Camera focal length (pixels)",
            VariableType::Float,
        )
        .set_default_value(Some(json!(500.0)));

        node.add_input_pin("scale", "Scale", "Depth scale factor", VariableType::Float)
            .set_default_value(Some(json!(1.0)));

        node.add_output_pin("exec_out", "Output", "Done", VariableType::Execution);

        node.add_output_pin(
            "points",
            "Points",
            "3D point coordinates [x, y, z]",
            VariableType::Generic,
        );

        node.add_output_pin(
            "point_count",
            "Count",
            "Number of points",
            VariableType::Integer,
        );

        node
    }

    #[allow(unused_variables)]
    async fn run(&self, context: &mut ExecutionContext) -> Result<()> {
        #[cfg(feature = "execute")]
        {
            context.deactivate_exec_pin("exec_out").await?;

            let depth_map: DepthMap = context.evaluate_pin("depth_map").await?;
            let focal_length: f64 = context.evaluate_pin("focal_length").await.unwrap_or(500.0);
            let scale: f64 = context.evaluate_pin("scale").await.unwrap_or(1.0);

            let cx = depth_map.width as f64 / 2.0;
            let cy = depth_map.height as f64 / 2.0;

            let mut points: Vec<[f64; 3]> =
                Vec::with_capacity((depth_map.width * depth_map.height) as usize);

            for y in 0..depth_map.height {
                for x in 0..depth_map.width {
                    if let Some(d) = depth_map.get_depth(x, y)
                        && d > 0.01
                    {
                        let z = d as f64 * scale;
                        let px = (x as f64 - cx) * z / focal_length;
                        let py = (y as f64 - cy) * z / focal_length;
                        points.push([px, py, z]);
                    }
                }
            }

            let point_count = points.len() as i64;

            context.set_pin_value("points", json!(points)).await?;
            context
                .set_pin_value("point_count", json!(point_count))
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
pub struct DepthColorizeNode {}

#[async_trait]
impl NodeLogic for DepthColorizeNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "depth_colorize",
            "Colorize Depth",
            "Convert depth map to rainbow-colored visualization",
            "AI/ML/ONNX/Vision",
        );
        node.set_flowscript_name("onnx", "depthColorize");

        node.add_icon("/flow/icons/palette.svg");

        node.add_input_pin(
            "exec_in",
            "Input",
            "Initiate Execution",
            VariableType::Execution,
        );

        node.add_input_pin(
            "depth_map",
            "Depth Map",
            "Input depth map",
            VariableType::Struct,
        )
        .set_schema::<DepthMap>();

        node.add_output_pin("exec_out", "Output", "Done", VariableType::Execution);

        node.add_output_pin(
            "colored_image",
            "Colored Image",
            "Rainbow-colored depth visualization",
            VariableType::Struct,
        )
        .set_schema::<NodeImage>();

        node
    }

    #[allow(unused_variables)]
    async fn run(&self, context: &mut ExecutionContext) -> Result<()> {
        #[cfg(feature = "execute")]
        {
            context.deactivate_exec_pin("exec_out").await?;

            let depth_map: DepthMap = context.evaluate_pin("depth_map").await?;
            let colored = depth_map.to_colored_image();
            let node_image = NodeImage::new(context, colored).await;

            context
                .set_pin_value("colored_image", json!(node_image))
                .await?;
            context.activate_exec_pin("exec_out").await?;
            Ok(())
        }

        #[cfg(not(feature = "execute"))]
        Err(anyhow!("Execute feature not enabled"))
    }
}
