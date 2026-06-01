use flow_like_catalog_core::{BoundingBox, NodeImage};

use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json};

#[crate::register_node]
#[derive(Default)]
pub struct CropImageNode {}

impl CropImageNode {
    pub fn new() -> Self {
        CropImageNode {}
    }
}

#[async_trait]
impl NodeLogic for CropImageNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new("crop_image", "Crop Image", "Crop Image", "Image/Transform");
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

        node.add_input_pin("bbox", "Box", "Bounding Box", VariableType::Struct)
            .set_schema::<BoundingBox>()
            .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin(
            "use_ref",
            "Use Reference",
            "Use Reference of the image, transforming the original instead of a copy",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(false))); // default false since we typically want to crop the source image multiple times

        // outputs
        node.add_output_pin(
            "exec_out",
            "Output",
            "Done with the Execution",
            VariableType::Execution,
        );

        node.add_output_pin(
            "image_out",
            "Cropped",
            "Cropped Image object",
            VariableType::Struct,
        )
        .set_schema::<NodeImage>();

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        // fetch inputs
        let mut node_img: NodeImage = context.evaluate_pin("image_in").await?;
        let use_ref: bool = context.evaluate_pin("use_ref").await?;
        let bbox: BoundingBox = context.evaluate_pin("bbox").await?;
        if !use_ref {
            node_img = node_img.copy_image(context).await?;
        }
        let img = node_img.get_image(context).await?;

        // crop image
        {
            let mut img_guard = img.lock().await;
            let (x, y, w, h) = bbox.x1y1wh();
            let img_cropped = img_guard.crop_imm(x, y, w, h);
            *img_guard = img_cropped;
        }

        // set outputs
        context.set_pin_value("image_out", json!(node_img)).await?;
        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }
}
