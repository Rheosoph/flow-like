use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_catalog_core::NodeImage;
use flow_like_types::{Ok, async_trait, json::json};

#[crate::register_node]
#[derive(Default)]
pub struct ContrastImageNode {}

impl ContrastImageNode {
    pub fn new() -> Self {
        ContrastImageNode {}
    }
}

#[async_trait]
impl NodeLogic for ContrastImageNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "contrast_image",
            "Contrast",
            "Adjust Image Contrast",
            "Image/Transform",
        );
        node.set_flowscript_name("image", "contrast");
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

        node.add_input_pin("contrast", "Contrast", "Contrast", VariableType::Float);

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
            "Image with Applied Contrast",
            VariableType::Struct,
        )
        .set_schema::<NodeImage>();

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        // fetch inputs
        let mut node_img: NodeImage = context.evaluate_pin("image_in").await?;
        let contrast: f32 = context.evaluate_pin("contrast").await?;
        let use_ref: bool = context.evaluate_pin("use_ref").await?;
        if !use_ref {
            node_img = node_img.copy_image(context).await?;
        }
        let img = node_img.get_image(context).await?;

        // adjust contrast
        {
            let mut img_guard = img.lock().await;
            let img_contrast = img_guard.adjust_contrast(contrast);
            *img_guard = img_contrast;
        }

        // set outputs
        context.set_pin_value("image_out", json!(node_img)).await?;
        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }
}
