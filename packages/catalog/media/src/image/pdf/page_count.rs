#[cfg(feature = "execute")]
use crate::image::pdf::load_pdf_from_flowpath;
use flow_like::flow::execution::context::ExecutionContext;
use flow_like::flow::node::{Node, NodeLogic};
use flow_like::flow::pin::PinOptions;
use flow_like::flow::variable::VariableType;
use flow_like_catalog_core::FlowPath;
use flow_like_types::{async_trait, json::json};

#[crate::register_node]
#[derive(Default)]
pub struct PdfPageCountNode;

impl PdfPageCountNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for PdfPageCountNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "pdf_page_count",
            "PDF Page Count",
            "Count pages in a PDF",
            "Image/PDF",
        );
        node.set_flowscript_name("pdf", "pageCount");
        node.add_icon("/flow/icons/path.svg");

        node.add_input_pin(
            "exec_in",
            "Input",
            "Trigger execution",
            VariableType::Execution,
        );

        node.add_input_pin("pdf", "PDF", "PDF file", VariableType::Struct)
            .set_schema::<FlowPath>()
            .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin(
            "exec_out",
            "Output",
            "Execution done",
            VariableType::Execution,
        );

        node.add_output_pin("page_count", "Pages", "Page count", VariableType::Integer)
            .set_default_value(Some(json!(0)));

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let pdf_path: FlowPath = context.evaluate_pin("pdf").await?;
        let pdf = load_pdf_from_flowpath(context, &pdf_path).await?;
        let page_count = pdf.pages().len() as i64;

        context
            .set_pin_value("page_count", json!(page_count))
            .await?;
        context.activate_exec_pin("exec_out").await?;

        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "Media processing requires the 'execute' feature"
        ))
    }
}
