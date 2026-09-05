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
pub struct PptxDuplicateSlideNode;

impl PptxDuplicateSlideNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for PptxDuplicateSlideNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "pptx_duplicate_slide",
            "Duplicate Slide",
            "Clone a slide at a given index, inserting the copy at a target position. Preserves formatting, layouts, and master references.",
            "Document/PPTX",
        );
        node.set_flowscript_name("pptx", "duplicateSlide");
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
            "Index of the slide to clone (1-based)",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(1)));

        node.add_input_pin(
            "target_index",
            "Target Index",
            "Position to insert the cloned slide (1-based)",
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
            "Execution continues after duplication",
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
        let target_index: i64 = context.evaluate_pin("target_index").await?;
        let output: FlowPath = context.evaluate_pin("output").await?;

        if slide_index < 1 || target_index < 1 {
            return Err(flow_like_types::anyhow!("Indices must be >= 1"));
        }
        let src_num = slide_index as u32;

        let template_bytes = template.get(context, false).await?;
        let mut files = read_zip(&template_bytes)?;

        let src_key = format!("ppt/slides/slide{src_num}.xml");
        if !files.contains_key(&src_key) {
            return Err(flow_like_types::anyhow!("Source slide {src_num} not found"));
        }

        let max_num = files
            .keys()
            .filter_map(|k| {
                k.strip_prefix("ppt/slides/slide")?
                    .strip_suffix(".xml")?
                    .parse::<u32>()
                    .ok()
            })
            .max()
            .unwrap_or(0);
        let new_num = max_num + 1;

        let new_slide_key = format!("ppt/slides/slide{new_num}.xml");
        let src_bytes = files.get(&src_key).cloned().unwrap_or_default();
        files.insert(new_slide_key.clone(), src_bytes);

        let src_rels_key = format!("ppt/slides/_rels/slide{src_num}.xml.rels");
        let new_rels_key = format!("ppt/slides/_rels/slide{new_num}.xml.rels");
        if let Some(rels_bytes) = files.get(&src_rels_key).cloned() {
            files.insert(new_rels_key, rels_bytes);
        }

        let ct_key = "[Content_Types].xml";
        if let Some(ct_bytes) = files.get(ct_key) {
            let mut ct = String::from_utf8_lossy(ct_bytes).to_string();
            let part = format!(
                r#"<Override PartName="/ppt/slides/slide{new_num}.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.slide+xml"/>"#
            );
            if !ct.contains(&format!("slide{new_num}.xml")) {
                ct = ct.replace("</Types>", &format!("{part}</Types>"));
                files.insert(ct_key.to_string(), ct.into_bytes());
            }
        }

        let pres_rels_key = "ppt/_rels/presentation.xml.rels";
        let new_rid = if let Some(rels_bytes) = files.get(pres_rels_key) {
            let mut rels = String::from_utf8_lossy(rels_bytes).to_string();
            let rid_re = Regex::new(r#"rId(\d+)"#)?;
            let max_rid = rid_re
                .captures_iter(&rels)
                .filter_map(|c| c.get(1)?.as_str().parse::<u32>().ok())
                .max()
                .unwrap_or(0);
            let new_rid = max_rid + 1;
            let rel = format!(
                r#"<Relationship Id="rId{new_rid}" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slide" Target="slides/slide{new_num}.xml"/>"#
            );
            rels = rels.replace("</Relationships>", &format!("{rel}</Relationships>"));
            files.insert(pres_rels_key.to_string(), rels.into_bytes());
            new_rid
        } else {
            return Err(flow_like_types::anyhow!("Missing presentation.xml.rels"));
        };

        let pres_key = "ppt/presentation.xml";
        if let Some(pres_bytes) = files.get(pres_key) {
            let mut pres = String::from_utf8_lossy(pres_bytes).to_string();

            let sld_id_re = Regex::new(r#"id="(\d+)""#)?;
            let list_re = Regex::new(r"<p:sldIdLst>([\s\S]*?)</p:sldIdLst>")?;

            let max_sld_id = if let Some(list_match) = list_re.find(&pres) {
                sld_id_re
                    .captures_iter(list_match.as_str())
                    .filter_map(|c| c.get(1)?.as_str().parse::<u32>().ok())
                    .max()
                    .unwrap_or(255)
            } else {
                255
            };
            let new_sld_id = max_sld_id + 1;

            let new_entry = format!(r#"<p:sldId id="{new_sld_id}" r:id="rId{new_rid}"/>"#);

            if let Some(cap) = list_re.captures(&pres) {
                let inner = cap.get(1).map(|m| m.as_str()).unwrap_or("");
                let entry_re = Regex::new(r#"<p:sldId[^/]*/>"#)?;
                let mut entries: Vec<String> = entry_re
                    .find_iter(inner)
                    .map(|m| m.as_str().to_string())
                    .collect();

                let insert_pos = ((target_index - 1) as usize).min(entries.len());
                entries.insert(insert_pos, new_entry);

                let new_inner = entries.join("");
                let new_list = format!("<p:sldIdLst>{new_inner}</p:sldIdLst>");
                pres = list_re.replace(&pres, new_list.as_str()).to_string();
            } else {
                let new_list = format!("<p:sldIdLst>{new_entry}</p:sldIdLst>");
                pres = pres.replace("</p:presentation>", &format!("{new_list}</p:presentation>"));
            }

            files.insert(pres_key.to_string(), pres.into_bytes());
        }

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
