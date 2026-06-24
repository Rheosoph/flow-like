use super::*;

#[crate::register_node]
#[derive(Default)]
pub struct PickCodecBackendNode;

impl PickCodecBackendNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for PickCodecBackendNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "video_pick_codec_backend",
            "Pick Codec Backend",
            "Choose the preferred compiled backend for a codec and operation",
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
            "selection",
            "Selection",
            "Preferred backend selection",
            VariableType::Struct,
        )
        .set_schema::<BackendSelectionInfo>();
        node.add_output_pin(
            "backend",
            "Backend",
            "Preferred backend name",
            VariableType::String,
        );
        node.add_output_pin(
            "found",
            "Found",
            "True when a preferred backend exists",
            VariableType::Boolean,
        );
        node.add_output_pin(
            "support",
            "Support",
            "Compiled codec support registry",
            VariableType::Struct,
        )
        .set_schema::<CodecSupportInfo>()
        .set_value_type(ValueType::Array);
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        let codec: String = context.evaluate_pin("codec").await?;
        let direction: String = context.evaluate_pin("direction").await?;
        let codec = codec_id(&codec);
        let direction = codec_direction(&direction)?;
        let backend = video_utils_rs::preferred_backend_for_codec(&codec, direction)
            .map(|backend| format!("{backend:?}"));
        let selection = BackendSelectionInfo {
            codec: codec.to_string(),
            direction: format!("{direction:?}").to_ascii_lowercase(),
            found: backend.is_some(),
            backend: backend.clone(),
        };

        context
            .set_pin_value("found", json!(selection.found))
            .await?;
        context
            .set_pin_value("backend", json!(backend.unwrap_or_default()))
            .await?;
        context.set_pin_value("selection", json!(selection)).await?;
        context
            .set_pin_value("support", json!(codec_support_info()))
            .await?;
        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(execute_feature_error())
    }
}
