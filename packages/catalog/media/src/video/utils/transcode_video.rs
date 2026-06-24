use super::*;

#[crate::register_node]
#[derive(Default)]
pub struct TranscodeVideoNode;

impl TranscodeVideoNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for TranscodeVideoNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "video_transcode_video",
            "Transcode Video",
            "Packet-copy when allowed or decode/encode a selected video stream into a target container",
            "Video/Transcode",
        );
        add_video_icon_and_scores(&mut node);
        add_exec_pins(&mut node);
        add_flow_path_input(&mut node, "source", "Source", "Source media FlowPath");
        add_flow_path_input(&mut node, "target", "Target", "Target media FlowPath");
        node.add_input_pin(
            "output_codec",
            "Output Codec",
            "Codec to encode, or copy to only packet-copy/remux",
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
            "allow_packet_copy",
            "Allow Packet Copy",
            "Use copy/remux when no encode stage is requested",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(true)));
        node.add_input_pin(
            "preserve_non_video",
            "Preserve Non-Video",
            "Copy compatible non-video packets",
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
        add_flow_path_output(&mut node, "result", "Result", "Written media FlowPath");
        node.add_output_pin(
            "report",
            "Report",
            "Video transcode report",
            VariableType::Struct,
        )
        .set_schema::<VideoTranscodeReport>();
        node.add_output_pin(
            "operation",
            "Operation",
            "Selected operation",
            VariableType::String,
        );
        node.add_output_pin(
            "bytes_written",
            "Bytes Written",
            "Bytes written to the target",
            VariableType::Integer,
        );
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        let source: FlowPath = context.evaluate_pin("source").await?;
        let target: FlowPath = context.evaluate_pin("target").await?;
        let output_codec: String = context.evaluate_pin("output_codec").await?;
        let video_track_id: i64 = context.evaluate_pin("video_track_id").await?;
        let allow_packet_copy: bool = context.evaluate_pin("allow_packet_copy").await?;
        let preserve_non_video: bool = context.evaluate_pin("preserve_non_video").await?;
        let bitrate: i64 = context.evaluate_pin("bitrate").await?;
        let (source_store, source_location) = flow_path_object(context, &source).await?;
        let (target_store, target_location) = flow_path_object(context, &target).await?;
        let requested_track = optional_track_id(video_track_id)?;
        let wants_copy_only = matches!(
            output_codec.trim().to_ascii_lowercase().as_str(),
            "" | "copy" | "packet_copy" | "packet-copy"
        );

        let report = if wants_copy_only {
            if !allow_packet_copy {
                return Err(flow_like_types::anyhow!(
                    "output_codec is copy but allow_packet_copy is false"
                ));
            }
            let report = video_utils_rs::remux_object_between_stores(
                source_store.as_ref(),
                &source_location,
                target_store.as_ref(),
                &target_location,
                None,
            )
            .await?;
            VideoTranscodeReport {
                source: source_location.to_string(),
                target: target_location.to_string(),
                source_format: report.source_format.as_str().to_owned(),
                target_format: report.target_format.as_str().to_owned(),
                operation: format!("{:?}", report.operation),
                video_track_id: None,
                input_packets: 0,
                output_packets: 0,
                decoded_video_frames: 0,
                encoded_video_packets: 0,
                copied_packets: 0,
                dropped_packets: 0,
                bytes_written: report.bytes_written,
            }
        } else {
            let demuxed =
                video_utils_rs::demux_object(source_store.as_ref(), &source_location).await?;
            let stream = selected_video_stream(&demuxed.media, requested_track)?;
            let track_id = stream.track_id;
            let width = stream
                .width
                .ok_or_else(|| flow_like_types::anyhow!("Source video stream has no width"))?;
            let height = stream
                .height
                .ok_or_else(|| flow_like_types::anyhow!("Source video stream has no height"))?;
            let frame_duration = packet_frame_duration(&demuxed.packets, track_id);
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
            VideoTranscodeReport {
                source: source_location.to_string(),
                target: target_location.to_string(),
                source_format: demuxed.format.as_str().to_owned(),
                target_format: mux_report.target_format.as_str().to_owned(),
                operation: "VideoTranscodeMux".to_owned(),
                video_track_id: Some(processed.video_track_id),
                input_packets: processed.input_packets,
                output_packets: processed.packets.len(),
                decoded_video_frames: processed.decoded_frames,
                encoded_video_packets: processed.encoded_video_packets,
                copied_packets: processed.copied_packets,
                dropped_packets: processed.dropped_packets,
                bytes_written: mux_report.bytes_written,
            }
        };

        context.set_pin_value("result", json!(target)).await?;
        context
            .set_pin_value("bytes_written", json!(report.bytes_written as i64))
            .await?;
        context
            .set_pin_value("operation", json!(report.operation.clone()))
            .await?;
        context.set_pin_value("report", json!(report)).await?;
        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(execute_feature_error())
    }
}
