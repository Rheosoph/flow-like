use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_types::{
    async_trait,
    image::{self},
    json::json,
    utils::data_url::image_to_data_url,
};

use flow_like_catalog_core::NodeImage;

#[crate::register_node]
#[derive(Default)]
pub struct WriteImageDataUrlNode {}

impl WriteImageDataUrlNode {
    pub fn new() -> Self {
        WriteImageDataUrlNode {}
    }
}

#[async_trait]
impl NodeLogic for WriteImageDataUrlNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "image_write_dataurl",
            "Write Image to Data URL",
            "Writes an image to a data URL",
            "Web/Camera",
        );
        node.set_version(1);

        node.add_icon("/flow/icons/image.svg");

        node.add_input_pin(
            "exec_in",
            "Execute",
            "Initiate the HTTP request",
            VariableType::Execution,
        );
        node.add_input_pin(
            "image",
            "Image",
            "The image to write to a data URL",
            VariableType::Struct,
        )
        .set_schema::<NodeImage>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin(
            "format",
            "Format",
            "The format of the image (e.g., png, jpeg)",
            VariableType::String,
        )
        .set_options(
            PinOptions::new()
                .set_valid_values(vec![
                    "png".to_string(),
                    "jpeg".to_string(),
                    "webp".to_string(),
                ])
                .build(),
        )
        .set_default_value(Some(json!("png")));

        node.add_output_pin(
            "exec_out",
            "",
            "Done with the Execution",
            VariableType::Execution,
        );

        node.add_output_pin(
            "url",
            "Data URL",
            "The data URL of the written image",
            VariableType::String,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        let image: NodeImage = context.evaluate_pin("image").await?;
        let format: String = context.evaluate_pin("format").await?;
        let format: image::ImageFormat = match format.as_str() {
            "png" => image::ImageFormat::Png,
            "jpeg" => image::ImageFormat::Jpeg,
            "webp" => image::ImageFormat::WebP,
            _ => return Err(flow_like_types::anyhow!("Unsupported image format")),
        };
        let image_ref = image.get_image(context).await?;

        let data_url = {
            let image = image_ref.lock().await;
            image_to_data_url(&image, format).await?
        };

        context.set_pin_value("url", json!(data_url)).await?;
        context.activate_exec_pin("exec_out").await?;

        Ok(())
    }
}
