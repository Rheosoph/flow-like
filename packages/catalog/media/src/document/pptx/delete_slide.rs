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
pub struct PptxDeleteSlideNode;

impl PptxDeleteSlideNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for PptxDeleteSlideNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "pptx_delete_slide",
            "Delete Slide",
            "Remove a slide at the given index from a PPTX file",
            "Document/PPTX",
        );
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
            "slide_index",
            "Slide Index",
            "Index of the slide to delete (1-based)",
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
            "Execution continues after deletion",
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
        let slide_index: i64 = context.evaluate_pin("slide_index").await?;
        let output: FlowPath = context.evaluate_pin("output").await?;

        if slide_index < 1 {
            return Err(flow_like_types::anyhow!("Slide index must be >= 1"));
        }

        let template_bytes = template.get(context, false).await?;
        let mut files = read_zip(&template_bytes)?;

        let pres_key = "ppt/presentation.xml";
        let pres_bytes = files
            .get(pres_key)
            .ok_or_else(|| flow_like_types::anyhow!("Missing ppt/presentation.xml"))?;
        let mut pres = String::from_utf8_lossy(pres_bytes).to_string();

        let list_re = Regex::new(r"<p:sldIdLst>([\s\S]*?)</p:sldIdLst>")?;
        let entry_re = Regex::new(r#"<p:sldId[^/]*/>"#)?;

        let inner = list_re
            .captures(&pres)
            .and_then(|c| c.get(1))
            .map(|m| m.as_str().to_string())
            .unwrap_or_default();

        let entries: Vec<String> = entry_re
            .find_iter(&inner)
            .map(|m| m.as_str().to_string())
            .collect();

        let idx = (slide_index - 1) as usize;
        if idx >= entries.len() {
            return Err(flow_like_types::anyhow!(
                "Slide index {slide_index} out of range (total: {})",
                entries.len()
            ));
        }

        let removed_entry = &entries[idx];
        let rid_re = Regex::new(r#"r:id="(rId\d+)""#)?;
        let rid = rid_re
            .captures(removed_entry)
            .and_then(|c| c.get(1))
            .map(|m| m.as_str().to_string())
            .unwrap_or_default();

        let pres_rels_key = "ppt/_rels/presentation.xml.rels";
        let slide_target = if let Some(rels_bytes) = files.get(pres_rels_key) {
            let rels = String::from_utf8_lossy(rels_bytes).to_string();
            let target_re = Regex::new(&format!(
                r#"<Relationship[^>]*Id="{rid}"[^>]*Target="([^"]+)"[^>]*/>"#
            ))?;
            let target = target_re
                .captures(&rels)
                .and_then(|c| c.get(1))
                .map(|m| m.as_str().to_string());

            let cleaned = target_re.replace(&rels, "").to_string();
            files.insert(pres_rels_key.to_string(), cleaned.into_bytes());

            target
        } else {
            None
        };

        let slide_filename = slide_target
            .as_deref()
            .and_then(|t| t.strip_prefix("slides/"))
            .unwrap_or("");

        if !slide_filename.is_empty() {
            let full_key = format!("ppt/slides/{slide_filename}");
            files.remove(&full_key);

            let rels_key = format!("ppt/slides/_rels/{slide_filename}.rels");
            files.remove(&rels_key);

            let notes_name = slide_filename.replace("slide", "notesSlide");
            let notes_key = format!("ppt/notesSlides/{notes_name}");
            files.remove(&notes_key);
            let notes_rels_key = format!("ppt/notesSlides/_rels/{notes_name}.rels");
            files.remove(&notes_rels_key);

            let ct_key = "[Content_Types].xml";
            if let Some(ct_bytes) = files.get(ct_key) {
                let ct = String::from_utf8_lossy(ct_bytes).to_string();
                let part_re = Regex::new(&format!(
                    r#"<Override[^>]*PartName="/ppt/slides/{}"[^>]*/>"#,
                    flow_like_types::regex::escape(slide_filename)
                ))?;
                let ct = part_re.replace(&ct, "").to_string();
                files.insert(ct_key.to_string(), ct.into_bytes());
            }
        }

        let mut remaining = entries.clone();
        remaining.remove(idx);
        let new_inner = remaining.join("");
        let new_list = format!("<p:sldIdLst>{new_inner}</p:sldIdLst>");
        pres = list_re.replace(&pres, new_list.as_str()).to_string();
        files.insert(pres_key.to_string(), pres.into_bytes());

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
