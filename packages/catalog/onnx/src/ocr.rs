/// # ONNX OCR Nodes
/// Text detection and recognition from images
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
use flow_like_types::{Result, async_trait, json::json};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Detected text region in an image
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub struct TextRegion {
    /// Bounding box [x, y, width, height]
    pub bbox: [f32; 4],
    /// Polygon points for rotated text [[x1,y1], [x2,y2], [x3,y3], [x4,y4]]
    pub polygon: [[f32; 2]; 4],
    /// Detection confidence
    pub confidence: f32,
}

/// Recognized text from OCR
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub struct RecognizedText {
    /// The recognized text string
    pub text: String,
    /// Recognition confidence
    pub confidence: f32,
    /// Character-level confidences (if available)
    pub char_confidences: Vec<f32>,
}

/// Full OCR result combining detection and recognition
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub struct OcrResult {
    /// Detected and recognized text regions
    pub regions: Vec<OcrRegion>,
    /// Full text concatenated
    pub full_text: String,
}

/// Single OCR region with detection and recognition
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub struct OcrRegion {
    pub region: TextRegion,
    pub text: RecognizedText,
}

#[crate::register_node]
#[derive(Default)]
pub struct TextDetectionNode {}

impl TextDetectionNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for TextDetectionNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "onnx_text_detection",
            "Text Detection",
            "Detect text regions in images. Download models from: CRAFT (https://huggingface.co/quocanh34/craft_text_detection_onnx), DBNet (https://huggingface.co/Xenova/dbnet_resnet50_onnx), EAST (https://www.dropbox.com/s/r2ingd0l3zt8hxs/frozen_east_text_detection.tar.gz)",
            "AI/ML/ONNX/OCR",
        );
        node.set_version(1);

        node.add_icon("/flow/icons/text-search.svg");

        node.add_input_pin(
            "exec_in",
            "Input",
            "Initiate Execution",
            VariableType::Execution,
        );

        node.add_input_pin(
            "model",
            "Model",
            "ONNX Text Detection Model",
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
            "input_size",
            "Input Size",
            "Model input size",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(640)));

        node.add_output_pin("exec_out", "Output", "Done", VariableType::Execution);

        node.add_output_pin(
            "regions",
            "Regions",
            "Detected text regions",
            VariableType::Generic,
        );

        node.add_output_pin(
            "count",
            "Count",
            "Number of detected regions",
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
            let input_size: i64 = context.evaluate_pin("input_size").await.unwrap_or(640);
            let input_size = input_size as u32;

            let session_wrapper = model_ref.get_session(context).await?;
            let mut session_guard = session_wrapper.lock().await;
            let session = &mut session_guard.session;

            let img_wrapper = image.get_image(context).await?;
            let dyn_image = img_wrapper.lock().await;
            let (orig_w, orig_h) = dyn_image.dimensions();

            // Preprocess
            let resized = dyn_image.resize_exact(input_size, input_size, FilterType::Triangle);
            let rgb = resized.to_rgb8();

            let mut input = Array4::<f32>::zeros((1, 3, input_size as usize, input_size as usize));
            for y in 0..input_size {
                for x in 0..input_size {
                    let pixel = rgb.get_pixel(x, y);
                    input[[0, 0, y as usize, x as usize]] = pixel[0] as f32 / 255.0;
                    input[[0, 1, y as usize, x as usize]] = pixel[1] as f32 / 255.0;
                    input[[0, 2, y as usize, x as usize]] = pixel[2] as f32 / 255.0;
                }
            }

            let input_value = Value::from_array(input)?;
            let outputs = session.run(inputs![input_value])?;

            // Parse output - depends on model architecture
            // Generic approach: look for score map and geometry
            let mut regions: Vec<TextRegion> = Vec::new();

            // Try to extract score map from first output
            if let Some((_, tensor)) = outputs.iter().next()
                && let Ok(scores) = tensor.try_extract_array::<f32>()
            {
                let shape = scores.shape();

                // Simple connected component analysis on score map
                if shape.len() >= 3 {
                    let h = if shape.len() == 4 { shape[2] } else { shape[1] };
                    let w = if shape.len() == 4 { shape[3] } else { shape[2] };

                    let scale_x = orig_w as f32 / w as f32;
                    let scale_y = orig_h as f32 / h as f32;

                    // Find connected regions above threshold
                    let mut visited = vec![false; h * w];
                    for y in 0..h {
                        for x in 0..w {
                            let idx = y * w + x;
                            if visited[idx] {
                                continue;
                            }

                            let score = if shape.len() == 4 {
                                scores[[0, 0, y, x]]
                            } else {
                                scores[[0, y, x]]
                            };

                            if score > threshold as f32 {
                                // BFS to find connected region
                                let mut min_x = x;
                                let mut max_x = x;
                                let mut min_y = y;
                                let mut max_y = y;
                                let mut sum_score = 0.0f32;
                                let mut count = 0;

                                let mut stack = vec![(x, y)];
                                while let Some((cx, cy)) = stack.pop() {
                                    let cidx = cy * w + cx;
                                    if visited[cidx] {
                                        continue;
                                    }
                                    visited[cidx] = true;

                                    let s = if shape.len() == 4 {
                                        scores[[0, 0, cy, cx]]
                                    } else {
                                        scores[[0, cy, cx]]
                                    };

                                    if s > threshold as f32 {
                                        min_x = min_x.min(cx);
                                        max_x = max_x.max(cx);
                                        min_y = min_y.min(cy);
                                        max_y = max_y.max(cy);
                                        sum_score += s;
                                        count += 1;

                                        // Check neighbors
                                        if cx > 0 {
                                            stack.push((cx - 1, cy));
                                        }
                                        if cx < w - 1 {
                                            stack.push((cx + 1, cy));
                                        }
                                        if cy > 0 {
                                            stack.push((cx, cy - 1));
                                        }
                                        if cy < h - 1 {
                                            stack.push((cx, cy + 1));
                                        }
                                    }
                                }

                                if count > 4 {
                                    let x1 = min_x as f32 * scale_x;
                                    let y1 = min_y as f32 * scale_y;
                                    let x2 = (max_x + 1) as f32 * scale_x;
                                    let y2 = (max_y + 1) as f32 * scale_y;

                                    regions.push(TextRegion {
                                        bbox: [x1, y1, x2 - x1, y2 - y1],
                                        polygon: [[x1, y1], [x2, y1], [x2, y2], [x1, y2]],
                                        confidence: sum_score / count as f32,
                                    });
                                }
                            }
                        }
                    }
                }
            }

            let count = regions.len() as i64;
            context.set_pin_value("regions", json!(regions)).await?;
            context.set_pin_value("count", json!(count)).await?;
            context.activate_exec_pin("exec_out").await?;
            Ok(())
        }

        #[cfg(not(feature = "execute"))]
        Err(flow_like_types::anyhow!("Execute feature not enabled"))
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct TextRecognitionNode {}

impl TextRecognitionNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for TextRecognitionNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "onnx_text_recognition",
            "Text Recognition",
            "Recognize text from cropped text regions. Download models from: CRNN (https://huggingface.co/Xenova/crnn_onnx), TrOCR (https://huggingface.co/microsoft/trocr-base-printed), PaddleOCR (https://huggingface.co/aapot/paddleocr-onnx)",
            "AI/ML/ONNX/OCR",
        );
        node.set_version(1);

        node.add_icon("/flow/icons/text.svg");

        node.add_input_pin(
            "exec_in",
            "Input",
            "Initiate Execution",
            VariableType::Execution,
        );

        node.add_input_pin(
            "model",
            "Model",
            "ONNX Text Recognition Model",
            VariableType::Struct,
        )
        .set_schema::<NodeOnnxSession>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin(
            "image",
            "Image",
            "Cropped text region image",
            VariableType::Struct,
        )
        .set_schema::<NodeImage>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin("charset", "Charset", "Character set for decoding", VariableType::String)
            .set_default_value(Some(json!("0123456789abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ!\"#$%&'()*+,-./:;<=>?@[\\]^_`{|}~ ")));

        node.add_input_pin(
            "input_height",
            "Input Height",
            "Model expected input height",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(32)));

        node.add_output_pin("exec_out", "Output", "Done", VariableType::Execution);

        node.add_output_pin(
            "result",
            "Result",
            "Recognition result",
            VariableType::Struct,
        )
        .set_schema::<RecognizedText>();

        node.add_output_pin(
            "text",
            "Text",
            "Recognized text string",
            VariableType::String,
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
            let charset: String = context.evaluate_pin("charset").await?;
            let input_height: i64 = context.evaluate_pin("input_height").await.unwrap_or(32);
            let input_height = input_height as u32;

            let session_wrapper = model_ref.get_session(context).await?;
            let mut session_guard = session_wrapper.lock().await;
            let session = &mut session_guard.session;

            let img_wrapper = image.get_image(context).await?;
            let dyn_image = img_wrapper.lock().await;
            let (orig_w, orig_h) = dyn_image.dimensions();

            // Maintain aspect ratio, resize to fixed height
            let aspect = orig_w as f32 / orig_h as f32;
            let input_width = (input_height as f32 * aspect).round() as u32;
            let resized = dyn_image.resize_exact(input_width, input_height, FilterType::Triangle);
            let gray = resized.to_luma8();

            // Create input tensor [1, 1, H, W] or [1, 3, H, W]
            let mut input =
                Array4::<f32>::zeros((1, 1, input_height as usize, input_width as usize));
            for y in 0..input_height {
                for x in 0..input_width {
                    let pixel = gray.get_pixel(x, y);
                    input[[0, 0, y as usize, x as usize]] = pixel[0] as f32 / 255.0;
                }
            }

            let input_value = Value::from_array(input)?;
            let outputs = session.run(inputs![input_value])?;

            // CTC decode the output
            let chars: Vec<char> = charset.chars().collect();
            let mut text = String::new();
            let mut char_confidences = Vec::new();
            let mut prev_idx: Option<usize> = None;

            if let Some((_, tensor)) = outputs.iter().next()
                && let Ok(logits) = tensor.try_extract_array::<f32>()
            {
                let shape = logits.shape();

                // Expect shape [1, T, num_classes] or [T, num_classes]
                let (seq_len, num_classes) = if shape.len() == 3 {
                    (shape[1], shape[2])
                } else {
                    (shape[0], shape[1])
                };

                for t in 0..seq_len {
                    // Find max class
                    let mut max_idx = 0;
                    let mut max_val = f32::NEG_INFINITY;

                    for c in 0..num_classes {
                        let val = if shape.len() == 3 {
                            logits[[0, t, c]]
                        } else {
                            logits[[t, c]]
                        };
                        if val > max_val {
                            max_val = val;
                            max_idx = c;
                        }
                    }

                    // CTC blank is usually 0 or last class
                    let blank_idx = 0;
                    if max_idx != blank_idx
                        && Some(max_idx) != prev_idx
                        && let Some(ch) = chars.get(max_idx.saturating_sub(1))
                    {
                        text.push(*ch);
                        // Softmax for confidence
                        let conf = (max_val).exp();
                        char_confidences.push(conf);
                    }
                    prev_idx = Some(max_idx);
                }
            }

            let avg_conf = if char_confidences.is_empty() {
                0.0
            } else {
                char_confidences.iter().sum::<f32>() / char_confidences.len() as f32
            };

            let result = RecognizedText {
                text: text.clone(),
                confidence: avg_conf,
                char_confidences,
            };

            context.set_pin_value("result", json!(result)).await?;
            context.set_pin_value("text", json!(text)).await?;
            context.activate_exec_pin("exec_out").await?;
            Ok(())
        }

        #[cfg(not(feature = "execute"))]
        Err(flow_like_types::anyhow!("Execute feature not enabled"))
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct CropTextRegionsNode {}

#[async_trait]
impl NodeLogic for CropTextRegionsNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "crop_text_regions",
            "Crop Text Regions",
            "Crop detected text regions from image for recognition",
            "AI/ML/ONNX/OCR",
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

        node.add_input_pin(
            "regions",
            "Regions",
            "Detected text regions",
            VariableType::Generic,
        );

        node.add_input_pin(
            "padding",
            "Padding",
            "Padding around regions (pixels)",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(2)));

        node.add_output_pin("exec_out", "Output", "Done", VariableType::Execution);

        node.add_output_pin(
            "crops",
            "Crops",
            "Cropped region images",
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
            let regions: Vec<TextRegion> = context.evaluate_pin("regions").await?;
            let padding: i64 = context.evaluate_pin("padding").await.unwrap_or(2);

            let img_wrapper = image.get_image(context).await?;
            let dyn_image = img_wrapper.lock().await;
            let (img_w, img_h) = dyn_image.dimensions();

            let mut crops: Vec<NodeImage> = Vec::new();

            for region in regions {
                let x = (region.bbox[0] - padding as f32).max(0.0) as u32;
                let y = (region.bbox[1] - padding as f32).max(0.0) as u32;
                let w = ((region.bbox[2] + 2.0 * padding as f32) as u32).min(img_w - x);
                let h = ((region.bbox[3] + 2.0 * padding as f32) as u32).min(img_h - y);

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
        Err(flow_like_types::anyhow!("Execute feature not enabled"))
    }
}
