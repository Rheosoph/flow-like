use super::*;

#[crate::register_node]
#[derive(Default)]
pub struct ExtractThumbnailNode;

impl ExtractThumbnailNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for ExtractThumbnailNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "video_extract_thumbnail",
            "Extract Thumbnail",
            "Decode a video frame and write it as a still image",
            "Video/Preview",
        );
        node.set_flowscript_name("video", "extractThumbnail");
        add_video_icon_and_scores(&mut node);
        add_exec_pins(&mut node);
        add_flow_path_input(&mut node, "source", "Source", "Source media FlowPath");
        add_flow_path_input(&mut node, "target", "Target", "Target image FlowPath");
        node.add_input_pin(
            "frame_index",
            "Frame Index",
            "Decoded frame index to export",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(0)));
        node.add_input_pin(
            "video_track_id",
            "Video Track",
            "Video track id, or 0 for default",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(0)));
        node.add_input_pin(
            "format",
            "Format",
            "Output image format, or auto from target extension",
            VariableType::String,
        )
        .set_default_value(Some(json!("auto")));
        node.add_input_pin(
            "width",
            "Width",
            "Output width, or 0 to keep decoded width",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(0)));
        node.add_input_pin(
            "height",
            "Height",
            "Output height, or 0 to keep decoded height",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(0)));
        add_flow_path_output(&mut node, "result", "Result", "Written image FlowPath");
        node.add_output_pin("report", "Report", "Thumbnail report", VariableType::Struct)
            .set_schema::<VideoFrameExtractionReport>();
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        let source: FlowPath = context.evaluate_pin("source").await?;
        let target: FlowPath = context.evaluate_pin("target").await?;
        let frame_index: i64 = context.evaluate_pin("frame_index").await?;
        let video_track_id: i64 = context.evaluate_pin("video_track_id").await?;
        let format: String = context.evaluate_pin("format").await?;
        let width: i64 = context.evaluate_pin("width").await?;
        let height: i64 = context.evaluate_pin("height").await?;
        let frame_index = usize::try_from(frame_index.max(0))?;
        let (source_store, source_location) = flow_path_object(context, &source).await?;
        let (target_store, target_location) = flow_path_object(context, &target).await?;
        let demuxed = video_utils_rs::demux_object(source_store.as_ref(), &source_location).await?;
        let requested_track = optional_track_id(video_track_id)?;
        let stream = selected_video_stream(&demuxed.media, requested_track)?;
        let selected_track = stream.track_id;
        let output_format = image_format_for_target(&format, &target_location)?;
        let (output, report) = {
            let mut decoder = platform_video_decoder(stream)?;
            let mut decoded_count = 0usize;
            let mut selected = None;
            for packet in demuxed
                .packets
                .iter()
                .filter(|packet| packet.track_id == selected_track)
            {
                for frame in decoder.decode_packet(packet)? {
                    if decoded_count == frame_index {
                        selected = Some(frame);
                        break;
                    }
                    decoded_count += 1;
                }
                if selected.is_some() {
                    break;
                }
            }
            if selected.is_none() {
                for frame in decoder.flush()? {
                    if decoded_count == frame_index {
                        selected = Some(frame);
                        break;
                    }
                    decoded_count += 1;
                }
            }
            let frame = selected.ok_or_else(|| {
                flow_like_types::anyhow!("Frame index {} was not decoded", frame_index)
            })?;
            let input_width = frame.width;
            let input_height = frame.height;
            let output_frame = if width > 0 && height > 0 {
                frame.resize_nearest(u32::try_from(width)?, u32::try_from(height)?)?
            } else {
                frame
            };
            let mut encoder = image_encoder(output_format);
            let output = encoder.encode(&output_frame)?;
            let report = VideoFrameExtractionReport {
                source: source_location.to_string(),
                target: target_location.to_string(),
                video_track_id: selected_track,
                frame_index,
                decoded_frames: decoded_count + 1,
                input_width,
                input_height,
                output_width: output_frame.width,
                output_height: output_frame.height,
                output_format: image_format_name(output_format).to_owned(),
                bytes_written: output.len() as u64,
            };
            (output, report)
        };
        video_utils_rs::write_object_bytes(
            target_store.as_ref(),
            &target_location,
            Bytes::from(output),
        )
        .await?;

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
