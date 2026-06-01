use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_catalog_core::NodeImage;
#[cfg(feature = "execute")]
use flow_like_types::image::{DynamicImage, ImageBuffer, Luma};
use flow_like_types::{async_trait, json::json};
#[cfg(feature = "execute")]
use qrcode::{QrCode, types::Color};

#[crate::register_node]
#[derive(Default)]
pub struct WriteQrCodeNode {}

impl WriteQrCodeNode {
    pub fn new() -> Self {
        WriteQrCodeNode {}
    }
}

#[async_trait]
impl NodeLogic for WriteQrCodeNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "write_qrcode",
            "Write QR Code",
            "Encode text as a QR code image",
            "Data/QR",
        );
        node.set_version(1);
        node.add_icon("/flow/icons/barcode.svg");

        node.add_input_pin(
            "exec_in",
            "Input",
            "Initiate Execution",
            VariableType::Execution,
        );

        node.add_input_pin("data", "Data", "Text to encode", VariableType::String);

        node.add_input_pin(
            "scale",
            "Scale",
            "Pixels per QR module",
            VariableType::Integer,
        )
        .set_options(PinOptions::new().set_range((1., 64.)).build())
        .set_default_value(Some(json!(8)));

        node.add_input_pin(
            "margin",
            "Margin",
            "Quiet zone in modules",
            VariableType::Integer,
        )
        .set_options(PinOptions::new().set_range((0., 20.)).build())
        .set_default_value(Some(json!(4)));

        node.add_output_pin(
            "exec_out",
            "Output",
            "Done with the Execution",
            VariableType::Execution,
        );

        node.add_output_pin("image_out", "Image", "QR code image", VariableType::Struct)
            .set_schema::<NodeImage>();

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let data: String = context.evaluate_pin("data").await?;
        let scale: i64 = context.evaluate_pin("scale").await?;
        let margin: i64 = context.evaluate_pin("margin").await?;

        let scale = scale.max(1) as u32;
        let margin = margin.max(0) as u32;

        let code = QrCode::new(data.as_bytes())?;
        let module_count = code.width() as u32;
        let image_size = (module_count + margin * 2) * scale;
        let mut img = ImageBuffer::from_pixel(image_size, image_size, Luma([255u8]));
        let colors = code.to_colors();

        for y in 0..module_count {
            for x in 0..module_count {
                let index = (y * module_count + x) as usize;
                if colors[index] == Color::Dark {
                    let x0 = (x + margin) * scale;
                    let y0 = (y + margin) * scale;
                    for dy in 0..scale {
                        for dx in 0..scale {
                            img.put_pixel(x0 + dx, y0 + dy, Luma([0u8]));
                        }
                    }
                }
            }
        }

        let image = DynamicImage::ImageLuma8(img);
        let node_img = NodeImage::new(context, image).await;
        context.set_pin_value("image_out", json!(node_img)).await?;
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
