use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_catalog_core::NodeImage;
#[cfg(feature = "execute")]
use flow_like_types::anyhow;
#[cfg(feature = "execute")]
use flow_like_types::image::{DynamicImage, GenericImageView, imageops::FilterType};
use flow_like_types::{async_trait, json::json};
#[cfg(feature = "execute")]
use rxing::{BarcodeFormat, EncodeHints, MultiFormatWriter, Writer};

const WRITABLE_BARCODE_FORMATS: &[&str] = &[
    "AZTEC",
    "CODABAR",
    "CODE_39",
    "CODE_93",
    "CODE_128",
    "DATA_MATRIX",
    "EAN_8",
    "EAN_13",
    "ITF",
    "PDF_417",
    "QR_CODE",
    "TELEPEN",
    "UPC_A",
    "UPC_E",
];

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
            "Write Barcode",
            "Encode text as a barcode image",
            "Data/QR",
        );
        node.set_flowscript_name("image", "writeBarcode");
        node.set_version(2);
        node.add_icon("/flow/icons/barcode.svg");

        node.add_input_pin(
            "exec_in",
            "Input",
            "Initiate Execution",
            VariableType::Execution,
        );

        node.add_input_pin("data", "Data", "Text to encode", VariableType::String);

        node.add_input_pin("format", "Format", "Barcode Format", VariableType::String)
            .set_options(
                PinOptions::new()
                    .set_valid_values(
                        WRITABLE_BARCODE_FORMATS
                            .iter()
                            .map(|format| format.to_string())
                            .collect(),
                    )
                    .build(),
            )
            .set_default_value(Some(json!("QR_CODE")));

        node.add_input_pin(
            "scale",
            "Scale",
            "Pixels per barcode module",
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

        node.add_output_pin("image_out", "Image", "Barcode image", VariableType::Struct)
            .set_schema::<NodeImage>();

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let data: String = context.evaluate_pin("data").await?;
        let format_str: String = context.evaluate_pin("format").await?;
        let scale: i64 = context.evaluate_pin("scale").await?;
        let margin: i64 = context.evaluate_pin("margin").await?;

        let scale = scale.max(1) as u32;
        let margin = margin.max(0) as u32;
        let format = parse_barcode_format(&format_str)?;

        let hints = EncodeHints {
            Margin: Some(margin.to_string()),
            ..EncodeHints::default()
        };
        let bit_matrix = MultiFormatWriter.encode_with_hints(&data, &format, 0, 0, &hints)?;
        let mut image: DynamicImage = bit_matrix.into();

        if scale > 1 {
            let (width, height) = image.dimensions();
            let target_height = if is_one_dimensional(format) {
                (scale * 16).max(32)
            } else {
                height * scale
            };
            image = image.resize_exact(width * scale, target_height, FilterType::Nearest);
        }

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

#[cfg(feature = "execute")]
fn parse_barcode_format(format: &str) -> flow_like_types::Result<BarcodeFormat> {
    let normalized = format.trim().to_uppercase();
    let barcode_format = BarcodeFormat::from(normalized.as_str());
    if barcode_format == BarcodeFormat::UNSUPORTED_FORMAT
        || !WRITABLE_BARCODE_FORMATS
            .iter()
            .any(|supported| BarcodeFormat::from(*supported) == barcode_format)
    {
        return Err(anyhow!("Unsupported writable barcode format: {}", format));
    }
    Ok(barcode_format)
}

#[cfg(feature = "execute")]
fn is_one_dimensional(format: BarcodeFormat) -> bool {
    matches!(
        format,
        BarcodeFormat::CODABAR
            | BarcodeFormat::CODE_39
            | BarcodeFormat::CODE_93
            | BarcodeFormat::CODE_128
            | BarcodeFormat::EAN_8
            | BarcodeFormat::EAN_13
            | BarcodeFormat::ITF
            | BarcodeFormat::TELEPEN
            | BarcodeFormat::UPC_A
            | BarcodeFormat::UPC_E
    )
}
