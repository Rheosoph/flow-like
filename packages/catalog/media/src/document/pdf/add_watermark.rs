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
pub struct PdfAddWatermarkNode;

impl PdfAddWatermarkNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for PdfAddWatermarkNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "pdf_add_watermark",
            "Add Text Watermark",
            "Overlay a diagonal text watermark on all pages. Default: #FF4343 at 15% opacity.",
            "Document/PDF",
        );
        node.add_icon("/flow/icons/text.svg");
        node.set_scores(
            NodeScores::new()
                .set_privacy(7)
                .set_security(6)
                .set_performance(6)
                .set_governance(8)
                .set_reliability(7)
                .set_cost(10)
                .build(),
        );

        node.add_input_pin("exec_in", "Input", "Trigger", VariableType::Execution);
        node.add_input_pin("template", "Template", "PDF file", VariableType::Struct)
            .set_schema::<FlowPath>()
            .set_options(PinOptions::new().set_enforce_schema(true).build());
        node.add_input_pin("text", "Text", "Watermark text", VariableType::String);
        node.add_input_pin(
            "font_size",
            "Font Size",
            "Font size in points",
            VariableType::Float,
        )
        .set_default_value(Some(json!(60.0)));
        node.add_input_pin(
            "color",
            "Color",
            "Watermark color (hex)",
            VariableType::String,
        )
        .set_default_value(Some(json!("#FF4343")));
        node.add_input_pin("opacity", "Opacity", "0.0 to 1.0", VariableType::Float)
            .set_default_value(Some(json!(0.15)));
        node.add_input_pin(
            "rotation_deg",
            "Rotation",
            "Rotation in degrees",
            VariableType::Float,
        )
        .set_default_value(Some(json!(45.0)));
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
        use crate::document::styles::hex_to_rgb;

        context.deactivate_exec_pin("exec_out").await?;

        let template: FlowPath = context.evaluate_pin("template").await?;
        let text: String = context.evaluate_pin("text").await?;
        let font_size: f64 = context.evaluate_pin("font_size").await?;
        let color: String = context.evaluate_pin("color").await?;
        let opacity: f64 = context.evaluate_pin("opacity").await?;
        let rotation_deg: f64 = context.evaluate_pin("rotation_deg").await?;
        let output: FlowPath = context.evaluate_pin("output").await?;

        let bytes = template.get(context, false).await?;
        let mut doc = Document::load_mem(&bytes)?;

        let (r, g, b) = hex_to_rgb(&color);
        let angle_rad = rotation_deg * std::f64::consts::PI / 180.0;
        let cos_a = angle_rad.cos();
        let sin_a = angle_rad.sin();

        let page_ids: Vec<lopdf::ObjectId> = doc.page_iter().collect();

        for page_id in &page_ids {
            let (width, height) = get_page_size(&doc, *page_id);

            let x = width / 2.0;
            let y = height / 2.0;

            let escaped_text = text
                .replace('\\', "\\\\")
                .replace('(', "\\(")
                .replace(')', "\\)");

            let content_str = format!(
                "q\n/GS0 gs\nBT\n{cos} {sin} {neg_sin} {cos2} {x} {y} Tm\n/F1 {fs} Tf\n{r} {g} {b} rg\n({text}) Tj\nET\nQ\n",
                cos = cos_a,
                sin = sin_a,
                neg_sin = -sin_a,
                cos2 = cos_a,
                x = x,
                y = y,
                fs = font_size,
                r = r,
                g = g,
                b = b,
                text = escaped_text,
            );

            let gs_dict = dictionary! {
                "Type" => Object::Name(b"ExtGState".to_vec()),
                "ca" => Object::Real(opacity as f32),
                "CA" => Object::Real(opacity as f32),
            };
            let gs_id = doc.add_object(Object::Dictionary(gs_dict));

            let font_dict = dictionary! {
                "Type" => Object::Name(b"Font".to_vec()),
                "Subtype" => Object::Name(b"Type1".to_vec()),
                "BaseFont" => Object::Name(b"Helvetica".to_vec()),
            };
            let font_id = doc.add_object(Object::Dictionary(font_dict));

            let resources_dict = dictionary! {
                "ExtGState" => dictionary! { "GS0" => Object::Reference(gs_id) },
                "Font" => dictionary! { "F1" => Object::Reference(font_id) },
            };

            let watermark_stream = Stream::new(dictionary! {}, content_str.into_bytes());
            let stream_id = doc.add_object(Object::Stream(watermark_stream));

            if let Ok(page) = doc.get_object_mut(*page_id)
                && let Object::Dictionary(dict) = page
            {
                if let Ok(existing_resources) = dict.get(b"Resources") {
                    if let Object::Dictionary(res_dict) = existing_resources {
                        let mut merged = res_dict.clone();
                        merged.set(
                            "ExtGState",
                            dictionary! { "GS0" => Object::Reference(gs_id) },
                        );
                        if !merged.has(b"Font") {
                            merged.set("Font", dictionary! { "F1" => Object::Reference(font_id) });
                        } else if let Ok(Object::Dictionary(font_res)) = merged.get_mut(b"Font") {
                            font_res.set("F1", Object::Reference(font_id));
                        }
                        dict.set("Resources", Object::Dictionary(merged));
                    }
                } else {
                    dict.set("Resources", Object::Dictionary(resources_dict));
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

#[cfg(feature = "execute")]
fn get_page_size(doc: &Document, page_id: lopdf::ObjectId) -> (f64, f64) {
    if let Ok(page) = doc.get_object(page_id)
        && let Object::Dictionary(dict) = page
        && let Ok(Object::Array(media_box)) = dict.get(b"MediaBox")
        && media_box.len() == 4
    {
        let w = obj_to_f64(&media_box[2]).unwrap_or(612.0);
        let h = obj_to_f64(&media_box[3]).unwrap_or(792.0);
        return (w, h);
    }
    (612.0, 792.0)
}

#[cfg(feature = "execute")]
fn obj_to_f64(obj: &Object) -> Option<f64> {
    match obj {
        Object::Integer(n) => Some(*n as f64),
        Object::Real(n) => Some(*n as f64),
        _ => None,
    }
}
