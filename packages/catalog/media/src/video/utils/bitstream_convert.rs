use super::*;

#[crate::register_node]
#[derive(Default)]
pub struct BitstreamConvertNode;

impl BitstreamConvertNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for BitstreamConvertNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "video_bitstream_convert",
            "Bitstream Convert",
            "Convert H.264/H.265/AAC packet bitstream framing into an elementary output file",
            "Video/Packets",
        );
        add_video_icon_and_scores(&mut node);
        add_exec_pins(&mut node);
        add_flow_path_input(&mut node, "source", "Source", "Source media FlowPath");
        add_flow_path_input(&mut node, "target", "Target", "Target elementary FlowPath");
        node.add_input_pin("conversion", "Conversion", "h264_annex_b, h264_length_prefixed, h265_annex_b, h265_length_prefixed, aac_adts, or aac_raw", VariableType::String)
            .set_default_value(Some(json!("h264_annex_b")));
        node.add_input_pin(
            "track_id",
            "Track",
            "Track id, or 0 to select by conversion codec",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(0)));
        add_flow_path_output(&mut node, "result", "Result", "Written elementary FlowPath");
        node.add_output_pin(
            "report",
            "Report",
            "Bitstream conversion report",
            VariableType::Struct,
        )
        .set_schema::<BitstreamConvertReport>();
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        let source: FlowPath = context.evaluate_pin("source").await?;
        let target: FlowPath = context.evaluate_pin("target").await?;
        let conversion: String = context.evaluate_pin("conversion").await?;
        let track_id: i64 = context.evaluate_pin("track_id").await?;
        let conversion = conversion.trim().to_ascii_lowercase();
        let expected_codec = match conversion.as_str() {
            "h264_annex_b" | "h264_length_prefixed" => video_utils_rs::CodecId::H264,
            "h265_annex_b" | "h265_length_prefixed" => video_utils_rs::CodecId::H265,
            "aac_adts" | "aac_raw" => video_utils_rs::CodecId::Aac,
            other => {
                return Err(flow_like_types::anyhow!(
                    "Unsupported bitstream conversion: {}",
                    other
                ));
            }
        };
        let (source_store, source_location) = flow_path_object(context, &source).await?;
        let (target_store, target_location) = flow_path_object(context, &target).await?;
        let demuxed = video_utils_rs::demux_object(source_store.as_ref(), &source_location).await?;
        let selected_track = if let Some(track_id) = optional_track_id(track_id)? {
            track_id
        } else {
            demuxed
                .media
                .streams
                .iter()
                .find(|stream| stream.codec == expected_codec)
                .map(|stream| stream.track_id)
                .ok_or_else(|| flow_like_types::anyhow!("No {} track found", expected_codec))?
        };
        let stream = demuxed
            .media
            .stream(selected_track)
            .ok_or_else(|| flow_like_types::anyhow!("Track {} is missing", selected_track))?;
        if stream.codec != expected_codec {
            return Err(flow_like_types::anyhow!(
                "Track {} has codec {}, expected {}",
                selected_track,
                stream.codec,
                expected_codec
            ));
        }

        let mut output = Vec::<u8>::new();
        let mut packet_count = 0usize;
        for packet in demuxed
            .packets
            .iter()
            .filter(|packet| packet.track_id == selected_track)
        {
            let converted = match conversion.as_str() {
                "h264_annex_b" => video_utils_rs::bitstream::h264::h264_packet_to_annex_b(
                    packet,
                    stream.codec_config.as_ref(),
                )?,
                "h264_length_prefixed" => {
                    let config = stream.codec_config.as_ref().ok_or_else(|| {
                        flow_like_types::anyhow!(
                            "H.264 length-prefixed conversion requires avcC codec config"
                        )
                    })?;
                    video_utils_rs::bitstream::h264::h264_packet_to_length_prefixed(packet, config)?
                }
                "h265_annex_b" => video_utils_rs::bitstream::h265::h265_packet_to_annex_b(
                    packet,
                    stream.codec_config.as_ref(),
                )?,
                "h265_length_prefixed" => {
                    let config = stream.codec_config.as_ref().ok_or_else(|| {
                        flow_like_types::anyhow!(
                            "H.265 length-prefixed conversion requires hvcC codec config"
                        )
                    })?;
                    video_utils_rs::bitstream::h265::h265_packet_to_length_prefixed(packet, config)?
                }
                "aac_adts" => video_utils_rs::bitstream::aac::aac_packet_to_adts(
                    packet,
                    stream.codec_config.as_ref(),
                )?,
                "aac_raw" => video_utils_rs::bitstream::aac::aac_packet_to_raw(packet)?,
                _ => unreachable!(),
            };
            output.extend_from_slice(&converted);
            packet_count += 1;
        }
        let bytes_written = output.len() as u64;
        video_utils_rs::write_object_bytes(
            target_store.as_ref(),
            &target_location,
            Bytes::from(output),
        )
        .await?;
        let report = BitstreamConvertReport {
            source: source_location.to_string(),
            target: target_location.to_string(),
            codec: expected_codec.to_string(),
            conversion,
            track_id: selected_track,
            packets: packet_count,
            bytes_written,
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
