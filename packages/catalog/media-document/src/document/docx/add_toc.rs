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
pub struct DocxAddTocNode;

impl DocxAddTocNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for DocxAddTocNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "docx_add_toc",
            "Add Table of Contents",
            "Insert a TOC field that Word will populate on open",
            "Document/DOCX",
        );
        node.set_flowscript_name("docx", "addToc");
        node.add_icon("/flow/icons/text.svg");
        node.set_scores(
            NodeScores::new()
                .set_privacy(9)
                .set_security(8)
                .set_performance(8)
                .set_governance(8)
                .set_reliability(7)
                .set_cost(10)
                .build(),
        );

        node.add_input_pin("exec_in", "Input", "Trigger", VariableType::Execution);
        node.add_input_pin("template", "Template", "DOCX file", VariableType::Struct)
            .set_schema::<FlowPath>()
            .set_options(PinOptions::new().set_enforce_schema(true).build());
        node.add_input_pin("title", "Title", "TOC title", VariableType::String)
            .set_default_value(Some(json!("Table of Contents")));
        node.add_input_pin(
            "max_level",
            "Max Level",
            "Maximum heading level to include (1-6)",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(3)));
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
        let title: String = context.evaluate_pin("title").await?;
        let max_level: i64 = context.evaluate_pin("max_level").await?;
        let output: FlowPath = context.evaluate_pin("output").await?;

        let bytes = template.get(context, false).await?;
        let mut files = read_zip(&bytes)?;

        let toc_xml = format!(
            r#"<w:p><w:pPr><w:pStyle w:val="Heading1"/></w:pPr><w:r><w:t>{title}</w:t></w:r></w:p><w:p><w:r><w:fldChar w:fldCharType="begin"/></w:r><w:r><w:instrText xml:space="preserve"> TOC \o "1-{max_level}" \h \z \u </w:instrText></w:r><w:r><w:fldChar w:fldCharType="separate"/></w:r><w:r><w:t>Update this field to see the table of contents.</w:t></w:r><w:r><w:fldChar w:fldCharType="end"/></w:r></w:p>"#,
            title = quick_xml::escape::escape(&title),
            max_level = max_level.clamp(1, 6),
        );

        let doc_key = "word/document.xml".to_string();
        if let Some(doc_data) = files.get(&doc_key).cloned() {
            let mut xml = String::from_utf8_lossy(&doc_data).to_string();
            if let Some(pos) = xml.find("<w:body>") {
                let insert_pos = pos + "<w:body>".len();
                xml.insert_str(insert_pos, &toc_xml);
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
