use super::*;

#[crate::register_node]
#[derive(Default)]
pub struct AudioToWavNode;

impl AudioToWavNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for AudioToWavNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "video_audio_to_wav",
            "Audio To WAV",
            "Decode an audio/media object and write WAV PCM output",
            "Audio",
        );
        node.set_flowscript_name("audio", "toWav");
        add_video_icon_and_scores(&mut node);
        add_exec_pins(&mut node);
        add_flow_path_input(&mut node, "source", "Source", "Source audio/media FlowPath");
        add_flow_path_input(&mut node, "target", "Target", "Target WAV FlowPath");
        node.add_input_pin(
            "audio_track_id",
            "Audio Track",
            "Audio track id, or 0 for default",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(0)));
        add_flow_path_output(&mut node, "result", "Result", "Written WAV FlowPath");
        node.add_output_pin(
            "report",
            "Report",
            "Audio conversion report",
            VariableType::Struct,
        )
        .set_schema::<AudioTransformReport>();
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        let source: FlowPath = context.evaluate_pin("source").await?;
        let target: FlowPath = context.evaluate_pin("target").await?;
        let audio_track_id: i64 = context.evaluate_pin("audio_track_id").await?;
        let (source_store, source_location) = flow_path_object(context, &source).await?;
        let (target_store, target_location) = flow_path_object(context, &target).await?;
        let mut job = video_utils_rs::ObjectAudioTransformJob::new(
            video_utils_rs::AudioTransformPipeline::new(),
        );
        if let Some(track_id) = optional_track_id(audio_track_id)? {
            job = job.with_audio_track(track_id);
        }
        let report = video_utils_rs::transform_object_audio_file_to_wav_between_stores(
            source_store.as_ref(),
            &source_location,
            target_store.as_ref(),
            &target_location,
            &job,
        )
        .await?;
        let report = audio_transform_report(report);

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
