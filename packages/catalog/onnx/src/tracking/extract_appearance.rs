use super::{
    appearance_models::{DEFAULT_MODEL, model_options},
    embedding,
    types::{AppearanceObservation, TrackState},
};
#[cfg(feature = "execute")]
use super::{
    appearance_models::{ModelChoice, load_session},
    registry::now_ms,
};
use crate::onnx::NodeOnnxSession;
#[cfg(feature = "execute")]
use flow_like::flow::execution::LogLevel;
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic, NodeScores},
    pin::{PinOptions, ValueType},
    variable::VariableType,
};
use flow_like_catalog_core::{BoundingBox, FlowPath, NodeImage};
#[cfg(feature = "execute")]
use flow_like_model_provider::ml::{
    ndarray::Array4,
    ort::{
        inputs,
        session::Session,
        value::{TensorElementType, Value},
    },
};
#[cfg(feature = "execute")]
use flow_like_types::image::{DynamicImage, GenericImageView, imageops::FilterType};
use flow_like_types::{Result, anyhow, async_trait, json::json};

const NORMALIZATIONS: [&str; 4] = ["imagenet", "raw", "zero_one", "minus_one_one"];
const IMAGENET_MEAN: [f32; 3] = [0.485, 0.456, 0.406];
const IMAGENET_STD: [f32; 3] = [0.229, 0.224, 0.225];
const MAX_INPUT_SIDE: i64 = 4096;

/// Pixel scaling a re-identification model expects on its input.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Normalization {
    /// `(x / 255 - mean) / std` with the ImageNet channel statistics (torchreid / OSNet exports)
    Imagenet,
    /// `x` unchanged in 0–255, for models that normalize internally (FastReID `onnx_export.py`)
    Raw,
    /// `x / 255`
    ZeroOne,
    /// `x / 127.5 - 1`
    MinusOneOne,
}

impl Normalization {
    pub fn parse(value: &str) -> Result<Self> {
        match value {
            "imagenet" => Ok(Self::Imagenet),
            "raw" => Ok(Self::Raw),
            "zero_one" => Ok(Self::ZeroOne),
            "minus_one_one" => Ok(Self::MinusOneOne),
            other => Err(anyhow!(
                "`normalization` must be one of {}, got {other:?}",
                NORMALIZATIONS.join(", ")
            )),
        }
    }

    /// Scales one 8-bit value of RGB channel `channel` (0..3).
    pub fn apply(self, channel: usize, value: u8) -> f32 {
        let value = f32::from(value);
        match self {
            Self::Imagenet => (value / 255.0 - IMAGENET_MEAN[channel]) / IMAGENET_STD[channel],
            Self::Raw => value,
            Self::ZeroOne => value / 255.0,
            Self::MinusOneOne => value / 127.5 - 1.0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Layout {
    Nchw,
    Nhwc,
}

/// Tensor geometry the model expects, resolved from its declared input shape.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ModelInput {
    pub layout: Layout,
    pub width: u32,
    pub height: u32,
    /// Crops per inference call
    pub batch: usize,
}

impl ModelInput {
    /// Resolves `[N,3,H,W]` or `[N,H,W,3]`, where dimensions ≤ 0 are dynamic. Dynamic spatial
    /// dimensions take the fallbacks; a dynamic batch takes `batch_size`, a fixed batch of 1 means
    /// one crop per call and any larger fixed batch is rejected.
    pub fn resolve(
        shape: &[i64],
        fallback_width: u32,
        fallback_height: u32,
        batch_size: usize,
    ) -> Result<Self> {
        let &[batch, d1, d2, d3] = shape else {
            return Err(anyhow!(
                "Appearance model input must be a 4-D image tensor [N,3,H,W] or [N,H,W,3], got shape {shape:?}"
            ));
        };
        let (layout, height, width) = if d1 == 3 {
            (Layout::Nchw, d2, d3)
        } else if d3 == 3 {
            (Layout::Nhwc, d1, d2)
        } else if d1 <= 0 {
            (Layout::Nchw, d2, d3)
        } else {
            return Err(anyhow!(
                "Appearance model input {shape:?} has no 3-channel RGB dimension"
            ));
        };
        let batch = match batch {
            dynamic if dynamic <= 0 => batch_size,
            1 => 1,
            fixed => {
                return Err(anyhow!(
                    "Appearance model has a fixed batch size of {fixed}; use a model with batch size 1 or a dynamic batch dimension"
                ));
            }
        };
        Ok(Self {
            layout,
            width: spatial_dim(width, fallback_width, "width")?,
            height: spatial_dim(height, fallback_height, "height")?,
            batch,
        })
    }

    /// Values in one crop's tensor slice, or `None` on overflow.
    pub fn sample_len(&self) -> Option<usize> {
        (self.width as usize)
            .checked_mul(self.height as usize)?
            .checked_mul(3)
    }

    pub fn tensor_shape(&self, batch: usize) -> (usize, usize, usize, usize) {
        let (width, height) = (self.width as usize, self.height as usize);
        match self.layout {
            Layout::Nchw => (batch, 3, height, width),
            Layout::Nhwc => (batch, height, width, 3),
        }
    }
}

fn spatial_dim(dim: i64, fallback: u32, axis: &str) -> Result<u32> {
    if dim <= 0 {
        return Ok(fallback);
    }
    u32::try_from(dim).map_err(|_| anyhow!("Appearance model input {axis} {dim} is too large"))
}

/// Validated preprocessing pins.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CropSettings {
    pub normalization: Normalization,
    pub input_width: u32,
    pub input_height: u32,
    pub padding: f32,
    pub min_box_size: f32,
    pub batch_size: usize,
}

impl CropSettings {
    pub fn from_pins(
        normalization: &str,
        input_width: i64,
        input_height: i64,
        padding: f32,
        min_box_size: f32,
        batch_size: i64,
    ) -> Result<Self> {
        if !(0.0..=1.0).contains(&padding) {
            return Err(anyhow!("`padding` must be between 0 and 1, got {padding}"));
        }
        if !min_box_size.is_finite() || min_box_size < 0.0 {
            return Err(anyhow!(
                "`min_box_size` must be a non-negative number of pixels, got {min_box_size}"
            ));
        }
        let batch_size = usize::try_from(batch_size)
            .ok()
            .filter(|size| *size >= 1)
            .ok_or_else(|| anyhow!("`batch_size` must be at least 1, got {batch_size}"))?;
        Ok(Self {
            normalization: Normalization::parse(normalization)?,
            input_width: input_side("input_width", input_width)?,
            input_height: input_side("input_height", input_height)?,
            padding,
            min_box_size,
            batch_size,
        })
    }
}

fn input_side(pin: &str, value: i64) -> Result<u32> {
    if !(1..=MAX_INPUT_SIDE).contains(&value) {
        return Err(anyhow!(
            "`{pin}` must be between 1 and {MAX_INPUT_SIDE} pixels, got {value}"
        ));
    }
    Ok(value as u32)
}

/// Pixel rectangle inside the image.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CropRect {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum CropSkip {
    NonFinite,
    TooSmall { width: f32, height: f32 },
}

impl std::fmt::Display for CropSkip {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NonFinite => write!(f, "box coordinates are not finite"),
            Self::TooSmall { width, height } => write!(
                f,
                "padded and clipped box of {width:.1}x{height:.1} px is below `min_box_size`"
            ),
        }
    }
}

/// Grows the box by `padding` of its size per side, clips it to the image and rounds outwards
/// to whole pixels. Boxes whose clipped size is below `min_box_size` on either side are skipped.
pub fn crop_rect(
    bbox: &BoundingBox,
    image_width: u32,
    image_height: u32,
    padding: f32,
    min_box_size: f32,
) -> std::result::Result<CropRect, CropSkip> {
    if [bbox.x1, bbox.y1, bbox.x2, bbox.y2]
        .iter()
        .any(|value| !value.is_finite())
    {
        return Err(CropSkip::NonFinite);
    }
    let (image_w, image_h) = (image_width as f32, image_height as f32);
    let pad_x = (bbox.x2 - bbox.x1) * padding;
    let pad_y = (bbox.y2 - bbox.y1) * padding;
    let left = (bbox.x1 - pad_x).clamp(0.0, image_w);
    let right = (bbox.x2 + pad_x).clamp(0.0, image_w);
    let top = (bbox.y1 - pad_y).clamp(0.0, image_h);
    let bottom = (bbox.y2 + pad_y).clamp(0.0, image_h);
    if [left, right, top, bottom]
        .iter()
        .any(|value| value.is_nan())
    {
        return Err(CropSkip::NonFinite);
    }

    let (width, height) = (right - left, bottom - top);
    if width <= 0.0 || height <= 0.0 || width < min_box_size || height < min_box_size {
        return Err(CropSkip::TooSmall { width, height });
    }

    let x = left.floor() as u32;
    let y = top.floor() as u32;
    let x_end = (right.ceil() as u32).min(image_width);
    let y_end = (bottom.ceil() as u32).min(image_height);
    Ok(CropRect {
        x,
        y,
        width: x_end - x,
        height: y_end - y,
    })
}

/// Lost tracks carry a predicted box, so they keep their embedding instead of being re-embedded.
pub fn is_predicted(observation: &AppearanceObservation) -> bool {
    observation.state == TrackState::Lost
}

/// What happens to every element of `detections`, keyed by its index there.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CropPlan {
    pub crops: Vec<(usize, CropRect)>,
    pub skipped: Vec<(usize, CropSkip)>,
    /// Passed through with their incoming embedding, without running the model
    pub predicted: Vec<usize>,
}

pub fn plan_crops(
    detections: &[AppearanceObservation],
    image_width: u32,
    image_height: u32,
    padding: f32,
    min_box_size: f32,
) -> CropPlan {
    let mut plan = CropPlan::default();
    for (index, detection) in detections.iter().enumerate() {
        if is_predicted(detection) {
            plan.predicted.push(index);
            continue;
        }
        match crop_rect(
            &detection.bbox,
            image_width,
            image_height,
            padding,
            min_box_size,
        ) {
            Ok(rect) => plan.crops.push((index, rect)),
            Err(skip) => plan.skipped.push((index, skip)),
        }
    }
    plan
}

/// Writes one packed RGB8 crop into its slice of the batch tensor.
pub fn write_sample(layout: Layout, normalization: Normalization, rgb: &[u8], out: &mut [f32]) {
    match layout {
        Layout::Nhwc => {
            for (index, (target, value)) in out.iter_mut().zip(rgb).enumerate() {
                *target = normalization.apply(index % 3, *value);
            }
        }
        Layout::Nchw => {
            let plane = out.len() / 3;
            for (pixel, values) in rgb.chunks_exact(3).take(plane).enumerate() {
                for (channel, value) in values.iter().enumerate() {
                    out[channel * plane + pixel] = normalization.apply(channel, *value);
                }
            }
        }
    }
}

/// Embedding length of a `[B, D]` or `[B, D, 1, 1]` output holding `len` values for `batch` crops.
pub fn embedding_dimension(len: usize, shape: &[usize], batch: usize) -> Result<usize> {
    if shape.len() >= 2 && shape[0] != batch {
        return Err(anyhow!(
            "Appearance model returned {} rows (output shape {shape:?}) for a batch of {batch} crops",
            shape[0]
        ));
    }
    if batch == 0 || len == 0 || !len.is_multiple_of(batch) {
        return Err(anyhow!(
            "Appearance model output of {len} values (shape {shape:?}) cannot be split into {batch} embeddings"
        ));
    }
    Ok(len / batch)
}

/// Splits `values` into rows of `dimension` and L2-normalizes each; `indices` are the detection
/// indices of the rows.
pub fn normalized_rows(
    values: &[f32],
    dimension: usize,
    indices: &[usize],
) -> Result<Vec<(usize, Vec<f32>)>> {
    if dimension == 0 {
        return Err(anyhow!("Appearance embedding dimension must be at least 1"));
    }
    values
        .chunks_exact(dimension)
        .zip(indices)
        .map(|(row, &index)| {
            embedding::normalized(row)
                .map(|row| (index, row))
                .ok_or_else(|| {
                    anyhow!("Appearance embedding for detection {index} is zero or not finite")
                })
        })
        .collect()
}

/// Camera, session and time stamped onto every observation; empty / 0 keeps the incoming value.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct IdentityOverrides {
    pub camera_id: String,
    pub session_id: String,
    pub timestamp_ms: i64,
}

impl IdentityOverrides {
    pub fn new(camera_id: String, session_id: String, timestamp_ms: i64) -> Result<Self> {
        if timestamp_ms < 0 {
            return Err(anyhow!(
                "`timestamp_ms` must be a Unix time in milliseconds or 0, got {timestamp_ms}"
            ));
        }
        Ok(Self {
            camera_id,
            session_id,
            timestamp_ms,
        })
    }

    /// Stamps identity context; a timestamp missing both here and on the observation becomes `now_ms`.
    pub fn apply(
        &self,
        mut observation: AppearanceObservation,
        now_ms: i64,
    ) -> AppearanceObservation {
        if !self.camera_id.is_empty() {
            observation.camera_id.clone_from(&self.camera_id);
        }
        if !self.session_id.is_empty() {
            observation.session_id.clone_from(&self.session_id);
        }
        if self.timestamp_ms != 0 {
            observation.timestamp_ms = self.timestamp_ms;
        } else if observation.timestamp_ms == 0 {
            observation.timestamp_ms = now_ms;
        }
        observation
    }
}

/// One observation per embedded or predicted detection, in input order. `embeddings` must be
/// sorted by index and hold no predicted detection.
pub fn assemble(
    detections: Vec<AppearanceObservation>,
    embeddings: Vec<(usize, Vec<f32>)>,
    identity: &IdentityOverrides,
    now_ms: i64,
) -> Vec<AppearanceObservation> {
    let mut embeddings = embeddings.into_iter().peekable();
    detections
        .into_iter()
        .enumerate()
        .filter_map(|(index, mut detection)| {
            if !is_predicted(&detection) {
                detection.embedding = embeddings.next_if(|(row, _)| *row == index)?.1;
            }
            detection.detection_index = u32::try_from(index).ok();
            Some(identity.apply(detection, now_ms))
        })
        .collect()
}

#[cfg(feature = "execute")]
#[derive(Default)]
struct Extraction {
    embeddings: Vec<(usize, Vec<f32>)>,
    skipped: Vec<(usize, CropSkip)>,
    dimension: usize,
}

#[cfg(feature = "execute")]
fn extract_embeddings(
    session: &mut Session,
    image: &DynamicImage,
    detections: &[AppearanceObservation],
    settings: &CropSettings,
) -> Result<Extraction> {
    let input = session
        .inputs()
        .first()
        .ok_or_else(|| anyhow!("Appearance model declares no inputs"))?;
    let input_name = input.name().to_string();
    let dtype = input.dtype();
    let (Some(TensorElementType::Float32), Some(shape)) =
        (dtype.tensor_type(), dtype.tensor_shape())
    else {
        return Err(anyhow!(
            "Appearance model input `{input_name}` must be a float32 image tensor, got {dtype}"
        ));
    };
    let model_input = ModelInput::resolve(
        shape,
        settings.input_width,
        settings.input_height,
        settings.batch_size,
    )?;
    let output_name = session
        .outputs()
        .first()
        .map(|output| output.name().to_string())
        .ok_or_else(|| anyhow!("Appearance model declares no outputs"))?;

    let (image_width, image_height) = image.dimensions();
    let plan = plan_crops(
        detections,
        image_width,
        image_height,
        settings.padding,
        settings.min_box_size,
    );

    let mut extraction = Extraction {
        embeddings: Vec::with_capacity(plan.crops.len()),
        skipped: plan.skipped,
        dimension: 0,
    };
    for chunk in plan.crops.chunks(model_input.batch) {
        let tensor = crop_batch(image, chunk, &model_input, settings.normalization)?;
        let outputs = session.run(inputs![input_name.as_str() => Value::from_array(tensor)?])?;
        let output = outputs
            .get(&output_name)
            .ok_or_else(|| anyhow!("Appearance model produced no output `{output_name}`"))?
            .try_extract_array::<f32>()
            .map_err(|error| {
                anyhow!("Appearance model output `{output_name}` must be a float32 tensor: {error}")
            })?;
        let values: Vec<f32> = output.iter().copied().collect();
        let dimension = embedding_dimension(values.len(), output.shape(), chunk.len())?;
        if extraction.dimension != 0 && extraction.dimension != dimension {
            return Err(anyhow!(
                "Appearance model returned {dimension}-dimensional embeddings after {}-dimensional ones",
                extraction.dimension
            ));
        }
        extraction.dimension = dimension;
        let indices: Vec<usize> = chunk.iter().map(|(index, _)| *index).collect();
        extraction
            .embeddings
            .extend(normalized_rows(&values, dimension, &indices)?);
    }
    Ok(extraction)
}

#[cfg(feature = "execute")]
fn crop_batch(
    image: &DynamicImage,
    crops: &[(usize, CropRect)],
    input: &ModelInput,
    normalization: Normalization,
) -> Result<Array4<f32>> {
    let too_large = || {
        anyhow!(
            "Appearance model input of {} crops at {}x{} px is too large",
            crops.len(),
            input.width,
            input.height
        )
    };
    let sample_len = input.sample_len().ok_or_else(too_large)?;
    let total = sample_len.checked_mul(crops.len()).ok_or_else(too_large)?;
    let mut buffer = vec![0.0f32; total];
    for ((_, rect), sample) in crops.iter().zip(buffer.chunks_exact_mut(sample_len)) {
        let crop = image.crop_imm(rect.x, rect.y, rect.width, rect.height);
        let rgb = if (rect.width, rect.height) == (input.width, input.height) {
            crop.into_rgb8()
        } else {
            crop.resize_exact(input.width, input.height, FilterType::Triangle)
                .into_rgb8()
        };
        write_sample(input.layout, normalization, rgb.as_raw(), sample);
    }
    Ok(Array4::from_shape_vec(
        input.tensor_shape(crops.len()),
        buffer,
    )?)
}

#[crate::register_node]
#[derive(Default)]
pub struct ExtractAppearanceNode {}

impl ExtractAppearanceNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for ExtractAppearanceNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "tracking_extract_appearance",
            "Extract Appearance",
            "Crops every detection from the frame and runs a re-identification model on the crops in batches. The built-in models are Intel OpenVINO person re-identification models (Apache-2.0): the chosen one is downloaded into Cache Dir on first use, checked against its SHA-256 and read from there afterwards. With Model set to custom, connect your own session from Load ONNX and set Normalization to match its export: imagenet for torchreid/OSNet exports, raw for FastReID onnx_export.py exports, which normalize inside the model. Each detection becomes an observation with an L2-normalized appearance embedding, keeping its box, camera, session, tracker id, track id and state (track-only fields such as hits and velocity are not carried). Lost tracks from Track Detections (Include Lost) carry a predicted box, so they are passed through with their existing embedding instead of being cropped. Feed the result into Track Detections for appearance-aware tracking or into Associate Entities to recognize the same object across cameras. Boxes that are not finite or smaller than Min Box Size are left out.",
            "AI/ML/Tracking",
        );
        node.set_flowscript_name("tracking", "extractAppearance");
        node.set_receiver("image_in");
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
            "person-openvino-0270 (6 MB, default) or person-openvino-0265 (9.6 MB): Intel OpenVINO person re-identification models, Apache-2.0, downloaded into Cache Dir on first use. custom uses the Custom Model pin.",
            VariableType::String,
        )
        .set_options(PinOptions::new().set_valid_values(model_options()).build())
        .set_default_value(Some(json!(DEFAULT_MODEL)));

        node.add_input_pin(
            "cache_dir",
            "Cache Dir",
            "Folder the built-in model is downloaded to when missing and loaded from when present. Not used with a custom model.",
            VariableType::Struct,
        )
        .set_schema::<FlowPath>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin(
            "custom_model",
            "Custom Model",
            "Session from Load ONNX, used when Model is custom. Takes an RGB float tensor [N,3,H,W] or [N,H,W,3] and returns one embedding per crop, shaped [N,D] or [N,D,1,1].",
            VariableType::Struct,
        )
        .set_schema::<NodeOnnxSession>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin(
            "image_in",
            "Image",
            "Frame the detections were found in",
            VariableType::Struct,
        )
        .set_schema::<NodeImage>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin(
            "detections",
            "Detections",
            "Boxes in image pixel coordinates, from Object Detection or tracks from Track Detections",
            VariableType::Struct,
        )
        .set_schema::<BoundingBox>()
        .set_value_type(ValueType::Array)
        .set_default_value(Some(json!([])));

        node.add_input_pin(
            "camera_id",
            "Camera ID",
            "Camera stamped on every observation; empty keeps the incoming value",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_input_pin(
            "session_id",
            "Session ID",
            "Camera session stamped on every observation; empty keeps the incoming value",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_input_pin(
            "timestamp_ms",
            "Timestamp (ms)",
            "Frame capture time in Unix milliseconds; 0 keeps the incoming value, or uses the current time when there is none",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(0)));

        node.add_input_pin(
            "normalization",
            "Normalization",
            "Pixel scaling a custom model expects (the built-in models normalize their input themselves, so any value works for them): imagenet ((x/255 - mean) / std) for torchreid/OSNet exports, raw (0–255 unchanged) for FastReID onnx_export.py exports that normalize inside the model, zero_one (x/255) or minus_one_one (x/127.5 - 1)",
            VariableType::String,
        )
        .set_options(
            PinOptions::new()
                .set_valid_values(NORMALIZATIONS.iter().map(|value| value.to_string()).collect())
                .build(),
        )
        .set_default_value(Some(json!("imagenet")));

        node.add_input_pin(
            "input_width",
            "Input Width",
            "Crop width in pixels, used only when the model's input width is dynamic",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(128)));

        node.add_input_pin(
            "input_height",
            "Input Height",
            "Crop height in pixels, used only when the model's input height is dynamic",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(256)));

        node.add_input_pin(
            "padding",
            "Padding",
            "Fraction of the box width and height added on each side before cropping",
            VariableType::Float,
        )
        .set_options(PinOptions::new().set_range((0., 1.)).build())
        .set_default_value(Some(json!(0.0)));

        node.add_input_pin(
            "min_box_size",
            "Min Box Size",
            "Boxes narrower or shorter than this many pixels after padding and clipping to the image are skipped",
            VariableType::Float,
        )
        .set_default_value(Some(json!(4.0)));

        node.add_input_pin(
            "batch_size",
            "Batch Size",
            "Crops per inference call, used only when the model's batch dimension is dynamic",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(16)));

        node.add_output_pin(
            "exec_out",
            "Output",
            "Done with the Execution",
            VariableType::Execution,
        );

        node.add_output_pin(
            "observations",
            "Observations",
            "One observation per cropped detection with its new embedding, plus every lost track with its existing one; each carries its index in Detections",
            VariableType::Struct,
        )
        .set_schema::<AppearanceObservation>()
        .set_value_type(ValueType::Array)
        .set_default_value(Some(json!([])));

        node.add_output_pin(
            "dimensions",
            "Dimensions",
            "Embedding length of the model output; 0 when no detection was cropped",
            VariableType::Integer,
        );

        node.set_scores(
            NodeScores::new()
                .set_privacy(5)
                .set_security(8)
                .set_performance(6)
                .set_governance(5)
                .set_reliability(8)
                .set_cost(9)
                .build(),
        );

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let model = ModelChoice::parse(&context.evaluate_pin::<String>("model").await?)?;
        let node_image: NodeImage = context.evaluate_pin("image_in").await?;
        let detections: Vec<AppearanceObservation> = context.evaluate_pin("detections").await?;
        let identity = IdentityOverrides::new(
            context.evaluate_pin("camera_id").await?,
            context.evaluate_pin("session_id").await?,
            context.evaluate_pin("timestamp_ms").await?,
        )?;
        let settings = CropSettings::from_pins(
            &context.evaluate_pin::<String>("normalization").await?,
            context.evaluate_pin("input_width").await?,
            context.evaluate_pin("input_height").await?,
            context.evaluate_pin("padding").await?,
            context.evaluate_pin("min_box_size").await?,
            context.evaluate_pin("batch_size").await?,
        )?;

        let extraction = if detections.iter().all(is_predicted) {
            Extraction::default()
        } else {
            let image_ref = node_image.get_image(context).await?;
            match model {
                ModelChoice::Builtin(model) => {
                    let cache_dir: FlowPath = context.evaluate_pin("cache_dir").await?;
                    let mut session = load_session(context, &cache_dir, model).await?;
                    let image = image_ref.lock().await;
                    extract_embeddings(&mut session, &image, &detections, &settings)?
                }
                ModelChoice::Custom => {
                    let node_session: NodeOnnxSession =
                        context.evaluate_pin("custom_model").await?;
                    let session_ref = node_session.get_session(context).await?;
                    let image = image_ref.lock().await;
                    let mut session = session_ref.lock().await;
                    extract_embeddings(&mut session.session, &image, &detections, &settings)?
                }
            }
        };

        for (index, skip) in &extraction.skipped {
            context.log_message(
                &format!("Extract Appearance skipped detection {index}: {skip}"),
                LogLevel::Debug,
            );
        }

        let observations = assemble(detections, extraction.embeddings, &identity, now_ms());
        context
            .set_pin_value("observations", json!(observations))
            .await?;
        context
            .set_pin_value("dimensions", json!(extraction.dimension))
            .await?;
        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> Result<()> {
        Err(anyhow!(
            "Extract Appearance requires the 'execute' feature. Rebuild with --features execute"
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tracking::types::TrackedObject;

    fn bbox(x1: f32, y1: f32, x2: f32, y2: f32) -> BoundingBox {
        BoundingBox {
            x1,
            y1,
            x2,
            y2,
            score: 0.9,
            class_idx: 0,
            class_name: None,
        }
    }

    fn detection(x1: f32, y1: f32, x2: f32, y2: f32) -> AppearanceObservation {
        AppearanceObservation {
            bbox: bbox(x1, y1, x2, y2),
            ..Default::default()
        }
    }

    fn rect(x: u32, y: u32, width: u32, height: u32) -> CropRect {
        CropRect {
            x,
            y,
            width,
            height,
        }
    }

    fn assert_close(actual: f32, expected: f32) {
        assert!(
            (actual - expected).abs() < 1e-5,
            "expected {expected}, got {actual}"
        );
    }

    fn error_of<T: std::fmt::Debug>(result: Result<T>) -> String {
        result.expect_err("expected an error").to_string()
    }

    #[test]
    fn crop_rect_without_padding_keeps_the_box() {
        let crop = crop_rect(&bbox(10.0, 20.0, 50.0, 100.0), 200, 200, 0.0, 4.0);
        assert_eq!(crop, Ok(rect(10, 20, 40, 80)));
    }

    #[test]
    fn crop_rect_pads_each_side_by_a_fraction_of_the_box() {
        let crop = crop_rect(&bbox(10.0, 20.0, 50.0, 100.0), 200, 200, 0.1, 4.0);
        assert_eq!(crop, Ok(rect(6, 12, 48, 96)));
    }

    #[test]
    fn crop_rect_clamps_to_the_image() {
        let crop = crop_rect(&bbox(-10.0, -5.0, 30.0, 25.0), 20, 20, 0.5, 4.0);
        assert_eq!(crop, Ok(rect(0, 0, 20, 20)));
    }

    #[test]
    fn crop_rect_rounds_outwards_to_whole_pixels() {
        let crop = crop_rect(&bbox(10.4, 20.6, 30.2, 40.1), 100, 100, 0.0, 4.0);
        assert_eq!(crop, Ok(rect(10, 20, 21, 21)));
        let edge = crop_rect(&bbox(90.5, 0.0, 99.7, 10.0), 100, 100, 0.0, 0.0);
        assert_eq!(edge, Ok(rect(90, 0, 10, 10)));
    }

    #[test]
    fn crop_rect_skips_boxes_below_min_size() {
        assert!(matches!(
            crop_rect(&bbox(0.0, 0.0, 3.0, 10.0), 100, 100, 0.0, 4.0),
            Err(CropSkip::TooSmall { .. })
        ));
        assert!(crop_rect(&bbox(0.0, 0.0, 3.0, 10.0), 100, 100, 0.0, 3.0).is_ok());
        assert!(matches!(
            crop_rect(&bbox(198.0, 0.0, 250.0, 50.0), 200, 200, 0.0, 4.0),
            Err(CropSkip::TooSmall { .. })
        ));
        assert!(matches!(
            crop_rect(&bbox(300.0, 300.0, 400.0, 400.0), 200, 200, 0.0, 0.0),
            Err(CropSkip::TooSmall { .. })
        ));
        assert!(matches!(
            crop_rect(&bbox(50.0, 50.0, 40.0, 60.0), 200, 200, 0.0, 0.0),
            Err(CropSkip::TooSmall { .. })
        ));
    }

    #[test]
    fn crop_rect_skips_non_finite_boxes() {
        assert_eq!(
            crop_rect(&bbox(f32::NAN, 0.0, 10.0, 10.0), 100, 100, 0.0, 0.0),
            Err(CropSkip::NonFinite)
        );
        assert_eq!(
            crop_rect(&bbox(0.0, 0.0, 10.0, f32::INFINITY), 100, 100, 0.5, 0.0),
            Err(CropSkip::NonFinite)
        );
        assert_eq!(
            crop_rect(&bbox(-3.0e38, 0.0, 3.0e38, 10.0), 100, 100, 0.0, 0.0),
            Err(CropSkip::NonFinite)
        );
    }

    #[test]
    fn plan_keeps_input_indices_and_chunks_by_batch() {
        let detections = vec![
            detection(0.0, 0.0, 10.0, 10.0),
            detection(0.0, 0.0, 1.0, 1.0),
            detection(10.0, 10.0, 30.0, 30.0),
            detection(f32::NAN, 0.0, 1.0, 1.0),
            detection(20.0, 20.0, 40.0, 40.0),
            detection(30.0, 30.0, 50.0, 50.0),
        ];
        let plan = plan_crops(&detections, 100, 100, 0.0, 4.0);
        let indices: Vec<usize> = plan.crops.iter().map(|(index, _)| *index).collect();
        assert_eq!(indices, vec![0, 2, 4, 5]);
        assert_eq!(
            plan.skipped
                .iter()
                .map(|(index, _)| *index)
                .collect::<Vec<_>>(),
            vec![1, 3]
        );

        let input = ModelInput::resolve(&[-1, 3, 256, 128], 128, 256, 3).unwrap();
        let chunks: Vec<Vec<usize>> = plan
            .crops
            .chunks(input.batch)
            .map(|chunk| chunk.iter().map(|(index, _)| *index).collect())
            .collect();
        assert_eq!(chunks, vec![vec![0, 2, 4], vec![5]]);
    }

    #[test]
    fn normalization_presets() {
        assert_close(Normalization::ZeroOne.apply(0, 0), 0.0);
        assert_close(Normalization::ZeroOne.apply(2, 255), 1.0);
        assert_close(Normalization::MinusOneOne.apply(0, 0), -1.0);
        assert_close(Normalization::MinusOneOne.apply(1, 255), 1.0);
        assert_close(Normalization::Imagenet.apply(0, 255), (1.0 - 0.485) / 0.229);
        assert_close(Normalization::Imagenet.apply(2, 0), -0.406 / 0.225);
    }

    #[test]
    fn raw_normalization_keeps_pixel_values() {
        for (channel, value) in [(0, 0u8), (1, 1), (2, 128), (0, 255)] {
            assert_eq!(Normalization::Raw.apply(channel, value), f32::from(value));
        }
        let rgb = [0, 51, 102, 153, 204, 255];
        let mut nchw = [0.0f32; 6];
        write_sample(Layout::Nchw, Normalization::Raw, &rgb, &mut nchw);
        assert_eq!(nchw, [0.0, 153.0, 51.0, 204.0, 102.0, 255.0]);
    }

    #[test]
    fn normalization_parses_pin_values() {
        let parsed: Vec<Normalization> = NORMALIZATIONS
            .iter()
            .map(|value| Normalization::parse(value).unwrap())
            .collect();
        assert_eq!(
            parsed,
            vec![
                Normalization::Imagenet,
                Normalization::Raw,
                Normalization::ZeroOne,
                Normalization::MinusOneOne,
            ]
        );
        assert_eq!(
            CropSettings::from_pins("raw", 128, 256, 0.0, 4.0, 1)
                .unwrap()
                .normalization,
            Normalization::Raw
        );
        assert!(error_of(Normalization::parse("RAW")).contains("raw"));
        assert!(error_of(Normalization::parse("bgr")).contains("`normalization`"));
    }

    #[test]
    fn write_sample_packs_channels_by_layout() {
        let rgb = [0, 51, 102, 153, 204, 255];
        let mut nchw = [0.0f32; 6];
        write_sample(Layout::Nchw, Normalization::ZeroOne, &rgb, &mut nchw);
        let expected_nchw = [0.0, 0.6, 0.2, 0.8, 0.4, 1.0];
        for (actual, expected) in nchw.iter().zip(expected_nchw) {
            assert_close(*actual, expected);
        }

        let mut nhwc = [0.0f32; 6];
        write_sample(Layout::Nhwc, Normalization::ZeroOne, &rgb, &mut nhwc);
        let expected_nhwc = [0.0, 0.2, 0.4, 0.6, 0.8, 1.0];
        for (actual, expected) in nhwc.iter().zip(expected_nhwc) {
            assert_close(*actual, expected);
        }
    }

    #[test]
    fn write_sample_applies_per_channel_statistics() {
        let mut out = [0.0f32; 3];
        write_sample(
            Layout::Nchw,
            Normalization::Imagenet,
            &[255, 255, 255],
            &mut out,
        );
        for (channel, value) in out.iter().enumerate() {
            assert_close(
                *value,
                (1.0 - IMAGENET_MEAN[channel]) / IMAGENET_STD[channel],
            );
        }
    }

    #[test]
    fn resolves_fixed_nchw_input() {
        let input = ModelInput::resolve(&[1, 3, 256, 128], 64, 64, 16).unwrap();
        assert_eq!(
            input,
            ModelInput {
                layout: Layout::Nchw,
                width: 128,
                height: 256,
                batch: 1,
            }
        );
        assert_eq!(input.tensor_shape(1), (1, 3, 256, 128));
        assert_eq!(input.sample_len(), Some(3 * 256 * 128));
    }

    #[test]
    fn resolves_nhwc_input_with_dynamic_batch() {
        let input = ModelInput::resolve(&[-1, 224, 224, 3], 64, 64, 8).unwrap();
        assert_eq!(input.layout, Layout::Nhwc);
        assert_eq!((input.width, input.height, input.batch), (224, 224, 8));
        assert_eq!(input.tensor_shape(5), (5, 224, 224, 3));
    }

    #[test]
    fn dynamic_spatial_dims_use_the_pin_fallbacks() {
        let input = ModelInput::resolve(&[-1, 3, -1, -1], 128, 256, 4).unwrap();
        assert_eq!((input.width, input.height), (128, 256));
        let partial = ModelInput::resolve(&[0, 3, 256, -1], 100, 50, 4).unwrap();
        assert_eq!(
            (partial.width, partial.height, partial.batch),
            (100, 256, 4)
        );
        let unknown_channels = ModelInput::resolve(&[-1, -1, -1, -1], 128, 256, 4).unwrap();
        assert_eq!(unknown_channels.layout, Layout::Nchw);
    }

    #[test]
    fn rejects_unsupported_input_shapes() {
        assert!(
            error_of(ModelInput::resolve(&[4, 3, 256, 128], 1, 1, 1))
                .contains("fixed batch size of 4")
        );
        assert!(error_of(ModelInput::resolve(&[3, 256, 128], 1, 1, 1)).contains("4-D"));
        assert!(error_of(ModelInput::resolve(&[1, 1, 256, 128], 1, 1, 1)).contains("3-channel"));
    }

    #[test]
    fn validates_pin_ranges_by_name() {
        let valid = CropSettings::from_pins("imagenet", 128, 256, 0.1, 4.0, 16).unwrap();
        assert_eq!(valid.normalization, Normalization::Imagenet);
        assert_eq!((valid.input_width, valid.input_height), (128, 256));
        assert_eq!(valid.batch_size, 16);

        let cases = [
            (
                CropSettings::from_pins("imagenet", 128, 256, 0.0, 4.0, 0),
                "`batch_size`",
            ),
            (
                CropSettings::from_pins("imagenet", 128, 256, 0.0, 4.0, -3),
                "`batch_size`",
            ),
            (
                CropSettings::from_pins("imagenet", 0, 256, 0.0, 4.0, 1),
                "`input_width`",
            ),
            (
                CropSettings::from_pins("imagenet", 128, -1, 0.0, 4.0, 1),
                "`input_height`",
            ),
            (
                CropSettings::from_pins("imagenet", 128, 99_999, 0.0, 4.0, 1),
                "`input_height`",
            ),
            (
                CropSettings::from_pins("imagenet", 128, 256, 1.5, 4.0, 1),
                "`padding`",
            ),
            (
                CropSettings::from_pins("imagenet", 128, 256, -0.1, 4.0, 1),
                "`padding`",
            ),
            (
                CropSettings::from_pins("imagenet", 128, 256, f32::NAN, 4.0, 1),
                "`padding`",
            ),
            (
                CropSettings::from_pins("imagenet", 128, 256, 0.0, -1.0, 1),
                "`min_box_size`",
            ),
            (
                CropSettings::from_pins("gray", 128, 256, 0.0, 4.0, 1),
                "`normalization`",
            ),
        ];
        for (result, pin) in cases {
            let message = error_of(result);
            assert!(message.contains(pin), "{message} should name {pin}");
        }
        assert!(
            error_of(IdentityOverrides::new(String::new(), String::new(), -5))
                .contains("`timestamp_ms`")
        );
    }

    #[test]
    fn splits_output_into_rows() {
        assert_eq!(embedding_dimension(8, &[2, 4], 2).unwrap(), 4);
        assert_eq!(embedding_dimension(8, &[2, 4, 1, 1], 2).unwrap(), 4);
        assert_eq!(embedding_dimension(4, &[4], 1).unwrap(), 4);
        assert!(error_of(embedding_dimension(12, &[3, 4], 2)).contains("3 rows"));
        assert!(embedding_dimension(7, &[7], 2).is_err());
        assert!(embedding_dimension(0, &[2, 0], 2).is_err());

        let rows = normalized_rows(&[3.0, 4.0, 0.0, 2.0], 2, &[4, 9]).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].0, 4);
        assert_close(rows[0].1[0], 0.6);
        assert_close(rows[0].1[1], 0.8);
        assert_eq!(rows[1], (9, vec![0.0, 1.0]));
    }

    #[test]
    fn degenerate_embedding_names_the_detection() {
        let message = error_of(normalized_rows(&[1.0, 0.0, 0.0, 0.0], 2, &[2, 7]));
        assert!(message.contains("detection 7"), "{message}");
        let non_finite = error_of(normalized_rows(&[f32::NAN, 1.0], 2, &[3]));
        assert!(non_finite.contains("detection 3"), "{non_finite}");
    }

    #[test]
    fn assemble_keeps_incoming_fields_and_applies_identity_rules() {
        let mut tracked = detection(0.0, 0.0, 10.0, 10.0);
        tracked.track_id = Some(7);
        tracked.camera_id = "incoming".into();
        tracked.session_id = "session-a".into();
        tracked.timestamp_ms = 55;
        tracked.embedding = vec![1.0];
        tracked.detection_index = Some(99);
        let detections = vec![
            tracked,
            detection(1.0, 1.0, 2.0, 2.0),
            detection(5.0, 5.0, 25.0, 25.0),
        ];
        let identity = IdentityOverrides::new("cam".into(), String::new(), 0).unwrap();

        let observations = assemble(
            detections,
            vec![(0, vec![0.6, 0.8]), (2, vec![1.0, 0.0])],
            &identity,
            1_000,
        );

        assert_eq!(observations.len(), 2);
        let first = &observations[0];
        assert_eq!(first.track_id, Some(7));
        assert_eq!(first.camera_id, "cam");
        assert_eq!(first.session_id, "session-a");
        assert_eq!(first.timestamp_ms, 55);
        assert_eq!(first.detection_index, Some(0));
        assert_eq!(first.embedding, vec![0.6, 0.8]);

        let second = &observations[1];
        assert_eq!(second.detection_index, Some(2));
        assert_eq!(second.bbox.x2, 25.0);
        assert_eq!(second.timestamp_ms, 1_000);
        assert_eq!(second.track_id, None);
    }

    fn lost(x1: f32, y1: f32, x2: f32, y2: f32) -> AppearanceObservation {
        AppearanceObservation {
            state: TrackState::Lost,
            ..detection(x1, y1, x2, y2)
        }
    }

    #[test]
    fn plan_passes_lost_tracks_through_without_cropping() {
        let detections = vec![
            detection(0.0, 0.0, 10.0, 10.0),
            lost(20.0, 20.0, 60.0, 60.0),
            detection(0.0, 0.0, 1.0, 1.0),
            lost(f32::NAN, 0.0, 1.0, 1.0),
            detection(30.0, 30.0, 50.0, 50.0),
        ];
        let plan = plan_crops(&detections, 100, 100, 0.0, 4.0);
        assert_eq!(
            plan.crops,
            vec![(0, rect(0, 0, 10, 10)), (4, rect(30, 30, 20, 20))]
        );
        assert_eq!(
            plan.skipped
                .iter()
                .map(|(index, _)| *index)
                .collect::<Vec<_>>(),
            vec![2]
        );
        assert_eq!(plan.predicted, vec![1, 3]);
        assert!(!detections.iter().all(is_predicted));

        let only_lost = vec![lost(0.0, 0.0, 10.0, 10.0), lost(5.0, 5.0, 50.0, 50.0)];
        assert!(only_lost.iter().all(is_predicted));
        let plan = plan_crops(&only_lost, 100, 100, 0.0, 4.0);
        assert!(plan.crops.is_empty() && plan.skipped.is_empty());
        assert_eq!(plan.predicted, vec![0, 1]);
    }

    #[test]
    fn assemble_passes_lost_tracks_through_with_their_embedding() {
        let mut lost_track = lost(20.0, 20.0, 40.0, 40.0);
        lost_track.track_id = Some(3);
        lost_track.tracker_id = "tracker-a".into();
        lost_track.embedding = vec![0.0, 1.0];
        let mut unembedded = lost(60.0, 60.0, 70.0, 70.0);
        unembedded.track_id = Some(4);
        let detections = vec![
            detection(0.0, 0.0, 10.0, 10.0),
            lost_track,
            detection(0.0, 0.0, 1.0, 1.0),
            unembedded,
            detection(30.0, 30.0, 50.0, 50.0),
        ];
        let identity = IdentityOverrides::new("cam".into(), "s".into(), 0).unwrap();

        let observations = assemble(
            detections,
            vec![(0, vec![1.0, 0.0]), (4, vec![0.6, 0.8])],
            &identity,
            500,
        );

        let indices: Vec<Option<u32>> = observations
            .iter()
            .map(|observation| observation.detection_index)
            .collect();
        assert_eq!(indices, vec![Some(0), Some(1), Some(3), Some(4)]);

        let passed = &observations[1];
        assert_eq!(passed.state, TrackState::Lost);
        assert_eq!(passed.embedding, vec![0.0, 1.0]);
        assert_eq!(passed.track_id, Some(3));
        assert_eq!(passed.tracker_id, "tracker-a");
        assert_eq!((passed.bbox.x1, passed.bbox.x2), (20.0, 40.0));
        assert_eq!(
            (
                passed.camera_id.as_str(),
                passed.session_id.as_str(),
                passed.timestamp_ms
            ),
            ("cam", "s", 500)
        );

        assert!(observations[2].embedding.is_empty());
        assert_eq!(observations[2].track_id, Some(4));
        assert_eq!(observations[0].embedding, vec![1.0, 0.0]);
        assert_eq!(observations[3].embedding, vec![0.6, 0.8]);
        assert_eq!(observations[3].state, TrackState::Tracked);
    }

    #[test]
    fn tracked_object_input_keeps_its_observation_fields() {
        let tracked = TrackedObject {
            bbox: BoundingBox {
                class_name: Some("car".into()),
                ..bbox(1.0, 2.0, 30.0, 40.0)
            },
            embedding: vec![0.0, 1.0],
            camera_id: "cam-1".into(),
            session_id: "session-1".into(),
            tracker_id: "tracker-1".into(),
            track_id: Some(9),
            timestamp_ms: 1_234,
            detection_index: Some(5),
            state: TrackState::Tracked,
            hits: 4,
            ..Default::default()
        };
        let lost_track = TrackedObject {
            track_id: Some(10),
            detection_index: None,
            state: TrackState::Lost,
            ..tracked.clone()
        };
        let input: Vec<AppearanceObservation> =
            flow_like_types::json::from_value(json!([tracked, lost_track])).unwrap();
        let before: Vec<flow_like_types::Value> = input.iter().map(|o| json!(o)).collect();

        let observations = assemble(
            input,
            vec![(0, vec![0.6, 0.8])],
            &IdentityOverrides::default(),
            99,
        );

        assert_eq!(observations.len(), 2);
        assert_eq!(observations[0].tracker_id, "tracker-1");
        assert_eq!(observations[0].state, TrackState::Tracked);
        assert_eq!(observations[1].tracker_id, "tracker-1");
        assert_eq!(observations[1].state, TrackState::Lost);
        for (index, (observation, mut expected)) in observations.iter().zip(before).enumerate() {
            expected["detection_index"] = json!(index);
            if index == 0 {
                expected["embedding"] = json!([0.6f32, 0.8f32]);
            }
            assert_eq!(json!(observation), expected, "observation {index}");
        }
    }

    #[test]
    fn explicit_identity_overrides_incoming_values() {
        let mut incoming = detection(0.0, 0.0, 10.0, 10.0);
        incoming.camera_id = "old".into();
        incoming.session_id = "old".into();
        incoming.timestamp_ms = 5;
        let identity = IdentityOverrides::new("cam".into(), "s2".into(), 42).unwrap();
        let observation = identity.apply(incoming, 1_000);
        assert_eq!(observation.camera_id, "cam");
        assert_eq!(observation.session_id, "s2");
        assert_eq!(observation.timestamp_ms, 42);
    }

    #[test]
    fn empty_detections_produce_no_observations() {
        let identity = IdentityOverrides::default();
        assert!(assemble(Vec::new(), Vec::new(), &identity, 1).is_empty());
        assert_eq!(plan_crops(&[], 100, 100, 0.0, 4.0), CropPlan::default());
    }

    /// Runs a downloaded built-in model on labelled person crops named `<identity>_*.jpg`:
    /// `FLOW_LIKE_REID_MODEL=<model.onnx> FLOW_LIKE_REID_CROPS=<dir> cargo test ... -- --ignored`
    #[cfg(feature = "execute")]
    #[test]
    #[ignore]
    fn built_in_model_separates_identities() {
        use crate::onnx::execution_providers::{
            configured_session_builder, ensure_ort_initialized,
        };
        use flow_like_types::image::{RgbImage, imageops};

        let model = std::env::var("FLOW_LIKE_REID_MODEL").expect("FLOW_LIKE_REID_MODEL");
        let crops = std::env::var("FLOW_LIKE_REID_CROPS").expect("FLOW_LIKE_REID_CROPS");
        let mut files: Vec<_> = std::fs::read_dir(crops)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|path| path.extension().is_some_and(|ext| ext == "jpg"))
            .collect();
        files.sort();
        assert!(files.len() >= 4, "need at least four crops");

        let (width, height) = (128u32, 256u32);
        let mut canvas = RgbImage::new(width * files.len() as u32, height);
        let mut identities = Vec::new();
        let mut detections = Vec::new();
        for (index, file) in files.iter().enumerate() {
            let crop = flow_like_types::image::open(file).unwrap().to_rgb8();
            let crop = imageops::resize(&crop, width, height, imageops::FilterType::Triangle);
            let x = width * index as u32;
            imageops::replace(&mut canvas, &crop, i64::from(x), 0);
            identities.push(file.file_name().unwrap().to_string_lossy()[..4].to_string());
            detections.push(AppearanceObservation {
                bbox: bbox(x as f32, 0.0, (x + width) as f32, height as f32),
                ..Default::default()
            });
        }

        ensure_ort_initialized().unwrap();
        let mut session = configured_session_builder()
            .unwrap()
            .commit_from_memory(&std::fs::read(model).unwrap())
            .unwrap();
        let settings = CropSettings::from_pins("imagenet", 128, 256, 0.0, 4.0, 16).unwrap();
        let extraction = extract_embeddings(
            &mut session,
            &DynamicImage::ImageRgb8(canvas),
            &detections,
            &settings,
        )
        .unwrap();
        assert_eq!(extraction.embeddings.len(), files.len());
        assert!(extraction.dimension > 0);

        let (mut same, mut different) = (Vec::new(), Vec::new());
        for (a, (row_a, embedding_a)) in extraction.embeddings.iter().enumerate() {
            for (row_b, embedding_b) in &extraction.embeddings[a + 1..] {
                let similarity = embedding::cosine(embedding_a, embedding_b);
                if identities[*row_a] == identities[*row_b] {
                    same.push(similarity);
                } else {
                    different.push(similarity);
                }
            }
        }
        let mean = |values: &[f32]| values.iter().sum::<f32>() / values.len() as f32;
        assert!(!same.is_empty() && !different.is_empty());
        let min = |values: &[f32]| values.iter().copied().fold(f32::INFINITY, f32::min);
        let max = |values: &[f32]| values.iter().copied().fold(f32::NEG_INFINITY, f32::max);
        println!(
            "dimension {}, same identity mean {:.3} min {:.3}, different mean {:.3} max {:.3}",
            extraction.dimension,
            mean(&same),
            min(&same),
            mean(&different),
            max(&different)
        );
        assert!(mean(&same) > mean(&different) + 0.1);
    }

    #[test]
    fn node_declares_the_specified_pins() {
        let node = ExtractAppearanceNode::new().get_node();
        assert_eq!(node.name, "tracking_extract_appearance");
        assert_eq!(node.category, "AI/ML/Tracking");
        for name in [
            "exec_in",
            "model",
            "cache_dir",
            "custom_model",
            "image_in",
            "detections",
            "camera_id",
            "session_id",
            "timestamp_ms",
            "normalization",
            "input_width",
            "input_height",
            "padding",
            "min_box_size",
            "batch_size",
            "exec_out",
            "observations",
            "dimensions",
        ] {
            assert!(node.get_pin_by_name(name).is_some(), "missing pin {name}");
        }
        let model = node.get_pin_by_name("model").unwrap();
        assert_eq!(
            model
                .options
                .as_ref()
                .and_then(|options| options.valid_values.clone()),
            Some(model_options())
        );
        let normalization = node.get_pin_by_name("normalization").unwrap();
        assert_eq!(
            normalization
                .options
                .as_ref()
                .and_then(|options| options.valid_values.clone()),
            Some(NORMALIZATIONS.map(String::from).to_vec())
        );
    }
}
