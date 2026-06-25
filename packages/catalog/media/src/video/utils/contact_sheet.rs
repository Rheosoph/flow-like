use super::*;

#[crate::register_node]
#[derive(Default)]
pub struct ContactSheetNode;

impl ContactSheetNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for ContactSheetNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "video_contact_sheet",
            "Contact Sheet",
            "Sample decoded frames and write a preview grid image",
            "Video/Preview",
        );
        add_video_icon_and_scores(&mut node);
        add_exec_pins(&mut node);
        add_flow_path_input(&mut node, "source", "Source", "Source media FlowPath");
        add_flow_path_input(&mut node, "target", "Target", "Target image FlowPath");
        node.add_input_pin(
            "max_frames",
            "Max Frames",
            "Maximum frames in the sheet",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(12)));
        node.add_input_pin(
            "every_n_frames",
            "Every N Frames",
            "Sampling interval in decoded frames",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(30)));
        node.add_input_pin(
            "columns",
            "Columns",
            "Grid column count",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(4)));
        node.add_input_pin(
            "cell_width",
            "Cell Width",
            "Cell width in pixels",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(160)));
        node.add_input_pin(
            "cell_height",
            "Cell Height",
            "Cell height in pixels",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(90)));
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
        add_flow_path_output(
            &mut node,
            "result",
            "Result",
            "Written contact sheet FlowPath",
        );
        node.add_output_pin(
            "report",
            "Report",
            "Contact sheet image report",
            VariableType::Struct,
        )
        .set_schema::<ContactSheetReport>();
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        let source: FlowPath = context.evaluate_pin("source").await?;
        let target: FlowPath = context.evaluate_pin("target").await?;
        let max_frames: i64 = context.evaluate_pin("max_frames").await?;
        let every_n_frames: i64 = context.evaluate_pin("every_n_frames").await?;
        let columns: i64 = context.evaluate_pin("columns").await?;
        let cell_width: i64 = context.evaluate_pin("cell_width").await?;
        let cell_height: i64 = context.evaluate_pin("cell_height").await?;
        let video_track_id: i64 = context.evaluate_pin("video_track_id").await?;
        let format: String = context.evaluate_pin("format").await?;
        let max_frames = usize::try_from(max_frames.max(1))?;
        let every_n_frames = usize::try_from(every_n_frames.max(1))?;
        let columns = u32::try_from(columns.max(1))?;
        let cell_width = u32::try_from(cell_width.max(1))?;
        let cell_height = u32::try_from(cell_height.max(1))?;
        let (source_store, source_location) = flow_path_object(context, &source).await?;
        let (target_store, target_location) = flow_path_object(context, &target).await?;
        let demuxed = video_utils_rs::demux_object(source_store.as_ref(), &source_location).await?;
        let requested_track = optional_track_id(video_track_id)?;
        let stream = selected_video_stream(&demuxed.media, requested_track)?;
        let selected_track = stream.track_id;
        let input_width = stream.width.unwrap_or(0);
        let input_height = stream.height.unwrap_or(0);
        let output_format = image_format_for_target(&format, &target_location)?;
        let (output, report) = {
            let mut decoder = platform_video_decoder(stream)?;
            let mut decoded_index = 0usize;
            let mut frames = Vec::new();
            for packet in demuxed
                .packets
                .iter()
                .filter(|packet| packet.track_id == selected_track)
            {
                for frame in decoder.decode_packet(packet)? {
                    if decoded_index.is_multiple_of(every_n_frames) {
                        frames.push(frame);
                        if frames.len() >= max_frames {
                            break;
                        }
                    }
                    decoded_index += 1;
                }
                if frames.len() >= max_frames {
                    break;
                }
            }
            if frames.len() < max_frames {
                for frame in decoder.flush()? {
                    if decoded_index.is_multiple_of(every_n_frames) {
                        frames.push(frame);
                        if frames.len() >= max_frames {
                            break;
                        }
                    }
                    decoded_index += 1;
                }
            }
            if frames.is_empty() {
                return Err(flow_like_types::anyhow!("No video frames were decoded"));
            }

            let rows = ((frames.len() as u32) + columns - 1) / columns;
            let mut sheet = video_utils_rs::RgbaFrame::solid(
                columns * cell_width,
                rows * cell_height,
                [0, 0, 0, 255],
            );
            for (index, frame) in frames.iter().enumerate() {
                let resized = frame.resize_nearest(cell_width, cell_height)?;
                let x = (index as u32 % columns) * cell_width;
                let y = (index as u32 / columns) * cell_height;
                sheet.overlay(&resized, x as i32, y as i32);
            }
            let mut encoder = image_encoder(output_format);
            let output = encoder.encode(&sheet)?;
            let report = ContactSheetReport {
                source: source_location.to_string(),
                target: target_location.to_string(),
                video_track_id: selected_track,
                frame_count: frames.len(),
                input_width,
                input_height,
                output_width: sheet.width,
                output_height: sheet.height,
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
