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
pub struct PdfExtractPagesNode;

impl PdfExtractPagesNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for PdfExtractPagesNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "pdf_extract_pages",
            "Extract Pages",
            "Extract specific pages (non-contiguous) from a PDF",
            "Document/PDF",
        );
        node.set_flowscript_name("pdf", "extractPages");
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

        node.add_input_pin("exec_in", "Input", "Trigger", VariableType::Execution);
        node.add_input_pin("template", "Template", "PDF file", VariableType::Struct)
            .set_schema::<FlowPath>()
            .set_options(PinOptions::new().set_enforce_schema(true).build());
        node.add_input_pin(
            "pages",
            "Pages",
            "Array of page numbers to extract (1-based)",
            VariableType::Integer,
        )
        .set_value_type(flow_like::flow::pin::ValueType::Array);
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
        let page_nums: Vec<i64> = context.evaluate_pin("pages").await?;
        let output: FlowPath = context.evaluate_pin("output").await?;

        let bytes = template.get(context, false).await?;
        let mut doc = Document::load_mem(&bytes)?;

        let total_pages = doc.page_iter().count();
        let keep_set: std::collections::HashSet<u32> = page_nums
            .iter()
            .filter(|&&n| n >= 1 && (n as usize) <= total_pages)
            .map(|&n| n as u32)
            .collect();

        let pages_to_remove: Vec<u32> = (1..=total_pages as u32)
            .filter(|n| !keep_set.contains(n))
            .collect();

        doc.delete_pages(&pages_to_remove);

        let mut buf = Vec::new();
        doc.save_to(&mut buf)?;
        output.put(context, buf, false).await?;
        context.set_pin_value("result", json!(output)).await?;
        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!("Requires the 'execute' feature"))
    }
}
