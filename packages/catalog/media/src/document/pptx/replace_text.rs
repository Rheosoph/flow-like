use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic, NodeScores},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_catalog_core::FlowPath;
use flow_like_types::{async_trait, json::json};

#[cfg(feature = "execute")]
use crate::document::openxml::{
    OpenXmlFormat, read_zip, replace_text_in_xml, replace_text_in_xml_markdown, write_zip,
};

#[crate::register_node]
#[derive(Default)]
pub struct PptxReplaceTextNode {}

impl PptxReplaceTextNode {
    pub fn new() -> Self {
        PptxReplaceTextNode {}
    }
}

#[async_trait]
impl NodeLogic for PptxReplaceTextNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "pptx_replace_text",
            "Replace Text in PPTX",
            "Replaces text placeholders in a PowerPoint (PPTX) file with plain or markdown-formatted text",
            "Document/PPTX",
        );
        node.set_flowscript_name("pptx", "replaceText");
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
            "Path to the PPTX template file",
            VariableType::Struct,
        )
        .set_schema::<FlowPath>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin(
            "placeholder",
            "Placeholder",
            "The placeholder text to find in the template",
            VariableType::String,
        );

        node.add_input_pin(
            "replacement",
            "Replacement",
            "The replacement text (supports markdown when enabled)",
            VariableType::String,
        );

        node.add_input_pin(
            "use_markdown",
            "Use Markdown",
            "Parse replacement text as markdown for rich formatting",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(false)));

        node.add_input_pin(
            "output",
            "Output Path",
            "Path where the resulting PPTX file will be saved",
            VariableType::Struct,
        )
        .set_schema::<FlowPath>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin(
            "exec_out",
            "Done",
            "Execution continues after replacement",
            VariableType::Execution,
        );

        node.add_output_pin(
            "result",
            "Result",
            "Path to the generated PPTX file",
            VariableType::Struct,
        )
        .set_schema::<FlowPath>();

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let template: FlowPath = context.evaluate_pin("template").await?;
        let placeholder: String = context.evaluate_pin("placeholder").await?;
        let replacement: String = context.evaluate_pin("replacement").await?;
        let use_markdown: bool = context.evaluate_pin("use_markdown").await?;
        let output: FlowPath = context.evaluate_pin("output").await?;

        let template_bytes = template.get(context, false).await?;
        let mut files = read_zip(&template_bytes)?;

        let slide_keys: Vec<String> = files
            .keys()
            .filter(|k| {
                (k.starts_with("ppt/slides/slide") && k.ends_with(".xml"))
                    || (k.starts_with("ppt/slideLayouts/") && k.ends_with(".xml"))
                    || (k.starts_with("ppt/slideMasters/") && k.ends_with(".xml"))
            })
            .cloned()
            .collect();

        for key in slide_keys {
            let xml_bytes = match files.get(&key) {
                Some(bytes) => bytes.clone(),
                None => continue,
            };

            let updated = if use_markdown {
                replace_text_in_xml_markdown(
                    &xml_bytes,
                    &placeholder,
                    &replacement,
                    "a:t",
                    "a:r",
                    "a:rPr",
                    "a:p",
                    OpenXmlFormat::Pptx,
                )?
            } else {
                replace_text_in_xml(&xml_bytes, &placeholder, &replacement, "a:t", "a:r", "a:p")?
            };

            files.insert(key, updated);
        }

        let result_bytes = write_zip(&files)?;
        output.put(context, result_bytes, false).await?;

        context.set_pin_value("result", json!(output)).await?;
        context.activate_exec_pin("exec_out").await?;

        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "PPTX text replacement requires the 'execute' feature"
        ))
    }
}
