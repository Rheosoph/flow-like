use crate::onnx::{
    NodeOnnxSession,
    tensor::{NamedTensor, TensorValues},
};
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::{PinOptions, ValueType},
    variable::VariableType,
};
use flow_like_ml_core::{Annotation, BoundingBox, TensorData};
use flow_like_types::{Result, anyhow, async_trait, json::json};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct GroundingClass {
    pub class_id: u32,
    pub token_indices: Vec<usize>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TokenAggregation {
    Maximum,
    Mean,
    Minimum,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct GroundingConfig {
    pub labels: Vec<String>,
    pub classes: Vec<GroundingClass>,
    pub logits_output: String,
    pub boxes_output: String,
    pub token_aggregation: TokenAggregation,
    pub score_threshold: f32,
    pub nms_iou_threshold: f32,
    pub max_detections: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct GroundingDetection {
    pub bounds: BoundingBox,
    pub confidence: f32,
    pub label: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct GroundingResult {
    pub annotation: Annotation,
    pub detections: Vec<GroundingDetection>,
}

fn float_tensor<'a>(outputs: &'a [NamedTensor], name: &str) -> Result<(&'a [usize], &'a [f32])> {
    let mut matches = outputs.iter().filter(|value| value.name == name);
    let tensor = matches
        .next()
        .ok_or_else(|| anyhow!("Missing output tensor '{name}'"))?;
    if matches.next().is_some() {
        return Err(anyhow!("Duplicate output tensor '{name}'"));
    }
    tensor.validate()?;
    let TensorValues::Float32(values) = &tensor.data else {
        return Err(anyhow!("Tensor '{name}' must contain float32 values"));
    };
    Ok((&tensor.shape, values))
}

fn sigmoid(value: f32) -> f32 {
    if value >= 0.0 {
        1.0 / (1.0 + (-value).exp())
    } else {
        let exp = value.exp();
        exp / (1.0 + exp)
    }
}

/// Decode normalized cxcywh boxes and token logits using the caller's label map.
/// Each query selects its highest scoring class before class-aware NMS.
pub fn decode_grounding(
    outputs: &[NamedTensor],
    config: &GroundingConfig,
) -> Result<GroundingResult> {
    flow_like_ml_core::validate_labels(&config.labels)?;
    if config.labels.is_empty()
        || config.labels.len() > 1024
        || config.classes.is_empty()
        || config.classes.len() > config.labels.len()
        || !(1..=4096).contains(&config.max_detections)
        || !config.score_threshold.is_finite()
        || !(0.0..=1.0).contains(&config.score_threshold)
        || !config.nms_iou_threshold.is_finite()
        || !(0.0..=1.0).contains(&config.nms_iou_threshold)
    {
        return Err(anyhow!(
            "Invalid GroundingDINO labels, thresholds, or detection limit"
        ));
    }
    let (shape, logits) = float_tensor(outputs, &config.logits_output)?;
    if shape.len() != 3 || shape[0] != 1 || !(1..=4096).contains(&shape[1]) || shape[2] == 0 {
        return Err(anyhow!(
            "GroundingDINO logits must have shape [1, queries, tokens]"
        ));
    }
    let queries = shape[1];
    let tokens = shape[2];
    let (box_shape, boxes) = float_tensor(outputs, &config.boxes_output)?;
    if box_shape != [1, queries, 4] {
        return Err(anyhow!(
            "GroundingDINO boxes must have shape [1, queries, 4]"
        ));
    }
    let mut ids = std::collections::HashSet::new();
    for class in &config.classes {
        let mut positions = std::collections::HashSet::new();
        if class.class_id as usize >= config.labels.len()
            || !ids.insert(class.class_id)
            || class.token_indices.is_empty()
            || class
                .token_indices
                .iter()
                .any(|p| *p >= tokens || !positions.insert(*p))
        {
            return Err(anyhow!(
                "Class token mappings need unique valid classes and token positions"
            ));
        }
    }
    let mut detections = Vec::new();
    for query in 0..queries {
        let row = &logits[query * tokens..(query + 1) * tokens];
        let mut best = None;
        for class in &config.classes {
            let scores = class.token_indices.iter().map(|&i| sigmoid(row[i]));
            let score = match config.token_aggregation {
                TokenAggregation::Maximum => scores.fold(0.0, f32::max),
                TokenAggregation::Minimum => scores.fold(1.0, f32::min),
                TokenAggregation::Mean => scores.sum::<f32>() / class.token_indices.len() as f32,
            };
            if best.is_none_or(|(_, current)| score > current) {
                best = Some((class.class_id, score));
            }
        }
        let Some((class_id, confidence)) = best else {
            continue;
        };
        if confidence < config.score_threshold {
            continue;
        }
        let [cx, cy, width, height] = boxes[query * 4..query * 4 + 4] else {
            unreachable!()
        };
        if [cx, cy, width, height]
            .iter()
            .any(|v| !(0.0..=1.0).contains(v))
        {
            return Err(anyhow!(
                "GroundingDINO boxes must be normalized cxcywh coordinates"
            ));
        }
        let bounds = BoundingBox {
            class_id,
            x_min: (cx - width / 2.0).max(0.0),
            y_min: (cy - height / 2.0).max(0.0),
            x_max: (cx + width / 2.0).min(1.0),
            y_max: (cy + height / 2.0).min(1.0),
        };
        if bounds.x_min >= bounds.x_max || bounds.y_min >= bounds.y_max {
            continue;
        }
        bounds.validate(config.labels.len())?;
        detections.push(GroundingDetection {
            label: config.labels[class_id as usize].clone(),
            bounds,
            confidence,
        });
    }
    detections.sort_by(|a, b| b.confidence.total_cmp(&a.confidence));
    let mut kept: Vec<GroundingDetection> = Vec::new();
    for detection in detections {
        if kept.iter().any(|existing| {
            existing.bounds.class_id == detection.bounds.class_id
                && existing.bounds.iou(&detection.bounds) > config.nms_iou_threshold as f64
        }) {
            continue;
        }
        kept.push(detection);
        if kept.len() == config.max_detections {
            break;
        }
    }
    Ok(GroundingResult {
        annotation: Annotation::Boxes {
            boxes: kept.iter().map(|d| d.bounds.clone()).collect(),
        },
        detections: kept,
    })
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SamPointKind {
    Background,
    Foreground,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SamPoint {
    pub x: f32,
    pub y: f32,
    pub kind: SamPointKind,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SamPrompt {
    pub image_width: usize,
    pub image_height: usize,
    pub points: Vec<SamPoint>,
    /// Original-image pixel coordinates [left, top, right, bottom].
    pub box_xyxy: Option<[f32; 4]>,
    /// Unthresholded previous decoder output, shaped [1,1,mask_size,mask_size].
    pub mask_logits: Option<TensorData>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SamConfig {
    pub encoder_size: usize,
    pub embedding_channels: usize,
    pub embedding_height: usize,
    pub embedding_width: usize,
    pub mask_size: usize,
    pub masks_output: String,
    pub iou_output: String,
    pub low_res_output: Option<String>,
    pub mask_threshold: f32,
}

impl Default for SamConfig {
    fn default() -> Self {
        Self {
            encoder_size: 1024,
            embedding_channels: 256,
            embedding_height: 64,
            embedding_width: 64,
            mask_size: 256,
            masks_output: "masks".into(),
            iou_output: "iou_predictions".into(),
            low_res_output: Some("low_res_masks".into()),
            mask_threshold: 0.0,
        }
    }
}

impl SamConfig {
    fn validate(&self) -> Result<()> {
        if [
            self.encoder_size,
            self.embedding_channels,
            self.embedding_height,
            self.embedding_width,
            self.mask_size,
        ]
        .iter()
        .any(|v| *v == 0 || *v > 4096)
            || !self.mask_threshold.is_finite()
            || self.masks_output.is_empty()
            || self.iou_output.is_empty()
        {
            return Err(anyhow!(
                "Invalid SAM dimensions, output names, or threshold"
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SamMask {
    pub width: usize,
    pub height: usize,
    pub foreground: Vec<bool>,
    pub predicted_iou: f32,
    pub candidate_index: usize,
    pub low_resolution_logits: Option<TensorData>,
}

fn validate_prompt(prompt: &SamPrompt) -> Result<()> {
    if prompt.image_width == 0
        || prompt.image_height == 0
        || prompt
            .image_width
            .checked_mul(prompt.image_height)
            .is_none_or(|v| v > 16_777_216)
        || prompt.points.len() > 4096
        || (prompt.points.is_empty() && prompt.box_xyxy.is_none() && prompt.mask_logits.is_none())
    {
        return Err(anyhow!(
            "SAM needs bounded image dimensions and at least one point, box, or mask prompt"
        ));
    }
    let valid = |x: f32, y: f32| {
        x.is_finite()
            && y.is_finite()
            && x >= 0.0
            && y >= 0.0
            && x <= prompt.image_width as f32
            && y <= prompt.image_height as f32
    };
    if prompt.points.iter().any(|point| !valid(point.x, point.y)) {
        return Err(anyhow!("SAM point lies outside the original image"));
    }
    if let Some([x0, y0, x1, y1]) = prompt.box_xyxy {
        if !valid(x0, y0) || !valid(x1, y1) || x1 <= x0 || y1 <= y0 {
            return Err(anyhow!(
                "SAM box must have positive area inside the original image"
            ));
        }
    }
    Ok(())
}

/// Build the standard SAM decoder inputs. Points use the rounded resized image
/// dimensions used by ResizeLongestSide, before bottom/right image padding.
pub fn sam_decoder_inputs(
    mut embeddings: NamedTensor,
    prompt: &SamPrompt,
    config: &SamConfig,
) -> Result<Vec<NamedTensor>> {
    config.validate()?;
    validate_prompt(prompt)?;
    embeddings.validate()?;
    if embeddings.shape
        != [
            1,
            config.embedding_channels,
            config.embedding_height,
            config.embedding_width,
        ]
        || !matches!(embeddings.data, TensorValues::Float32(_))
    {
        return Err(anyhow!(
            "SAM embeddings must be float32 [1, channels, height, width] matching the decoder configuration"
        ));
    }
    embeddings.name = "image_embeddings".into();
    let scale = config.encoder_size as f64 / prompt.image_width.max(prompt.image_height) as f64;
    let width = (prompt.image_width as f64 * scale + 0.5).floor();
    let height = (prompt.image_height as f64 * scale + 0.5).floor();
    if width < 1.0 || height < 1.0 {
        return Err(anyhow!("SAM resized image has a zero dimension"));
    }
    let transform = |x: f32, y: f32| {
        [
            x * (width / prompt.image_width as f64) as f32,
            y * (height / prompt.image_height as f64) as f32,
        ]
    };
    let mut coordinates = Vec::new();
    let mut labels = Vec::new();
    for point in &prompt.points {
        coordinates.extend_from_slice(&transform(point.x, point.y));
        labels.push(match point.kind {
            SamPointKind::Background => 0.0,
            SamPointKind::Foreground => 1.0,
        });
    }
    if let Some([x0, y0, x1, y1]) = prompt.box_xyxy {
        coordinates.extend_from_slice(&transform(x0, y0));
        coordinates.extend_from_slice(&transform(x1, y1));
        labels.extend_from_slice(&[2.0, 3.0]);
    } else {
        coordinates.extend_from_slice(&[0.0, 0.0]);
        labels.push(-1.0);
    }
    let mask_shape = vec![1, 1, config.mask_size, config.mask_size];
    let (mask, has_mask) = if let Some(mask) = &prompt.mask_logits {
        mask.validate(16_777_216)?;
        if mask.shape != mask_shape {
            return Err(anyhow!("SAM prior mask has an incompatible shape"));
        }
        (mask.values.clone(), 1.0)
    } else {
        (vec![0.0; config.mask_size * config.mask_size], 0.0)
    };
    let tensor = |name: &str, shape: Vec<usize>, values: Vec<f32>| NamedTensor {
        name: name.into(),
        shape,
        data: TensorValues::Float32(values),
    };
    let inputs = vec![
        embeddings,
        tensor("point_coords", vec![1, labels.len(), 2], coordinates),
        tensor("point_labels", vec![1, labels.len()], labels),
        tensor("mask_input", mask_shape, mask),
        tensor("has_mask_input", vec![1], vec![has_mask]),
        tensor(
            "orig_im_size",
            vec![2],
            vec![prompt.image_height as f32, prompt.image_width as f32],
        ),
    ];
    for value in &inputs {
        value.validate()?;
    }
    Ok(inputs)
}

pub fn decode_sam(
    outputs: &[NamedTensor],
    prompt: &SamPrompt,
    config: &SamConfig,
) -> Result<SamMask> {
    config.validate()?;
    validate_prompt(prompt)?;
    let (shape, masks) = float_tensor(outputs, &config.masks_output)?;
    if shape.len() != 4
        || shape[0] != 1
        || shape[1] == 0
        || shape[1] > 256
        || shape[2] != prompt.image_height
        || shape[3] != prompt.image_width
    {
        return Err(anyhow!(
            "SAM masks must be [1, candidates, original_height, original_width]"
        ));
    }
    let candidates = shape[1];
    let (iou_shape, scores) = float_tensor(outputs, &config.iou_output)?;
    if iou_shape != [1, candidates] {
        return Err(anyhow!("SAM IoU output must be [1, candidates]"));
    }
    let index = (0..candidates)
        .max_by(|a, b| scores[*a].total_cmp(&scores[*b]).then_with(|| b.cmp(a)))
        .ok_or_else(|| anyhow!("SAM produced no masks"))?;
    let pixels = prompt.image_width * prompt.image_height;
    let foreground = masks[index * pixels..(index + 1) * pixels]
        .iter()
        .map(|value| *value > config.mask_threshold)
        .collect();
    let low_resolution_logits = if let Some(name) = &config.low_res_output {
        let (shape, values) = float_tensor(outputs, name)?;
        if shape != [1, candidates, config.mask_size, config.mask_size] {
            return Err(anyhow!(
                "SAM low-resolution masks have incompatible dimensions"
            ));
        }
        let count = config.mask_size * config.mask_size;
        Some(TensorData {
            shape: vec![1, 1, config.mask_size, config.mask_size],
            values: values[index * count..(index + 1) * count].to_vec(),
        })
    } else {
        None
    };
    Ok(SamMask {
        width: prompt.image_width,
        height: prompt.image_height,
        foreground,
        predicted_iou: scores[index],
        candidate_index: index,
        low_resolution_logits,
    })
}

fn model_node(name: &str, title: &str, description: &str, script: &str) -> Node {
    let mut node = Node::new(name, title, description, "AI/ML/ONNX/Inspection");
    node.set_flowscript_name("onnx", script);
    node.add_input_pin("exec_in", "Input", "Run inference", VariableType::Execution);
    node.add_input_pin(
        "model",
        "Model",
        "Loaded compatible ONNX model",
        VariableType::Struct,
    )
    .set_schema::<NodeOnnxSession>()
    .set_options(PinOptions::new().set_enforce_schema(true).build());
    node.add_output_pin(
        "exec_out",
        "Done",
        "Inference completed",
        VariableType::Execution,
    );
    node
}

#[crate::register_node]
#[derive(Default)]
pub struct GroundingDinoDetectionNode;

#[async_trait]
impl NodeLogic for GroundingDinoDetectionNode {
    fn get_node(&self) -> Node {
        let mut node = model_node(
            "onnx_grounding_dino_detection",
            "GroundingDINO Detection",
            "Detect the configured classes with a compatible text-conditioned ONNX model",
            "groundingDino",
        );
        node.add_input_pin(
            "inputs",
            "Inputs",
            "Image and token tensors prepared for the supplied model",
            VariableType::Struct,
        )
        .set_schema::<NamedTensor>()
        .set_value_type(ValueType::Array)
        .set_options(PinOptions::new().set_enforce_schema(true).build());
        node.add_input_pin(
            "config",
            "Configuration",
            "Label-to-token mapping, output names and thresholds",
            VariableType::Struct,
        )
        .set_schema::<GroundingConfig>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());
        node.add_output_pin(
            "result",
            "Detections",
            "Normalized boxes with caller-supplied class IDs",
            VariableType::Struct,
        )
        .set_schema::<GroundingResult>();
        node
    }
    async fn run(&self, context: &mut ExecutionContext) -> Result<()> {
        #[cfg(feature = "execute")]
        {
            context.deactivate_exec_pin("exec_out").await?;
            let model: NodeOnnxSession = context.evaluate_pin("model").await?;
            let inputs: Vec<NamedTensor> = context.evaluate_pin("inputs").await?;
            let config: GroundingConfig = context.evaluate_pin("config").await?;
            let session = model.get_session(context).await?;
            let outputs =
                crate::onnx::tensor::infer_tensors(&mut session.lock().await.session, inputs)?;
            context
                .set_pin_value("result", json!(decode_grounding(&outputs, &config)?))
                .await?;
            context.activate_exec_pin("exec_out").await?;
            Ok(())
        }
        #[cfg(not(feature = "execute"))]
        {
            let _ = context;
            Err(anyhow!("ONNX execution requires the execute feature"))
        }
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct SamImageEncoderNode;

#[async_trait]
impl NodeLogic for SamImageEncoderNode {
    fn get_node(&self) -> Node {
        let mut node = model_node(
            "onnx_sam_image_encoder",
            "SAM Image Encoder",
            "Encode a preprocessed image using a compatible SAM ONNX encoder",
            "samEncode",
        );
        node.add_input_pin(
            "image",
            "Image Tensor",
            "Normalized and padded float32 NCHW image expected by the supplied encoder",
            VariableType::Struct,
        )
        .set_schema::<NamedTensor>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());
        node.add_input_pin(
            "output_name",
            "Embedding Output",
            "Name of the encoder embedding output",
            VariableType::String,
        )
        .set_default_value(Some(json!("image_embeddings")));
        node.add_output_pin(
            "embeddings",
            "Embeddings",
            "Image embeddings for SAM prompt decoding",
            VariableType::Struct,
        )
        .set_schema::<NamedTensor>();
        node
    }
    async fn run(&self, context: &mut ExecutionContext) -> Result<()> {
        #[cfg(feature = "execute")]
        {
            context.deactivate_exec_pin("exec_out").await?;
            let model: NodeOnnxSession = context.evaluate_pin("model").await?;
            let image: NamedTensor = context.evaluate_pin("image").await?;
            image.validate()?;
            if image.shape.len() != 4
                || image.shape[0] != 1
                || image.shape[1] != 3
                || !matches!(image.data, TensorValues::Float32(_))
            {
                return Err(anyhow!("SAM image must be float32 [1,3,height,width]"));
            }
            let output_name: String = context.evaluate_pin("output_name").await?;
            let session = model.get_session(context).await?;
            let outputs =
                crate::onnx::tensor::infer_tensors(&mut session.lock().await.session, vec![image])?;
            let (shape, _) = float_tensor(&outputs, &output_name)?;
            if shape.len() != 4 || shape[0] != 1 {
                return Err(anyhow!("SAM encoder must output NCHW embeddings"));
            }
            let embeddings = outputs
                .into_iter()
                .find(|v| v.name == output_name)
                .ok_or_else(|| anyhow!("SAM embedding output is missing"))?;
            context
                .set_pin_value("embeddings", json!(embeddings))
                .await?;
            context.activate_exec_pin("exec_out").await?;
            Ok(())
        }
        #[cfg(not(feature = "execute"))]
        {
            let _ = context;
            Err(anyhow!("ONNX execution requires the execute feature"))
        }
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct SamPromptSegmentationNode;

#[async_trait]
impl NodeLogic for SamPromptSegmentationNode {
    fn get_node(&self) -> Node {
        let mut node = model_node(
            "onnx_sam_prompt_segmentation",
            "SAM Prompt Segmentation",
            "Decode point, box and previous-mask prompts with a compatible SAM ONNX decoder",
            "samSegment",
        );
        node.add_input_pin(
            "embeddings",
            "Embeddings",
            "Float32 image encoder embeddings",
            VariableType::Struct,
        )
        .set_schema::<NamedTensor>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());
        node.add_input_pin(
            "prompt",
            "Prompt",
            "Points and boxes in original-image pixel coordinates",
            VariableType::Struct,
        )
        .set_schema::<SamPrompt>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());
        node.add_input_pin(
            "config",
            "Configuration",
            "SAM encoder geometry and decoder output names",
            VariableType::Struct,
        )
        .set_schema::<SamConfig>()
        .set_options(PinOptions::new().set_enforce_schema(true).build())
        .set_default_value(Some(json!(SamConfig::default())));
        node.add_output_pin(
            "mask",
            "Mask",
            "Highest predicted-IoU mask with optional reusable logits",
            VariableType::Struct,
        )
        .set_schema::<SamMask>();
        node
    }
    async fn run(&self, context: &mut ExecutionContext) -> Result<()> {
        #[cfg(feature = "execute")]
        {
            context.deactivate_exec_pin("exec_out").await?;
            let model: NodeOnnxSession = context.evaluate_pin("model").await?;
            let embeddings: NamedTensor = context.evaluate_pin("embeddings").await?;
            let prompt: SamPrompt = context.evaluate_pin("prompt").await?;
            let config: SamConfig = context.evaluate_pin("config").await?;
            let inputs = sam_decoder_inputs(embeddings, &prompt, &config)?;
            let session = model.get_session(context).await?;
            let outputs =
                crate::onnx::tensor::infer_tensors(&mut session.lock().await.session, inputs)?;
            context
                .set_pin_value("mask", json!(decode_sam(&outputs, &prompt, &config)?))
                .await?;
            context.activate_exec_pin("exec_out").await?;
            Ok(())
        }
        #[cfg(not(feature = "execute"))]
        {
            let _ = context;
            Err(anyhow!("ONNX execution requires the execute feature"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inspection_nodes_register_unique_ids_and_tensor_pin_contracts() {
        use flow_like::flow::pin::{Pin, schemas_are_compatible};

        let nodes = [
            GroundingDinoDetectionNode.get_node(),
            SamImageEncoderNode.get_node(),
            SamPromptSegmentationNode.get_node(),
            crate::onnx::tensor::TensorInferenceNode.get_node(),
        ];
        let catalog = crate::get_catalog();
        for node in &nodes {
            assert_eq!(
                catalog
                    .iter()
                    .filter(|entry| entry.get_node().name == node.name)
                    .count(),
                1,
                "{} must register exactly once",
                node.name
            );
            assert_eq!(
                catalog
                    .iter()
                    .filter(|entry| {
                        let other = entry.get_node();
                        other.flowscript_namespace() == node.flowscript_namespace()
                            && other.flowscript_alias() == node.flowscript_alias()
                    })
                    .count(),
                1,
                "{} has a duplicate FlowScript name",
                node.name
            );
            for name in ["exec_in", "exec_out"] {
                let pin = node.get_pin_by_name(name).unwrap();
                assert_eq!(pin.data_type, VariableType::Execution);
                assert_eq!(pin.value_type, ValueType::Normal);
            }
            let model = node.get_pin_by_name("model").unwrap();
            assert_eq!(model.schema, Pin::schema_string_for::<NodeOnnxSession>());
        }
        let grounding_inputs = nodes[0].get_pin_by_name("inputs").unwrap();
        let generic_outputs = nodes[3].get_pin_by_name("outputs").unwrap();
        assert_eq!(grounding_inputs.value_type, ValueType::Array);
        assert_eq!(
            grounding_inputs.schema,
            Pin::schema_string_for::<NamedTensor>()
        );
        assert!(schemas_are_compatible(
            generic_outputs.schema.as_deref(),
            grounding_inputs.schema.as_deref()
        ));

        let embeddings = nodes[1].get_pin_by_name("embeddings").unwrap();
        let decoder_embeddings = nodes[2].get_pin_by_name("embeddings").unwrap();
        assert_eq!(embeddings.value_type, ValueType::Normal);
        assert_eq!(embeddings.data_type, VariableType::Struct);
        assert_eq!(embeddings.schema, Pin::schema_string_for::<NamedTensor>());
        assert!(schemas_are_compatible(
            embeddings.schema.as_deref(),
            decoder_embeddings.schema.as_deref()
        ));
        let config = nodes[2].get_pin_by_name("config").unwrap();
        let default: SamConfig =
            flow_like_types::json::from_slice(config.default_value.as_ref().unwrap()).unwrap();
        default.validate().unwrap();
    }

    fn tensor(name: &str, shape: &[usize], values: &[f32]) -> NamedTensor {
        NamedTensor {
            name: name.into(),
            shape: shape.to_vec(),
            data: TensorValues::Float32(values.to_vec()),
        }
    }
    #[test]
    fn grounding_uses_custom_token_map_and_class_aware_nms() {
        let outputs = vec![
            tensor(
                "logits",
                &[1, 3, 4],
                &[
                    -9.0, 4.0, -9.0, -9.0, -9.0, 3.0, -9.0, -9.0, -9.0, -9.0, -9.0, 5.0,
                ],
            ),
            tensor(
                "boxes",
                &[1, 3, 4],
                &[0.5, 0.5, 0.4, 0.4, 0.51, 0.5, 0.4, 0.4, 0.5, 0.5, 0.4, 0.4],
            ),
        ];
        let mut config = GroundingConfig {
            labels: vec!["scratch".into(), "missing screw".into()],
            classes: vec![
                GroundingClass {
                    class_id: 0,
                    token_indices: vec![1],
                },
                GroundingClass {
                    class_id: 1,
                    token_indices: vec![3],
                },
            ],
            logits_output: "logits".into(),
            boxes_output: "boxes".into(),
            token_aggregation: TokenAggregation::Maximum,
            score_threshold: 0.5,
            nms_iou_threshold: 0.5,
            max_detections: 10,
        };
        let result = decode_grounding(&outputs, &config).unwrap();
        assert_eq!(result.detections.len(), 2);
        assert_eq!(result.detections[0].label, "missing screw");
        assert_eq!(result.detections[1].bounds.class_id, 0);
        assert!((result.detections[0].bounds.x_min - 0.3).abs() < 1e-6);
        config.classes[0].token_indices = vec![4];
        assert!(decode_grounding(&outputs, &config).is_err());
    }
    fn prompt() -> SamPrompt {
        SamPrompt {
            image_width: 800,
            image_height: 401,
            points: vec![SamPoint {
                x: 400.0,
                y: 200.5,
                kind: SamPointKind::Foreground,
            }],
            box_xyxy: None,
            mask_logits: None,
        }
    }
    #[test]
    fn sam_coordinates_follow_rounded_resize_and_padding_labels() {
        let config = SamConfig::default();
        let mut prompt = prompt();
        let embeddings = tensor("embedding", &[1, 256, 64, 64], &vec![0.0; 256 * 64 * 64]);
        let inputs = sam_decoder_inputs(embeddings.clone(), &prompt, &config).unwrap();
        let (_, coordinates) = float_tensor(&inputs, "point_coords").unwrap();
        assert_eq!(coordinates, &[512.0, 256.5, 0.0, 0.0]);
        assert_eq!(
            float_tensor(&inputs, "point_labels").unwrap().1,
            &[1.0, -1.0]
        );
        prompt.box_xyxy = Some([0.0, 0.0, 800.0, 401.0]);
        let inputs = sam_decoder_inputs(embeddings, &prompt, &config).unwrap();
        assert_eq!(
            float_tensor(&inputs, "point_labels").unwrap().1,
            &[1.0, 2.0, 3.0]
        );
        assert_eq!(
            &float_tensor(&inputs, "point_coords").unwrap().1[4..],
            &[1024.0, 513.0]
        );
    }
    #[test]
    fn sam_selects_best_iou_and_preserves_matching_low_resolution_logits() {
        let mut prompt = prompt();
        prompt.image_width = 2;
        prompt.image_height = 1;
        prompt.points[0].x = 0.5;
        prompt.points[0].y = 0.5;
        let config = SamConfig {
            mask_size: 2,
            ..Default::default()
        };
        let outputs = vec![
            tensor("masks", &[1, 2, 1, 2], &[1.0, 1.0, -1.0, 2.0]),
            tensor("iou_predictions", &[1, 2], &[0.4, 0.9]),
            tensor(
                "low_res_masks",
                &[1, 2, 2, 2],
                &[1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0],
            ),
        ];
        let mask = decode_sam(&outputs, &prompt, &config).unwrap();
        assert_eq!(mask.foreground, vec![false, true]);
        assert_eq!(mask.candidate_index, 1);
        assert_eq!(
            mask.low_resolution_logits.unwrap().values,
            vec![5.0, 6.0, 7.0, 8.0]
        );
        prompt.points[0].x = 3.0;
        assert!(decode_sam(&outputs, &prompt, &config).is_err());
    }
}
