use super::*;

#[crate::register_node]
#[derive(Default)]
pub struct ParseSubtitlesNode;

impl ParseSubtitlesNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for ParseSubtitlesNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "video_parse_subtitles",
            "Parse Subtitles",
            "Parse SRT or WebVTT sidecar subtitles into cue structs",
            "Subtitles",
        );
        node.set_flowscript_name("video", "parseSubtitles");
        node.add_icon("/flow/icons/text.svg");
        add_exec_pins(&mut node);
        add_flow_path_input(&mut node, "sidecar", "Sidecar", "Subtitle sidecar FlowPath");
        node.add_input_pin("format", "Format", "Subtitle format", VariableType::String)
            .set_options(
                PinOptions::new()
                    .set_valid_values(vec!["srt".to_owned(), "webvtt".to_owned()])
                    .build(),
            )
            .set_default_value(Some(json!("srt")));
        node.add_output_pin("cues", "Cues", "Parsed subtitle cues", VariableType::Struct)
            .set_schema::<SubtitleCue>()
            .set_value_type(ValueType::Array);
        node.add_output_pin("count", "Count", "Cue count", VariableType::Integer);
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        let sidecar: FlowPath = context.evaluate_pin("sidecar").await?;
        let format: String = context.evaluate_pin("format").await?;
        let format = subtitle_format(&format)?;
        let text = String::from_utf8(sidecar.get(context, false).await?)?;
        let events = video_utils_rs::parse_subtitles(format, &text)?;
        let cues = cues_from_events(&events);

        context
            .set_pin_value("count", json!(cues.len() as i64))
            .await?;
        context.set_pin_value("cues", json!(cues)).await?;
        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(execute_feature_error())
    }
}
