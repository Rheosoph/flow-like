use crate::image::NodeImage;
use flow_like::flow::{
    board::Board,
    execution::{LogLevel, context::ExecutionContext},
    node::{Node, NodeLogic, remove_pin},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_types::{
    anyhow, async_trait,
    json::json,
    rxing::{BarcodeFormat, DecodeHints, Exceptions::NotFoundException, RXingResult, helpers},
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::{collections::HashSet, sync::Arc};

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub struct BarcodePoint {
    pub x: f32,
    pub y: f32,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub struct Barcode {
    text: String,
    raw_bytes: Vec<u8>,
    num_bits: usize,
    format: String,
    timestamp: u128,
    line_count: usize,
    points: Vec<BarcodePoint>,
}

impl From<RXingResult> for Barcode {
    fn from(value: RXingResult) -> Self {
        let points = value
            .getPoints()
            .iter()
            .map(|p| BarcodePoint { x: p.x, y: p.y })
            .collect();
        Barcode {
            text: value.getText().to_string(),
            raw_bytes: value.getRawBytes().to_vec(),
            num_bits: value.getNumBits(),
            format: value.getBarcodeFormat().to_string(),
            timestamp: value.getTimestamp(),
            line_count: value.line_count(),
            points,
        }
    }
}

/// # Detect and Decode (Bar)codes in Images
#[crate::register_node]
#[derive(Default)]
pub struct ReadBarcodesNode {}

impl ReadBarcodesNode {
    pub fn new() -> Self {
        ReadBarcodesNode {}
    }
}

#[async_trait]
impl NodeLogic for ReadBarcodesNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "read_barcodes",
            "Read QR-/Barcode",
            "Read/Decode QR Codes and Barcodes",
            "Image/Content",
        );
        node.add_icon("/flow/icons/barcode.svg");

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
            "filter",
            "Filter?",
            "Filter for Certain Code Type",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(false)));

        // outputs
        node.add_output_pin(
            "exec_out",
            "Output",
            "Done with the Execution",
            VariableType::Execution,
        );

        node.add_output_pin(
            "results",
            "Results",
            "Detected/Decoded Codes",
            VariableType::Struct,
        )
        .set_schema::<Barcode>()
        .set_value_type(flow_like::flow::pin::ValueType::Array);

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        // fetch inputs
        let apply_filter: bool = context.evaluate_pin("filter").await?;
        let node_img: NodeImage = context.evaluate_pin("image_in").await?;

        // prepare image
        let img = node_img.get_image(context).await?;
        let (img_vec, w, h) = {
            let img_guard = img.lock().await;
            let (w, h) = (img_guard.width(), img_guard.height());
            let img_vec = img_guard
                .clone()
                .into_luma8() // decoding works best with grayscale images
                .to_vec();
            (img_vec, w, h)
        };

        // detect + decode (bar)codes
        let results_rxing = match apply_filter {
            // many codes & many types (potentially expensive)
            false => match helpers::detect_multiple_in_luma(img_vec, w, h) {
                Ok(results) => results,
                Err(NotFoundException(_)) => {
                    context.log_message("No Codes Detected / Decoded!", LogLevel::Warn);
                    vec![]
                }
                Err(e) => return Err(anyhow!("Decoder Error: {}", e)),
            },
            // many codes & single type
            true => {
                let mut hints = DecodeHints::default();
                let format_str: String = context.evaluate_pin("format").await?;
                let bc_type = BarcodeFormat::from(format_str);
                hints.PossibleFormats = Some(HashSet::from([bc_type]));
                match helpers::detect_multiple_in_luma_with_hints(img_vec, w, h, &mut hints) {
                    Ok(results) => results,
                    Err(NotFoundException(_)) => {
                        context.log_message("No Codes Detected / Decoded!", LogLevel::Warn);
                        vec![]
                    }
                    Err(e) => return Err(anyhow!("Decoder Error: {}", e)),
                }
            }
        };

        // map to serializable results
        let results = results_rxing
            .into_iter()
            .map(Barcode::from)
            .collect::<Vec<Barcode>>();

        // set outputs
        context.set_pin_value("results", json!(results)).await?;
        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }

    async fn on_update(&self, node: &mut Node, _board: &Board) {
        let apply_filter = node
            .get_pin_by_name("filter")
            .and_then(|pin| pin.default_value.clone())
            .and_then(|bytes| flow_like_types::json::from_slice::<bool>(&bytes).ok())
            .unwrap_or_default();

        let format_pin = node.get_pin_by_name("format").cloned();

        if apply_filter {
            if format_pin.is_some() {
                return;
            }
            node.add_input_pin("format", "Format", "Barcode Format", VariableType::String)
                .set_options(
                    PinOptions::new()
                        .set_valid_values(vec![
                            "AZTEC".to_string(),
                            "CODABAR".to_string(),
                            "CODE_39".to_string(),
                            "CODE_93".to_string(),
                            "CODE_128".to_string(),
                            "DATA_MATRIX".to_string(),
                            "EAN_8".to_string(),
                            "EAN_13".to_string(),
                            "ITF".to_string(),
                            "MAXICODE".to_string(),
                            "PDF_417".to_string(),
                            "QR_CODE".to_string(),
                            "MICRO_QR_CODE".to_string(),
                            "RECTANGULAR_MICRO_QR_CODE".to_string(),
                            "RSS_14".to_string(),
                            "RSS_EXPANDED".to_string(),
                            "TELEPEN".to_string(),
                            "UPC_A".to_string(),
                            "UPC_E".to_string(),
                            "UPC_EAN_EXTENSION".to_string(),
                            "DXFilmEdge".to_string(),
                        ])
                        .build(),
                );
        } else {
            remove_pin(node, format_pin);
        }
    }
}
