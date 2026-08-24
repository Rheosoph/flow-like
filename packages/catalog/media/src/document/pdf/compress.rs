#[cfg(feature = "execute")]
use lopdf::Document;

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
pub struct PdfCompressNode;

impl PdfCompressNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for PdfCompressNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "pdf_compress",
            "Compress PDF",
            "Optimize and compress a PDF by deduplicating objects and compressing streams.",
            "Document/PDF",
        );
        node.set_flowscript_name("pdf", "compress");
        node.add_icon("/flow/icons/compress.svg");
        node.set_scores(
            NodeScores::new()
                .set_privacy(8)
                .set_security(8)
                .set_performance(6)
                .set_governance(8)
                .set_reliability(8)
                .set_cost(10)
                .build(),
        );

        node.add_input_pin("exec_in", "Input", "Trigger", VariableType::Execution);
        node.add_input_pin("template", "Template", "PDF file", VariableType::Struct)
            .set_schema::<FlowPath>()
            .set_options(PinOptions::new().set_enforce_schema(true).build());
        node.add_input_pin("output", "Output Path", "Save path", VariableType::Struct)
            .set_schema::<FlowPath>()
            .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin("exec_out", "Done", "Continues", VariableType::Execution);
        node.add_output_pin("result", "Result", "Output file path", VariableType::Struct)
            .set_schema::<FlowPath>();
        node.add_output_pin(
            "original_size",
            "Original Size",
            "Size in bytes before compression",
            VariableType::Integer,
        );
        node.add_output_pin(
            "compressed_size",
            "Compressed Size",
            "Size in bytes after compression",
            VariableType::Integer,
        );

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let template: FlowPath = context.evaluate_pin("template").await?;
        let output: FlowPath = context.evaluate_pin("output").await?;

        let bytes = template.get(context, false).await?;
        let original_size = bytes.len() as i64;

        let mut doc = Document::load_mem(&bytes)?;

        doc.compress();
        doc.prune_objects();
        doc.delete_zero_length_streams();
        doc.renumber_objects();

        let mut buf = Vec::new();
        doc.save_to(&mut buf)?;
        let compressed_size = buf.len() as i64;

        output.put(context, buf, false).await?;
        context.set_pin_value("result", json!(output)).await?;
        context
            .set_pin_value("original_size", json!(original_size))
            .await?;
        context
            .set_pin_value("compressed_size", json!(compressed_size))
            .await?;
        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!("Requires the 'execute' feature"))
    }
}
