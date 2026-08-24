use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_catalog_core::NodeImage;
use flow_like_types::{Ok, async_trait, image::GenericImageView, json::json};

#[crate::register_node]
#[derive(Default)]
pub struct ImageDimsNode {}

impl ImageDimsNode {
    pub fn new() -> Self {
        ImageDimsNode {}
    }
}

#[async_trait]
impl NodeLogic for ImageDimsNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "get_dimensions",
            "Get Dimensions",
            "Get Image Dimensions",
            "Image/Metadata",
        );
        node.set_flowscript_name("image", "getDimensions");
        node.set_receiver("image_in");
        node.set_version(1);
        node.add_icon("/flow/icons/dir.svg");

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

        // outputs
        // todo: output as tuple
        node.add_output_pin(
            "exec_out",
            "Output",
            "Done with the Execution",
            VariableType::Execution,
        );
        node.add_output_pin("width", "width", "Image Width", VariableType::Integer);
        node.add_output_pin("height", "height", "Image Height", VariableType::Integer);
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        let node_image: NodeImage = context.evaluate_pin("image_in").await?;

        let img = node_image.get_image(context).await?;
        let (width, height) = {
            let img_guard = img.lock().await;
            img_guard.dimensions()
        };

        context.set_pin_value("width", json!(width)).await?;
        context.set_pin_value("height", json!(height)).await?;
        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }
}
