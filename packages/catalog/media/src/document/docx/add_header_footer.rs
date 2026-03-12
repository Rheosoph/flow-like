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

#[crate::register_node]
#[derive(Default)]
pub struct DocxAddHeaderFooterNode;

impl DocxAddHeaderFooterNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for DocxAddHeaderFooterNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "docx_add_header_footer",
            "Set Header/Footer",
            "Set header and/or footer text in a DOCX document",
            "Document/DOCX",
        );
        node.add_icon("/flow/icons/text.svg");
        node.set_scores(
            NodeScores::new()
                .set_privacy(8)
                .set_security(7)
                .set_performance(7)
                .set_governance(8)
                .set_reliability(7)
                .set_cost(10)
                .build(),
        );

        node.add_input_pin("exec_in", "Input", "Trigger", VariableType::Execution);
        node.add_input_pin("template", "Template", "DOCX file", VariableType::Struct)
            .set_schema::<FlowPath>()
            .set_options(PinOptions::new().set_enforce_schema(true).build());
        node.add_input_pin("header_text", "Header Text", "Text for the header", VariableType::String)
            .set_default_value(Some(json!("")));
        node.add_input_pin("footer_text", "Footer Text", "Text for the footer", VariableType::String)
            .set_default_value(Some(json!("")));
        node.add_input_pin("include_page_number", "Include Page Number", "Add page number to footer", VariableType::Boolean)
            .set_default_value(Some(json!(false)));
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
        let header_text: String = context.evaluate_pin("header_text").await?;
        let footer_text: String = context.evaluate_pin("footer_text").await?;
        let include_page_number: bool = context.evaluate_pin("include_page_number").await?;
        let output: FlowPath = context.evaluate_pin("output").await?;

        let bytes = template.get(context, false).await?;
        let mut files = read_zip(&bytes)?;

        let rels_key = "word/_rels/document.xml.rels".to_string();
        let doc_key = "word/document.xml".to_string();
        let ct_key = "[Content_Types].xml".to_string();

        let mut rels_xml = files
            .get(&rels_key)
            .map(|b| String::from_utf8_lossy(b).to_string())
            .unwrap_or_default();
        let mut doc_xml = files
            .get(&doc_key)
            .map(|b| String::from_utf8_lossy(b).to_string())
            .unwrap_or_default();
        let mut ct_xml = files
            .get(&ct_key)
            .map(|b| String::from_utf8_lossy(b).to_string())
            .unwrap_or_default();

        let mut rid_counter = flow_like_types::regex::Regex::new(r#"Id="rId(\d+)""#)?
            .captures_iter(&rels_xml)
            .filter_map(|c| c[1].parse::<u32>().ok())
            .max()
            .unwrap_or(0);

        let mut sect_refs = String::new();

        if !header_text.is_empty() {
            rid_counter += 1;
            let header_rid = format!("rId{}", rid_counter);
            let header_xml = format!(
                r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:hdr xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">
<w:p><w:pPr><w:jc w:val="center"/></w:pPr><w:r><w:t xml:space="preserve">{}</w:t></w:r></w:p>
</w:hdr>"#,
                quick_xml::escape::escape(&header_text)
            );
            files.insert("word/header1.xml".to_string(), header_xml.into_bytes());

            let rel = format!(
                r#"<Relationship Id="{}" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/header" Target="header1.xml"/>"#,
                header_rid
            );
            rels_xml = rels_xml.replace("</Relationships>", &format!("{}\n</Relationships>", rel));

            if !ct_xml.contains("header1.xml") {
                ct_xml = ct_xml.replace(
                    "</Types>",
                    r#"<Override PartName="/word/header1.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.header+xml"/>
</Types>"#,
                );
            }

            sect_refs.push_str(&format!(
                r#"<w:headerReference w:type="default" r:id="{}"/>"#,
                header_rid
            ));
        }

        if !footer_text.is_empty() || include_page_number {
            rid_counter += 1;
            let footer_rid = format!("rId{}", rid_counter);
            let mut footer_content = String::new();
            if !footer_text.is_empty() {
                footer_content.push_str(&format!(
                    r#"<w:p><w:pPr><w:jc w:val="center"/></w:pPr><w:r><w:t xml:space="preserve">{}</w:t></w:r></w:p>"#,
                    quick_xml::escape::escape(&footer_text)
                ));
            }
            if include_page_number {
                footer_content.push_str(
                    r#"<w:p><w:pPr><w:jc w:val="center"/></w:pPr><w:r><w:fldChar w:fldCharType="begin"/></w:r><w:r><w:instrText xml:space="preserve"> PAGE </w:instrText></w:r><w:r><w:fldChar w:fldCharType="end"/></w:r></w:p>"#,
                );
            }

            let footer_xml = format!(
                r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:ftr xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">
{}</w:ftr>"#,
                footer_content
            );
            files.insert("word/footer1.xml".to_string(), footer_xml.into_bytes());

            let rel = format!(
                r#"<Relationship Id="{}" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/footer" Target="footer1.xml"/>"#,
                footer_rid
            );
            rels_xml = rels_xml.replace("</Relationships>", &format!("{}\n</Relationships>", rel));

            if !ct_xml.contains("footer1.xml") {
                ct_xml = ct_xml.replace(
                    "</Types>",
                    r#"<Override PartName="/word/footer1.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.footer+xml"/>
</Types>"#,
                );
            }

            sect_refs.push_str(&format!(
                r#"<w:footerReference w:type="default" r:id="{}"/>"#,
                footer_rid
            ));
        }

        if !sect_refs.is_empty() {
            if let Some(pos) = doc_xml.find("<w:sectPr") {
                if let Some(close) = doc_xml[pos..].find('>') {
                    let insert_pos = pos + close + 1;
                    doc_xml.insert_str(insert_pos, &sect_refs);
                }
            }
        }

        files.insert(rels_key, rels_xml.into_bytes());
        files.insert(doc_key, doc_xml.into_bytes());
        files.insert(ct_key, ct_xml.into_bytes());

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
