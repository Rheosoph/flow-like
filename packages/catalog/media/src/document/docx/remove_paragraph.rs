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
pub struct DocxRemoveParagraphNode;

impl DocxRemoveParagraphNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for DocxRemoveParagraphNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "docx_remove_paragraph",
            "Remove Paragraph",
            "Remove paragraphs containing a specific placeholder. Useful for conditional content.",
            "Document/DOCX",
        );
        node.set_flowscript_name("docx", "removeParagraph");
        node.add_icon("/flow/icons/text.svg");
        node.set_scores(
            NodeScores::new()
                .set_privacy(8)
                .set_security(7)
                .set_performance(8)
                .set_governance(8)
                .set_reliability(8)
                .set_cost(10)
                .build(),
        );

        node.add_input_pin("exec_in", "Input", "Trigger", VariableType::Execution);
        node.add_input_pin(
            "template",
            "Template",
            "DOCX file to modify",
            VariableType::Struct,
        )
        .set_schema::<FlowPath>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());
        node.add_input_pin(
            "placeholder",
            "Placeholder",
            "Text to search for — paragraphs containing this are removed",
            VariableType::String,
        );
        node.add_input_pin(
            "output",
            "Output Path",
            "Where to save the result",
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
        let placeholder: String = context.evaluate_pin("placeholder").await?;
        let output: FlowPath = context.evaluate_pin("output").await?;

        let bytes = template.get(context, false).await?;
        let mut files = read_zip(&bytes)?;

        let targets: Vec<String> = files
            .keys()
            .filter(|k| {
                *k == "word/document.xml"
                    || (k.starts_with("word/header") && k.ends_with(".xml"))
                    || (k.starts_with("word/footer") && k.ends_with(".xml"))
            })
            .cloned()
            .collect();

        for key in targets {
            if let Some(data) = files.get(&key).cloned() {
                let xml = String::from_utf8_lossy(&data).to_string();
                let updated = remove_paragraphs_containing(&xml, &placeholder);
                files.insert(key, updated.into_bytes());
            }
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
fn remove_paragraphs_containing(xml: &str, placeholder: &str) -> String {
    let mut result = String::new();
    let mut search_pos = 0;

    while search_pos < xml.len() {
        let p_start = xml[search_pos..]
            .find("<w:p ")
            .or_else(|| xml[search_pos..].find("<w:p>"));
        let p_start = match p_start {
            Some(offset) => search_pos + offset,
            None => {
                result.push_str(&xml[search_pos..]);
                break;
            }
        };

        result.push_str(&xml[search_pos..p_start]);

        let p_end = xml[p_start..].find("</w:p>");
        let p_end = match p_end {
            Some(offset) => p_start + offset + "</w:p>".len(),
            None => {
                result.push_str(&xml[p_start..]);
                break;
            }
        };

        let paragraph = &xml[p_start..p_end];
        if !paragraph.contains(placeholder) {
            result.push_str(paragraph);
        }

        search_pos = p_end;
    }

    result
}
