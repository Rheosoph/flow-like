use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic, NodeScores},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_catalog_core::FlowPath;
use flow_like_types::{async_trait, json::json};

#[cfg(feature = "execute")]
use crate::document::openxml::read_zip;

#[crate::register_node]
#[derive(Default)]
pub struct DocxExtractTextNode;

impl DocxExtractTextNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for DocxExtractTextNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "docx_extract_text",
            "Extract Text",
            "Extract all text content from a DOCX file as plain text",
            "Document/DOCX",
        );
        node.add_icon("/flow/icons/text.svg");
        node.set_scores(
            NodeScores::new()
                .set_privacy(8)
                .set_security(7)
                .set_performance(8)
                .set_governance(8)
                .set_reliability(9)
                .set_cost(10)
                .build(),
        );

        node.add_input_pin("exec_in", "Input", "Trigger", VariableType::Execution);
        node.add_input_pin(
            "template",
            "Template",
            "DOCX file to extract from",
            VariableType::Struct,
        )
        .set_schema::<FlowPath>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin("exec_out", "Done", "Continues", VariableType::Execution);
        node.add_output_pin(
            "text",
            "Text",
            "Extracted text content",
            VariableType::String,
        );

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let template: FlowPath = context.evaluate_pin("template").await?;
        let bytes = template.get(context, false).await?;
        let files = read_zip(&bytes)?;

        let mut all_text = String::new();

        let mut targets: Vec<&String> = files
            .keys()
            .filter(|k| {
                *k == "word/document.xml"
                    || (k.starts_with("word/header") && k.ends_with(".xml"))
                    || (k.starts_with("word/footer") && k.ends_with(".xml"))
            })
            .collect();
        targets.sort();

        for key in targets {
            if let Some(data) = files.get(key) {
                let xml = String::from_utf8_lossy(data);
                let text = extract_text_from_docx_xml(&xml);
                if !text.is_empty() {
                    if !all_text.is_empty() {
                        all_text.push_str("\n\n");
                    }
                    all_text.push_str(&text);
                }
            }
        }

        context.set_pin_value("text", json!(all_text)).await?;
        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!("Requires the 'execute' feature"))
    }
}

#[cfg(feature = "execute")]
fn extract_text_from_docx_xml(xml: &str) -> String {
    let mut paragraphs: Vec<String> = Vec::new();
    let mut current_para = String::new();
    let mut in_t = false;
    let mut in_p = false;

    let mut reader = quick_xml::Reader::from_str(xml);
    let mut buf = Vec::new();

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(quick_xml::events::Event::Start(ref e)) => {
                let name = e.name();
                if name.as_ref() == b"w:p" {
                    in_p = true;
                    current_para.clear();
                } else if name.as_ref() == b"w:t" {
                    in_t = true;
                }
            }
            Ok(quick_xml::events::Event::End(ref e)) => {
                let name = e.name();
                if name.as_ref() == b"w:t" {
                    in_t = false;
                } else if name.as_ref() == b"w:p" {
                    in_p = false;
                    if !current_para.is_empty() {
                        paragraphs.push(current_para.clone());
                    }
                }
            }
            Ok(quick_xml::events::Event::Text(ref e)) if in_t && in_p => {
                if let Ok(text) = e.decode() {
                    current_para.push_str(&text);
                }
            }
            Ok(quick_xml::events::Event::Eof) => break,
            Err(_) => break,
            _ => {}
        }
        buf.clear();
    }

    paragraphs.join("\n")
}
