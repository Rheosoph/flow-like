use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic, NodeScores},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_catalog_core::FlowPath;
use flow_like_types::{async_trait, json::json};

#[cfg(feature = "execute")]
use crate::document::openxml::{read_zip, write_zip};
#[cfg(feature = "execute")]
use crate::document::styles::{
    ParagraphStyle, TextAlignment, defaults, hex_to_ooxml, pt_to_half_points,
};

#[crate::register_node]
#[derive(Default)]
pub struct DocxAddParagraphNode;

impl DocxAddParagraphNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for DocxAddParagraphNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "docx_add_paragraph",
            "Add Paragraph",
            "Append a styled paragraph to a DOCX document",
            "Document/DOCX",
        );
        node.add_icon("/flow/icons/text.svg");
        node.set_scores(
            NodeScores::new()
                .set_privacy(8)
                .set_security(8)
                .set_performance(7)
                .set_governance(8)
                .set_reliability(8)
                .set_cost(10)
                .build(),
        );

        node.add_input_pin("exec_in", "Input", "Trigger", VariableType::Execution);
        node.add_input_pin(
            "template",
            "Template",
            "DOCX file to append to",
            VariableType::Struct,
        )
        .set_schema::<FlowPath>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());
        node.add_input_pin(
            "text",
            "Text",
            "Paragraph text (supports markdown bold/italic)",
            VariableType::String,
        );
        node.add_input_pin(
            "style",
            "Style",
            "Paragraph style: Normal, Heading1-6, Title, Subtitle, Quote",
            VariableType::String,
        )
        .set_default_value(Some(json!("Normal")));
        node.add_input_pin(
            "font_family",
            "Font Family",
            "Override font",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));
        node.add_input_pin(
            "font_size",
            "Font Size",
            "Override size in points (0 = use style default)",
            VariableType::Float,
        )
        .set_default_value(Some(json!(0.0)));
        node.add_input_pin(
            "font_color",
            "Font Color",
            "Override text color (hex)",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));
        node.add_input_pin("bold", "Bold", "Force bold", VariableType::Boolean)
            .set_default_value(Some(json!(false)));
        node.add_input_pin("italic", "Italic", "Force italic", VariableType::Boolean)
            .set_default_value(Some(json!(false)));
        node.add_input_pin(
            "alignment",
            "Alignment",
            "Text alignment: Left, Center, Right, Justify",
            VariableType::String,
        )
        .set_default_value(Some(json!("Left")));
        node.add_input_pin(
            "output",
            "Output Path",
            "Where to save",
            VariableType::Struct,
        )
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
        let text: String = context.evaluate_pin("text").await?;
        let style_str: String = context.evaluate_pin("style").await?;
        let font_family: String = context.evaluate_pin("font_family").await?;
        let font_size: f64 = context.evaluate_pin("font_size").await?;
        let font_color: String = context.evaluate_pin("font_color").await?;
        let bold: bool = context.evaluate_pin("bold").await?;
        let italic: bool = context.evaluate_pin("italic").await?;
        let alignment_str: String = context.evaluate_pin("alignment").await?;
        let output: FlowPath = context.evaluate_pin("output").await?;

        let bytes = template.get(context, false).await?;
        let mut files = read_zip(&bytes)?;

        let style = match style_str.as_str() {
            "Heading1" => ParagraphStyle::Heading1,
            "Heading2" => ParagraphStyle::Heading2,
            "Heading3" => ParagraphStyle::Heading3,
            "Heading4" => ParagraphStyle::Heading4,
            "Heading5" => ParagraphStyle::Heading5,
            "Heading6" => ParagraphStyle::Heading6,
            "Title" => ParagraphStyle::Title,
            "Subtitle" => ParagraphStyle::Subtitle,
            "Quote" => ParagraphStyle::Quote,
            _ => ParagraphStyle::Normal,
        };

        let alignment = match alignment_str.as_str() {
            "Center" => TextAlignment::Center,
            "Right" => TextAlignment::Right,
            "Justify" => TextAlignment::Justify,
            _ => TextAlignment::Left,
        };

        let paragraph_xml = build_paragraph(
            &text,
            &style,
            &alignment,
            if font_family.is_empty() {
                None
            } else {
                Some(&font_family)
            },
            if font_size > 0.0 {
                Some(font_size as f32)
            } else {
                None
            },
            if font_color.is_empty() {
                None
            } else {
                Some(&font_color)
            },
            bold,
            italic,
        );

        let doc_key = "word/document.xml".to_string();
        if let Some(doc_data) = files.get(&doc_key).cloned() {
            let mut xml = String::from_utf8_lossy(&doc_data).to_string();
            if let Some(pos) = xml.rfind("<w:sectPr") {
                xml.insert_str(pos, &paragraph_xml);
            } else if let Some(pos) = xml.rfind("</w:body>") {
                xml.insert_str(pos, &paragraph_xml);
            }
            files.insert(doc_key, xml.into_bytes());
        }

        let result_bytes = write_zip(&files)?;
        output.put(context, result_bytes, false).await?;
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
#[allow(clippy::too_many_arguments)]
fn build_paragraph(
    text: &str,
    style: &ParagraphStyle,
    alignment: &TextAlignment,
    font_family: Option<&str>,
    font_size_pt: Option<f32>,
    font_color: Option<&str>,
    bold: bool,
    italic: bool,
) -> String {
    let escaped = quick_xml::escape::escape(text);

    let mut p_pr = String::new();
    p_pr.push_str("<w:pPr>");

    let style_id = style.to_style_id();
    if style_id != "Normal" {
        p_pr.push_str(&format!(r#"<w:pStyle w:val="{}"/>"#, style_id));
    }
    p_pr.push_str(&format!(r#"<w:jc w:val="{}"/>"#, alignment.to_ooxml_docx()));
    p_pr.push_str("</w:pPr>");

    let mut r_pr = String::new();
    r_pr.push_str("<w:rPr>");

    let font = font_family.unwrap_or(defaults::FONT_SANS);
    r_pr.push_str(&format!(
        r#"<w:rFonts w:ascii="{f}" w:hAnsi="{f}"/>"#,
        f = quick_xml::escape::escape(font)
    ));

    let size = font_size_pt.unwrap_or_else(|| style.font_size_pt());
    let half_pts = pt_to_half_points(size);
    r_pr.push_str(&format!(
        r#"<w:sz w:val="{}"/><w:szCs w:val="{}"/>"#,
        half_pts, half_pts
    ));

    let color = font_color.unwrap_or(defaults::TEXT);
    r_pr.push_str(&format!(r#"<w:color w:val="{}"/>"#, hex_to_ooxml(color)));

    if bold {
        r_pr.push_str("<w:b/>");
    }
    if italic {
        r_pr.push_str("<w:i/>");
    }

    r_pr.push_str("</w:rPr>");

    format!(
        "<w:p>{}<w:r>{}<w:t xml:space=\"preserve\">{}</w:t></w:r></w:p>",
        p_pr, r_pr, escaped
    )
}
