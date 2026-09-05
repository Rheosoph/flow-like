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

#[crate::register_node]
#[derive(Default)]
pub struct DocxAddHyperlinkNode;

impl DocxAddHyperlinkNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for DocxAddHyperlinkNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "docx_add_hyperlink",
            "Add Hyperlink",
            "Append a hyperlink to a DOCX document. Default color: #FF4343.",
            "Document/DOCX",
        );
        node.set_flowscript_name("docx", "addHyperlink");
        node.add_icon("/flow/icons/text.svg");
        node.set_scores(
            NodeScores::new()
                .set_privacy(8)
                .set_security(6)
                .set_performance(8)
                .set_governance(8)
                .set_reliability(8)
                .set_cost(10)
                .build(),
        );

        node.add_input_pin("exec_in", "Input", "Trigger", VariableType::Execution);
        node.add_input_pin("template", "Template", "DOCX file", VariableType::Struct)
            .set_schema::<FlowPath>()
            .set_options(PinOptions::new().set_enforce_schema(true).build());
        node.add_input_pin(
            "display_text",
            "Display Text",
            "Visible link text",
            VariableType::String,
        );
        node.add_input_pin("url", "URL", "Hyperlink URL", VariableType::String);
        node.add_input_pin(
            "font_color",
            "Font Color",
            "Link color (hex)",
            VariableType::String,
        )
        .set_default_value(Some(json!(defaults::LINK_COLOR)));
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
        use crate::document::styles::hex_to_ooxml;

        context.deactivate_exec_pin("exec_out").await?;

        let template: FlowPath = context.evaluate_pin("template").await?;
        let display_text: String = context.evaluate_pin("display_text").await?;
        let url: String = context.evaluate_pin("url").await?;
        let font_color: String = context.evaluate_pin("font_color").await?;
        let output: FlowPath = context.evaluate_pin("output").await?;

        let bytes = template.get(context, false).await?;
        let mut files = read_zip(&bytes)?;

        let rels_key = "word/_rels/document.xml.rels".to_string();
        let doc_key = "word/document.xml".to_string();

        let mut rels_xml = files
            .get(&rels_key)
            .map(|b| String::from_utf8_lossy(b).to_string())
            .unwrap_or_default();

        let rid_num = flow_like_types::regex::Regex::new(r#"Id="rId(\d+)""#)?
            .captures_iter(&rels_xml)
            .filter_map(|c| c[1].parse::<u32>().ok())
            .max()
            .unwrap_or(0)
            + 1;
        let r_id = format!("rId{}", rid_num);

        let new_rel = format!(
            r#"<Relationship Id="{}" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/hyperlink" Target="{}" TargetMode="External"/>"#,
            r_id,
            quick_xml::escape::escape(&url)
        );
        rels_xml = rels_xml.replace(
            "</Relationships>",
            &format!("{}\n</Relationships>", new_rel),
        );
        files.insert(rels_key, rels_xml.into_bytes());

        let color = hex_to_ooxml(&font_color);
        let hyperlink_xml = format!(
            r#"<w:p><w:hyperlink r:id="{}"><w:r><w:rPr><w:rStyle w:val="Hyperlink"/><w:color w:val="{}"/><w:u w:val="single"/></w:rPr><w:t xml:space="preserve">{}</w:t></w:r></w:hyperlink></w:p>"#,
            r_id,
            color,
            quick_xml::escape::escape(&display_text)
        );

        if let Some(doc_data) = files.get(&doc_key).cloned() {
            let mut xml = String::from_utf8_lossy(&doc_data).to_string();
            if let Some(pos) = xml.rfind("<w:sectPr") {
                xml.insert_str(pos, &hyperlink_xml);
            } else if let Some(pos) = xml.rfind("</w:body>") {
                xml.insert_str(pos, &hyperlink_xml);
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
