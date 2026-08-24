#[cfg(feature = "execute")]
use lopdf::{Document, Object, ObjectId};

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
pub struct PdfMergeNode;

impl PdfMergeNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for PdfMergeNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "pdf_merge",
            "Merge PDFs",
            "Concatenate multiple PDF files into one",
            "Document/PDF",
        );
        node.set_flowscript_name("pdf", "merge");
        node.add_icon("/flow/icons/text.svg");
        node.set_scores(
            NodeScores::new()
                .set_privacy(8)
                .set_security(7)
                .set_performance(6)
                .set_governance(8)
                .set_reliability(7)
                .set_cost(10)
                .build(),
        );

        node.add_input_pin("exec_in", "Input", "Trigger", VariableType::Execution);
        node.add_input_pin(
            "documents",
            "Documents",
            "Array of PDF file paths to merge in order",
            VariableType::Struct,
        )
        .set_schema::<FlowPath>()
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

        let documents: Vec<FlowPath> = context.evaluate_pin("documents").await?;
        let output: FlowPath = context.evaluate_pin("output").await?;

        if documents.is_empty() {
            return Err(flow_like_types::anyhow!("No documents to merge"));
        }

        let first_bytes = documents[0].get(context, false).await?;
        let mut merged = Document::load_mem(&first_bytes)?;

        for doc_path in documents.iter().skip(1) {
            let bytes = doc_path.get(context, false).await?;
            let doc = Document::load_mem(&bytes)?;
            merge_documents(&mut merged, doc)?;
        }

        let mut buf = Vec::new();
        merged.save_to(&mut buf)?;
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

#[cfg(feature = "execute")]
fn merge_documents(target: &mut Document, mut source: Document) -> flow_like_types::Result<()> {
    let max_id = target.objects.keys().map(|&(id, _)| id).max().unwrap_or(0);
    source.renumber_objects_with(max_id + 1);

    let page_ids: Vec<ObjectId> = source.page_iter().collect();
    for (id, object) in source.objects {
        target.objects.insert(id, object);
    }

    let pages_id = target.catalog()?.get(b"Pages")?.as_reference()?;

    if let Ok(Object::Dictionary(pages_dict)) = target.get_object_mut(pages_id) {
        if let Ok(Object::Array(kids)) = pages_dict.get_mut(b"Kids") {
            for pid in &page_ids {
                kids.push(Object::Reference(*pid));
            }
        }
        if let Ok(Object::Integer(count)) = pages_dict.get_mut(b"Count") {
            *count += page_ids.len() as i64;
        }
    }

    for pid in page_ids {
        if let Ok(Object::Dictionary(page)) = target.get_object_mut(pid) {
            page.set("Parent", Object::Reference(pages_id));
        }
    }

    Ok(())
}
