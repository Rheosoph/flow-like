#[cfg(feature = "execute")]
use lopdf::{Document, Object};

use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic, NodeScores},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_catalog_core::FlowPath;
use flow_like_types::{async_trait, json::json};

#[crate::register_node]
#[derive(Default)]
pub struct PdfReplaceTextNode;

impl PdfReplaceTextNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for PdfReplaceTextNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "pdf_replace_text",
            "Replace Text in PDF",
            "Attempts to find and replace text in a PDF. Best-effort: PDF text replacement may not work for all documents due to complex text encoding and fragmented content streams.",
            "Document/PDF",
        );
        node.add_icon("/flow/icons/text.svg");

        node.set_scores(
            NodeScores::new()
                .set_privacy(8)
                .set_security(6)
                .set_performance(6)
                .set_governance(7)
                .set_reliability(5)
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
            "PDF file to modify",
            VariableType::Struct,
        )
        .set_schema::<FlowPath>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin(
            "placeholder",
            "Placeholder",
            "Text to find in the PDF",
            VariableType::String,
        );

        node.add_input_pin(
            "replacement",
            "Replacement",
            "Plain text replacement value",
            VariableType::String,
        );

        node.add_input_pin(
            "output",
            "Output Path",
            "Path to save the modified PDF",
            VariableType::Struct,
        )
        .set_schema::<FlowPath>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin(
            "exec_out",
            "Output",
            "Execution continues after replacement",
            VariableType::Execution,
        );

        node.add_output_pin("result", "Result", "Output file path", VariableType::Struct)
            .set_schema::<FlowPath>()
            .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin(
            "replaced_count",
            "Replaced Count",
            "Number of text replacements made",
            VariableType::Integer,
        );

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let template: FlowPath = context.evaluate_pin("template").await?;
        let placeholder: String = context.evaluate_pin("placeholder").await?;
        let replacement: String = context.evaluate_pin("replacement").await?;
        let output: FlowPath = context.evaluate_pin("output").await?;

        let bytes = template.get(context, false).await?;
        let mut doc = Document::load_mem(&bytes)?;

        let mut replaced_count: i64 = 0;

        for object in doc.objects.values_mut() {
            replaced_count += replace_in_object(object, &placeholder, &replacement);
        }

        let mut buf = Vec::new();
        doc.save_to(&mut buf)?;

        output.put(context, buf, false).await?;

        context.set_pin_value("result", json!(output)).await?;
        context
            .set_pin_value("replaced_count", json!(replaced_count))
            .await?;
        context.activate_exec_pin("exec_out").await?;

        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "PDF processing requires the 'execute' feature"
        ))
    }
}

#[cfg(feature = "execute")]
fn replace_in_object(object: &mut Object, placeholder: &str, replacement: &str) -> i64 {
    let mut count = 0i64;
    match *object {
        Object::String(ref mut bytes, _) => {
            if let Ok(text) = std::str::from_utf8(bytes)
                && text.contains(placeholder)
            {
                let new_text = text.replace(placeholder, replacement);
                let occurrences = text.matches(placeholder).count() as i64;
                *bytes = new_text.into_bytes();
                count += occurrences;
            }
        }
        Object::Array(ref mut arr) => {
            for item in arr.iter_mut() {
                count += replace_in_object(item, placeholder, replacement);
            }
        }
        Object::Dictionary(ref mut dict) => {
            for (_key, value) in dict.iter_mut() {
                count += replace_in_object(value, placeholder, replacement);
            }
        }
        Object::Stream(ref mut stream) => {
            for (_key, value) in stream.dict.iter_mut() {
                count += replace_in_object(value, placeholder, replacement);
            }
            if let Ok(text) = std::str::from_utf8(&stream.content)
                && text.contains(placeholder)
            {
                let new_text = text.replace(placeholder, replacement);
                let occurrences = text.matches(placeholder).count() as i64;
                stream.content = new_text.into_bytes();
                count += occurrences;
            }
        }
        _ => {}
    }
    count
}
