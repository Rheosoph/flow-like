use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_catalog_core::NodeImage;
use flow_like_types::{
    Ok, anyhow, async_trait,
    image::{ColorType, DynamicImage},
    json::json,
};

#[crate::register_node]
#[derive(Default)]
pub struct ConvertImageNode {}

impl ConvertImageNode {
    /// Constructor
    pub fn new() -> Self {
        ConvertImageNode {}
    }
}

#[async_trait]
impl NodeLogic for ConvertImageNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "convert_image",
            "Color Convert",
            "Convert Image Color/Pixel Type (e.g. to Grayscale)",
            "Image/Transform",
        );
        node.set_version(1);
        node.add_icon("/flow/icons/image.svg");

        // inputs
        node.add_input_pin(
            "exec_in",
            "Input",
            "Initiate Execution",
            VariableType::Execution,
        );
        node.add_input_pin("image_in", "Image", "Image object", VariableType::Struct)
            .set_schema::<NodeImage>()
            .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin(
            "pixel_type",
            "Pixel Type",
            "Target Pixel Type",
            VariableType::String,
        )
        .set_options(
            PinOptions::new()
                .set_valid_values(vec![
                    "RGB".to_string(),
                    "RGBA".to_string(),
                    "Luma".to_string(),
                    "LumaA".to_string(),
                ])
                .build(),
        )
        .set_default_value(Some(json!("Luma")));

        node.add_input_pin(
            "use_ref",
            "Use Reference",
            "Use Reference of the image, transforming the original instead of a copy",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(true)));

        // outputs
        node.add_output_pin(
            "exec_out",
            "Output",
            "Done with the Execution",
            VariableType::Execution,
        );
        node.add_output_pin(
            "image_out",
            "Image",
            "Image with Target Color/Pixel Type",
            VariableType::Struct,
        )
        .set_schema::<NodeImage>();

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        // fetch inputs
        let mut node_img: NodeImage = context.evaluate_pin("image_in").await?;
        let pixel_type: String = context.evaluate_pin("pixel_type").await?;
        let use_ref: bool = context.evaluate_pin("use_ref").await?;
        if !use_ref {
            node_img = node_img.copy_image(context).await?;
        }
        let img = node_img.get_image(context).await?;
        let target = match pixel_type.as_str() {
            "RGB" => ColorType::Rgba8,
            "RGBA" => ColorType::Rgba8,
            "Luma" => ColorType::L8,
            "LumaA" => ColorType::La8,
            _ => return Err(anyhow!("Unknown Target Color Type!")),
        };

        // convert color
        {
            let mut img_guard = img.lock().await;
            // determine input and target color type
            let color = img_guard.color();
            // apply color conversion (if different)
            if color != target {
                let img_converted = match target {
                    ColorType::Rgb8 => DynamicImage::ImageRgb8(img_guard.to_rgb8()),
                    ColorType::Rgba8 => DynamicImage::ImageRgba8(img_guard.to_rgba8()),
                    ColorType::L8 => DynamicImage::ImageLuma8(img_guard.to_luma8()),
                    ColorType::La8 => DynamicImage::ImageLumaA8(img_guard.to_luma_alpha8()),
                    _ => return Err(anyhow!("Unknown Color Type!")),
                };
                *img_guard = img_converted;
            }
        }

        // set outputs
        context.set_pin_value("image_out", json!(node_img)).await?;
        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }
}
