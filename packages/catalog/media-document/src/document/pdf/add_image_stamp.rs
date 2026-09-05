#[cfg(feature = "execute")]
use lopdf::{Document, Object, Stream, dictionary};

use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic, NodeScores},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_catalog_core::FlowPath;
use flow_like_types::{async_trait, json::json};

#[crate::register_node]
#[derive(Default)]
pub struct PdfAddImageStampNode;

impl PdfAddImageStampNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for PdfAddImageStampNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "pdf_add_image_stamp",
            "Add Image Stamp",
            "Stamp an image at a specified position on selected PDF pages.",
            "Document/PDF",
        );
        node.set_flowscript_name("pdf", "addImageStamp");
        node.add_icon("/flow/icons/image.svg");
        node.set_scores(
            NodeScores::new()
                .set_privacy(7)
                .set_security(6)
                .set_performance(5)
                .set_governance(8)
                .set_reliability(7)
                .set_cost(10)
                .build(),
        );

        node.add_input_pin("exec_in", "Input", "Trigger", VariableType::Execution);
        node.add_input_pin("template", "Template", "PDF file", VariableType::Struct)
            .set_schema::<FlowPath>()
            .set_options(PinOptions::new().set_enforce_schema(true).build());
        node.add_input_pin(
            "image",
            "Image",
            "Image file (PNG/JPEG)",
            VariableType::Struct,
        )
        .set_schema::<FlowPath>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());
        node.add_input_pin("x", "X", "X position in points", VariableType::Float)
            .set_default_value(Some(json!(50.0)));
        node.add_input_pin("y", "Y", "Y position in points", VariableType::Float)
            .set_default_value(Some(json!(50.0)));
        node.add_input_pin(
            "width",
            "Width",
            "Image width in points",
            VariableType::Float,
        )
        .set_default_value(Some(json!(100.0)));
        node.add_input_pin(
            "height",
            "Height",
            "Image height in points",
            VariableType::Float,
        )
        .set_default_value(Some(json!(100.0)));
        node.add_input_pin(
            "pages",
            "Pages",
            "Page numbers (empty = all)",
            VariableType::Integer,
        )
        .set_value_type(flow_like::flow::pin::ValueType::Array)
        .set_default_value(Some(json!([])));
        node.add_input_pin("output", "Output Path", "Save path", VariableType::Struct)
            .set_schema::<FlowPath>()
            .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin("exec_out", "Done", "Continues", VariableType::Execution);
        node.add_output_pin("result", "Result", "Output file path", VariableType::Struct)
            .set_schema::<FlowPath>();

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let template: FlowPath = context.evaluate_pin("template").await?;
        let image_path: FlowPath = context.evaluate_pin("image").await?;
        let x: f64 = context.evaluate_pin("x").await?;
        let y: f64 = context.evaluate_pin("y").await?;
        let width: f64 = context.evaluate_pin("width").await?;
        let height: f64 = context.evaluate_pin("height").await?;
        let pages: Vec<i64> = context.evaluate_pin("pages").await.unwrap_or_default();
        let output: FlowPath = context.evaluate_pin("output").await?;

        let bytes = template.get(context, false).await?;
        let image_bytes = image_path.get(context, false).await?;

        let mut doc = Document::load_mem(&bytes)?;

        let is_png = image_bytes.starts_with(&[0x89, 0x50, 0x4E, 0x47]);
        let filter = if is_png { "FlateDecode" } else { "DCTDecode" };

        let img_stream = Stream::new(
            dictionary! {
                "Type" => Object::Name(b"XObject".to_vec()),
                "Subtype" => Object::Name(b"Image".to_vec()),
                "Width" => Object::Integer(width as i64),
                "Height" => Object::Integer(height as i64),
                "ColorSpace" => Object::Name(b"DeviceRGB".to_vec()),
                "BitsPerComponent" => Object::Integer(8),
                "Filter" => Object::Name(filter.as_bytes().to_vec()),
            },
            image_bytes.to_vec(),
        );
        let img_id = doc.add_object(Object::Stream(img_stream));

        let page_ids: Vec<lopdf::ObjectId> = doc.page_iter().collect();
        let target_pages: std::collections::HashSet<usize> = if pages.is_empty() {
            (1..=page_ids.len()).collect()
        } else {
            pages.iter().map(|p| *p as usize).collect()
        };

        for (idx, page_id) in page_ids.iter().enumerate() {
            let page_num = idx + 1;
            if !target_pages.contains(&page_num) {
                continue;
            }

            let content_str = format!(
                "q\n{w} 0 0 {h} {x} {y} cm\n/Im0 Do\nQ\n",
                w = width,
                h = height,
                x = x,
                y = y,
            );
            let stamp_stream = Stream::new(dictionary! {}, content_str.into_bytes());
            let stream_id = doc.add_object(Object::Stream(stamp_stream));

            let xobj_dict = dictionary! {
                "Im0" => Object::Reference(img_id),
            };

            if let Ok(page) = doc.get_object_mut(*page_id)
                && let Object::Dictionary(dict) = page
            {
                if let Ok(Object::Dictionary(resources)) = dict.get_mut(b"Resources") {
                    resources.set("XObject", Object::Dictionary(xobj_dict));
                } else {
                    let resources = dictionary! {
                        "XObject" => Object::Dictionary(xobj_dict),
                    };
                    dict.set("Resources", Object::Dictionary(resources));
                }

                let existing_contents = dict.get(b"Contents").ok().cloned();
                match existing_contents {
                    Some(Object::Array(mut arr)) => {
                        arr.push(Object::Reference(stream_id));
                        dict.set("Contents", Object::Array(arr));
                    }
                    Some(Object::Reference(existing_ref)) => {
                        dict.set(
                            "Contents",
                            Object::Array(vec![
                                Object::Reference(existing_ref),
                                Object::Reference(stream_id),
                            ]),
                        );
                    }
                    _ => {
                        dict.set("Contents", Object::Reference(stream_id));
                    }
                }
            }
        }

        let mut buf = Vec::new();
        doc.save_to(&mut buf)?;
        output.put(context, buf, false).await?;
        context.set_pin_value("result", json!(output)).await?;
        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!("Requires the 'execute' feature"))
    }
}
