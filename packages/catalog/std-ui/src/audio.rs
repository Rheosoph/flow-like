use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_catalog_core::FlowPath;
use flow_like_types::{Value, async_trait, json::json};
use std::time::Duration;

#[crate::register_node]
#[derive(Default)]
pub struct PlaySound;

impl PlaySound {
    pub fn new() -> Self {
        Self
    }
}

async fn fail(
    context: &mut ExecutionContext,
    code: &str,
    message: &str,
) -> flow_like_types::Result<()> {
    context
        .set_pin_value("error", json!({ "code": code, "message": message }))
        .await?;
    context.activate_exec_pin("exec_error").await
}

#[async_trait]
impl NodeLogic for PlaySound {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "ui_play_sound",
            "Play Sound",
            "Play audio on the invoking frontend and continue when playback finishes, including when the Event runs remotely",
            "UI/Audio",
        );
        node.set_flowscript_name("ui", "playSound");
        node.add_icon("/flow/icons/audio.svg");
        node.set_long_running(true);
        node.add_input_pin("exec_in", "▶", "Play the sound", VariableType::Execution);
        node.add_input_pin(
            "audio_url",
            "Audio URL",
            "Audio or signed download URL. Supply either Audio URL or Audio Path",
            VariableType::String,
        )
        .set_default_value(Some(json!("")))
        .set_options(PinOptions::new().set_optional(true).build());
        node.add_input_pin(
            "audio_path", "Audio Path", "FlowPath from Text to Speech, Capture Camera Audio, or file nodes. Supply either Audio Path or Audio URL", VariableType::Struct,
        ).set_schema::<FlowPath>()
            .set_default_value(Some(Value::Null))
            .set_options(PinOptions::new().set_optional(true).build());
        node.add_input_pin(
            "volume",
            "Volume",
            "Playback volume from 0 to 1, subject to device volume controls",
            VariableType::Float,
        )
        .set_default_value(Some(json!(1.0)))
        .set_options(
            PinOptions::new()
                .set_range((0.0, 1.0))
                .set_step(0.05)
                .build(),
        );
        node.add_input_pin(
            "timeout_seconds", "Timeout", "Maximum seconds for the frontend to load, obtain a Play tap if required, and finish audio. Playback stops on timeout", VariableType::Integer,
        ).set_default_value(Some(json!(300)))
            .set_options(PinOptions::new().set_range((1.0, 600.0)).set_step(1.0).build());
        node.add_output_pin(
            "exec_out",
            "▶",
            "Playback finished",
            VariableType::Execution,
        );
        node.add_output_pin(
            "exec_error",
            "Error",
            "Playback failed, was cancelled, or timed out",
            VariableType::Execution,
        );
        node.add_output_pin(
            "duration_seconds",
            "Duration",
            "Duration of the completed sound in seconds",
            VariableType::Float,
        );
        node.add_output_pin(
            "error",
            "Error",
            "Structured error code and message",
            VariableType::Generic,
        );
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("exec_error").await?;
        context.set_pin_value("error", Value::Null).await?;
        let duration = context.get_pin_by_name("duration_seconds").await?;
        context.clear_pin_override(duration.id());
        duration.reset().await;
        let volume: f64 = context.evaluate_pin("volume").await?;
        let timeout: i64 = context.evaluate_pin("timeout_seconds").await?;
        if !volume.is_finite() || !(0.0..=1.0).contains(&volume) || !(1..=600).contains(&timeout) {
            return fail(
                context,
                "invalid_arguments",
                "Volume must be between 0 and 1 and Timeout between 1 and 600 seconds",
            )
            .await;
        }
        let url: String = context.evaluate_pin("audio_url").await?;
        let path: Option<FlowPath> = context.evaluate_pin("audio_path").await?;
        if url.trim().is_empty() == path.is_none() {
            return fail(
                context,
                "invalid_arguments",
                "Supply exactly one Audio URL or Audio Path",
            )
            .await;
        }
        let timeout = Duration::from_secs(timeout as u64);
        let cancellation = context.cancellation_token().unwrap_or_default();
        let source = tokio::select! {
            biased;
            _ = cancellation.cancelled() => return fail(context, "cancelled", "Sound playback was cancelled").await,
            source = tokio::time::timeout(timeout, crate::audio_source::resolve_audio_source(context, url, path, timeout)) => {
                match source {
                    Ok(source) => source,
                    Err(_) => return fail(context, "source_timeout", "The audio source was not ready before its deadline").await,
                }
            },
        };
        let source = match source {
            Ok(source) => source,
            Err(error) => return fail(context, "invalid_audio_source", &error.to_string()).await,
        };
        let response = match context
            .request_device("audio.play", json!({"url":source,"volume":volume}), timeout)
            .await
        {
            Ok(response) => response,
            Err(_) => {
                return fail(
                    context,
                    "playback_failed",
                    "The frontend playback acknowledgement could not be read",
                )
                .await;
            }
        };
        if response["ok"] != true {
            let error = &response["error"];
            return fail(
                context,
                error["code"].as_str().unwrap_or("playback_failed"),
                error["message"]
                    .as_str()
                    .unwrap_or("The invoking frontend could not play this sound"),
            )
            .await;
        }
        let value = &response["value"];
        let duration = value["durationSeconds"].as_f64();
        if value["status"] != "finished"
            || !duration.is_some_and(|value| value.is_finite() && value >= 0.0)
        {
            return fail(
                context,
                "invalid_response",
                "The frontend did not confirm completed sound playback",
            )
            .await;
        }
        context
            .set_pin_value("duration_seconds", json!(duration))
            .await?;
        context.activate_exec_pin("exec_out").await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sound_node_accepts_file_paths_and_urls_for_remote_events() {
        let node = PlaySound.get_node();
        assert!(!node.only_offline);
        assert!(node.get_pin_by_name("audio_path").unwrap().is_optional());
        assert!(node.get_pin_by_name("audio_url").unwrap().is_optional());
        let names: std::collections::HashSet<_> = node.pins.values().map(|pin| &pin.name).collect();
        assert_eq!(names.len(), node.pins.len());
        assert!(node.get_pin_by_name("exec_error").is_some());
    }

    #[cfg(feature = "execute")]
    async fn context(reply: Option<Value>) -> ExecutionContext {
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
        use std::sync::{Arc, Weak};
        let logic: Arc<dyn NodeLogic> = Arc::new(PlaySound);
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
        let channel =
            InProcessChannel::register(flow_like_types::create_id(), Duration::from_secs(900))
                .await;
        let callback: InterComCallback = reply.map(|reply| {
            let channel = channel.clone();
            Arc::new(move |event: flow_like_types::intercom::InterComEvent| {
                let reply = reply.clone();
                let channel = channel.clone();
                Box::pin(async move {
                    if event.event_type == "a2ui" {
                        if let flow_like::a2ui::A2UIServerMessage::DeviceCommand {request_id, command, args, timeout_ms, ..} = flow_like_types::json::from_value(event.payload)? {
                            assert_eq!(command, "audio.play");
                            assert_eq!(args, json!({"url":"https://files.example/sound.wav?signature=keep%2Bthis", "volume":1.0}));
                            assert_eq!(timeout_ms, 300_000, "playback must retain a timeout longer than ordinary device commands");
                            assert_eq!(channel.push(ChannelPush {channel_id:channel.channel_id().to_owned(), request_id:Some(request_id), kind:ChannelPushKind::Reply, value:reply}).await, InProcessPushResult::Delivered);
                        }
                    }
                    Ok(())
                }) as futures::future::BoxFuture<'static, flow_like_types::Result<()>>
            }) as Arc<_>
        });
        let attached = callback.is_some();
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
            Some(channel as Arc<dyn Channel>),
        )
        .await;
        if attached {
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
        context
            .set_pin_value(
                "audio_url",
                json!("https://files.example/sound.wav?signature=keep%2Bthis"),
            )
            .await
            .unwrap();
        context
    }

    #[cfg(feature = "execute")]
    async fn run(context: &mut ExecutionContext) {
        tokio::time::timeout(Duration::from_secs(2), PlaySound.run(context))
            .await
            .expect("test frontend replies immediately")
            .unwrap();
    }

    #[cfg(feature = "execute")]
    #[tokio::test]
    async fn node_waits_for_finished_acknowledgement_and_retains_signed_url() {
        let mut context = context(Some(
            json!({"ok":true,"value":{"status":"finished","durationSeconds":5.25}}),
        ))
        .await;
        run(&mut context).await;
        assert!(context.evaluate_pin::<bool>("exec_out").await.unwrap());
        assert!(!context.evaluate_pin::<bool>("exec_error").await.unwrap());
        assert_eq!(
            context
                .evaluate_pin::<f64>("duration_seconds")
                .await
                .unwrap(),
            5.25
        );
    }

    #[cfg(feature = "execute")]
    #[tokio::test]
    async fn node_routes_playback_errors_and_rejects_started_acknowledgements() {
        for (reply, code) in [
            (
                json!({"ok":false,"error":{"code":"playback_failed","message":"Cannot decode sound"}}),
                "playback_failed",
            ),
            (
                json!({"ok":true,"value":{"status":"started","durationSeconds":5.25}}),
                "invalid_response",
            ),
            (
                json!({"ok":true,"value":{"status":"finished","durationSeconds":-1}}),
                "invalid_response",
            ),
        ] {
            let mut context = context(Some(reply)).await;
            run(&mut context).await;
            assert!(context.evaluate_pin::<bool>("exec_error").await.unwrap());
            assert_eq!(
                context.evaluate_pin::<Value>("error").await.unwrap()["code"],
                code
            );
        }
    }

    #[cfg(feature = "execute")]
    #[tokio::test]
    async fn no_frontend_clears_stale_duration_and_output_overrides() {
        let mut context = context(None).await;
        context
            .set_pin_value("duration_seconds", json!(42.0))
            .await
            .unwrap();
        let pin = context.get_pin_by_name("duration_seconds").await.unwrap();
        context.override_pin_value(pin.id(), json!(42.0));
        context.activate_exec_pin("exec_out").await.unwrap();
        run(&mut context).await;
        assert!(pin.get_raw_value().await.is_none());
        assert!(
            !context
                .context_pin_overrides
                .as_ref()
                .is_some_and(|pins| pins.contains_key(pin.id()))
        );
        assert!(!context.evaluate_pin::<bool>("exec_out").await.unwrap());
        assert_eq!(
            context.evaluate_pin::<Value>("error").await.unwrap()["code"],
            "frontend_unavailable"
        );
    }

    #[cfg(feature = "execute")]
    #[tokio::test]
    async fn invalid_controls_or_ambiguous_source_do_not_request_playback() {
        for (pin, value) in [
            ("volume", json!(-0.1)),
            ("volume", json!(1.1)),
            ("timeout_seconds", json!(0)),
            ("timeout_seconds", json!(601)),
            ("audio_url", json!("")),
            (
                "audio_path",
                json!(FlowPath::new("sound.wav".into(), "temporary".into(), None)),
            ),
        ] {
            let mut context = context(None).await;
            context.set_pin_value(pin, value).await.unwrap();
            run(&mut context).await;
            assert_eq!(
                context.evaluate_pin::<Value>("error").await.unwrap()["code"],
                "invalid_arguments"
            );
        }
    }
}
