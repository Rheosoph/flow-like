#[cfg(feature = "execute")]
use crate::document::openxml::{
    OpenXmlFormat, read_zip, replace_text_in_xml, replace_text_in_xml_markdown, write_zip,
};
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_catalog_core::FlowPath;
use flow_like_types::{async_trait, json::json};

#[crate::register_node]
#[derive(Default)]
pub struct DocxReplaceTextNode;

impl DocxReplaceTextNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for DocxReplaceTextNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "docx_replace_text",
            "Replace Text in DOCX",
            "Replace text placeholders in a DOCX template file with plain text or markdown",
            "Document/DOCX",
        );
        node.add_icon("/flow/icons/text.svg");

        node.add_input_pin(
            "exec_in",
            "Input",
            "Trigger execution",
            VariableType::Execution,
        );

        node.add_input_pin(
            "template",
            "Template",
            "DOCX template file",
            VariableType::Struct,
        )
        .set_schema::<FlowPath>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin(
            "placeholder",
            "Placeholder",
            "Placeholder text to find (e.g. {{name}})",
            VariableType::String,
        );

        node.add_input_pin(
            "replacement",
            "Replacement",
            "Replacement text (supports markdown when enabled)",
            VariableType::String,
        );

        node.add_input_pin(
            "use_markdown",
            "Use Markdown",
            "Parse the replacement text as markdown",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(false)));

        node.add_input_pin(
            "output",
            "Output Path",
            "Where to save the resulting DOCX file",
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
            "Path to the output file",
            VariableType::Struct,
        )
        .set_schema::<FlowPath>();

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let template_path: FlowPath = context.evaluate_pin("template").await?;
        let placeholder: String = context.evaluate_pin("placeholder").await?;
        let replacement: String = context.evaluate_pin("replacement").await?;
        let use_markdown: bool = context.evaluate_pin("use_markdown").await?;
        let output_path: FlowPath = context.evaluate_pin("output").await?;

        let template_bytes = template_path.get(context, false).await?;
        let mut files = read_zip(&template_bytes)?;

        let xml_targets: Vec<String> = files
            .keys()
            .filter(|k| {
                *k == "word/document.xml"
                    || (k.starts_with("word/header") && k.ends_with(".xml"))
                    || (k.starts_with("word/footer") && k.ends_with(".xml"))
            })
            .cloned()
            .collect();

        for target in xml_targets {
            let xml_bytes = files
                .get(&target)
                .ok_or_else(|| flow_like_types::anyhow!("Missing XML part: {}", target))?
                .clone();

            let replaced = if use_markdown {
                replace_text_in_xml_markdown(
                    &xml_bytes,
                    &placeholder,
                    &replacement,
                    "w:t",
                    "w:r",
                    "w:rPr",
                    "w:p",
                    OpenXmlFormat::Docx,
                )?
            } else {
                replace_text_in_xml(&xml_bytes, &placeholder, &replacement, "w:t", "w:r", "w:p")?
            };

            files.insert(target, replaced);
        }

        let result_bytes = write_zip(&files)?;
        output_path.put(context, result_bytes, false).await?;

        context.set_pin_value("result", json!(output_path)).await?;
        context.activate_exec_pin("exec_out").await?;

        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "DOCX processing requires the 'execute' feature"
        ))
    }
}
