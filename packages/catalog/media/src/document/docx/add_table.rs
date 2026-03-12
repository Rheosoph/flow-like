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
use crate::document::styles::defaults;
#[cfg(feature = "execute")]
use crate::document::styles::{hex_to_ooxml, pt_to_half_points, cm_to_twips};

#[crate::register_node]
#[derive(Default)]
pub struct DocxAddTableNode;

impl DocxAddTableNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for DocxAddTableNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "docx_add_table",
            "Add Table",
            "Insert a styled table from JSON data. Default: branded header with #FF4343, zebra rows.",
            "Document/DOCX",
        );
        node.add_icon("/flow/icons/text.svg");
        node.set_scores(
            NodeScores::new()
                .set_privacy(8)
                .set_security(7)
                .set_performance(7)
                .set_governance(8)
                .set_reliability(8)
                .set_cost(10)
                .build(),
        );

        node.add_input_pin("exec_in", "Input", "Trigger", VariableType::Execution);
        node.add_input_pin("template", "Template", "DOCX file to add table to", VariableType::Struct)
            .set_schema::<FlowPath>()
            .set_options(PinOptions::new().set_enforce_schema(true).build());
        node.add_input_pin("data", "Data", "JSON array of arrays (first row = headers if header_row=true)", VariableType::String);
        node.add_input_pin("header_row", "Header Row", "Style first row as header", VariableType::Boolean)
            .set_default_value(Some(json!(true)));
        node.add_input_pin("alternate_rows", "Alternate Rows", "Zebra striping", VariableType::Boolean)
            .set_default_value(Some(json!(true)));
        node.add_input_pin("border_color", "Border Color", "Table border color (hex)", VariableType::String)
            .set_default_value(Some(json!(defaults::BORDER)));
        node.add_input_pin("font_size", "Font Size", "Font size in points", VariableType::Float)
            .set_default_value(Some(json!(10.0)));
        node.add_input_pin("output", "Output Path", "Where to save", VariableType::Struct)
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
        let data_json: String = context.evaluate_pin("data").await?;
        let header_row: bool = context.evaluate_pin("header_row").await?;
        let alternate_rows: bool = context.evaluate_pin("alternate_rows").await?;
        let border_color: String = context.evaluate_pin("border_color").await?;
        let font_size: f64 = context.evaluate_pin("font_size").await?;
        let output: FlowPath = context.evaluate_pin("output").await?;

        let data: Vec<Vec<String>> = flow_like_types::json::from_str(&data_json)?;
        if data.is_empty() {
            return Err(flow_like_types::anyhow!("Empty table data"));
        }

        let bytes = template.get(context, false).await?;
        let mut files = read_zip(&bytes)?;

        let table_xml = build_table(
            &data,
            header_row,
            alternate_rows,
            &border_color,
            font_size as f32,
        );

        let doc_key = "word/document.xml".to_string();
        if let Some(doc_data) = files.get(&doc_key).cloned() {
            let mut xml = String::from_utf8_lossy(&doc_data).to_string();
            if let Some(pos) = xml.rfind("<w:sectPr") {
                xml.insert_str(pos, &table_xml);
            } else if let Some(pos) = xml.rfind("</w:body>") {
                xml.insert_str(pos, &table_xml);
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
fn build_table(
    data: &[Vec<String>],
    header_row: bool,
    alternate_rows: bool,
    border_color: &str,
    font_size_pt: f32,
) -> String {
    let col_count = data.iter().map(|r| r.len()).max().unwrap_or(1);
    let total_width = cm_to_twips(16.0);
    let col_width = total_width / col_count as i32;
    let border = hex_to_ooxml(border_color);
    let half_pts = pt_to_half_points(font_size_pt);

    let mut xml = String::new();
    xml.push_str("<w:tbl>");

    xml.push_str("<w:tblPr>");
    xml.push_str(r#"<w:tblW w:w="0" w:type="auto"/>"#);
    let border_xml = format!(
        r#"<w:tblBorders>
<w:top w:val="single" w:sz="4" w:space="0" w:color="{b}"/>
<w:left w:val="single" w:sz="4" w:space="0" w:color="{b}"/>
<w:bottom w:val="single" w:sz="4" w:space="0" w:color="{b}"/>
<w:right w:val="single" w:sz="4" w:space="0" w:color="{b}"/>
<w:insideH w:val="single" w:sz="4" w:space="0" w:color="{b}"/>
<w:insideV w:val="single" w:sz="4" w:space="0" w:color="{b}"/>
</w:tblBorders>"#,
        b = border
    );
    xml.push_str(&border_xml);
    xml.push_str("<w:tblLook w:val=\"04A0\" w:firstRow=\"1\" w:lastRow=\"0\" w:firstColumn=\"1\" w:lastColumn=\"0\" w:noHBand=\"0\" w:noVBand=\"1\"/>");
    xml.push_str("</w:tblPr>");

    xml.push_str("<w:tblGrid>");
    for _ in 0..col_count {
        xml.push_str(&format!("<w:gridCol w:w=\"{}\"/>", col_width));
    }
    xml.push_str("</w:tblGrid>");

    for (row_idx, row) in data.iter().enumerate() {
        let is_header = header_row && row_idx == 0;
        let bg_color = if is_header {
            hex_to_ooxml(defaults::PRIMARY)
        } else if alternate_rows && row_idx % 2 == 0 {
            hex_to_ooxml(defaults::SURFACE)
        } else {
            hex_to_ooxml(defaults::BACKGROUND)
        };
        let text_color = if is_header {
            "FFFFFF".to_string()
        } else {
            hex_to_ooxml(defaults::TEXT)
        };

        xml.push_str("<w:tr>");
        for col_idx in 0..col_count {
            let cell_text = row.get(col_idx).map(|s| s.as_str()).unwrap_or("");
            xml.push_str("<w:tc>");
            xml.push_str(&format!(
                r#"<w:tcPr><w:shd w:val="clear" w:color="auto" w:fill="{}"/></w:tcPr>"#,
                bg_color
            ));
            xml.push_str("<w:p><w:r><w:rPr>");
            xml.push_str(&format!(
                r#"<w:sz w:val="{}"/><w:szCs w:val="{}"/>"#,
                half_pts, half_pts
            ));
            xml.push_str(&format!(r#"<w:color w:val="{}"/>"#, text_color));
            if is_header {
                xml.push_str("<w:b/>");
            }
            xml.push_str("</w:rPr>");
            xml.push_str(&format!(
                "<w:t xml:space=\"preserve\">{}</w:t>",
                quick_xml::escape::escape(cell_text)
            ));
            xml.push_str("</w:r></w:p>");
            xml.push_str("</w:tc>");
        }
        xml.push_str("</w:tr>");
    }

    xml.push_str("</w:tbl>");
    xml
}
