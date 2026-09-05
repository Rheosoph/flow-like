use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_catalog_core::NodeImage;
use flow_like_types::{
    Ok, async_trait,
    image::{GenericImageView, imageops::FilterType},
    json::json,
};

#[crate::register_node]
#[derive(Default)]
pub struct ResizeImageNode {}

impl ResizeImageNode {
    pub fn new() -> Self {
        ResizeImageNode {}
    }
}

#[async_trait]
impl NodeLogic for ResizeImageNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "resize_image",
            "Resize Image",
            "Resize Image",
            "Image/Transform",
        );
        node.set_flowscript_name("image", "resize");
        node.set_receiver("image_in");
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
            "use_ref",
            "Use Reference",
            "Use Reference of the image, transforming the original instead of a copy",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(true)));

        node.add_input_pin("mode", "Resize Mode", "Resize Mode", VariableType::String)
            .set_options(
                PinOptions::new()
                    .set_valid_values(vec![
                        "keep_aspect".to_string(),
                        "exact".to_string(),
                        "to_fill".to_string(),
                    ])
                    .build(),
            )
            .set_default_value(Some(json!("keep_aspect")));

        node.add_input_pin(
            "filter",
            "Filter",
            "Resize Filter Algorithm",
            VariableType::String,
        )
        .set_options(
            PinOptions::new()
                .set_valid_values(vec![
                    "Nearest".to_string(),
                    "Triangle".to_string(),
                    "CatmullRom".to_string(),
                    "Gaussian".to_string(),
                    "Lanczos3".to_string(),
                ])
                .build(),
        )
        .set_default_value(Some(json!("Lanczos3")));

        node.add_input_pin(
            "width_in",
            "Width",
            "Resized Image Target Width",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(512)));

        node.add_input_pin(
            "height_in",
            "Height",
            "Resized Image Target Height",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(512)));

        // outputs
        node.add_output_pin(
            "exec_out",
            "Output",
            "Done with the Execution",
            VariableType::Execution,
        );
        node.add_output_pin("image_out", "Image", "Image object", VariableType::Struct)
            .set_schema::<NodeImage>();

        node.add_output_pin(
            "width_out",
            "Width",
            "Resized Image Result Width",
            VariableType::Integer,
        );
        node.add_output_pin(
            "height_out",
            "Height",
            "Resized Image Result Height",
            VariableType::Integer,
        );
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let mut node_img: NodeImage = context.evaluate_pin("image_in").await?;
        let use_ref: bool = context.evaluate_pin("use_ref").await?;

        if !use_ref {
            node_img = node_img.copy_image(context).await?;
        }

        let target_width: u32 = context.evaluate_pin("width_in").await?;
        let target_height: u32 = context.evaluate_pin("height_in").await?;
        let mode: String = context.evaluate_pin("mode").await?;
        let filter_in: String = context.evaluate_pin("filter").await?;
        let filter = {
            match filter_in.as_str() {
                "Nearest" => Ok(FilterType::Nearest),
                "Triangle" => Ok(FilterType::Triangle),
                "CatmullRom" => Ok(FilterType::CatmullRom),
                "Gaussian" => Ok(FilterType::Gaussian),
                "Lanczos3" => Ok(FilterType::Lanczos3),
                _ => Ok(FilterType::Lanczos3),
            }
        }?;

        let img = node_img.get_image(context).await?;

        let (result_width, result_height) = {
            let mut img_guard = img.lock().await;

            let resized_img = match mode.as_str() {
                "exact" => img_guard.resize_exact(target_width, target_height, filter),
                "to_fill" => img_guard.resize_to_fill(target_width, target_height, filter),
                _ => img_guard.resize(target_width, target_height, filter),
            };

            *img_guard = resized_img;

            let (result_width, result_height) = img_guard.dimensions();
            (result_width, result_height)
        };

        context.set_pin_value("image_out", json!(node_img)).await?;
        context
            .set_pin_value("width_out", json!(result_width))
            .await?;
        context
            .set_pin_value("height_out", json!(result_height))
            .await?;
        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }
}
