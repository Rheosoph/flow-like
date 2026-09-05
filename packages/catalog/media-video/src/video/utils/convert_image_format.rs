use super::*;

#[crate::register_node]
#[derive(Default)]
pub struct ConvertImageFormatNode;

impl ConvertImageFormatNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for ConvertImageFormatNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "video_convert_image_format",
            "Convert Image Format",
            "Decode a still image and write it as PNG, JPEG, GIF, WebP, or AVIF",
            "Image",
        );
        node.set_flowscript_name("image", "convertFormat");
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
        add_flow_path_output(&mut node, "result", "Result", "Written image FlowPath");
        node.add_output_pin(
            "report",
            "Report",
            "Image conversion report",
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
        let (source_store, source_location) = flow_path_object(context, &source).await?;
        let (target_store, target_location) = flow_path_object(context, &target).await?;
        let bytes =
            video_utils_rs::read_object_bytes(source_store.as_ref(), &source_location).await?;
        let mut decoder = video_utils_rs::ImageRgbaDecoder::new();
        let frame = decoder.decode(&bytes)?;
        let output_format = image_format_for_target(&format, &target_location)?;
        let mut encoder = image_encoder(output_format);
        let output = encoder.encode(&frame)?;
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
            output_width: frame.width,
            output_height: frame.height,
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
