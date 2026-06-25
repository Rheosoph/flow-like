use super::*;

#[crate::register_node]
#[derive(Default)]
pub struct ExtractSubtitleTrackNode;

impl ExtractSubtitleTrackNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for ExtractSubtitleTrackNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "video_extract_subtitle_track",
            "Extract Subtitle Track",
            "Extract a subtitle track to an SRT or WebVTT sidecar",
            "Subtitles",
        );
        node.add_icon("/flow/icons/text.svg");
        add_exec_pins(&mut node);
        add_flow_path_input(&mut node, "source", "Source", "Source media FlowPath");
        add_flow_path_input(&mut node, "target", "Target", "Target sidecar FlowPath");
        node.add_input_pin(
            "format",
            "Format",
            "Output subtitle format",
            VariableType::String,
        )
        .set_options(
            PinOptions::new()
                .set_valid_values(vec!["srt".to_owned(), "webvtt".to_owned()])
                .build(),
        )
        .set_default_value(Some(json!("srt")));
        node.add_input_pin(
            "track_id",
            "Track ID",
            "Subtitle track id; 0 uses first subtitle track",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(0)));
        add_flow_path_output(&mut node, "result", "Result", "Written sidecar FlowPath");
        node.add_output_pin(
            "report",
            "Report",
            "Subtitle extraction report",
            VariableType::Struct,
        )
        .set_schema::<SubtitleTrackExtractReport>();
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        let source: FlowPath = context.evaluate_pin("source").await?;
        let target: FlowPath = context.evaluate_pin("target").await?;
        let format: String = context.evaluate_pin("format").await?;
        let track_id: i64 = context.evaluate_pin("track_id").await?;
        let format = subtitle_format(&format)?;
        let (source_store, source_location) = flow_path_object(context, &source).await?;
        let (target_store, target_location) = flow_path_object(context, &target).await?;
        let subtitle_report = video_utils_rs::extract_subtitle_track_to_sidecar_between_stores(
            source_store.as_ref(),
            &source_location,
            target_store.as_ref(),
            &target_location,
            optional_track_id(track_id)?,
            format,
        )
        .await?;
        let report = SubtitleTrackExtractReport {
            source: source_location.to_string(),
            target: target_location.to_string(),
            subtitle_track_id: subtitle_report.subtitle_track_id,
            event_count: subtitle_report.event_count,
        };

        context.set_pin_value("result", json!(target)).await?;
        context.set_pin_value("report", json!(report)).await?;
        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(execute_feature_error())
    }
}
