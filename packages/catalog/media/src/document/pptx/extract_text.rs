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
pub struct PptxExtractTextNode;

impl PptxExtractTextNode {
    pub fn new() -> Self {
        Self
    }
}

#[cfg(feature = "execute")]
fn extract_slide_number(key: &str) -> Option<u32> {
    let name = key.strip_prefix("ppt/slides/slide")?.strip_suffix(".xml")?;
    name.parse().ok()
}

#[async_trait]
impl NodeLogic for PptxExtractTextNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "pptx_extract_text",
            "Extract Text",
            "Extract all text content from all slides as plain text",
            "Document/PPTX",
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

        node.add_input_pin(
            "exec_in",
            "Input",
            "Trigger execution",
            VariableType::Execution,
        );

        node.add_input_pin(
            "template",
            "Template",
            "Path to the PPTX file",
            VariableType::Struct,
        )
        .set_schema::<FlowPath>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin(
            "exec_out",
            "Done",
            "Execution continues after extraction",
            VariableType::Execution,
        );

        node.add_output_pin(
            "text",
            "Text",
            "Extracted text from all slides",
            VariableType::String,
        );

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        use flow_like_types::regex::Regex;

        context.deactivate_exec_pin("exec_out").await?;

        let template: FlowPath = context.evaluate_pin("template").await?;
        let template_bytes = template.get(context, false).await?;
        let files = read_zip(&template_bytes)?;

        let mut slide_keys: Vec<String> = files
            .keys()
            .filter(|k| k.starts_with("ppt/slides/slide") && k.ends_with(".xml"))
            .cloned()
            .collect();
        slide_keys.sort_by_key(|k| extract_slide_number(k).unwrap_or(0));

        let re = Regex::new(r"<a:t>([^<]*)</a:t>")?;
        let mut all_text = Vec::new();

        for key in &slide_keys {
            if let Some(bytes) = files.get(key) {
                let xml = String::from_utf8_lossy(bytes);
                let mut slide_text = Vec::new();
                for cap in re.captures_iter(&xml) {
                    if let Some(m) = cap.get(1) {
                        let text = m.as_str().trim();
                        if !text.is_empty() {
                            slide_text.push(text.to_string());
                        }
                    }
                }
                if !slide_text.is_empty() {
                    all_text.push(slide_text.join(" "));
                }
            }
        }

        let result = all_text.join("\n\n");
        context.set_pin_value("text", json!(result)).await?;
        context.activate_exec_pin("exec_out").await?;

        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "This node requires the 'execute' feature"
        ))
    }
}
