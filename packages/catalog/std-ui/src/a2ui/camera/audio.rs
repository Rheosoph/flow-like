use super::{CameraFrame, base, begin, command};
use anyhow::ensure;
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_catalog_core::FlowPath;
use flow_like_types::{Value, async_trait, json::json};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CameraAudioFile {
    pub url: String,
    pub name: String,
    pub r#type: String,
    pub size: u64,
    pub flow_path: Option<FlowPath>,
}

/// A recent microphone window uploaded by the invoking screen.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CameraAudioClip {
    pub surface_id: String,
    pub component_id: String,
    pub session_id: String,
    pub clip_id: String,
    pub captured_at: String,
    pub started_at: String,
    pub ended_at: String,
    pub duration_ms: f64,
    pub requested_duration_ms: u64,
    pub sample_rate: u32,
    pub channels: u16,
    pub audio: CameraAudioFile,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CameraInput {
    pub frame: CameraFrame,
    pub audio: CameraAudioClip,
}

fn duration_ms(seconds: f64) -> flow_like_types::Result<u64> {
    ensure!(
        seconds.is_finite() && (0.1..=300.0).contains(&seconds),
        "Audio duration must be between 0.1 and 300 seconds"
    );
    Ok((seconds * 1000.0).round() as u64)
}

impl CameraAudioClip {
    fn validate(&self, target: &Value, remote: bool) -> flow_like_types::Result<()> {
        ensure!(
            target["surfaceId"].as_str() == Some(self.surface_id.as_str())
                && target["componentId"].as_str() == Some(self.component_id.as_str())
                && target["sessionId"].as_str() == Some(self.session_id.as_str()),
            "The audio belongs to a different camera session"
        );
        ensure!(
            !self.clip_id.is_empty() && self.clip_id.len() <= 512,
            "The audio clip identifier is invalid"
        );
        ensure!(
            target["durationMs"].as_u64() == Some(self.requested_duration_ms)
                && (100..=300_000).contains(&self.requested_duration_ms)
                && self.duration_ms.is_finite()
                && self.duration_ms > 0.0
                && self.duration_ms <= self.requested_duration_ms as f64 + 0.1,
            "The audio window does not match the requested duration"
        );
        ensure!(
            self.sample_rate == 16_000 && self.channels == 1 && self.audio.r#type == "audio/wav",
            "Camera audio must be a mono 16 kHz WAV file"
        );
        let expected_bytes = 44.0 + (self.duration_ms * 16.0).round() * 2.0;
        ensure!(
            self.audio.size > 44
                && self.audio.size <= 9_600_044
                && (self.audio.size as f64 - expected_bytes).abs() <= 2.0,
            "The audio file size does not match its duration"
        );
        let started = chrono::DateTime::parse_from_rfc3339(&self.started_at)?;
        let ended = chrono::DateTime::parse_from_rfc3339(&self.ended_at)?;
        chrono::DateTime::parse_from_rfc3339(&self.captured_at)?;
        ensure!(
            ended >= started
                && ((ended - started).num_milliseconds() as f64 - self.duration_ms).abs() <= 5.0,
            "The audio timestamps do not match its duration"
        );
        ensure!(
            !self.audio.name.is_empty(),
            "The audio file name is missing"
        );
        let url = flow_like_types::reqwest::Url::parse(&self.audio.url)?;
        ensure!(
            url.username().is_empty() && url.password().is_none(),
            "The audio URL must not contain credentials"
        );
        ensure!(
            !remote
                || (matches!(url.scheme(), "http" | "https")
                    && url.host_str() != Some("asset.localhost")),
            "Remote camera audio requires a temporary HTTP download URL"
        );
        Ok(())
    }
}

fn capture_node(combined: bool) -> Node {
    let mut node = if combined {
        base(
            "a2ui_camera_capture_input",
            "Capture Camera Input",
            "Capture a camera frame and recent microphone audio from the active screen before uploading both temporary files",
            "captureCameraInput",
        )
    } else {
        base(
            "a2ui_camera_capture_audio",
            "Capture Camera Audio",
            "Request recent microphone audio from a Camera View the user started with audio enabled; returns the available part of the requested window",
            "captureCameraAudio",
        )
    };
    for (name, label, description) in [
        ("surface_id", "Surface ID", "The camera's rendered surface"),
        ("component_id", "Camera ID", "The Camera View component id"),
        (
            "session_id",
            "Session ID",
            "Current sessionId from Camera View's value binding or ready Event",
        ),
    ] {
        node.add_input_pin(name, label, description, VariableType::String);
    }
    node.add_input_pin(
        "duration_seconds", "Duration", "Recent audio window in seconds, from 0.1 to 300. A shorter recording or buffer returns the available audio", VariableType::Float,
    ).set_default_value(Some(json!(10.0)))
        .set_options(PinOptions::new().set_range((0.1, 300.0)).set_step(0.1).build());
    if combined {
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
            "Camera frame captured with the audio window",
            VariableType::Struct,
        )
        .set_schema::<CameraFrame>();
    }
    node.add_output_pin(
        "clip",
        "Clip",
        "Audio file, actual time window and camera session identity",
        VariableType::Struct,
    )
    .set_schema::<CameraAudioClip>();
    node.add_output_pin(
        "audio_url",
        "Audio URL",
        "Temporary audio URL; remote captures return a signed download URL",
        VariableType::String,
    );
    node.add_output_pin(
        "audio_path",
        "Audio Path",
        "Optional temporary FlowPath for Speech to Text and file nodes in this run",
        VariableType::Struct,
    )
    .set_schema::<FlowPath>();
    node.add_output_pin(
        "recorded_duration_seconds",
        "Recorded Duration",
        "Actual audio duration in seconds",
        VariableType::Float,
    );
    node
}

async fn failure(
    context: &mut ExecutionContext,
    code: &str,
    message: String,
) -> flow_like_types::Result<()> {
    context
        .set_pin_value("error", json!({"code":code,"message":message}))
        .await?;
    context.activate_exec_pin("exec_error").await
}

async fn capture(context: &mut ExecutionContext, combined: bool) -> flow_like_types::Result<()> {
    begin(context).await?;
    for name in [
        "clip",
        "audio_url",
        "audio_path",
        "recorded_duration_seconds",
    ] {
        let pin = context.get_pin_by_name(name).await?;
        context.clear_pin_override(&pin.id);
        pin.reset().await;
    }
    if combined {
        let pin = context.get_pin_by_name("frame").await?;
        context.clear_pin_override(&pin.id);
        pin.reset().await;
    }
    let seconds: f64 = context.evaluate_pin("duration_seconds").await?;
    let duration = match duration_ms(seconds) {
        Ok(value) => value,
        Err(error) => return failure(context, "invalid_arguments", error.to_string()).await,
    };
    let surface_id: String = context.evaluate_pin("surface_id").await?;
    let component_id: String = context.evaluate_pin("component_id").await?;
    let session_id: String = context.evaluate_pin("session_id").await?;
    let mut target = json!({"surfaceId":surface_id,"componentId":component_id,"sessionId":session_id,"durationMs":duration});
    if combined {
        target["maxWidth"] = json!(context.evaluate_pin::<i64>("max_width").await?);
        target["quality"] = json!(context.evaluate_pin::<f64>("quality").await?);
    }
    let operation = if combined {
        "camera.captureInput"
    } else {
        "camera.captureAudio"
    };
    let Some(response) = command(context, operation, target.clone()).await? else {
        return Ok(());
    };
    let parsed = (|| -> flow_like_types::Result<(Option<CameraFrame>, CameraAudioClip)> {
        let (frame, clip) = if combined {
            let input: CameraInput = flow_like_types::json::from_value(response)?;
            ensure!(
                input.frame.surface_id == surface_id
                    && input.frame.component_id == component_id
                    && input.frame.session_id == session_id,
                "The frame belongs to a different camera session"
            );
            (Some(input.frame), input.audio)
        } else {
            (None, flow_like_types::json::from_value(response)?)
        };
        clip.validate(&target, !context.execution_environment().is_local())?;
        Ok((frame, clip))
    })();
    let (frame, mut clip) = match parsed {
        Ok(value) => value,
        Err(error) => return failure(context, "invalid_audio", error.to_string()).await,
    };
    if context.execution_environment().is_local() && clip.audio.flow_path.is_none() {
        if let Some(path) =
            crate::a2ui::elements::get_file_input_files::decode_local_file_url(&clip.audio.url)
        {
            let path = std::path::PathBuf::from(path);
            let local_path = async {
                let metadata = flow_like_types::tokio::fs::metadata(&path).await?;
                ensure!(
                    metadata.is_file() && metadata.len() == clip.audio.size,
                    "The temporary audio file does not match its reported size"
                );
                FlowPath::from_pathbuf(path, context).await
            }
            .await;
            match local_path {
                Ok(path) => clip.audio.flow_path = Some(path),
                Err(error) => return failure(context, "invalid_audio", error.to_string()).await,
            }
        }
    }
    if let Some(frame) = frame {
        context.set_pin_value("frame", json!(frame)).await?;
    }
    context
        .set_pin_value("audio_url", json!(clip.audio.url))
        .await?;
    context
        .set_pin_value("audio_path", json!(clip.audio.flow_path))
        .await?;
    context
        .set_pin_value(
            "recorded_duration_seconds",
            json!(clip.duration_ms / 1000.0),
        )
        .await?;
    context.set_pin_value("clip", json!(clip)).await?;
    context.activate_exec_pin("exec_out").await
}

#[crate::register_node]
#[derive(Default)]
pub struct CaptureCameraAudio;

#[async_trait]
impl NodeLogic for CaptureCameraAudio {
    fn get_node(&self) -> Node {
        capture_node(false)
    }
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        capture(context, false).await
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct CaptureCameraInput;

#[async_trait]
impl NodeLogic for CaptureCameraInput {
    fn get_node(&self) -> Node {
        capture_node(true)
    }
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        capture(context, true).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(feature = "execute")]
    async fn execution_context(combined: bool, reply: Option<Value>) -> ExecutionContext {
        use ahash::AHashMap;
        use flow_like::{
            flow::{
                board::ExecutionStage,
                execution::{
                    LogLevel, context::ExecutionContextCache, internal_node::InternalNode,
                    internal_pin::InternalPin,
                },
            },
            profile::Profile,
            state::{FlowLikeConfig, FlowLikeState, FlowLikeStores},
            utils::http::HTTPClient,
        };
        use flow_like_types::{
            channel::{
                Channel, ChannelPush, ChannelPushKind, InProcessChannel, InProcessPushResult,
            },
            intercom::InterComCallback,
            sync::{Mutex, RwLock},
        };
        use std::{
            sync::{Arc, Weak},
            time::Duration,
        };

        let logic: Arc<dyn NodeLogic> = if combined {
            Arc::new(CaptureCameraInput)
        } else {
            Arc::new(CaptureCameraAudio)
        };
        let node = logic.get_node();
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
        let current = Arc::new(InternalNode::new(node, pins, logic, names));
        for pin in current.pins.iter() {
            pin.init_node(Arc::downgrade(&current));
            pin.init_connected_to(Vec::new());
            pin.init_depends_on(Vec::new());
        }
        let channel = if reply.is_some() {
            Some(
                InProcessChannel::register(flow_like_types::create_id(), Duration::from_secs(60))
                    .await,
            )
        } else {
            None
        };
        let callback: InterComCallback = channel.as_ref().map(|channel| {
            let channel = channel.clone();
            let reply = reply.clone().unwrap();
            Arc::new(move |event: flow_like_types::intercom::InterComEvent| {
                let channel = channel.clone();
                let reply = reply.clone();
                Box::pin(async move {
                    if event.event_type != "a2ui" {
                        return Ok(());
                    }
                    let message = flow_like_types::json::from_value(event.payload)?;
                    if let flow_like::a2ui::A2UIServerMessage::DeviceCommand {
                        request_id,
                        command,
                        args,
                        ..
                    } = message
                    {
                        assert_eq!(
                            command,
                            if combined {
                                "camera.captureInput"
                            } else {
                                "camera.captureAudio"
                            }
                        );
                        assert_eq!(args["surfaceId"], "screen");
                        assert_eq!(args["componentId"], "camera");
                        assert_eq!(args["sessionId"], "active");
                        assert_eq!(args["durationMs"], 10_000);
                        if combined {
                            assert_eq!(args["maxWidth"], 1280);
                            assert_eq!(args["quality"], 0.85);
                        }
                        assert_eq!(
                            channel
                                .push(ChannelPush {
                                    channel_id: channel.channel_id().to_owned(),
                                    request_id: Some(request_id),
                                    kind: ChannelPushKind::Reply,
                                    value: reply,
                                })
                                .await,
                            InProcessPushResult::Delivered
                        );
                    }
                    Ok(())
                })
                    as futures::future::BoxFuture<'static, flow_like_types::Result<()>>
            }) as Arc<_>
        });
        let mut context = ExecutionContext::new(
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
            callback,
            Arc::new(RwLock::new(Vec::new())),
            None,
            None,
            Arc::new(AHashMap::new()),
            channel.map(|channel| channel as Arc<dyn Channel>),
        )
        .await;
        if reply.is_some() {
            context.execution_cache = Some(ExecutionContextCache {
                stores: FlowLikeStores::default(),
                app_id: "app".into(),
                model_usage_app_id: None,
                board_dir: flow_like_storage::Path::from("board"),
                board_id: "board".into(),
                node_id: current.shared_node_id(),
                sub: "user".into(),
                shadow: false,
            });
        }
        for (name, value) in [
            ("surface_id", "screen"),
            ("component_id", "camera"),
            ("session_id", "active"),
        ] {
            context.set_pin_value(name, json!(value)).await.unwrap();
        }
        context
    }

    #[cfg(feature = "execute")]
    async fn run_capture(context: &mut ExecutionContext, combined: bool) {
        let logic: Box<dyn NodeLogic> = if combined {
            Box::new(CaptureCameraInput)
        } else {
            Box::new(CaptureCameraAudio)
        };
        tokio::time::timeout(std::time::Duration::from_secs(2), logic.run(context))
            .await
            .expect("mock client must reply without a device")
            .unwrap();
    }

    #[cfg(feature = "execute")]
    #[tokio::test]
    async fn node_run_clears_old_outputs_and_overrides_before_routing_frontend_unavailable() {
        for combined in [false, true] {
            let mut context = execution_context(combined, None).await;
            let mut outputs = vec![
                "clip",
                "audio_url",
                "audio_path",
                "recorded_duration_seconds",
            ];
            if combined {
                outputs.push("frame");
            }
            for name in &outputs {
                let stale = match *name {
                    "audio_url" => json!("https://files.example/old.wav"),
                    "recorded_duration_seconds" => json!(5.0),
                    _ => json!({"old":true}),
                };
                context.set_pin_value(name, stale.clone()).await.unwrap();
                let pin = context.get_pin_by_name(name).await.unwrap();
                context.override_pin_value(&pin.id, stale);
            }
            context.activate_exec_pin("exec_out").await.unwrap();
            run_capture(&mut context, combined).await;
            for name in outputs {
                let pin = context.get_pin_by_name(name).await.unwrap();
                assert!(pin.get_raw_value().await.is_none(), "stale {name} survived");
                assert!(
                    !context
                        .context_pin_overrides
                        .as_ref()
                        .is_some_and(|pins| pins.contains_key(pin.id()))
                );
            }
            assert_eq!(
                context
                    .evaluate_pin::<f64>("duration_seconds")
                    .await
                    .unwrap(),
                10.0
            );
            assert_eq!(
                context.evaluate_pin::<String>("session_id").await.unwrap(),
                "active"
            );
            assert_eq!(
                context.evaluate_pin::<Value>("error").await.unwrap()["code"],
                "frontend_unavailable"
            );
            assert!(!context.evaluate_pin::<bool>("exec_out").await.unwrap());
            assert!(context.evaluate_pin::<bool>("exec_error").await.unwrap());
        }
    }

    #[cfg(feature = "execute")]
    #[tokio::test]
    async fn node_run_routes_invalid_duration_before_requesting_a_client() {
        for combined in [false, true] {
            let mut context = execution_context(combined, None).await;
            context
                .set_pin_value("duration_seconds", json!(301.0))
                .await
                .unwrap();
            run_capture(&mut context, combined).await;
            assert_eq!(
                context.evaluate_pin::<Value>("error").await.unwrap()["code"],
                "invalid_arguments"
            );
            assert_eq!(
                context
                    .evaluate_pin::<f64>("duration_seconds")
                    .await
                    .unwrap(),
                301.0
            );
            assert!(context.evaluate_pin::<bool>("exec_error").await.unwrap());
            assert!(!context.evaluate_pin::<bool>("exec_out").await.unwrap());
        }
    }

    #[cfg(feature = "execute")]
    #[tokio::test]
    async fn node_run_rejects_audio_from_an_old_camera_session() {
        let mut clip = clip();
        clip.session_id = "old".into();
        let mut context = execution_context(false, Some(json!({"ok":true,"value":clip}))).await;
        run_capture(&mut context, false).await;
        assert_eq!(
            context.evaluate_pin::<Value>("error").await.unwrap()["code"],
            "invalid_audio"
        );
        assert!(context.evaluate_pin::<bool>("exec_error").await.unwrap());
        assert!(!context.evaluate_pin::<bool>("exec_out").await.unwrap());
        assert!(
            context
                .get_pin_by_name("clip")
                .await
                .unwrap()
                .get_raw_value()
                .await
                .is_none()
        );
    }

    #[cfg(feature = "execute")]
    fn five_second_wav() -> Vec<u8> {
        let mut wav = Vec::new();
        wav.extend_from_slice(b"RIFF");
        wav.extend_from_slice(&160_036u32.to_le_bytes());
        wav.extend_from_slice(b"WAVEfmt ");
        wav.extend_from_slice(&16u32.to_le_bytes());
        wav.extend_from_slice(&1u16.to_le_bytes());
        wav.extend_from_slice(&1u16.to_le_bytes());
        wav.extend_from_slice(&16_000u32.to_le_bytes());
        wav.extend_from_slice(&32_000u32.to_le_bytes());
        wav.extend_from_slice(&2u16.to_le_bytes());
        wav.extend_from_slice(&16u16.to_le_bytes());
        wav.extend_from_slice(b"data");
        wav.extend_from_slice(&160_000u32.to_le_bytes());
        wav.resize(160_044, 0);
        wav
    }

    #[cfg(feature = "execute")]
    #[tokio::test]
    async fn node_run_resolves_local_uploaded_audio_for_speech_to_text() {
        struct TemporaryAudio(std::path::PathBuf);
        impl Drop for TemporaryAudio {
            fn drop(&mut self) {
                let _ = std::fs::remove_file(&self.0);
            }
        }
        let file = TemporaryAudio(
            std::env::temp_dir().join(format!("camera-audio-{}.wav", flow_like_types::create_id())),
        );
        let wav = five_second_wav();
        std::fs::write(&file.0, &wav).unwrap();
        let mut clip = clip();
        let url = flow_like_types::reqwest::Url::from_file_path(&file.0).unwrap();
        clip.audio.url = format!("asset://localhost{}?filename=recording.wav", url.path());
        assert!(clip.audio.flow_path.is_none());
        let mut context = execution_context(false, Some(json!({"ok":true,"value":clip}))).await;
        run_capture(&mut context, false).await;
        assert!(context.evaluate_pin::<bool>("exec_out").await.unwrap());
        assert!(!context.evaluate_pin::<bool>("exec_error").await.unwrap());
        let output: FlowPath = context.evaluate_pin("audio_path").await.unwrap();
        assert_eq!(output.get(&mut context, false).await.unwrap(), wav);
        assert_eq!(
            context.evaluate_pin::<String>("audio_url").await.unwrap(),
            clip.audio.url
        );
        std::fs::write(&file.0, b"incomplete upload").unwrap();
        run_capture(&mut context, false).await;
        assert_eq!(
            context.evaluate_pin::<Value>("error").await.unwrap()["code"],
            "invalid_audio"
        );
        assert!(context.evaluate_pin::<bool>("exec_error").await.unwrap());
        assert!(!context.evaluate_pin::<bool>("exec_out").await.unwrap());
        assert!(
            context
                .get_pin_by_name("audio_path")
                .await
                .unwrap()
                .get_raw_value()
                .await
                .is_none()
        );
    }

    #[cfg(feature = "execute")]
    #[tokio::test]
    async fn node_run_preserves_the_temporary_flow_path_consumed_by_speech_to_text() {
        use flow_like_storage::{files::store::FlowLikeStore, object_store::memory::InMemory};
        use flow_like_types::dispatch::REQUEST_FILES_STORE_REF;
        use std::sync::Arc;

        let mut clip = clip();
        let path = FlowPath::new(
            "apps/app/invoke/clip.wav".into(),
            REQUEST_FILES_STORE_REF.into(),
            None,
        );
        clip.audio.flow_path = Some(path.clone());
        let frame = json!({
            "surfaceId":"screen","componentId":"camera","sessionId":"active","frameId":"frame",
            "capturedAt":"2026-09-13T12:00:05.000Z","width":1280,"height":720,"mirrored":false,"orientation":0,
            "image":{"url":"https://files.example/frame.jpg","name":"frame.jpg","type":"image/jpeg","size":1000}
        });
        for combined in [false, true] {
            let response = if combined {
                json!({"frame":frame,"audio":clip})
            } else {
                json!(clip)
            };
            let mut context =
                execution_context(combined, Some(json!({"ok":true,"value":response}))).await;
            context
                .set_cache(
                    REQUEST_FILES_STORE_REF,
                    Arc::new(FlowLikeStore::Memory(Arc::new(InMemory::new()))),
                )
                .await;
            let wav = five_second_wav();
            path.put(&mut context, wav.clone(), true).await.unwrap();
            run_capture(&mut context, combined).await;
            let output: FlowPath = context.evaluate_pin("audio_path").await.unwrap();
            // SpeechToTextNode reads its audio pin with exactly this FlowPath operation.
            assert_eq!(output.get(&mut context, false).await.unwrap(), wav);
            assert_eq!(output.store_ref, REQUEST_FILES_STORE_REF);
            assert_eq!(
                context
                    .evaluate_pin::<f64>("recorded_duration_seconds")
                    .await
                    .unwrap(),
                5.0
            );
            assert_eq!(
                context.evaluate_pin::<String>("audio_url").await.unwrap(),
                clip.audio.url
            );
            assert!(context.evaluate_pin::<bool>("exec_out").await.unwrap());
            assert!(!context.evaluate_pin::<bool>("exec_error").await.unwrap());
            if combined {
                assert_eq!(
                    context.evaluate_pin::<Value>("frame").await.unwrap()["frameId"],
                    "frame"
                );
            }
        }
    }

    fn clip() -> CameraAudioClip {
        flow_like_types::json::from_value(json!({
            "surfaceId":"screen","componentId":"camera","sessionId":"active","clipId":"clip",
            "capturedAt":"2026-09-13T12:00:05.000Z","startedAt":"2026-09-13T12:00:00.000Z","endedAt":"2026-09-13T12:00:05.000Z",
            "durationMs":5000.0,"requestedDurationMs":10000,"sampleRate":16000,"channels":1,
            "audio":{"url":"https://files.example/clip.wav?signature=opaque","name":"clip.wav","type":"audio/wav","size":160044}
        })).unwrap()
    }
    fn target() -> Value {
        json!({"surfaceId":"screen","componentId":"camera","sessionId":"active","durationMs":10000})
    }
    #[test]
    fn ten_second_request_accepts_five_seconds_of_available_audio() {
        let clip = clip();
        clip.validate(&target(), true).unwrap();
        assert_eq!(clip.duration_ms / 1000.0, 5.0);
        assert!(clip.audio.flow_path.is_none());
    }
    #[test]
    fn rejects_wrong_session_overlong_clip_and_inconsistent_wav_metadata() {
        let mut changed = clip();
        changed.session_id = "old".into();
        assert!(changed.validate(&target(), true).is_err());
        changed = clip();
        changed.duration_ms = 11000.0;
        assert!(changed.validate(&target(), true).is_err());
        changed = clip();
        changed.audio.size = 100;
        assert!(changed.validate(&target(), true).is_err());
        changed = clip();
        changed.sample_rate = 48000;
        assert!(changed.validate(&target(), true).is_err());
        changed = clip();
        changed.ended_at = "2026-09-13T12:00:09Z".into();
        assert!(changed.validate(&target(), true).is_err());
    }
    #[test]
    fn remote_audio_rejects_client_only_urls_and_preserves_signed_url() {
        let mut clip = clip();
        let signed = clip.audio.url.clone();
        clip.validate(&target(), true).unwrap();
        assert_eq!(clip.audio.url, signed);
        for url in [
            "blob:https://example.com/clip",
            "asset://localhost/cache/clip.wav",
            "http://asset.localhost/cache/clip.wav",
            "https://asset.localhost/cache/clip.wav",
        ] {
            clip.audio.url = url.into();
            assert!(clip.validate(&target(), true).is_err(), "accepted {url}");
            clip.validate(&target(), false).unwrap();
        }
    }
    #[test]
    fn duration_bounds_and_remote_node_contracts_are_explicit() {
        for invalid in [f64::NAN, f64::INFINITY, -1.0, 0.0, 0.09, 301.0] {
            assert!(duration_ms(invalid).is_err());
        }
        assert_eq!(duration_ms(10.0).unwrap(), 10000);
        for node in [CaptureCameraAudio.get_node(), CaptureCameraInput.get_node()] {
            assert!(!node.only_offline);
            let names: std::collections::HashSet<_> =
                node.pins.values().map(|pin| &pin.name).collect();
            assert_eq!(names.len(), node.pins.len());
            assert!(node.get_pin_by_name("session_id").is_some());
            assert!(node.get_pin_by_name("clip").is_some());
            assert!(
                node.get_pin_by_name("audio_path")
                    .unwrap()
                    .schema
                    .as_ref()
                    .unwrap()
                    .contains("store_ref")
            );
        }
    }
}
