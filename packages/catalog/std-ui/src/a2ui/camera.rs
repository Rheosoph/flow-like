use flow_like::flow::{
    execution::{context::ExecutionContext, egress::GuardedHttpClient},
    node::{Node, NodeLogic},
    pin::{PinOptions, ValueType},
    variable::VariableType,
};
use flow_like_catalog_core::{FlowPath, NodeImage};
use flow_like_types::{Value, async_trait, json::json};
use futures::StreamExt;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::{io::Cursor, time::Duration};

pub mod audio;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CameraFrameImage {
    pub url: String,
    pub name: String,
    pub r#type: String,
    pub size: u64,
    pub flow_path: Option<FlowPath>,
}

/// Captured pixels are upright and unmirrored. Overlay coordinates use this image space.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CameraFrame {
    pub surface_id: String,
    pub component_id: String,
    pub session_id: String,
    pub frame_id: String,
    pub captured_at: String,
    pub width: u32,
    pub height: u32,
    pub mirrored: bool,
    pub orientation: u32,
    pub image: CameraFrameImage,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CameraAnnotation {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: CameraAnnotationType,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub x: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub y: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub width: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub height: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub points: Option<Vec<[f64; 2]>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub background: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub confidence: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub opacity: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub font_size: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub z_index: Option<i32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum CameraAnnotationType {
    Box,
    Text,
    Point,
    Polygon,
    Blur,
    Dim,
}

fn base(name: &str, label: &str, description: &str, function: &str) -> Node {
    let mut node = Node::new(name, label, description, "UI/Camera");
    node.set_flowscript_name("ui", function);
    node.add_icon("/flow/icons/a2ui.svg");
    node.set_long_running(true);
    node.add_input_pin("exec_in", "▶", "Trigger", VariableType::Execution);
    node.add_output_pin("exec_out", "▶", "Completed", VariableType::Execution);
    node.add_output_pin(
        "exec_error",
        "Error",
        "Camera or client unavailable",
        VariableType::Execution,
    );
    node.add_output_pin(
        "error",
        "Error",
        "Structured code and message",
        VariableType::Generic,
    );
    node
}

fn frame_input(node: &mut Node) {
    node.add_input_pin(
        "frame",
        "Frame",
        "Frame from Camera View or Capture Camera Frame",
        VariableType::Struct,
    )
    .set_schema::<CameraFrame>();
}

fn target(frame: &CameraFrame) -> Value {
    json!({"surfaceId":frame.surface_id,"componentId":frame.component_id,"sessionId":frame.session_id,"frameId":frame.frame_id})
}

async fn begin(context: &mut ExecutionContext) -> flow_like_types::Result<()> {
    context.deactivate_exec_pin("exec_out").await?;
    context.deactivate_exec_pin("exec_error").await?;
    context.set_pin_value("error", Value::Null).await
}

async fn command(
    context: &mut ExecutionContext,
    command: &str,
    args: Value,
) -> flow_like_types::Result<Option<Value>> {
    let response = context
        .request_device(command, args, Duration::from_secs(30))
        .await?;
    if response.get("ok").and_then(Value::as_bool) == Some(true) {
        Ok(Some(response.get("value").cloned().unwrap_or(Value::Null)))
    } else {
        context
            .set_pin_value(
                "error",
                response.get("error").cloned().unwrap_or_else(
                    || json!({"code":"camera_error","message":"Camera request failed"}),
                ),
            )
            .await?;
        context.activate_exec_pin("exec_error").await?;
        Ok(None)
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct CaptureCameraFrame;

impl CaptureCameraFrame {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for CaptureCameraFrame {
    fn get_node(&self) -> Node {
        let mut node = base(
            "a2ui_camera_capture",
            "Capture Camera Frame",
            "Capture from a camera the user started on the invoking screen",
            "captureCameraFrame",
        );
        node.add_input_pin(
            "surface_id",
            "Surface ID",
            "The camera's rendered surface",
            VariableType::String,
        );
        node.add_input_pin(
            "component_id",
            "Camera ID",
            "The Camera View component id",
            VariableType::String,
        );
        node.add_input_pin(
            "session_id",
            "Session ID",
            "The current sessionId from Camera View's value binding or ready Event",
            VariableType::String,
        );
        node.add_input_pin(
            "max_width",
            "Max Width",
            "JPEG width limit in pixels",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(1280)));
        node.add_input_pin(
            "quality",
            "Quality",
            "JPEG quality from 0.1 to 1",
            VariableType::Float,
        )
        .set_default_value(Some(json!(0.85)));
        node.add_output_pin(
            "frame",
            "Frame",
            "Image reference and session/frame identity",
            VariableType::Struct,
        )
        .set_schema::<CameraFrame>();
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        begin(context).await?;
        let surface_id: String = context.evaluate_pin("surface_id").await?;
        let component_id: String = context.evaluate_pin("component_id").await?;
        let session_id: String = context.evaluate_pin("session_id").await?;
        let max_width: i64 = context.evaluate_pin("max_width").await?;
        let quality: f64 = context.evaluate_pin("quality").await?;
        if let Some(value) = command(context, "camera.capture", json!({"surfaceId":surface_id,"componentId":component_id,"sessionId":session_id,"maxWidth":max_width,"quality":quality})).await? {
            let frame: CameraFrame = flow_like_types::json::from_value(value)?;
            context.set_pin_value("frame", json!(frame)).await?;
            context.activate_exec_pin("exec_out").await?;
        }
        Ok(())
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct ControlCamera;
impl ControlCamera {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for ControlCamera {
    fn get_node(&self) -> Node {
        let mut node = base(
            "a2ui_camera_control",
            "Control Camera",
            "Freeze, resume or stop the camera session that produced a frame",
            "controlCamera",
        );
        frame_input(&mut node);
        node.add_input_pin(
            "operation",
            "Operation",
            "Camera operation",
            VariableType::String,
        )
        .set_options(
            PinOptions::new()
                .set_valid_values(vec!["freeze".into(), "resume".into(), "stop".into()])
                .build(),
        )
        .set_default_value(Some(json!("freeze")));
        node
    }
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        begin(context).await?;
        let frame: CameraFrame = context.evaluate_pin("frame").await?;
        let operation: String = context.evaluate_pin("operation").await?;
        if !["freeze", "resume", "stop"].contains(&operation.as_str()) {
            return Err(flow_like_types::anyhow!("Unknown camera operation"));
        }
        if command(context, &format!("camera.{operation}"), target(&frame))
            .await?
            .is_some()
        {
            context.activate_exec_pin("exec_out").await?;
        }
        Ok(())
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct UpdateCameraOverlays;
impl UpdateCameraOverlays {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for UpdateCameraOverlays {
    fn get_node(&self) -> Node {
        let mut node = base(
            "a2ui_camera_update_overlays",
            "Update Camera Overlays",
            "Set annotations on the matching live frame; an empty array clears them",
            "updateCameraOverlays",
        );
        frame_input(&mut node);
        node.add_input_pin(
            "overlays",
            "Overlays",
            "Normalized boxes, text, points, polygons, blur or dim regions",
            VariableType::Struct,
        )
        .set_value_type(ValueType::Array)
        .set_schema::<CameraAnnotation>()
        .set_default_value(Some(json!([])));
        node.add_input_pin(
            "effects",
            "Effects",
            "Preview-only grayscale, sepia, blur, brightness and contrast",
            VariableType::Generic,
        )
        .set_default_value(Some(json!({})));
        node.add_input_pin(
            "ttl_ms",
            "Lifetime (ms)",
            "Remove annotations after 100 to 60000 milliseconds",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(5000)));
        node
    }
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        begin(context).await?;
        let frame: CameraFrame = context.evaluate_pin("frame").await?;
        let overlays: Value = context.evaluate_pin("overlays").await?;
        let effects: Value = context.evaluate_pin("effects").await?;
        let ttl: i64 = context.evaluate_pin("ttl_ms").await?;
        let mut args = target(&frame);
        args["overlays"] = overlays;
        args["effects"] = effects;
        args["ttlMs"] = json!(ttl);
        if command(context, "camera.updateOverlays", args)
            .await?
            .is_some()
        {
            context.activate_exec_pin("exec_out").await?;
        }
        Ok(())
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct ReadCameraImage;
impl ReadCameraImage {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for ReadCameraImage {
    fn get_node(&self) -> Node {
        let mut node = base(
            "a2ui_camera_read_image",
            "Read Camera Image",
            "Load a captured camera frame into an image for OCR or detection nodes",
            "readCameraImage",
        );
        frame_input(&mut node);
        node.add_output_pin(
            "image",
            "Image",
            "Captured image pixels",
            VariableType::Struct,
        )
        .set_schema::<NodeImage>();
        node
    }
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        begin(context).await?;
        let output = context.get_pin_by_name("image").await?;
        context.clear_pin_override(output.id());
        output.reset().await;
        let frame: CameraFrame = context.evaluate_pin("frame").await?;
        let limit = 16 * 1024 * 1024;
        let result: flow_like_types::Result<NodeImage> = async {
            if frame.image.size > limit as u64 {
                return Err(flow_like_types::anyhow!("Camera image exceeds 16 MiB"));
            }
            let bytes = if let Some(path) = frame.image.flow_path {
                path.get(context, false).await?
            } else if let Some(path) = context
                .execution_environment()
                .is_local()
                .then(|| {
                    crate::a2ui::elements::get_file_input_files::decode_local_file_url(
                        &frame.image.url,
                    )
                })
                .flatten()
            {
                use flow_like_types::tokio::io::AsyncReadExt;

                let file = flow_like_types::tokio::fs::File::open(path).await?;
                let metadata = file.metadata().await?;
                anyhow::ensure!(metadata.is_file(), "Camera image is not a regular file");
                anyhow::ensure!(
                    metadata.len() <= limit as u64,
                    "Camera image exceeds 16 MiB"
                );
                let mut bytes = Vec::with_capacity(metadata.len() as usize);
                file.take(limit as u64 + 1).read_to_end(&mut bytes).await?;
                bytes
            } else {
                let client = GuardedHttpClient::new(context.execution_environment())?;
                let response = client
                    .get(&frame.image.url)?
                    .timeout(Duration::from_secs(30))
                    .send()
                    .await
                    .map_err(flow_like_types::reqwest::Error::without_url)?
                    .error_for_status()
                    .map_err(flow_like_types::reqwest::Error::without_url)?;
                let mut stream = response.bytes_stream();
                let mut data = Vec::new();
                while let Some(chunk) = stream.next().await {
                    let chunk = chunk.map_err(flow_like_types::reqwest::Error::without_url)?;
                    if data.len() + chunk.len() > limit {
                        return Err(flow_like_types::anyhow!("Camera image exceeds 16 MiB"));
                    }
                    data.extend_from_slice(&chunk);
                }
                data
            };
            if bytes.len() > limit {
                return Err(flow_like_types::anyhow!("Camera image exceeds 16 MiB"));
            }
            let mut reader = flow_like_types::image::ImageReader::new(Cursor::new(bytes))
                .with_guessed_format()?;
            let mut limits = flow_like_types::image::Limits::default();
            limits.max_image_width = Some(4096);
            limits.max_image_height = Some(4096);
            limits.max_alloc = Some(128 * 1024 * 1024);
            reader.limits(limits);
            let image = reader.decode()?;
            Ok(NodeImage::new(context, image).await)
        }
        .await;
        match result {
            Ok(image) => {
                context.set_pin_value("image", json!(image)).await?;
                context.activate_exec_pin("exec_out").await?;
            }
            Err(error) => {
                context
                    .set_pin_value(
                        "error",
                        json!({"code":"image_unavailable","message":error.to_string()}),
                    )
                    .await?;
                context.activate_exec_pin("exec_error").await?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(feature = "execute")]
    async fn image_context() -> ExecutionContext {
        use ahash::AHashMap;
        use flow_like::{
            flow::{
                board::ExecutionStage,
                execution::{LogLevel, internal_node::InternalNode, internal_pin::InternalPin},
            },
            profile::Profile,
            state::{FlowLikeConfig, FlowLikeState},
            utils::http::HTTPClient,
        };
        use flow_like_types::sync::{Mutex, RwLock};
        use std::sync::{Arc, Weak};

        let node = ReadCameraImage.get_node();
        let mut pins = AHashMap::new();
        let mut names = AHashMap::<String, Vec<Arc<InternalPin>>>::new();
        for pin in node.pins.values() {
            let internal = Arc::new(InternalPin::new(pin, false));
            names
                .entry(pin.name.clone())
                .or_default()
                .push(internal.clone());
            pins.insert(pin.id.clone(), internal);
        }
        let current = Arc::new(InternalNode::new(
            node,
            pins,
            Arc::new(ReadCameraImage),
            names,
        ));
        for pin in current.pins.iter() {
            pin.init_node(Arc::downgrade(&current));
            pin.init_connected_to(Vec::new());
            pin.init_depends_on(Vec::new());
        }
        ExecutionContext::new(
            Arc::new(AHashMap::from_iter([(
                current.node_id().to_owned(),
                current.clone(),
            )])),
            &Weak::new(),
            &Arc::new(FlowLikeState::new(
                FlowLikeConfig::new(),
                HTTPClient::new_without_refetch(),
            )),
            &current,
            &Arc::new(Mutex::new(AHashMap::new())),
            &Arc::new(RwLock::new(AHashMap::new())),
            LogLevel::Debug,
            ExecutionStage::Dev,
            Arc::new(Profile::default()),
            None,
            Arc::new(RwLock::new(Vec::new())),
            None,
            None,
            Arc::new(AHashMap::new()),
            None,
        )
        .await
    }

    #[cfg(feature = "execute")]
    struct ImageTestDirectory(std::path::PathBuf);

    #[cfg(feature = "execute")]
    impl ImageTestDirectory {
        fn new() -> Self {
            let path =
                std::env::temp_dir().join(format!("camera-image-{}", flow_like_types::create_id()));
            std::fs::create_dir(&path).unwrap();
            Self(path)
        }
    }

    #[cfg(feature = "execute")]
    impl Drop for ImageTestDirectory {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[cfg(feature = "execute")]
    async fn read_local_image(context: &mut ExecutionContext, url: &str, declared_size: u64) {
        context.set_pin_value("frame", json!({
            "surfaceId":"screen","componentId":"camera","sessionId":"active","frameId":"frame",
            "capturedAt":"2026-09-13T12:00:05.000Z","width":8,"height":6,"mirrored":false,"orientation":0,
            "image":{"url":url,"name":"frame.png","type":"image/png","size":declared_size}
        })).await.unwrap();
        ReadCameraImage.run(context).await.unwrap();
    }

    #[cfg(feature = "execute")]
    #[tokio::test]
    async fn local_camera_asset_urls_produce_an_image_for_ocr() {
        let directory = ImageTestDirectory::new();
        let path = directory.0.join("camera frame.png");
        flow_like_types::image::DynamicImage::new_rgba8(8, 6)
            .save(&path)
            .unwrap();
        let url = flow_like_types::reqwest::Url::from_file_path(&path).unwrap();
        let size = std::fs::metadata(&path).unwrap().len();
        let mut context = image_context().await;
        for url in [
            url.to_string(),
            format!("asset://localhost{}?filename=frame.png", url.path()),
            format!("http://asset.localhost{}", url.path()),
        ] {
            read_local_image(&mut context, &url, size).await;
            assert!(context.evaluate_pin::<bool>("exec_out").await.unwrap());
            assert!(!context.evaluate_pin::<bool>("exec_error").await.unwrap());
            let image: NodeImage = context.evaluate_pin("image").await.unwrap();
            let pixels = image.get_image(&mut context).await.unwrap();
            let pixels = pixels.lock().await;
            assert_eq!((pixels.width(), pixels.height()), (8, 6));
        }
    }

    #[cfg(feature = "execute")]
    #[tokio::test]
    async fn local_camera_images_enforce_actual_size_and_clear_stale_outputs() {
        let directory = ImageTestDirectory::new();
        let path = directory.0.join("oversized.png");
        std::fs::File::create(&path)
            .unwrap()
            .set_len(16 * 1024 * 1024 + 1)
            .unwrap();
        let url = flow_like_types::reqwest::Url::from_file_path(&path).unwrap();
        let mut context = image_context().await;
        let old = json!({"image_ref":"old"});
        context.set_pin_value("image", old.clone()).await.unwrap();
        let pin = context.get_pin_by_name("image").await.unwrap();
        context.override_pin_value(pin.id(), old);
        read_local_image(&mut context, url.as_str(), 1).await;
        assert!(context.evaluate_pin::<bool>("exec_error").await.unwrap());
        assert!(!context.evaluate_pin::<bool>("exec_out").await.unwrap());
        assert!(pin.get_raw_value().await.is_none());
        assert!(
            !context
                .context_pin_overrides
                .as_ref()
                .is_some_and(|pins| pins.contains_key(pin.id()))
        );
        let error: Value = context.evaluate_pin("error").await.unwrap();
        assert_eq!(error["code"], "image_unavailable");
        assert!(error["message"].as_str().unwrap().contains("16 MiB"));
    }

    #[cfg(feature = "execute")]
    #[tokio::test]
    async fn local_camera_images_reject_directories_and_oversized_dimensions() {
        let directory = ImageTestDirectory::new();
        let path = directory.0.join("wide.png");
        flow_like_types::image::DynamicImage::new_rgba8(4097, 1)
            .save(&path)
            .unwrap();
        let mut context = image_context().await;
        for path in [&directory.0, &path] {
            let url = flow_like_types::reqwest::Url::from_file_path(path).unwrap();
            read_local_image(&mut context, url.as_str(), 1).await;
            assert!(context.evaluate_pin::<bool>("exec_error").await.unwrap());
            assert!(!context.evaluate_pin::<bool>("exec_out").await.unwrap());
            assert!(
                context
                    .get_pin_by_name("image")
                    .await
                    .unwrap()
                    .get_raw_value()
                    .await
                    .is_none()
            );
        }
    }

    #[test]
    fn annotations_keep_normalized_schema_and_camel_case() {
        let value =
            json!({"id":"translated", "type":"text", "x":0.2,"y":0.4,"text":"Hello","fontSize":20});
        let annotation: CameraAnnotation = flow_like_types::json::from_value(value).unwrap();
        assert_eq!(annotation.font_size, Some(20.0));
        assert_eq!(
            flow_like_types::json::to_value(annotation).unwrap()["type"],
            "text"
        );
    }

    #[test]
    fn camera_nodes_are_available_to_remote_events() {
        let capture = CaptureCameraFrame.get_node();
        assert!(!capture.only_offline);
        assert!(capture.get_pin_by_name("surface_id").is_some());
        assert!(!capture.get_pin_by_name("session_id").unwrap().is_optional());
        assert!(capture.get_pin_by_name("frame").is_some());
        let controls = ControlCamera.get_node();
        assert!(controls.get_pin_by_name("frame").is_some());
        assert!(controls.get_pin_by_name("exec_error").is_some());
    }
}
