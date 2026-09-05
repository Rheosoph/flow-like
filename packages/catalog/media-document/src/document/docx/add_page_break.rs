use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic, NodeScores},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_catalog_core::FlowPath;
use flow_like_types::async_trait;
#[cfg(feature = "execute")]
use flow_like_types::json::json;

#[cfg(feature = "execute")]
use crate::document::openxml::{read_zip, write_zip};

#[crate::register_node]
#[derive(Default)]
pub struct DocxAddPageBreakNode;

impl DocxAddPageBreakNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for DocxAddPageBreakNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "docx_add_page_break",
            "Add Page Break",
            "Insert a page break into a DOCX document",
            "Document/DOCX",
        );
        node.set_flowscript_name("docx", "addPageBreak");
        node.add_icon("/flow/icons/text.svg");
        node.set_scores(
            NodeScores::new()
                .set_privacy(9)
                .set_security(8)
                .set_performance(9)
                .set_governance(9)
                .set_reliability(9)
                .set_cost(10)
                .build(),
        );

        node.add_input_pin("exec_in", "Input", "Trigger", VariableType::Execution);
        node.add_input_pin("template", "Template", "DOCX file", VariableType::Struct)
            .set_schema::<FlowPath>()
            .set_options(PinOptions::new().set_enforce_schema(true).build());
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
        let output: FlowPath = context.evaluate_pin("output").await?;

        let bytes = template.get(context, false).await?;
        let mut files = read_zip(&bytes)?;

        let page_break = r#"<w:p><w:r><w:br w:type="page"/></w:r></w:p>"#;

        let doc_key = "word/document.xml".to_string();
        if let Some(doc_data) = files.get(&doc_key).cloned() {
            let mut xml = String::from_utf8_lossy(&doc_data).to_string();
            if let Some(pos) = xml.rfind("<w:sectPr") {
                xml.insert_str(pos, page_break);
            } else if let Some(pos) = xml.rfind("</w:body>") {
                xml.insert_str(pos, page_break);
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
