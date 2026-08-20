/// # ONNX Model Loader Nodes
use crate::onnx::NodeOnnxSession;
#[cfg(feature = "execute")]
use crate::onnx::execution_providers::{configured_session_builder, ensure_ort_initialized};
#[cfg(feature = "execute")]
use crate::onnx::{Provider, SessionWithMeta, classification, detection};
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_catalog_core::FlowPath;
#[cfg(feature = "execute")]
use flow_like_model_provider::ml::ort::{
    session::{Session, builder::GraphOptimizationLevel},
    value::Outlet,
};
#[cfg(feature = "execute")]
use flow_like_types::json::json;
use flow_like_types::{Result, anyhow, async_trait};
#[cfg(feature = "execute")]
use std::borrow::Cow;

/// Sidecar names ONNX exporters use for tensors that do not fit in the 2 GB protobuf limit.
/// `optimum`/`transformers.js` write `model.onnx_data`, `torch.onnx.export` writes
/// `model.onnx.data`.
#[cfg(feature = "execute")]
pub fn external_data_candidates(file_name: &str) -> Vec<String> {
    let stem = file_name.strip_suffix(".onnx").unwrap_or(file_name);
    let mut candidates = vec![
        format!("{file_name}_data"),
        format!("{file_name}.data"),
        format!("{stem}.data"),
    ];
    candidates.retain(|candidate| candidate != file_name);
    candidates.dedup();
    candidates
}

/// Pull any external tensor file sitting next to the graph in the same store. The registered
/// name has to match the `location` the graph references, which is the bare file name.
#[cfg(feature = "execute")]
async fn fetch_external_data(
    context: &mut ExecutionContext,
    path: &FlowPath,
) -> Vec<(String, Vec<u8>)> {
    let (parent, file_name) = match path.path.rfind('/') {
        Some(index) => path.path.split_at(index + 1),
        None => ("", path.path.as_str()),
    };

    let mut found = Vec::new();
    for candidate in external_data_candidates(file_name) {
        let sibling = FlowPath::new(
            format!("{parent}{candidate}"),
            path.store_ref.clone(),
            path.cache_store_ref.clone(),
        );
        if let Ok(bytes) = sibling.get(context, false).await
            && !bytes.is_empty()
        {
            found.push((candidate, bytes));
        }
    }
    found
}

/// Retry a graph that the default loader rejected, relaxing one assumption at a time:
/// fp16 and q4f16 exports trip ORT's level-3 fusions, and graphs above the 2 GB protobuf limit
/// need their tensor sidecar handed over explicitly.
#[cfg(feature = "execute")]
async fn load_with_fallbacks(
    context: &mut ExecutionContext,
    path: &FlowPath,
    bytes: &[u8],
    direct_error: flow_like_model_provider::ml::ort::Error,
) -> Result<Session> {
    let reduced = configured_session_builder()?
        .with_optimization_level(GraphOptimizationLevel::Level2)
        .map_err(|error| anyhow!("Failed to lower graph optimization level: {error}"))?
        .commit_from_memory(bytes);
    let reduced_error = match reduced {
        Ok(session) => return Ok(session),
        Err(error) => error,
    };

    let external = fetch_external_data(context, path).await;
    if external.is_empty() {
        return Err(anyhow!(
            "Failed to load ONNX model `{}`: {direct_error} (also failed at reduced graph optimization: {reduced_error})",
            path.path
        ));
    }

    let names: Vec<String> = external.iter().map(|(name, _)| name.clone()).collect();
    let mut builder = configured_session_builder()?
        .with_optimization_level(GraphOptimizationLevel::Level2)
        .map_err(|error| anyhow!("Failed to lower graph optimization level: {error}"))?;
    for (name, data) in external {
        builder = builder
            .with_external_initializer_file_in_memory(&name, Cow::Owned(data))
            .map_err(|error| {
                anyhow!("Failed to register external tensor file `{name}`: {error}")
            })?;
    }

    builder.commit_from_memory(bytes).map_err(|error| {
        anyhow!(
            "Failed to load ONNX model `{}` with external tensor data [{}]: {error}",
            path.path,
            names.join(", ")
        )
    })
}

// ## Loader Utilities
// Identifying ONNX-I/Os
#[cfg(feature = "execute")]
static DFINE_INPUTS: [&str; 2] = ["images", "orig_target_sizes"];
#[cfg(feature = "execute")]
static DFINE_OUTPUTS: [&str; 3] = ["labels", "boxes", "scores"];
#[cfg(feature = "execute")]
static YOLO_INPUTS: [&str; 1] = ["images"];
#[cfg(feature = "execute")]
static YOLO_OUTPUTS: [&str; 1] = ["output0"];
#[cfg(feature = "execute")]
static TIMM_INPUTS: [&str; 1] = ["input0"];
#[cfg(feature = "execute")]
static TIMM_OUTPUTS: [&str; 1] = ["output0"];
#[cfg(feature = "execute")]
static BOX_LABEL_SCORE_OUTPUTS: [&str; 3] = ["boxes", "labels", "scores"];
#[cfg(feature = "execute")]
static SSD_MOBILENET_OUTPUTS: [&str; 4] = [
    "num_detections:0",
    "detection_boxes:0",
    "detection_scores:0",
    "detection_classes:0",
];
#[cfg(feature = "execute")]
static SSD_MOBILENET_OUTPUTS_NO_SUFFIX: [&str; 4] = [
    "num_detections",
    "detection_boxes",
    "detection_scores",
    "detection_classes",
];
#[cfg(feature = "execute")]
static YOLOV3_OUTPUTS: [&str; 3] = ["boxes", "scores", "indices"];

#[cfg(feature = "execute")]
/// Factory Function Matching ONNX Assets to a Provider-Frameworks
pub fn determine_provider(session: &Session) -> Result<Provider> {
    let input_names: Vec<&str> = session.inputs().iter().map(|i| i.name()).collect();
    let output_names: Vec<&str> = session.outputs().iter().map(|o| o.name()).collect();
    if input_names == DFINE_INPUTS && output_names == DFINE_OUTPUTS {
        let (input_width, input_height) = determine_input_shape(session, "images")?;
        Ok(Provider::DfineLike(detection::DfineLike {
            input_width,
            input_height,
        }))
    } else if input_names == YOLO_INPUTS && output_names == YOLO_OUTPUTS {
        let (input_width, input_height) = determine_input_shape(session, "images")?;
        Ok(Provider::YoloLike(detection::YoloLike {
            input_width,
            input_height,
        }))
    } else if input_names == TIMM_INPUTS && output_names == TIMM_OUTPUTS {
        let (input_width, input_height) = determine_input_shape(session, "input0")?;
        Ok(Provider::TimmLike(classification::TimmLike {
            input_width,
            input_height,
        }))
    } else if let Some((
        num_detections_output_name,
        boxes_output_name,
        scores_output_name,
        classes_output_name,
    )) = ssd_mobilenet_outputs(&output_names)
    {
        let input_name = first_input_name(session)?;
        Ok(Provider::SsdMobileNetLike(detection::SsdMobileNetLike {
            input_name,
            num_detections_output_name,
            boxes_output_name,
            scores_output_name,
            classes_output_name,
        }))
    } else if let Some((boxes_output_name, scores_output_name, indices_output_name)) =
        yolo_v3_outputs(session, &output_names)
    {
        let image_input = session
            .inputs()
            .iter()
            .find(|input| input_rank(input) == Some(4))
            .ok_or_else(|| anyhow!("Failed to determine YOLOv3 image input"))?;
        let shape_input = session
            .inputs()
            .iter()
            .find(|input| input.name() != image_input.name())
            .ok_or_else(|| anyhow!("Failed to determine YOLOv3 image-shape input"))?;
        let (input_width, input_height) = fixed_input_size(image_input, 416, InputLayout::Nchw);
        Ok(Provider::YoloV3Like(detection::YoloV3Like {
            image_input_name: image_input.name().to_string(),
            image_shape_input_name: shape_input.name().to_string(),
            boxes_output_name,
            scores_output_name,
            indices_output_name,
            input_width,
            input_height,
            image_shape_kind: numeric_input_kind(shape_input),
        }))
    } else if let Some((boxes_output_name, labels_output_name, scores_output_name)) =
        box_label_score_outputs(session, &output_names)
    {
        let input = session
            .inputs()
            .first()
            .ok_or_else(|| anyhow!("Object detection model has no inputs"))?;
        let rank = input_rank(input).unwrap_or(4);
        let (input_width, input_height) = if rank == 4 {
            fixed_input_size(input, 1200, InputLayout::Nchw)
        } else {
            (0, 0)
        };
        let preprocessing = if rank == 3 {
            detection::BoxLabelsScoresPreprocessing::DetectronBgrChw
        } else {
            detection::BoxLabelsScoresPreprocessing::ImagenetNchw
        };
        Ok(Provider::BoxLabelsScoresLike(
            detection::BoxLabelsScoresLike {
                input_name: input.name().to_string(),
                boxes_output_name,
                labels_output_name,
                scores_output_name,
                input_width,
                input_height,
                preprocessing,
            },
        ))
    } else if let Some((input_name, output_name, input_width, input_height, num_classes)) =
        yolo_v2_grid_signature(session)
    {
        Ok(Provider::YoloV2GridLike(detection::YoloV2GridLike {
            input_name,
            output_name,
            input_width,
            input_height,
            num_classes,
        }))
    } else if let Some((input_name, input_width, input_height)) = yolo_v4_signature(session) {
        Ok(Provider::YoloV4Like(detection::YoloV4Like {
            input_name,
            input_width,
            input_height,
        }))
    } else if is_retinanet_like(session) {
        let input = session
            .inputs()
            .first()
            .ok_or_else(|| anyhow!("RetinaNet model has no inputs"))?;
        let static_input_size = static_input_size(input, InputLayout::Nchw);
        let (input_width, input_height) = static_input_size.unwrap_or((0, 0));
        Ok(Provider::RetinaNetLike(detection::RetinaNetLike {
            input_name: input.name().to_string(),
            output_names: session
                .outputs()
                .iter()
                .map(|output| output.name().to_string())
                .collect(),
            input_width,
            input_height,
            resize_input: static_input_size.is_some(),
        }))
    } else {
        tracing::info!(
            "Model does not match known patterns, using Generic provider. Inputs: {:?}, Outputs: {:?}",
            input_names,
            output_names
        );
        Ok(Provider::Generic)
    }
}

#[cfg(feature = "execute")]
fn has_outputs(output_names: &[&str], expected: &[&str]) -> bool {
    expected
        .iter()
        .all(|expected_name| output_names.iter().any(|name| name == expected_name))
}

#[cfg(feature = "execute")]
fn first_input_name(session: &Session) -> Result<String> {
    session
        .inputs()
        .first()
        .map(|input| input.name().to_string())
        .ok_or_else(|| anyhow!("ONNX model has no inputs"))
}

#[cfg(feature = "execute")]
fn input_rank(input: &Outlet) -> Option<usize> {
    input.dtype().tensor_shape().map(|dims| dims.len())
}

#[cfg(feature = "execute")]
fn output_shape(output: &Outlet) -> Option<Vec<i64>> {
    output
        .dtype()
        .tensor_shape()
        .map(|dims| dims.iter().copied().collect())
}

#[cfg(feature = "execute")]
fn input_shape(input: &Outlet) -> Option<Vec<i64>> {
    input
        .dtype()
        .tensor_shape()
        .map(|dims| dims.iter().copied().collect())
}

#[cfg(feature = "execute")]
enum InputLayout {
    Nchw,
    Nhwc,
}

#[cfg(feature = "execute")]
fn fixed_input_size(input: &Outlet, fallback: u32, layout: InputLayout) -> (u32, u32) {
    static_input_size(input, layout).unwrap_or((fallback, fallback))
}

#[cfg(feature = "execute")]
fn static_input_size(input: &Outlet, layout: InputLayout) -> Option<(u32, u32)> {
    let shape = input_shape(input)?;

    match layout {
        InputLayout::Nchw if shape.len() == 4 => {
            Some((positive_dim(shape[3])?, positive_dim(shape[2])?))
        }
        InputLayout::Nhwc if shape.len() == 4 => {
            Some((positive_dim(shape[2])?, positive_dim(shape[1])?))
        }
        _ => None,
    }
}

#[cfg(feature = "execute")]
fn positive_dim(dim: i64) -> Option<u32> {
    if dim > 0 { Some(dim as u32) } else { None }
}

#[cfg(feature = "execute")]
fn numeric_input_kind(input: &Outlet) -> detection::YoloImageShapeKind {
    let ty = format!("{:?}", input.dtype());
    if ty.contains("Int64") {
        detection::YoloImageShapeKind::I64
    } else if ty.contains("Int32") {
        detection::YoloImageShapeKind::I32
    } else {
        detection::YoloImageShapeKind::F32
    }
}

#[cfg(feature = "execute")]
fn ssd_mobilenet_outputs(output_names: &[&str]) -> Option<(String, String, String, String)> {
    if has_outputs(output_names, &SSD_MOBILENET_OUTPUTS) {
        return Some((
            "num_detections:0".into(),
            "detection_boxes:0".into(),
            "detection_scores:0".into(),
            "detection_classes:0".into(),
        ));
    }

    if has_outputs(output_names, &SSD_MOBILENET_OUTPUTS_NO_SUFFIX) {
        return Some((
            "num_detections".into(),
            "detection_boxes".into(),
            "detection_scores".into(),
            "detection_classes".into(),
        ));
    }

    None
}

#[cfg(feature = "execute")]
fn box_label_score_outputs(
    session: &Session,
    output_names: &[&str],
) -> Option<(String, String, String)> {
    if has_outputs(output_names, &BOX_LABEL_SCORE_OUTPUTS) {
        return Some(("boxes".into(), "labels".into(), "scores".into()));
    }

    let boxes = session.outputs().iter().find(|output| {
        is_float_output(output)
            && output_shape(output)
                .map(|shape| {
                    (shape.len() == 2 || shape.len() == 3) && shape.last().copied() == Some(4)
                })
                .unwrap_or(false)
    })?;

    let labels = session.outputs().iter().find(|output| {
        output.name() != boxes.name()
            && is_integer_output(output)
            && output_shape(output)
                .map(|shape| shape.len() == 1 || shape.len() == 2)
                .unwrap_or(false)
    })?;

    let scores = session.outputs().iter().find(|output| {
        output.name() != boxes.name()
            && output.name() != labels.name()
            && is_float_output(output)
            && output_shape(output)
                .map(|shape| shape.len() == 1 || shape.len() == 2)
                .unwrap_or(false)
    })?;

    Some((
        boxes.name().to_string(),
        labels.name().to_string(),
        scores.name().to_string(),
    ))
}

#[cfg(feature = "execute")]
fn yolo_v3_outputs(session: &Session, output_names: &[&str]) -> Option<(String, String, String)> {
    if session.inputs().len() < 2 {
        return None;
    }

    if has_outputs(output_names, &YOLOV3_OUTPUTS) {
        return Some(("boxes".into(), "scores".into(), "indices".into()));
    }

    let boxes = session.outputs().iter().find(|output| {
        is_float_output(output)
            && output_shape(output)
                .map(|shape| shape.len() == 3 && shape.last().copied() == Some(4))
                .unwrap_or(false)
    })?;

    let scores = session.outputs().iter().find(|output| {
        output.name() != boxes.name()
            && is_float_output(output)
            && output_shape(output)
                .map(|shape| shape.len() == 3 && shape.get(1).copied() == Some(80))
                .unwrap_or(false)
    })?;

    let indices = session.outputs().iter().find(|output| {
        output.name() != boxes.name()
            && output.name() != scores.name()
            && is_integer_output(output)
            && output_shape(output)
                .map(|shape| {
                    shape.len() == 2 || (shape.len() == 3 && shape.last().copied() == Some(3))
                })
                .unwrap_or(false)
    })?;

    Some((
        boxes.name().to_string(),
        scores.name().to_string(),
        indices.name().to_string(),
    ))
}

#[cfg(feature = "execute")]
fn is_float_output(output: &Outlet) -> bool {
    format!("{:?}", output.dtype()).contains("Float")
}

#[cfg(feature = "execute")]
fn is_integer_output(output: &Outlet) -> bool {
    let ty = format!("{:?}", output.dtype());
    ty.contains("Int32") || ty.contains("Int64")
}

#[cfg(feature = "execute")]
fn yolo_v2_grid_signature(session: &Session) -> Option<(String, String, u32, u32, usize)> {
    if session.inputs().len() != 1 || session.outputs().len() != 1 {
        return None;
    }

    let input = session.inputs().first()?;
    let output = session.outputs().first()?;
    let shape = output_shape(output)?;
    if shape.len() != 4 || shape[2] != 13 || shape[3] != 13 {
        return None;
    }

    let num_classes = match shape[1] {
        125 => 20,
        425 => 80,
        _ => return None,
    };
    let (input_width, input_height) = fixed_input_size(input, 416, InputLayout::Nchw);
    Some((
        input.name().to_string(),
        output.name().to_string(),
        input_width,
        input_height,
        num_classes,
    ))
}

#[cfg(feature = "execute")]
fn yolo_v4_signature(session: &Session) -> Option<(String, u32, u32)> {
    if session.inputs().len() != 1 {
        return None;
    }

    let has_yolov4_output = session.outputs().iter().any(|output| {
        output_shape(output)
            .map(|shape| shape.len() == 5 && shape[3] == 3 && shape[4] == 85)
            .unwrap_or(false)
    });
    if !has_yolov4_output {
        return None;
    }

    let input = session.inputs().first()?;
    let (input_width, input_height) = fixed_input_size(input, 416, InputLayout::Nhwc);
    Some((input.name().to_string(), input_width, input_height))
}

#[cfg(feature = "execute")]
fn is_retinanet_like(session: &Session) -> bool {
    let mut class_heads = 0usize;
    let mut box_heads = 0usize;
    for output in session.outputs() {
        let Some(shape) = output_shape(output) else {
            continue;
        };
        if shape.len() != 4 {
            continue;
        }
        if shape[1] % 80 == 0 && shape[1] >= 80 {
            class_heads += 1;
        } else if shape[1] % 4 == 0 {
            box_heads += 1;
        }
    }

    class_heads >= 1 && box_heads >= 1
}

#[cfg(feature = "execute")]
pub fn determine_input_shape(session: &Session, input_name: &str) -> Result<(u32, u32)> {
    for input in session.inputs() {
        if input.name() == input_name
            && let Some(dims) = input.dtype().tensor_shape()
        {
            let d = dims.len();
            if d > 1 {
                let (w, h) = (dims[d - 2], dims[d - 1]);
                return Ok((w as u32, h as u32));
            }
        }
    }
    Err(anyhow!("Failed to determine input shape!"))
}

#[crate::register_node]
#[derive(Default)]
/// # Node to Load ONNX Runtime Session
/// Sets execution context cache
pub struct LoadOnnxNode {}

impl LoadOnnxNode {
    /// Create new LoadOnnxNode Instance
    pub fn new() -> Self {
        LoadOnnxNode {}
    }
}

#[async_trait]
impl NodeLogic for LoadOnnxNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "load_onnx",
            "Load ONNX",
            "Load ONNX Model from Path",
            "AI/ML/ONNX",
        );
        node.set_version(1);

        node.add_icon("/flow/icons/find_model.svg");

        // inputs
        node.add_input_pin(
            "exec_in",
            "Input",
            "Initiate Execution",
            VariableType::Execution,
        );

        node.add_input_pin("path", "Path", "Path ONNX File", VariableType::Struct)
            .set_schema::<FlowPath>()
            .set_options(PinOptions::new().set_enforce_schema(true).build());

        // outputs
        node.add_output_pin(
            "exec_out",
            "Output",
            "Done with the Execution",
            VariableType::Execution,
        );

        node.add_output_pin("model", "Model", "ONNX Model Session", VariableType::Struct)
            .set_schema::<NodeOnnxSession>()
            .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin(
            "accelerated",
            "Acceleration Configured",
            "Whether a GPU/NPU execution provider was configured; individual sessions may still fall back to CPU",
            VariableType::Boolean,
        );

        node.add_output_pin(
            "active_provider",
            "Configured Providers",
            "Execution providers configured in priority order, including CPU fallback",
            VariableType::String,
        );

        node
    }

    #[allow(unused_variables)]
    async fn run(&self, context: &mut ExecutionContext) -> Result<()> {
        #[cfg(feature = "execute")]
        {
            context.deactivate_exec_pin("exec_out").await?;

            // fetch inputs
            let path: FlowPath = context.evaluate_pin("path").await?;
            let bytes = path.get(context, false).await?;

            // Idempotent and cheap after the first call. This also protects embedded/library
            // callers that reach the node without going through a FlowLike runtime bootstrap.
            let ep_info = ensure_ort_initialized()?;

            // Bound separately so the builder is dropped before any await; it is not `Send`.
            let direct_load = configured_session_builder()?.commit_from_memory(&bytes);
            let session = match direct_load {
                Ok(session) => session,
                // fp16/q4f16 exports and graphs above the 2 GB protobuf limit both fail here.
                // Retry with relaxed optimization and the tensor sidecar pulled from the store.
                Err(direct_error) => {
                    load_with_fallbacks(context, &path, &bytes, direct_error).await?
                }
            };

            // wrap ONNX session with provider metadata
            // we try to determine the here to fail fast in case of incompatible ONNX assets
            let provider = determine_provider(&session)?;
            let session_with_meta = SessionWithMeta {
                session,
                provider,
                ep_active: ep_info.active_providers.clone(),
                accelerated: ep_info.accelerated,
            };
            let node_session = NodeOnnxSession::new(context, session_with_meta).await;

            // set outputs
            context.set_pin_value("model", json!(node_session)).await?;
            context
                .set_pin_value("accelerated", json!(ep_info.accelerated))
                .await?;
            context
                .set_pin_value(
                    "active_provider",
                    json!(ep_info.active_providers.join(", ")),
                )
                .await?;
            context.activate_exec_pin("exec_out").await?;
            Ok(())
        }

        #[cfg(not(feature = "execute"))]
        {
            Err(anyhow!(
                "ONNX execution requires the 'execute' feature. Rebuild with --features execute"
            ))
        }
    }
}
