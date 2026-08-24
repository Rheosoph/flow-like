use super::*;

#[crate::register_node]
#[derive(Default)]
pub struct BurnSubtitlesNode;

impl BurnSubtitlesNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for BurnSubtitlesNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "video_burn_subtitles",
            "Burn Subtitles Into Video",
            "Render an SRT/WebVTT sidecar into video frames and mux the result",
            "Subtitles",
        );
        node.set_flowscript_name("video", "burnSubtitles");
        add_video_icon_and_scores(&mut node);
        add_exec_pins(&mut node);
        add_flow_path_input(&mut node, "source", "Source", "Source media FlowPath");
        add_flow_path_input(&mut node, "sidecar", "Sidecar", "Subtitle sidecar FlowPath");
        add_flow_path_input(&mut node, "target", "Target", "Target media FlowPath");
        node.add_input_pin("format", "Format", "srt or webvtt", VariableType::String)
            .set_default_value(Some(json!("srt")));
        node.add_input_pin(
            "output_codec",
            "Output Codec",
            "Video codec to encode",
            VariableType::String,
        )
        .set_default_value(Some(json!("h264")));
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
            "bitrate",
            "Bitrate",
            "Target bitrate in bits per second, or 0 for backend default",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(0)));
        node.add_input_pin(
            "scale",
            "Scale",
            "Subtitle render scale",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(2)));
        node.add_input_pin(
            "margin_bottom",
            "Margin Bottom",
            "Subtitle bottom margin in pixels",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(24)));
        add_flow_path_output(&mut node, "result", "Result", "Written media FlowPath");
        node.add_output_pin(
            "report",
            "Report",
            "Subtitle burn-in report",
            VariableType::Struct,
        )
        .set_schema::<SubtitleBurnInReport>();
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        let source: FlowPath = context.evaluate_pin("source").await?;
        let sidecar: FlowPath = context.evaluate_pin("sidecar").await?;
        let target: FlowPath = context.evaluate_pin("target").await?;
        let format: String = context.evaluate_pin("format").await?;
        let output_codec: String = context.evaluate_pin("output_codec").await?;
        let video_track_id: i64 = context.evaluate_pin("video_track_id").await?;
        let preserve_non_video: bool = context.evaluate_pin("preserve_non_video").await?;
        let bitrate: i64 = context.evaluate_pin("bitrate").await?;
        let scale: i64 = context.evaluate_pin("scale").await?;
        let margin_bottom: i64 = context.evaluate_pin("margin_bottom").await?;
        let (source_store, source_location) = flow_path_object(context, &source).await?;
        let (sidecar_store, sidecar_location) = flow_path_object(context, &sidecar).await?;
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
        let format = subtitle_format(&format)?;
        let sidecar_bytes =
            video_utils_rs::read_object_bytes(sidecar_store.as_ref(), &sidecar_location).await?;
        let sidecar_text = String::from_utf8(sidecar_bytes.to_vec()).map_err(|err| {
            flow_like_types::anyhow!("Subtitle sidecar is not valid UTF-8: {}", err)
        })?;
        let events = video_utils_rs::parse_subtitles(format, &sidecar_text)?;
        let mut style = video_utils_rs::ObjectSubtitleBurnInJob::new(format).style;
        style.scale = u32::try_from(scale.max(1))?;
        style.margin_bottom = u32::try_from(margin_bottom.max(0))?;
        let processed = {
            let mut decoder = platform_video_decoder(stream)?;
            let mut encoder = platform_video_encoder(
                codec_id(&output_codec),
                width,
                height,
                stream.time_base,
                frame_duration,
                bitrate,
            )?;
            let output_codec_config = encoder.codec_config().map(Bytes::from);
            process_subtitle_burn(
                &demuxed,
                track_id,
                preserve_non_video,
                &events,
                &style,
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
        let report = SubtitleBurnInReport {
            source: source_location.to_string(),
            sidecar: sidecar_location.to_string(),
            target: target_location.to_string(),
            source_format: demuxed.format.as_str().to_owned(),
            target_format: mux_report.target_format.as_str().to_owned(),
            video_track_id: processed.video_track_id,
            event_count: events.len(),
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
