use flow_like::flow::{
    execution::{LogLevel, context::ExecutionContext},
    node::{Node, NodeLogic, NodeScores},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_catalog_core::{FilePrefix, FlowPath, read_image_header};
use flow_like_types::{async_trait, json::json};

#[crate::register_node]
#[derive(Default)]
pub struct ReadImageDimensionsNode {}

impl ReadImageDimensionsNode {
    pub fn new() -> Self {
        ReadImageDimensionsNode {}
    }
}

#[async_trait]
impl NodeLogic for ReadImageDimensionsNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "read_image_dimensions",
            "Read Image Dimensions",
            "Reads width, height, format and EXIF orientation from an image file's header without decoding its pixels. Only the leading bytes the header needs are fetched, so it costs a fraction of Read Image + Get Dimensions on large photos.",
            "Image/Content",
        );
        node.set_flowscript_name("image", "readDimensions");
        node.set_receiver("path");
        node.add_icon("/flow/icons/image.svg");
        node.set_scores(
            NodeScores::new()
                .set_privacy(9)
                .set_security(9)
                .set_performance(10)
                .set_governance(9)
                .set_reliability(9)
                .set_cost(10)
                .build(),
        );

        node.add_input_pin(
            "exec_in",
            "Input",
            "Initiate Execution",
            VariableType::Execution,
        );
        node.add_input_pin(
            "path",
            "Path",
            "Image file to inspect",
            VariableType::Struct,
        )
        .set_schema::<FlowPath>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());
        node.add_input_pin(
            "apply_exif_orientation",
            "Apply Exif Orientation",
            "Report the size of the upright image, swapping width and height when the EXIF orientation turns it by 90 degrees — the values Read Image + Get Dimensions return",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(true)));

        node.add_output_pin(
            "exec_out",
            "Output",
            "Done with the Execution",
            VariableType::Execution,
        );
        node.add_output_pin(
            "width",
            "Width",
            "Image width in pixels",
            VariableType::Integer,
        );
        node.add_output_pin(
            "height",
            "Height",
            "Image height in pixels",
            VariableType::Integer,
        );
        node.add_output_pin(
            "format",
            "Format",
            "Image format in lowercase, e.g. jpeg, png, webp or tiff",
            VariableType::String,
        );
        node.add_output_pin(
            "orientation",
            "Orientation",
            "EXIF orientation tag (1–8); 1 when the file carries none",
            VariableType::Integer,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        let path: FlowPath = context.evaluate_pin("path").await?;
        let apply_exif_orientation: bool = context.evaluate_pin("apply_exif_orientation").await?;

        let mut file = FilePrefix::open(&path, context).await?;
        let header = read_image_header(&mut file, apply_exif_orientation).await?;
        context.log_message(
            &format!(
                "Read the {} header of '{}' from {} of {} bytes",
                header.format,
                path.path,
                file.bytes().len(),
                file.meta().size
            ),
            LogLevel::Debug,
        );

        context.set_pin_value("width", json!(header.width)).await?;
        context
            .set_pin_value("height", json!(header.height))
            .await?;
        context
            .set_pin_value("format", json!(header.format))
            .await?;
        context
            .set_pin_value("orientation", json!(header.orientation))
            .await?;
        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::ReadImageDimensionsNode;
    use crate::image::content::read_from_path::decode_image;
    use flow_like::flow::{node::NodeLogic, variable::VariableType};
    use flow_like_catalog_core::parse_image_header;
    use flow_like_types::image::{
        ExtendedColorType, GenericImageView, ImageEncoder, Rgb, RgbImage, codecs::jpeg::JpegEncoder,
    };

    /// Little-endian TIFF block with one entry, Orientation 8: turn 90 degrees counter-clockwise.
    const ROTATE_270_EXIF: [u8; 26] = [
        0x49, 0x49, 0x2a, 0x00, 0x08, 0x00, 0x00, 0x00, 0x01, 0x00, 0x12, 0x01, 0x03, 0x00, 0x01,
        0x00, 0x00, 0x00, 0x08, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    ];

    fn rotated_photo() -> Vec<u8> {
        let frame = RgbImage::from_pixel(40, 24, Rgb([10, 120, 200]));
        let mut bytes = Vec::new();
        let mut encoder = JpegEncoder::new(&mut bytes);
        encoder.set_exif_metadata(ROTATE_270_EXIF.to_vec()).unwrap();
        encoder
            .write_image(frame.as_raw(), 40, 24, ExtendedColorType::Rgb8)
            .unwrap();
        bytes
    }

    #[test]
    fn header_dimensions_match_a_full_decode() {
        let bytes = rotated_photo();
        for apply in [true, false] {
            let header = parse_image_header(&bytes, apply).unwrap();
            let decoded = decode_image(bytes.clone(), apply).unwrap();
            assert_eq!((header.width, header.height), decoded.dimensions());
            assert_eq!(header.orientation, 8);
        }
    }

    #[test]
    fn node_declares_header_outputs_and_upright_default() {
        let node = ReadImageDimensionsNode::new().get_node();
        assert_eq!(node.name, "read_image_dimensions");
        let apply = node.get_pin_by_name("apply_exif_orientation").unwrap();
        let default: bool =
            flow_like_types::json::from_slice(apply.default_value.as_ref().unwrap()).unwrap();
        assert!(default);
        for (name, data_type) in [
            ("width", VariableType::Integer),
            ("height", VariableType::Integer),
            ("format", VariableType::String),
            ("orientation", VariableType::Integer),
            ("exec_out", VariableType::Execution),
        ] {
            assert_eq!(node.get_pin_by_name(name).unwrap().data_type, data_type);
        }
    }
}
