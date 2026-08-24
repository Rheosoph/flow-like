use super::*;

#[crate::register_node]
#[derive(Default)]
pub struct AddSubtitleTrackNode;

impl AddSubtitleTrackNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for AddSubtitleTrackNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "video_add_subtitle_track",
            "Add Subtitle Track",
            "Mux an SRT or WebVTT sidecar into a Matroska subtitle track",
            "Subtitles",
        );
        node.set_flowscript_name("video", "addSubtitleTrack");
        node.add_icon("/flow/icons/text.svg");
        add_exec_pins(&mut node);
        add_flow_path_input(&mut node, "source", "Source", "Source media FlowPath");
        add_flow_path_input(&mut node, "sidecar", "Sidecar", "Subtitle sidecar FlowPath");
        add_flow_path_input(&mut node, "target", "Target", "Target Matroska FlowPath");
        node.add_input_pin(
            "format",
            "Format",
            "Subtitle sidecar format",
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
            "Subtitle track id to create",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(100)));
        node.add_input_pin(
            "language",
            "Language",
            "Optional language tag",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));
        add_flow_path_output(&mut node, "result", "Result", "Written media FlowPath");
        node.add_output_pin(
            "report",
            "Report",
            "Subtitle mux report",
            VariableType::Struct,
        )
        .set_schema::<SubtitleTrackMuxReport>();
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        let source: FlowPath = context.evaluate_pin("source").await?;
        let sidecar: FlowPath = context.evaluate_pin("sidecar").await?;
        let target: FlowPath = context.evaluate_pin("target").await?;
        let format: String = context.evaluate_pin("format").await?;
        let track_id: i64 = context.evaluate_pin("track_id").await?;
        let language: String = context.evaluate_pin("language").await?;
        let format = subtitle_format(&format)?;
        let track_id = u32::try_from(track_id)?;
        let (source_store, source_location) = flow_path_object(context, &source).await?;
        let (sidecar_store, sidecar_location) = flow_path_object(context, &sidecar).await?;
        let (target_store, target_location) = flow_path_object(context, &target).await?;
        let mut job = video_utils_rs::ObjectSubtitleTrackJob::new(track_id, format);
        if let Some(language) = clean_optional(language) {
            job = job.with_language(language);
        }
        let subtitle_report = video_utils_rs::add_subtitle_sidecar_to_object_between_stores(
            source_store.as_ref(),
            &source_location,
            sidecar_store.as_ref(),
            &sidecar_location,
            target_store.as_ref(),
            &target_location,
            &job,
        )
        .await?;
        let report = SubtitleTrackMuxReport {
            source: source_location.to_string(),
            sidecar: sidecar_location.to_string(),
            target: target_location.to_string(),
            event_count: subtitle_report.event_count,
            subtitle_packets: subtitle_report.subtitle_packets,
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
