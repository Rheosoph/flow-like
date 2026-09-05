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
pub struct DocxMergeNode;

impl DocxMergeNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for DocxMergeNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "docx_merge",
            "Merge DOCX",
            "Concatenate multiple DOCX documents into one, with optional page breaks between them",
            "Document/DOCX",
        );
        node.set_flowscript_name("docx", "merge");
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
        node.add_input_pin(
            "documents",
            "Documents",
            "Array of DOCX file paths to merge in order",
            VariableType::Struct,
        )
        .set_schema::<FlowPath>()
        .set_value_type(flow_like::flow::pin::ValueType::Array);
        node.add_input_pin(
            "page_break",
            "Page Break Between",
            "Insert page break between documents",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(true)));
        node.add_input_pin(
            "output",
            "Output Path",
            "Where to save the merged file",
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

        let documents: Vec<FlowPath> = context.evaluate_pin("documents").await?;
        let page_break: bool = context.evaluate_pin("page_break").await?;
        let output: FlowPath = context.evaluate_pin("output").await?;

        if documents.is_empty() {
            return Err(flow_like_types::anyhow!("No documents provided"));
        }

        let first_bytes = documents[0].get(context, false).await?;
        let mut base_files = read_zip(&first_bytes)?;

        let body_close = "</w:body>";
        let page_break_xml = r#"<w:p><w:r><w:br w:type="page"/></w:r></w:p>"#;

        for doc_path in documents.iter().skip(1) {
            let doc_bytes = doc_path.get(context, false).await?;
            let doc_files = read_zip(&doc_bytes)?;

            if let Some(add_body) = doc_files.get("word/document.xml") {
                let add_xml = String::from_utf8_lossy(add_body);
                let content = extract_body_content(&add_xml);

                if let Some(base_body) = base_files.get("word/document.xml").cloned() {
                    let mut base_xml = String::from_utf8_lossy(&base_body).to_string();
                    if let Some(pos) = base_xml.rfind(body_close) {
                        let mut insert = String::new();
                        if page_break {
                            insert.push_str(page_break_xml);
                        }
                        insert.push_str(&content);
                        base_xml.insert_str(pos, &insert);
                        base_files.insert("word/document.xml".to_string(), base_xml.into_bytes());
                    }
                }
            }
        }

        let result_bytes = write_zip(&base_files)?;
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
fn extract_body_content(xml: &str) -> String {
    let body_start = xml.find("<w:body>");
    let body_end = xml.rfind("</w:body>");
    match (body_start, body_end) {
        (Some(start), Some(end)) => {
            let content_start = start + "<w:body>".len();
            let content = &xml[content_start..end];
            let sect_start = content.rfind("<w:sectPr");
            match sect_start {
                Some(pos) => content[..pos].to_string(),
                None => content.to_string(),
            }
        }
        _ => String::new(),
    }
}
