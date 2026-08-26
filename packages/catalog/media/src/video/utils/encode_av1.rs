use super::*;

#[crate::register_node]
#[derive(Default)]
pub struct EncodeAv1Node;

impl EncodeAv1Node {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for EncodeAv1Node {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "video_encode_av1",
            "Encode AV1",
            "Decode a selected video stream and encode it to AV1 with the Rust rav1e backend",
            "Video/Transcode",
        );
        node.set_flowscript_name("video", "encodeAv1");
        add_video_icon_and_scores(&mut node);
        add_exec_pins(&mut node);
        add_flow_path_input(&mut node, "source", "Source", "Source media FlowPath");
        add_flow_path_input(&mut node, "target", "Target", "Target AV1 media FlowPath");
        node.add_input_pin(
            "video_track_id",
            "Video Track",
            "Video track id, or 0 for default",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(0)));
        node.add_input_pin(
            "preserve_non_video",
            "Preserve Non-Video",
            "Copy non-video packets when possible",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(true)));
        node.add_input_pin(
            "speed",
            "Speed",
            "rav1e speed preset 0..10",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(10)));
        node.add_input_pin(
            "quantizer",
            "Quantizer",
            "rav1e quantizer 0..255",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(120)));
        node.add_input_pin(
            "max_key_frame_interval",
            "Key Interval",
            "Maximum keyframe interval",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(120)));
        node.add_input_pin(
            "threads",
            "Threads",
            "Worker threads, or 0 for rav1e default",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(0)));
        add_flow_path_output(&mut node, "result", "Result", "Written AV1 media FlowPath");
        node.add_output_pin(
            "report",
            "Report",
            "AV1 encode report",
            VariableType::Struct,
        )
        .set_schema::<VideoTransformReport>();
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        let source: FlowPath = context.evaluate_pin("source").await?;
        let target: FlowPath = context.evaluate_pin("target").await?;
        let video_track_id: i64 = context.evaluate_pin("video_track_id").await?;
        let preserve_non_video: bool = context.evaluate_pin("preserve_non_video").await?;
        let speed: i64 = context.evaluate_pin("speed").await?;
        let quantizer: i64 = context.evaluate_pin("quantizer").await?;
        let max_key_frame_interval: i64 = context.evaluate_pin("max_key_frame_interval").await?;
        let threads: i64 = context.evaluate_pin("threads").await?;
        let (source_store, source_location) = flow_path_object(context, &source).await?;
        let (target_store, target_location) = flow_path_object(context, &target).await?;
        let demuxed = video_utils_rs::demux_object(source_store.as_ref(), &source_location).await?;
        let requested_track = optional_track_id(video_track_id)?;
        let stream = selected_video_stream(&demuxed.media, requested_track)?;
        let track_id = stream.track_id;
        let width = stream
            .width
            .ok_or_else(|| flow_like_types::anyhow!("Source video stream has no width"))?;
        let height = stream
            .height
            .ok_or_else(|| flow_like_types::anyhow!("Source video stream has no height"))?;
        let frame_duration = packet_frame_duration(&demuxed.packets, track_id);
        let options = video_utils_rs::Rav1eAv1EncoderOptions::new()
            .with_speed(u8::try_from(speed.clamp(0, 10))?)
            .with_quantizer(usize::try_from(quantizer.clamp(0, 255))?)
            .with_max_key_frame_interval(u64::try_from(max_key_frame_interval.max(0))?)
            .with_threads(usize::try_from(threads.max(0))?);
        let processed = {
            let mut decoder = platform_video_decoder(stream)?;
            let mut encoder = video_utils_rs::Rav1eAv1Encoder::with_options(
                track_id,
                width,
                height,
                stream.time_base,
                frame_duration,
                options,
            )?;
            let output_codec_config = Some(encoder.codec_config());
            process_video_transform(
                &demuxed,
                track_id,
                preserve_non_video,
                &video_utils_rs::FrameTransformPipeline::new(),
                &mut decoder,
                &mut encoder,
                output_codec_config,
            )?
        };
        let mux_report = video_utils_rs::mux_object(
            target_store.as_ref(),
            &target_location,
            &processed.media,
            &processed.packets,
        )
        .await?;
        let report = VideoTransformReport {
            source: source_location.to_string(),
            target: target_location.to_string(),
            source_format: demuxed.format.as_str().to_owned(),
            target_format: mux_report.target_format.as_str().to_owned(),
            video_track_id: processed.video_track_id,
            input_packets: processed.input_packets,
            decoded_frames: processed.decoded_frames,
            encoded_video_packets: processed.encoded_video_packets,
            copied_packets: processed.copied_packets,
            bytes_written: mux_report.bytes_written,
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
