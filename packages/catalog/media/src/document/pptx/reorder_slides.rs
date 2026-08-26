use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic, NodeScores},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_catalog_core::FlowPath;
use flow_like_types::{async_trait, json::json};

#[cfg(feature = "execute")]
use crate::document::openxml::{read_zip, write_zip};

#[crate::register_node]
#[derive(Default)]
pub struct PptxReorderSlidesNode;

impl PptxReorderSlidesNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for PptxReorderSlidesNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "pptx_reorder_slides",
            "Reorder Slides",
            "Move a slide from one position to another",
            "Document/PPTX",
        );
        node.set_flowscript_name("pptx", "reorderSlides");
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

        node.add_input_pin(
            "from_index",
            "From Index",
            "Current position of the slide (1-based)",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(1)));

        node.add_input_pin(
            "to_index",
            "To Index",
            "Target position for the slide (1-based)",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(1)));

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
            "Execution continues after reordering",
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
        use flow_like_types::regex::Regex;

        context.deactivate_exec_pin("exec_out").await?;

        let template: FlowPath = context.evaluate_pin("template").await?;
        let from_index: i64 = context.evaluate_pin("from_index").await?;
        let to_index: i64 = context.evaluate_pin("to_index").await?;
        let output: FlowPath = context.evaluate_pin("output").await?;

        let template_bytes = template.get(context, false).await?;
        let mut files = read_zip(&template_bytes)?;

        let pres_xml = files
            .get("ppt/presentation.xml")
            .ok_or_else(|| flow_like_types::anyhow!("Missing ppt/presentation.xml"))?;
        let mut pres = String::from_utf8_lossy(pres_xml).to_string();

        let re = Regex::new(r#"<p:sldId[^/]*/>"#)?;
        let entries: Vec<String> = re
            .find_iter(&pres)
            .map(|m| m.as_str().to_string())
            .collect();

        let count = entries.len() as i64;
        if from_index < 1 || from_index > count || to_index < 1 || to_index > count {
            return Err(flow_like_types::anyhow!(
                "Index out of range: from={from_index}, to={to_index}, slides={count}"
            ));
        }

        let mut ordered = entries.clone();
        let item = ordered.remove((from_index - 1) as usize);
        ordered.insert((to_index - 1) as usize, item);

        let list_re = Regex::new(r"<p:sldIdLst>([\s\S]*?)</p:sldIdLst>")?;
        let new_list_content = ordered.join("");
        let new_list = format!("<p:sldIdLst>{new_list_content}</p:sldIdLst>");
        pres = list_re.replace(&pres, new_list.as_str()).to_string();

        files.insert("ppt/presentation.xml".to_string(), pres.into_bytes());

        let result_bytes = write_zip(&files)?;
        output.put(context, result_bytes, false).await?;

        context.set_pin_value("result", json!(output)).await?;
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
