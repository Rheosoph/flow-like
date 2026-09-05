use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_catalog_core::{FlowPath, NodeImage};
use flow_like_types::{
    Ok, async_trait,
    image::{DynamicImage, ImageDecoder, ImageReader},
    json::json,
};
use std::io::Cursor;

#[crate::register_node]
#[derive(Default)]
pub struct ReadImagePathNode {}

impl ReadImagePathNode {
    pub fn new() -> Self {
        ReadImagePathNode {}
    }
}

#[async_trait]
impl NodeLogic for ReadImagePathNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "read_image",
            "Read Image",
            "Read image from path",
            "Image/Content",
        );
        node.set_flowscript_name("image", "read");
        node.set_version(1);
        node.add_icon("/flow/icons/image.svg");

        node.add_input_pin(
            "exec_in",
            "Input",
            "Initiate Execution",
            VariableType::Execution,
        );
        node.add_input_pin("path", "Path", "FlowPath", VariableType::Struct)
            .set_schema::<FlowPath>()
            .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin(
            "apply_exif",
            "Apply Exif Orientation",
            "Apply Exif Orientation",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(false)));

        node.add_output_pin(
            "exec_out",
            "Output",
            "Done with the Execution",
            VariableType::Execution,
        );
        node.add_output_pin("image_out", "Image", "Image object", VariableType::Struct)
            .set_schema::<NodeImage>();

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        let path: FlowPath = context.evaluate_pin("path").await?;
        let bytes = path.get(context, false).await?;
        let apply_exif: bool = context.evaluate_pin("apply_exif").await?;

        let image = {
            let reader = ImageReader::new(Cursor::new(bytes)).with_guessed_format()?;
            let mut decoder = reader.into_decoder()?;
            let orientation = decoder.orientation()?;
            let mut img = DynamicImage::from_decoder(decoder)?;
            if !apply_exif {
                img.apply_orientation(orientation);
            }
            img
        };

        let node_img = NodeImage::new(context, image).await;
        context.set_pin_value("image_out", json!(node_img)).await?;
        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }
}
