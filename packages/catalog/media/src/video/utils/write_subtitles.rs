use super::*;

#[crate::register_node]
#[derive(Default)]
pub struct WriteSubtitlesNode;

impl WriteSubtitlesNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for WriteSubtitlesNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "video_write_subtitles",
            "Write Subtitles",
            "Write subtitle cue structs to an SRT or WebVTT sidecar",
            "Subtitles",
        );
        node.set_flowscript_name("video", "writeSubtitles");
        node.add_icon("/flow/icons/text.svg");
        add_exec_pins(&mut node);
        node.add_input_pin("cues", "Cues", "Subtitle cues", VariableType::Struct)
            .set_schema::<SubtitleCue>()
            .set_value_type(ValueType::Array);
        add_flow_path_input(&mut node, "target", "Target", "Subtitle sidecar FlowPath");
        node.add_input_pin("format", "Format", "Subtitle format", VariableType::String)
            .set_options(
                PinOptions::new()
                    .set_valid_values(vec!["srt".to_owned(), "webvtt".to_owned()])
                    .build(),
            )
            .set_default_value(Some(json!("srt")));
        add_flow_path_output(&mut node, "result", "Result", "Written sidecar FlowPath");
        node.add_output_pin("count", "Count", "Cue count", VariableType::Integer);
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        let cues: Vec<SubtitleCue> = context.evaluate_pin("cues").await?;
        let target: FlowPath = context.evaluate_pin("target").await?;
        let format: String = context.evaluate_pin("format").await?;
        let format = subtitle_format(&format)?;
        let events = events_from_cues(cues)?;
        let text = subtitle_text(format, &events);
        target.put(context, text.into_bytes(), false).await?;

        context.set_pin_value("result", json!(target)).await?;
        context
            .set_pin_value("count", json!(events.len() as i64))
            .await?;
        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(execute_feature_error())
    }
}
