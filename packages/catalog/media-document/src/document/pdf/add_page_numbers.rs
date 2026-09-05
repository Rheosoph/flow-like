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
pub struct PdfAddPageNumbersNode;

impl PdfAddPageNumbersNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for PdfAddPageNumbersNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "pdf_add_page_numbers",
            "Add Page Numbers",
            "Add 'Page X of Y' labels to each page of a PDF.",
            "Document/PDF",
        );
        node.set_flowscript_name("pdf", "addPageNumbers");
        node.add_icon("/flow/icons/text.svg");
        node.set_scores(
            NodeScores::new()
                .set_privacy(8)
                .set_security(7)
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
        node.add_input_pin(
            "position",
            "Position",
            "Position: bottom-center, bottom-right, bottom-left",
            VariableType::String,
        )
        .set_default_value(Some(json!("bottom-center")));
        node.add_input_pin(
            "font_size",
            "Font Size",
            "Font size in points",
            VariableType::Float,
        )
        .set_default_value(Some(json!(10.0)));
        node.add_input_pin(
            "margin",
            "Margin",
            "Margin from edge in points",
            VariableType::Float,
        )
        .set_default_value(Some(json!(30.0)));
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
        let position: String = context.evaluate_pin("position").await?;
        let font_size: f64 = context.evaluate_pin("font_size").await?;
        let margin: f64 = context.evaluate_pin("margin").await?;
        let output: FlowPath = context.evaluate_pin("output").await?;

        let bytes = template.get(context, false).await?;
        let mut doc = Document::load_mem(&bytes)?;

        let page_ids: Vec<lopdf::ObjectId> = doc.page_iter().collect();
        let total_pages = page_ids.len();

        let font_dict = dictionary! {
            "Type" => Object::Name(b"Font".to_vec()),
            "Subtype" => Object::Name(b"Type1".to_vec()),
            "BaseFont" => Object::Name(b"Helvetica".to_vec()),
        };
        let font_id = doc.add_object(Object::Dictionary(font_dict));

        for (idx, page_id) in page_ids.iter().enumerate() {
            let page_num = idx + 1;
            let label = format!("Page {} of {}", page_num, total_pages);
            let escaped = label
                .replace('\\', "\\\\")
                .replace('(', "\\(")
                .replace(')', "\\)");

            let (width, _height) = get_page_size(&doc, *page_id);

            let x = match position.as_str() {
                "bottom-left" => margin,
                "bottom-right" => width - margin - (label.len() as f64 * font_size * 0.5),
                _ => (width / 2.0) - (label.len() as f64 * font_size * 0.25),
            };
            let y = margin;

            let content_str = format!(
                "BT\n/F1 {fs} Tf\n{x} {y} Td\n({text}) Tj\nET\n",
                fs = font_size,
                x = x,
                y = y,
                text = escaped,
            );

            let stamp_stream = Stream::new(dictionary! {}, content_str.into_bytes());
            let stream_id = doc.add_object(Object::Stream(stamp_stream));

            if let Ok(page) = doc.get_object_mut(*page_id)
                && let Object::Dictionary(dict) = page
            {
                if let Ok(Object::Dictionary(resources)) = dict.get_mut(b"Resources") {
                    if let Ok(Object::Dictionary(fonts)) = resources.get_mut(b"Font") {
                        fonts.set("F1", Object::Reference(font_id));
                    } else {
                        resources.set("Font", dictionary! { "F1" => Object::Reference(font_id) });
                    }
                } else {
                    dict.set(
                            "Resources",
                            dictionary! { "Font" => dictionary! { "F1" => Object::Reference(font_id) } },
                        );
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
