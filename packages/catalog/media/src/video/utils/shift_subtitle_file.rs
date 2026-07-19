use super::*;

#[crate::register_node]
#[derive(Default)]
pub struct ShiftSubtitleFileNode;

impl ShiftSubtitleFileNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for ShiftSubtitleFileNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "video_shift_subtitle_file",
            "Shift Subtitle File",
            "Offset all SRT or WebVTT cues and write a new sidecar",
            "Subtitles",
        );
        node.add_icon("/flow/icons/text.svg");
        add_exec_pins(&mut node);
        add_flow_path_input(&mut node, "source", "Source", "Subtitle sidecar FlowPath");
        add_flow_path_input(&mut node, "target", "Target", "Target sidecar FlowPath");
        node.add_input_pin("format", "Format", "Subtitle format", VariableType::String)
            .set_options(
                PinOptions::new()
                    .set_valid_values(vec!["srt".to_owned(), "webvtt".to_owned()])
                    .build(),
            )
            .set_default_value(Some(json!("srt")));
        node.add_input_pin(
            "offset_ms",
            "Offset MS",
            "Positive or negative subtitle offset in milliseconds",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(0)));
        add_flow_path_output(&mut node, "result", "Result", "Written sidecar FlowPath");
        node.add_output_pin("count", "Count", "Shifted cue count", VariableType::Integer);
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        let source: FlowPath = context.evaluate_pin("source").await?;
        let target: FlowPath = context.evaluate_pin("target").await?;
        let format: String = context.evaluate_pin("format").await?;
        let offset_ms: i64 = context.evaluate_pin("offset_ms").await?;
        let format = subtitle_format(&format)?;
        let text = String::from_utf8(source.get(context, false).await?)?;
        let events = video_utils_rs::parse_subtitles(format, &text)?;
        let shifted = video_utils_rs::shift_events(&events, offset_ms);
        let output = subtitle_text(format, &shifted);
        target.put(context, output.into_bytes(), false).await?;

        context.set_pin_value("result", json!(target)).await?;
        context
            .set_pin_value("count", json!(shifted.len() as i64))
            .await?;
        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(execute_feature_error())
    }
}
