#[cfg(feature = "execute")]
use crate::image::pdf::{
    load_pdf_from_flowpath, pixmap_to_dynamic_image, resolve_bg_color, validate_scale,
};
use flow_like::flow::execution::context::ExecutionContext;
use flow_like::flow::node::{Node, NodeLogic};
use flow_like::flow::pin::{PinOptions, ValueType};
use flow_like::flow::variable::VariableType;
use flow_like_catalog_core::FlowPath;
use flow_like_catalog_core::NodeImage;
use flow_like_types::{async_trait, json::json};
#[cfg(feature = "execute")]
use hayro::hayro_interpret::InterpreterSettings;
#[cfg(feature = "execute")]
use hayro::{RenderSettings, render};

#[crate::register_node]
#[derive(Default)]
pub struct PdfToImagesNode;

impl PdfToImagesNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for PdfToImagesNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "pdf_to_images",
            "PDF To Images",
            "Render every PDF page as an ordered image array",
            "Image/PDF",
        );
        node.set_version(2);
        node.add_icon("/flow/icons/image.svg");

        node.add_input_pin(
            "exec_in",
            "Input",
            "Trigger execution",
            VariableType::Execution,
        );

        node.add_input_pin("pdf", "PDF", "PDF file", VariableType::Struct)
            .set_schema::<FlowPath>()
            .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin("scale", "Scale", "Render scale", VariableType::Float)
            .set_default_value(Some(json!(1.0)));

        node.add_input_pin(
            "bg_color",
            "Background Color",
            "Background color for rendered pages",
            VariableType::String,
        )
        .set_options(
            PinOptions::new()
                .set_valid_values(vec![
                    "Transparent".to_string(),
                    "White".to_string(),
                    "Black".to_string(),
                    "Red".to_string(),
                    "Green".to_string(),
                ])
                .build(),
        )
        .set_default_value(Some(json!("White")));

        node.add_output_pin(
            "exec_out",
            "Output",
            "Execution done",
            VariableType::Execution,
        );

        node.add_output_pin("images", "Images", "Rendered images", VariableType::Struct)
            .set_schema::<NodeImage>()
            .set_value_type(ValueType::Array);

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let pdf_path: FlowPath = context.evaluate_pin("pdf").await?;
        let scale: f32 = context.evaluate_pin("scale").await?;
        let bg_color: String = context.evaluate_pin("bg_color").await?;

        let images = render_pdf_pages(context, &pdf_path, scale, &bg_color).await?;

        context.set_pin_value("images", json!(images)).await?;
        context.activate_exec_pin("exec_out").await?;

        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "Media processing requires the 'execute' feature"
        ))
    }
}

#[cfg(feature = "execute")]
async fn render_pdf_pages(
    context: &mut ExecutionContext,
    pdf_path: &FlowPath,
    scale: f32,
    bg_color_hex: &str,
) -> flow_like_types::Result<Vec<NodeImage>> {
    validate_scale(scale)?;
    let pdf = load_pdf_from_flowpath(context, pdf_path).await?;
    let pages = pdf.pages();
    let mut images = Vec::with_capacity(pages.len());
    let interpreter_settings = InterpreterSettings::default();
    let render_settings = RenderSettings {
        x_scale: scale,
        y_scale: scale,
        width: None,
        height: None,
        bg_color: resolve_bg_color(bg_color_hex),
    };

    for page in pages.iter() {
        let pixmap = render(page, &interpreter_settings, &render_settings);
        let image = pixmap_to_dynamic_image(pixmap)?;
        let node_image = NodeImage::new(context, image).await;
        images.push(node_image);
    }

    Ok(images)
}
