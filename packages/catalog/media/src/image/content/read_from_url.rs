use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    variable::VariableType,
};
use flow_like_catalog_core::NodeImage;
use flow_like_types::{
    Ok, async_trait,
    image::{DynamicImage, ImageDecoder, ImageReader},
    json::json,
};
use std::io::Cursor;

#[crate::register_node]
#[derive(Default)]
pub struct ReadImageFromUrlNode {}

impl ReadImageFromUrlNode {
    pub fn new() -> Self {
        ReadImageFromUrlNode {}
    }
}

#[async_trait]
impl NodeLogic for ReadImageFromUrlNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "read_image_url",
            "Read Image (URL)",
            "Read image from path",
            "Image/Content",
        );
        node.set_flowscript_name("image", "readUrl");
        node.set_version(1);
        node.add_icon("/flow/icons/image.svg");

        node.add_input_pin(
            "exec_in",
            "Input",
            "Initiate Execution",
            VariableType::Execution,
        );
        node.add_input_pin(
            "signed_url",
            "Signed Url",
            "Signed Url",
            VariableType::String,
        );

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
        let signed_url: String = context.evaluate_pin("signed_url").await?;
        let bytes = flow_like_types::reqwest::get(&signed_url)
            .await
            .map_err(flow_like_types::reqwest::Error::without_url)?
            .bytes()
            .await
            .map_err(flow_like_types::reqwest::Error::without_url)?
            .to_vec();
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
