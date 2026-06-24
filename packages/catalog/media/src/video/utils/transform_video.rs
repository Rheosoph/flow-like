use super::*;

#[crate::register_node]
#[derive(Default)]
pub struct TransformVideoNode;

impl TransformVideoNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for TransformVideoNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "video_transform_video",
            "Transform Video",
            "Decode video frames, apply frame transforms, encode, and mux the result",
            "Video/Transcode",
        );
        add_video_icon_and_scores(&mut node);
        add_exec_pins(&mut node);
        add_flow_path_input(&mut node, "source", "Source", "Source media FlowPath");
        add_flow_path_input(&mut node, "target", "Target", "Target media FlowPath");
        node.add_input_pin(
            "output_codec",
            "Output Codec",
            "Video codec to encode, such as h264, h265, vp9, or av1",
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
        for (id, name, default) in [
            ("crop_x", "Crop X", 0),
            ("crop_y", "Crop Y", 0),
            ("crop_width", "Crop Width", 0),
            ("crop_height", "Crop Height", 0),
            ("resize_width", "Resize Width", 0),
            ("resize_height", "Resize Height", 0),
            ("rotate_degrees", "Rotate", 0),
            ("blur_radius", "Blur", 0),
        ] {
            node.add_input_pin(id, name, "", VariableType::Integer)
                .set_default_value(Some(json!(default)));
        }
        node.add_input_pin(
            "flip_horizontal",
            "Flip Horizontal",
            "",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(false)));
        node.add_input_pin("flip_vertical", "Flip Vertical", "", VariableType::Boolean)
            .set_default_value(Some(json!(false)));
        node.add_input_pin(
            "brightness",
            "Brightness",
            "-1.0 to 1.0",
            VariableType::Float,
        )
        .set_default_value(Some(json!(0.0)));
        node.add_input_pin(
            "contrast",
            "Contrast",
            "1.0 keeps contrast unchanged",
            VariableType::Float,
        )
        .set_default_value(Some(json!(1.0)));
        node.add_input_pin(
            "saturation",
            "Saturation",
            "1.0 keeps saturation unchanged",
            VariableType::Float,
        )
        .set_default_value(Some(json!(1.0)));
        add_flow_path_output(&mut node, "result", "Result", "Written media FlowPath");
        node.add_output_pin(
            "report",
            "Report",
            "Video transform report",
            VariableType::Struct,
        )
        .set_schema::<VideoTransformReport>();
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
        let preserve_non_video: bool = context.evaluate_pin("preserve_non_video").await?;
        let bitrate: i64 = context.evaluate_pin("bitrate").await?;
        let crop_x: i64 = context.evaluate_pin("crop_x").await?;
        let crop_y: i64 = context.evaluate_pin("crop_y").await?;
        let crop_width: i64 = context.evaluate_pin("crop_width").await?;
        let crop_height: i64 = context.evaluate_pin("crop_height").await?;
        let resize_width: i64 = context.evaluate_pin("resize_width").await?;
        let resize_height: i64 = context.evaluate_pin("resize_height").await?;
        let flip_horizontal: bool = context.evaluate_pin("flip_horizontal").await?;
        let flip_vertical: bool = context.evaluate_pin("flip_vertical").await?;
        let rotate_degrees: i64 = context.evaluate_pin("rotate_degrees").await?;
        let blur_radius: i64 = context.evaluate_pin("blur_radius").await?;
        let brightness: f64 = context.evaluate_pin("brightness").await?;
        let contrast: f64 = context.evaluate_pin("contrast").await?;
        let saturation: f64 = context.evaluate_pin("saturation").await?;

        let (source_store, source_location) = flow_path_object(context, &source).await?;
        let (target_store, target_location) = flow_path_object(context, &target).await?;
        let demuxed = video_utils_rs::demux_object(source_store.as_ref(), &source_location).await?;
        let requested_track = optional_track_id(video_track_id)?;
        let stream = selected_video_stream(&demuxed.media, requested_track)?;
        let track_id = stream.track_id;
        let mut width = if resize_width > 0 {
            u32::try_from(resize_width)?
        } else if crop_width > 0 {
            u32::try_from(crop_width)?
        } else {
            stream
                .width
                .ok_or_else(|| flow_like_types::anyhow!("Source video stream has no width"))?
        };
        let mut height = if resize_height > 0 {
            u32::try_from(resize_height)?
        } else if crop_height > 0 {
            u32::try_from(crop_height)?
        } else {
            stream
                .height
                .ok_or_else(|| flow_like_types::anyhow!("Source video stream has no height"))?
        };
        if matches!(rotate_degrees.rem_euclid(360), 90 | 270) {
            std::mem::swap(&mut width, &mut height);
        }
        let frame_duration = packet_frame_duration(&demuxed.packets, track_id);
        let pipeline = video_frame_pipeline(
            crop_x,
            crop_y,
            crop_width,
            crop_height,
            resize_width,
            resize_height,
            flip_horizontal,
            flip_vertical,
            rotate_degrees,
            blur_radius,
            brightness,
            contrast,
            saturation,
        )?;
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
                &pipeline,
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
        context
            .set_pin_value("bytes_written", json!(report.bytes_written as i64))
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
