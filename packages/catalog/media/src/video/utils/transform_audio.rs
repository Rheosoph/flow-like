use super::*;

#[crate::register_node]
#[derive(Default)]
pub struct TransformAudioNode;

impl TransformAudioNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for TransformAudioNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "video_transform_audio",
            "Transform Audio",
            "Decode audio, apply gain/normalization/fades, and write WAV PCM output",
            "Audio",
        );
        node.set_flowscript_name("audio", "transform");
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
        node.add_input_pin(
            "gain_factor",
            "Gain",
            "Linear gain factor",
            VariableType::Float,
        )
        .set_default_value(Some(json!(1.0)));
        node.add_input_pin(
            "gain_db",
            "Gain dB",
            "Gain in decibels",
            VariableType::Float,
        )
        .set_default_value(Some(json!(0.0)));
        node.add_input_pin(
            "normalize_peak",
            "Normalize Peak",
            "Target peak amplitude, or 0 to skip",
            VariableType::Float,
        )
        .set_default_value(Some(json!(0.0)));
        node.add_input_pin(
            "fade_in_seconds",
            "Fade In",
            "Fade-in seconds",
            VariableType::Float,
        )
        .set_default_value(Some(json!(0.0)));
        node.add_input_pin(
            "fade_out_seconds",
            "Fade Out",
            "Fade-out seconds",
            VariableType::Float,
        )
        .set_default_value(Some(json!(0.0)));
        node.add_input_pin(
            "fade_shape",
            "Fade Shape",
            "linear or equal_power",
            VariableType::String,
        )
        .set_default_value(Some(json!("linear")));
        add_flow_path_output(&mut node, "result", "Result", "Written WAV FlowPath");
        node.add_output_pin(
            "report",
            "Report",
            "Audio transform report",
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
        let gain_factor: f64 = context.evaluate_pin("gain_factor").await?;
        let gain_db: f64 = context.evaluate_pin("gain_db").await?;
        let normalize_peak: f64 = context.evaluate_pin("normalize_peak").await?;
        let fade_in_seconds: f64 = context.evaluate_pin("fade_in_seconds").await?;
        let fade_out_seconds: f64 = context.evaluate_pin("fade_out_seconds").await?;
        let fade_shape_name: String = context.evaluate_pin("fade_shape").await?;
        let (source_store, source_location) = flow_path_object(context, &source).await?;
        let (target_store, target_location) = flow_path_object(context, &target).await?;
        let pipeline = audio_pipeline(
            gain_factor,
            gain_db,
            normalize_peak,
            fade_in_seconds,
            fade_out_seconds,
            &fade_shape_name,
            None,
        )?;
        let mut job = video_utils_rs::ObjectAudioTransformJob::new(pipeline);
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
