use super::*;

#[crate::register_node]
#[derive(Default)]
pub struct TransformImageNode;

impl TransformImageNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for TransformImageNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "video_transform_image",
            "Transform Image",
            "Apply crop, resize, flip, rotate, blur, and color filters to a still image",
            "Image",
        );
        add_video_icon_and_scores(&mut node);
        add_exec_pins(&mut node);
        add_flow_path_input(&mut node, "source", "Source", "Source image FlowPath");
        add_flow_path_input(&mut node, "target", "Target", "Target image FlowPath");
        node.add_input_pin(
            "format",
            "Format",
            "Output image format, or auto from target extension",
            VariableType::String,
        )
        .set_default_value(Some(json!("auto")));
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
        add_flow_path_output(&mut node, "result", "Result", "Written image FlowPath");
        node.add_output_pin(
            "report",
            "Report",
            "Image transform report",
            VariableType::Struct,
        )
        .set_schema::<ImageOperationReport>();
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        let source: FlowPath = context.evaluate_pin("source").await?;
        let target: FlowPath = context.evaluate_pin("target").await?;
        let format: String = context.evaluate_pin("format").await?;
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
        let bytes =
            video_utils_rs::read_object_bytes(source_store.as_ref(), &source_location).await?;
        let mut decoder = video_utils_rs::ImageRgbaDecoder::new();
        let frame = decoder.decode(&bytes)?;
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
        let transformed = pipeline.apply(&frame)?;
        let output_format = image_format_for_target(&format, &target_location)?;
        let mut encoder = image_encoder(output_format);
        let output = encoder.encode(&transformed)?;
        let bytes_written = output.len() as u64;
        video_utils_rs::write_object_bytes(
            target_store.as_ref(),
            &target_location,
            Bytes::from(output),
        )
        .await?;

        let report = ImageOperationReport {
            source: source_location.to_string(),
            target: target_location.to_string(),
            input_width: frame.width,
            input_height: frame.height,
            output_width: transformed.width,
            output_height: transformed.height,
            output_format: image_format_name(output_format).to_owned(),
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
