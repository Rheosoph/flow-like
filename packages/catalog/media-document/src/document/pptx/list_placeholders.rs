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
use crate::document::openxml::read_zip;

#[crate::register_node]
#[derive(Default)]
pub struct PptxListPlaceholdersNode;

impl PptxListPlaceholdersNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for PptxListPlaceholdersNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "pptx_list_placeholders",
            "List Placeholders",
            "Scan all slides for {{...}} placeholder strings",
            "Document/PPTX",
        );
        node.set_flowscript_name("pptx", "listPlaceholders");
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
            "Execution continues after scanning",
            VariableType::Execution,
        );

        node.add_output_pin(
            "placeholders",
            "Placeholders",
            "List of unique placeholder names found",
            VariableType::String,
        )
        .set_value_type(flow_like::flow::pin::ValueType::Array);

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        use flow_like_types::regex::Regex;
        use std::collections::BTreeSet;

        context.deactivate_exec_pin("exec_out").await?;

        let template: FlowPath = context.evaluate_pin("template").await?;
        let template_bytes = template.get(context, false).await?;
        let files = read_zip(&template_bytes)?;

        let re = Regex::new(r"\{\{([^}]+)\}\}")?;
        let mut found = BTreeSet::new();

        for (key, bytes) in &files {
            if key.starts_with("ppt/slides/slide") && key.ends_with(".xml") {
                let xml = String::from_utf8_lossy(bytes);
                for cap in re.captures_iter(&xml) {
                    if let Some(m) = cap.get(1) {
                        found.insert(m.as_str().trim().to_string());
                    }
                }
            }
        }

        let placeholders: Vec<String> = found.into_iter().collect();
        context
            .set_pin_value("placeholders", json!(placeholders))
            .await?;
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
