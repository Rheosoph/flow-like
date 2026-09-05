use flow_like::flow::{
    board::Board,
    execution::context::ExecutionContext,
    node::{Node, NodeLogic, remove_pin},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_catalog_core::{FlowPath, NodeImage};
use flow_like_types::{
    anyhow, async_trait,
    image::{
        DynamicImage,
        codecs::{
            avif::AvifEncoder,
            bmp::BmpEncoder,
            farbfeld::FarbfeldEncoder,
            gif::GifEncoder,
            hdr::HdrEncoder,
            ico::IcoEncoder,
            jpeg::JpegEncoder,
            openexr::OpenExrEncoder,
            png::{CompressionType, FilterType, PngEncoder},
            pnm::PnmEncoder,
            qoi::QoiEncoder,
            tga::TgaEncoder,
            webp::WebPEncoder,
        },
    },
    json::json,
};
#[crate::register_node]
#[derive(Default)]
pub struct WriteImageNode {}

impl WriteImageNode {
    pub fn new() -> Self {
        WriteImageNode {}
    }
}

#[async_trait]
impl NodeLogic for WriteImageNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "write_image",
            "Write Image",
            "Write image to path",
            "Image/Content",
        );
        node.set_flowscript_name("image", "write");
        node.set_receiver("image_in");
        node.set_version(1);
        node.add_icon("/flow/icons/image.svg");

        node.add_input_pin(
            "exec_in",
            "Input",
            "Initiate Execution",
            VariableType::Execution,
        );

        node.add_input_pin(
            "image_in",
            "Image",
            "The image to write to path",
            VariableType::Struct,
        )
        .set_schema::<NodeImage>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin("path", "Path", "FlowPath", VariableType::Struct)
            .set_schema::<FlowPath>()
            .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin("type", "Type", "Image Type", VariableType::String)
            .set_options(
                PinOptions::new()
                    .set_valid_values(vec![
                        "JPEG".to_string(),
                        "PNG".to_string(),
                        "TIFF".to_string(),
                        "WebP".to_string(),
                        // "Gif".to_string(), // CURRENTLY NOT SUPPORTED
                        "AVIF".to_string(),
                        "BMP".to_string(),
                        "ICO".to_string(),
                        "PNM".to_string(),
                        "QOI".to_string(),
                        "TGA".to_string(),
                        "Farbfeld".to_string(),
                        "HDR".to_string(),
                        // "OpenExr".to_string(), // CURRENTLY NOT SUPPORTED
                    ])
                    .build(),
            )
            .set_default_value(Some(json!("JPEG")));

        node.add_input_pin(
            "quality",
            "Quality",
            "Encoding Quality",
            VariableType::Integer,
        )
        .set_options(PinOptions::new().set_range((0., 100.)).build())
        .set_default_value(Some(json!(100)));

        node.add_output_pin(
            "exec_out",
            "Output",
            "Done with the Execution",
            VariableType::Execution,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        let image_type: String = context.evaluate_pin("type").await?;
        let path: FlowPath = context.evaluate_pin("path").await?;

        let path = match image_type.as_str() {
            "JPEG" => path.set_extension(context, "jpeg"),
            "PNG" => path.set_extension(context, "png"),
            "TIFF" => path.set_extension(context, "tiff"),
            "WebP" => path.set_extension(context, "webp"),
            "Gif" => path.set_extension(context, "gif"),
            "AVIF" => path.set_extension(context, "avif"),
            "BMP" => path.set_extension(context, "bmp"),
            "ICO" => path.set_extension(context, "ico"),
            "PNM" => path.set_extension(context, "pnm"),
            "QOI" => path.set_extension(context, "qoi"),
            "TGA" => path.set_extension(context, "tga"),
            "Farbfeld" => path.set_extension(context, "ff"),
            "HDR" => path.set_extension(context, "hdr"),
            "OpenExr" => path.set_extension(context, "exr"),
            _ => return Err(anyhow!("Unsupported image type")),
        }
        .await?;

        let node_image: NodeImage = context.evaluate_pin("image_in").await?;

        let image = node_image.get_image(context).await?;
        let img = image.lock().await;

        let mut encoded = Vec::with_capacity(img.width() as usize * img.height() as usize * 4);

        match image_type.as_str() {
            "JPEG" => {
                let quality: u8 = context.evaluate_pin("quality").await?;
                let encoder = JpegEncoder::new_with_quality(&mut encoded, quality);
                let rgb_img = DynamicImage::ImageRgb8(img.to_rgb8());

                rgb_img.write_with_encoder(encoder)?;
            }
            "PNG" => {
                let compression_type: String = context.evaluate_pin("compression_type").await?;
                let compression = match compression_type.as_str() {
                    "Best" => CompressionType::Best,
                    "Fast" => CompressionType::Fast,
                    "Default" => CompressionType::Default,
                    _ => CompressionType::Default,
                };
                let filter: String = context.evaluate_pin("filter").await?;
                let filter = match filter.as_str() {
                    "NoFilter" => FilterType::NoFilter,
                    "Sub" => FilterType::Sub,
                    "Up" => FilterType::Up,
                    "Average" => FilterType::Avg,
                    "Adaptive" => FilterType::Adaptive,
                    "Paeth" => FilterType::Paeth,
                    _ => FilterType::Adaptive,
                };
                let encoder = PngEncoder::new_with_quality(&mut encoded, compression, filter);
                img.write_with_encoder(encoder)?;
            }
            "WebP" => {
                let encoder = WebPEncoder::new_lossless(&mut encoded);
                img.write_with_encoder(encoder)?;
            }
            "Gif" => {
                let speed: i32 = context.evaluate_pin("speed").await?;

                let _encoder = GifEncoder::new_with_speed(&mut encoded, speed);
                // encoder.encode_frame(img.as_ref())?;
            }
            "AVIF" => {
                let quality: u8 = context.evaluate_pin("quality").await?;
                let speed: u8 = context.evaluate_pin("speed").await?;
                let threads: usize = context.evaluate_pin("threads").await?;

                let mut encoder = AvifEncoder::new_with_speed_quality(&mut encoded, speed, quality);
                encoder = encoder.with_num_threads(Some(threads));
                img.write_with_encoder(encoder)?;
            }
            "BMP" => {
                let encoder = BmpEncoder::new(&mut encoded);
                img.write_with_encoder(encoder)?;
            }
            "ICO" => {
                let encoder = IcoEncoder::new(&mut encoded);
                img.write_with_encoder(encoder)?;
            }
            "PNM" => {
                let encoder = PnmEncoder::new(&mut encoded);
                let rgb_img = DynamicImage::ImageRgb8(img.to_rgb8());

                rgb_img.write_with_encoder(encoder)?;
            }
            "QOI" => {
                let encoder = QoiEncoder::new(&mut encoded);
                img.write_with_encoder(encoder)?;
            }
            "TGA" => {
                let encoder = TgaEncoder::new(&mut encoded);
                img.write_with_encoder(encoder)?;
            }
            "Farbfeld" => {
                let encoder = FarbfeldEncoder::new(&mut encoded);
                img.write_with_encoder(encoder)?;
            }
            "HDR" => {
                let encoder = HdrEncoder::new(&mut encoded);
                let rgb_img = DynamicImage::ImageRgb8(img.to_rgb8());

                rgb_img.write_with_encoder(encoder)?;
            }
            "OpenExr" => {
                let _encoder = OpenExrEncoder::new(&mut encoded);
            }
            _ => return Err(anyhow!("Unsupported image type")),
        };

        path.put(context, encoded, false).await?;

        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }

    async fn on_update(&self, node: &mut Node, _board: &Board) {
        let image_type = node
            .get_pin_by_name("type")
            .and_then(|pin| pin.default_value.clone())
            .and_then(|bytes| flow_like_types::json::from_slice::<String>(&bytes).ok())
            .unwrap_or_default()
            .clone();

        // jpeg + avif
        let quality_pin = node.get_pin_by_name("quality").cloned();

        // png
        let compression_type = node.get_pin_by_name("compression_type").cloned();
        let filter = node.get_pin_by_name("filter").cloned();

        // gif + avif
        let speed = node.get_pin_by_name("speed").cloned();

        // avif
        let threads = node.get_pin_by_name("threads").cloned();

        match image_type.as_str() {
            "JPEG" => {
                remove_pin(node, compression_type);
                remove_pin(node, filter);
                remove_pin(node, speed);
                remove_pin(node, threads);

                if quality_pin.is_none() {
                    node.add_input_pin(
                        "quality",
                        "Quality",
                        "Encoding Quality",
                        VariableType::Integer,
                    )
                    .set_options(PinOptions::new().set_range((0., 100.)).build())
                    .set_default_value(Some(json!(100)));
                }
            }
            "PNG" => {
                remove_pin(node, quality_pin);
                remove_pin(node, speed);
                remove_pin(node, threads);

                if compression_type.is_none() {
                    node.add_input_pin(
                        "compression_type",
                        "Compression Type",
                        "Compression Type",
                        VariableType::String,
                    )
                    .set_options(
                        PinOptions::new()
                            .set_valid_values(vec![
                                "Best".to_string(),
                                "Fast".to_string(),
                                "Default".to_string(),
                            ])
                            .build(),
                    )
                    .set_default_value(Some(json!("Default")));
                }

                if filter.is_none() {
                    node.add_input_pin("filter", "Filter", "Filter Type", VariableType::String)
                        .set_options(
                            PinOptions::new()
                                .set_valid_values(vec![
                                    "NoFilter".to_string(),
                                    "Sub".to_string(),
                                    "Up".to_string(),
                                    "Average".to_string(),
                                    "Adaptive".to_string(),
                                    "Paeth".to_string(),
                                ])
                                .build(),
                        )
                        .set_default_value(Some(json!("Adaptive")));
                }
            }
            "GIF" => {
                remove_pin(node, compression_type);
                remove_pin(node, filter);
                remove_pin(node, threads);
                remove_pin(node, quality_pin);

                if speed.is_none() {
                    node.add_input_pin("speed", "Speed", "Encoding Speed", VariableType::Integer)
                        .set_options(PinOptions::new().set_range((0., 10.)).build())
                        .set_default_value(Some(json!(10)));
                }
            }
            "AVIF" => {
                remove_pin(node, compression_type);
                remove_pin(node, filter);
                remove_pin(node, quality_pin);

                if speed.is_none() {
                    node.add_input_pin("speed", "Speed", "Encoding Speed", VariableType::Integer)
                        .set_options(PinOptions::new().set_range((0., 10.)).build())
                        .set_default_value(Some(json!(10)));
                }

                if threads.is_none() {
                    node.add_input_pin(
                        "threads",
                        "Threads",
                        "Number of Threads",
                        VariableType::Integer,
                    )
                    .set_options(PinOptions::new().set_range((1., 16.)).build())
                    .set_default_value(Some(json!(1)));
                }
            }
            _ => {
                remove_pin(node, compression_type);
                remove_pin(node, filter);
                remove_pin(node, speed);
                remove_pin(node, threads);
                remove_pin(node, quality_pin);
            }
        }
    }
}
