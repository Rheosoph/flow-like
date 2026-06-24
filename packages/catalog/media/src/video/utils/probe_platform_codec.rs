use super::*;

#[crate::register_node]
#[derive(Default)]
pub struct ProbePlatformCodecNode;

impl ProbePlatformCodecNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for ProbePlatformCodecNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "video_probe_platform_codec",
            "Probe Platform Codec",
            "Check whether the current host can decode or encode a codec through native platform APIs",
            "Diagnostics",
        );
        node.add_icon("/flow/icons/info.svg");
        add_exec_pins(&mut node);
        node.add_input_pin(
            "codec",
            "Codec",
            "Codec id such as h264, h265, av1, aac, or mp3",
            VariableType::String,
        )
        .set_default_value(Some(json!("h264")));
        node.add_input_pin(
            "direction",
            "Direction",
            "decode or encode",
            VariableType::String,
        )
        .set_default_value(Some(json!("decode")));
        node.add_output_pin(
            "probe",
            "Probe",
            "Platform codec probe result",
            VariableType::Struct,
        )
        .set_schema::<PlatformCodecProbeInfo>();
        node.add_output_pin(
            "supported",
            "Supported",
            "True when a matching native backend is available",
            VariableType::Boolean,
        );
        node.add_output_pin(
            "backend",
            "Backend",
            "Selected backend name when available",
            VariableType::String,
        );
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        let codec: String = context.evaluate_pin("codec").await?;
        let direction: String = context.evaluate_pin("direction").await?;
        let direction = codec_direction(&direction)?;
        let probe = platform_probe_info(video_utils_rs::probe_platform_codec(
            &codec_id(&codec),
            direction,
        ));

        context
            .set_pin_value("supported", json!(probe.supported))
            .await?;
        context
            .set_pin_value("backend", json!(probe.backend.clone().unwrap_or_default()))
            .await?;
        context.set_pin_value("probe", json!(probe)).await?;
        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(execute_feature_error())
    }
}
