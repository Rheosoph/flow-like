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
pub struct PptxMergeNode;

impl PptxMergeNode {
    pub fn new() -> Self {
        Self
    }
}

#[cfg(feature = "execute")]
fn count_slides(files: &std::collections::HashMap<String, Vec<u8>>) -> usize {
    files
        .keys()
        .filter(|k| {
            k.starts_with("ppt/slides/slide") && k.ends_with(".xml") && !k.contains("_rels")
        })
        .count()
}

#[cfg(feature = "execute")]
fn get_max_slide_num(files: &std::collections::HashMap<String, Vec<u8>>) -> u32 {
    files
        .keys()
        .filter_map(|k| {
            k.strip_prefix("ppt/slides/slide")
                .and_then(|rest| rest.strip_suffix(".xml"))
                .and_then(|num| num.parse::<u32>().ok())
        })
        .max()
        .unwrap_or(0)
}

#[cfg(feature = "execute")]
fn get_max_rid(rels_xml: &str) -> u32 {
    let mut max = 0u32;
    for caps in flow_like_types::regex::Regex::new(r#"Id="rId(\d+)""#)
        .unwrap()
        .captures_iter(rels_xml)
    {
        if let Ok(n) = caps[1].parse::<u32>()
            && n > max
        {
            max = n;
        }
    }
    max
}

#[cfg(feature = "execute")]
fn get_max_sld_id(pres_xml: &str) -> u32 {
    let mut max = 255u32;
    for caps in flow_like_types::regex::Regex::new(r#"id="(\d+)""#)
        .unwrap()
        .captures_iter(pres_xml)
    {
        if let Ok(n) = caps[1].parse::<u32>()
            && n > max
        {
            max = n;
        }
    }
    max
}

#[async_trait]
impl NodeLogic for PptxMergeNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "pptx_merge",
            "Merge Presentations",
            "Combine slides from multiple PPTX files into one. The base file's theme and masters are preserved.",
            "Document/PPTX",
        );
        node.set_flowscript_name("pptx", "merge");
        node.add_icon("/flow/icons/text.svg");
        node.set_scores(
            NodeScores::new()
                .set_privacy(7)
                .set_security(7)
                .set_performance(5)
                .set_governance(8)
                .set_reliability(7)
                .set_cost(10)
                .build(),
        );

        node.add_input_pin("exec_in", "Input", "Trigger", VariableType::Execution);
        node.add_input_pin(
            "base",
            "Base",
            "Base PPTX file (theme/masters kept)",
            VariableType::Struct,
        )
        .set_schema::<FlowPath>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());
        node.add_input_pin(
            "additional",
            "Additional",
            "Additional PPTX files to merge (array of paths)",
            VariableType::Struct,
        )
        .set_schema::<FlowPath>()
        .set_options(PinOptions::new().set_enforce_schema(true).build())
        .set_value_type(flow_like::flow::pin::ValueType::Array);
        node.add_input_pin(
            "output",
            "Output Path",
            "Where to save the merged file",
            VariableType::Struct,
        )
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

        let base_path: FlowPath = context.evaluate_pin("base").await?;
        let additional_paths: Vec<FlowPath> = context.evaluate_pin("additional").await?;
        let output: FlowPath = context.evaluate_pin("output").await?;

        let base_bytes = base_path.get(context, false).await?;
        let mut base_files = read_zip(&base_bytes)?;

        for additional_path in &additional_paths {
            let add_bytes = additional_path.get(context, false).await?;
            let add_files = read_zip(&add_bytes)?;

            let add_slide_count = count_slides(&add_files);
            if add_slide_count == 0 {
                continue;
            }

            let mut base_max_num = get_max_slide_num(&base_files);

            let pres_rels_key = "ppt/_rels/presentation.xml.rels".to_string();
            let pres_key = "ppt/presentation.xml".to_string();
            let ct_key = "[Content_Types].xml".to_string();

            let mut rels_xml = base_files
                .get(&pres_rels_key)
                .map(|b| String::from_utf8_lossy(b).to_string())
                .unwrap_or_default();
            let mut pres_xml = base_files
                .get(&pres_key)
                .map(|b| String::from_utf8_lossy(b).to_string())
                .unwrap_or_default();
            let mut ct_xml = base_files
                .get(&ct_key)
                .map(|b| String::from_utf8_lossy(b).to_string())
                .unwrap_or_default();

            let mut rid_counter = get_max_rid(&rels_xml);
            let mut sld_id_counter = get_max_sld_id(&pres_xml);

            let mut add_slide_keys: Vec<String> = add_files
                .keys()
                .filter(|k| {
                    k.starts_with("ppt/slides/slide") && k.ends_with(".xml") && !k.contains("_rels")
                })
                .cloned()
                .collect();
            add_slide_keys.sort_by_key(|k| {
                k.strip_prefix("ppt/slides/slide")
                    .and_then(|r| r.strip_suffix(".xml"))
                    .and_then(|n| n.parse::<u32>().ok())
                    .unwrap_or(0)
            });

            for src_key in &add_slide_keys {
                base_max_num += 1;
                rid_counter += 1;
                sld_id_counter += 1;

                let new_slide_key = format!("ppt/slides/slide{}.xml", base_max_num);
                let new_rels_key = format!("ppt/slides/_rels/slide{}.xml.rels", base_max_num);

                if let Some(slide_data) = add_files.get(src_key) {
                    let slide_xml = String::from_utf8_lossy(slide_data);
                    let cleaned = slide_xml
                        .replace("ppt/slideLayouts/", "../slideLayouts/")
                        .to_string();
                    base_files.insert(new_slide_key.clone(), cleaned.into_bytes());
                }

                let src_num = src_key
                    .strip_prefix("ppt/slides/slide")
                    .and_then(|r| r.strip_suffix(".xml"))
                    .unwrap_or("1");
                let src_rels_key = format!("ppt/slides/_rels/slide{}.xml.rels", src_num);
                if let Some(rels_data) = add_files.get(&src_rels_key) {
                    base_files.insert(new_rels_key, rels_data.clone());
                } else {
                    let default_rels = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
<Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slideLayout" Target="../slideLayouts/slideLayout1.xml"/>
</Relationships>"#.to_string();
                    base_files.insert(
                        format!("ppt/slides/_rels/slide{}.xml.rels", base_max_num),
                        default_rels.into_bytes(),
                    );
                }

                let new_rel = format!(
                    r#"<Relationship Id="rId{}" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slide" Target="slides/slide{}.xml"/>"#,
                    rid_counter, base_max_num
                );
                rels_xml = rels_xml.replace(
                    "</Relationships>",
                    &format!("{}\n</Relationships>", new_rel),
                );

                let new_sld_id = format!(
                    r#"<p:sldId id="{}" r:id="rId{}"/>"#,
                    sld_id_counter, rid_counter
                );
                pres_xml =
                    pres_xml.replace("</p:sldIdLst>", &format!("{}\n</p:sldIdLst>", new_sld_id));

                if !ct_xml.contains(&format!("/ppt/slides/slide{}.xml", base_max_num)) {
                    let ct_entry = format!(
                        r#"<Override PartName="/ppt/slides/slide{}.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.slide+xml"/>"#,
                        base_max_num
                    );
                    ct_xml = ct_xml.replace("</Types>", &format!("{}\n</Types>", ct_entry));
                }
            }

            base_files.insert(pres_rels_key, rels_xml.into_bytes());
            base_files.insert(pres_key, pres_xml.into_bytes());
            base_files.insert(ct_key, ct_xml.into_bytes());
        }

        let result_bytes = write_zip(&base_files)?;
        output.put(context, result_bytes, false).await?;
        context.set_pin_value("result", json!(output)).await?;
        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!("Requires the 'execute' feature"))
    }
}
