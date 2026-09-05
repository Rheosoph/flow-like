#[cfg(feature = "execute")]
use lopdf::{Document, Object};

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

#[crate::register_node]
#[derive(Default)]
pub struct PdfExtractTextNode;

impl PdfExtractTextNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for PdfExtractTextNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "pdf_extract_text",
            "Extract Text",
            "Extract all text content from a PDF document.",
            "Document/PDF",
        );
        node.set_flowscript_name("pdf", "extractText");
        node.add_icon("/flow/icons/text.svg");
        node.set_scores(
            NodeScores::new()
                .set_privacy(6)
                .set_security(7)
                .set_performance(5)
                .set_governance(8)
                .set_reliability(6)
                .set_cost(10)
                .build(),
        );

        node.add_input_pin("exec_in", "Input", "Trigger", VariableType::Execution);
        node.add_input_pin("template", "Template", "PDF file", VariableType::Struct)
            .set_schema::<FlowPath>()
            .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin("exec_out", "Done", "Continues", VariableType::Execution);
        node.add_output_pin("text", "Text", "Extracted text", VariableType::String);

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let template: FlowPath = context.evaluate_pin("template").await?;
        let bytes = template.get(context, false).await?;
        let doc = Document::load_mem(&bytes)?;

        let mut result = String::new();

        for (page_num, page_id) in doc.page_iter().enumerate() {
            if page_num > 0 {
                result.push_str("\n\n--- Page ");
                result.push_str(&(page_num + 1).to_string());
                result.push_str(" ---\n\n");
            }

            let text = extract_page_text(&doc, page_id);
            result.push_str(&text);
        }

        context.set_pin_value("text", json!(result)).await?;
        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!("Requires the 'execute' feature"))
    }
}

#[cfg(feature = "execute")]
fn extract_page_text(doc: &Document, page_id: lopdf::ObjectId) -> String {
    let content_data = match doc.get_page_content(page_id) {
        Ok(data) => data,
        Err(_) => return String::new(),
    };

    let operations = match lopdf::content::Content::decode(&content_data) {
        Ok(content) => content.operations,
        Err(_) => return String::new(),
    };

    let mut text = String::new();

    for op in &operations {
        match op.operator.as_str() {
            "Tj" => {
                if let Some(Object::String(bytes, _)) = op.operands.first() {
                    text.push_str(&String::from_utf8_lossy(bytes));
                }
            }
            "TJ" => {
                if let Some(Object::Array(arr)) = op.operands.first() {
                    for item in arr {
                        if let Object::String(bytes, _) = item {
                            text.push_str(&String::from_utf8_lossy(bytes));
                        }
                    }
                }
            }
            "'" | "\"" => {
                text.push('\n');
                if let Some(Object::String(bytes, _)) = op.operands.last() {
                    text.push_str(&String::from_utf8_lossy(bytes));
                }
            }
            _ => {}
        }
    }

    text
}
