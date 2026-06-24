use super::*;

#[crate::register_node]
#[derive(Default)]
pub struct AnalyzeAudioNode;

impl AnalyzeAudioNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for AnalyzeAudioNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "video_analyze_audio",
            "Analyze Audio",
            "Decode audio and report waveform, peak/RMS, and silence ranges",
            "Audio",
        );
        add_video_icon_and_scores(&mut node);
        add_exec_pins(&mut node);
        add_flow_path_input(&mut node, "source", "Source", "Source audio/media FlowPath");
        node.add_input_pin(
            "waveform_buckets",
            "Waveform Buckets",
            "Number of waveform buckets",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(64)));
        node.add_input_pin(
            "silence_threshold_db",
            "Silence dB",
            "RMS threshold in dB",
            VariableType::Float,
        )
        .set_default_value(Some(json!(-60.0)));
        node.add_input_pin(
            "window_ms",
            "Window ms",
            "Silence analysis window",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(20)));
        node.add_input_pin(
            "min_silence_ms",
            "Min Silence ms",
            "Minimum silence duration",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(500)));
        node.add_output_pin(
            "report",
            "Report",
            "Audio analysis report",
            VariableType::Struct,
        )
        .set_schema::<AudioAnalysisReport>();
        node.add_output_pin(
            "waveform",
            "Waveform",
            "Waveform buckets",
            VariableType::Struct,
        )
        .set_schema::<WaveformBucketInfo>()
        .set_value_type(ValueType::Array);
        node.add_output_pin(
            "silence",
            "Silence",
            "Detected silence ranges",
            VariableType::Struct,
        )
        .set_schema::<SilenceRangeInfo>()
        .set_value_type(ValueType::Array);
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        let source: FlowPath = context.evaluate_pin("source").await?;
        let waveform_buckets: i64 = context.evaluate_pin("waveform_buckets").await?;
        let silence_threshold_db: f64 = context.evaluate_pin("silence_threshold_db").await?;
        let window_ms: i64 = context.evaluate_pin("window_ms").await?;
        let min_silence_ms: i64 = context.evaluate_pin("min_silence_ms").await?;
        let (store, location) = flow_path_object(context, &source).await?;
        let frames = decode_audio_file(store.as_ref(), &location).await?;
        let decoded_frames = frames.len();
        let audio = concat_audio_frames(&frames)?;
        let waveform = video_utils_rs::waveform_peaks(&audio, waveform_buckets.max(1) as usize)?
            .into_iter()
            .map(|bucket| waveform_bucket_info(bucket, audio.sample_rate))
            .collect::<Vec<_>>();
        let window_samples = ((window_ms.max(1) as f64 / 1000.0) * audio.sample_rate as f64)
            .round()
            .max(1.0) as usize;
        let min_duration_samples = ((min_silence_ms.max(1) as f64 / 1000.0)
            * audio.sample_rate as f64)
            .round()
            .max(1.0) as usize;
        let silence = video_utils_rs::detect_silence(
            &audio,
            silence_threshold_db as f32,
            window_samples,
            min_duration_samples,
        )?
        .into_iter()
        .map(|range| silence_range_info(range, audio.sample_rate))
        .collect::<Vec<_>>();
        let report = AudioAnalysisReport {
            duration_seconds: audio.duration_seconds(),
            sample_rate: audio.sample_rate,
            channels: audio.channels,
            decoded_frames,
            sample_frames: audio.sample_frames(),
            peak_amplitude: audio.peak_amplitude(),
            rms: audio.rms(),
            waveform_buckets: waveform.len(),
            silence_ranges: silence.len(),
        };

        context.set_pin_value("report", json!(report)).await?;
        context.set_pin_value("waveform", json!(waveform)).await?;
        context.set_pin_value("silence", json!(silence)).await?;
        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(execute_feature_error())
    }
}
