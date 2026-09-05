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
pub struct PdfRotatePagesNode;

impl PdfRotatePagesNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for PdfRotatePagesNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "pdf_rotate_pages",
            "Rotate Pages",
            "Rotate pages by 90, 180, or 270 degrees",
            "Document/PDF",
        );
        node.set_flowscript_name("pdf", "rotatePages");
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
            "Page numbers to rotate (1-based). Empty array = all pages.",
            VariableType::Integer,
        )
        .set_value_type(flow_like::flow::pin::ValueType::Array)
        .set_default_value(Some(json!([])));
        node.add_input_pin(
            "rotation",
            "Rotation",
            "Rotation degrees: 90, 180, or 270",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(90)));
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
        let rotation: i64 = context.evaluate_pin("rotation").await?;
        let output: FlowPath = context.evaluate_pin("output").await?;

        let rotation = match rotation {
            90 | 180 | 270 => rotation,
            _ => return Err(flow_like_types::anyhow!("Rotation must be 90, 180, or 270")),
        };

        let bytes = template.get(context, false).await?;
        let mut doc = Document::load_mem(&bytes)?;

        let all_pages: Vec<lopdf::ObjectId> = doc.page_iter().collect();
        let target_indices: std::collections::HashSet<usize> = if page_nums.is_empty() {
            (0..all_pages.len()).collect()
        } else {
            page_nums
                .iter()
                .filter(|&&n| n >= 1 && (n as usize) <= all_pages.len())
                .map(|&n| (n - 1) as usize)
                .collect()
        };

        for (idx, &page_id) in all_pages.iter().enumerate() {
            if !target_indices.contains(&idx) {
                continue;
            }
            if let Ok(page) = doc.get_object_mut(page_id)
                && let Object::Dictionary(dict) = page
            {
                let current = dict
                    .get(b"Rotate")
                    .ok()
                    .and_then(|o| {
                        if let Object::Integer(n) = o {
                            Some(*n)
                        } else {
                            None
                        }
                    })
                    .unwrap_or(0);
                let new_rotation = (current + rotation) % 360;
                dict.set("Rotate", Object::Integer(new_rotation));
            }
        }

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
