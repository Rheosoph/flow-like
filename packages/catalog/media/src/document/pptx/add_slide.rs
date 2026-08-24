#[cfg(feature = "execute")]
use crate::document::openxml::{read_zip, write_zip};

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
pub struct PptxAddSlideNode;

impl PptxAddSlideNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for PptxAddSlideNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "pptx_add_slide",
            "Add Slide",
            "Add a blank slide to a PPTX presentation.",
            "Document/PPTX",
        );
        node.set_flowscript_name("pptx", "addSlide");
        node.add_icon("/flow/icons/file.svg");
        node.set_scores(
            NodeScores::new()
                .set_privacy(9)
                .set_security(8)
                .set_performance(7)
                .set_governance(8)
                .set_reliability(8)
                .set_cost(10)
                .build(),
        );

        node.add_input_pin("exec_in", "Input", "Trigger", VariableType::Execution);
        node.add_input_pin("template", "Template", "PPTX file", VariableType::Struct)
            .set_schema::<FlowPath>()
            .set_options(PinOptions::new().set_enforce_schema(true).build());
        node.add_input_pin("output", "Output Path", "Save path", VariableType::Struct)
            .set_schema::<FlowPath>()
            .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin("exec_out", "Done", "Continues", VariableType::Execution);
        node.add_output_pin("result", "Result", "Output file path", VariableType::Struct)
            .set_schema::<FlowPath>();
        node.add_output_pin(
            "slide_number",
            "Slide Number",
            "New slide's index (1-based)",
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
        let mut files = read_zip(&bytes)?;

        let slide_num = next_slide_number(&files);
        let slide_path = format!("ppt/slides/slide{}.xml", slide_num);
        let slide_rels_path = format!("ppt/slides/_rels/slide{}.xml.rels", slide_num);

        files.insert(slide_path.clone(), build_blank_slide().into_bytes());

        let layout_rel = find_slide_layout_target(&files);
        files.insert(slide_rels_path, build_slide_rels(&layout_rel).into_bytes());

        update_presentation_xml(&mut files, slide_num);
        update_content_types(&mut files, slide_num);

        let buf = write_zip(&files)?;
        output.put(context, buf, false).await?;
        context.set_pin_value("result", json!(output)).await?;
        context
            .set_pin_value("slide_number", json!(count_slides(&files)))
            .await?;
        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!("Requires the 'execute' feature"))
    }
}

#[cfg(feature = "execute")]
fn next_slide_number(files: &std::collections::HashMap<String, Vec<u8>>) -> u32 {
    let mut max = 0u32;
    for key in files.keys() {
        if let Some(rest) = key.strip_prefix("ppt/slides/slide")
            && let Some(num_str) = rest.strip_suffix(".xml")
            && let Ok(n) = num_str.parse::<u32>()
        {
            max = max.max(n);
        }
    }
    max + 1
}

#[cfg(feature = "execute")]
fn count_slides(files: &std::collections::HashMap<String, Vec<u8>>) -> i64 {
    files
        .keys()
        .filter(|k| {
            k.starts_with("ppt/slides/slide") && k.ends_with(".xml") && !k.contains("_rels")
        })
        .count() as i64
}

#[cfg(feature = "execute")]
fn find_slide_layout_target(files: &std::collections::HashMap<String, Vec<u8>>) -> String {
    for key in files.keys() {
        if key.starts_with("ppt/slides/_rels/slide")
            && key.ends_with(".xml.rels")
            && let Some(data) = files.get(key)
        {
            let content = String::from_utf8_lossy(data);
            if let Some(pos) = content.find("slideLayout")
                && let Some(start) = content[..pos].rfind("Target=\"")
            {
                let target_start = start + 8;
                if let Some(end) = content[target_start..].find('"') {
                    return content[target_start..target_start + end].to_string();
                }
            }
        }
    }
    "../slideLayouts/slideLayout1.xml".to_string()
}

#[cfg(feature = "execute")]
fn build_blank_slide() -> String {
    r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<p:sld xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"
  xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"
  xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main">
  <p:cSld>
    <p:spTree>
      <p:nvGrpSpPr><p:cNvPr id="1" name=""/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr>
      <p:grpSpPr/>
    </p:spTree>
  </p:cSld>
</p:sld>"#
        .to_string()
}

#[cfg(feature = "execute")]
fn build_slide_rels(layout_target: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slideLayout" Target="{layout_target}"/>
</Relationships>"#
    )
}

#[cfg(feature = "execute")]
fn update_presentation_xml(files: &mut std::collections::HashMap<String, Vec<u8>>, slide_num: u32) {
    if let Some(pres_data) = files.get("ppt/presentation.xml") {
        let mut content = String::from_utf8_lossy(pres_data).to_string();
        let pres_rels_data = files.get("ppt/_rels/presentation.xml.rels").cloned();

        let new_rid = next_rid(&pres_rels_data);
        let new_sld_id = next_sld_id(&content);

        let sld_entry = format!(r#"<p:sldId id="{}" r:id="{}"/>"#, new_sld_id, new_rid);

        if let Some(pos) = content.find("</p:sldIdLst>") {
            content.insert_str(pos, &sld_entry);
        } else if let Some(pos) = content.find("</p:sldMasterIdLst>") {
            let insert_pos = pos + "</p:sldMasterIdLst>".len();
            content.insert_str(
                insert_pos,
                &format!("<p:sldIdLst>{}</p:sldIdLst>", sld_entry),
            );
        }

        files.insert("ppt/presentation.xml".to_string(), content.into_bytes());

        let rel_entry = format!(
            r#"<Relationship Id="{}" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slide" Target="slides/slide{}.xml"/>"#,
            new_rid, slide_num
        );
        if let Some(rels_data) = files.get("ppt/_rels/presentation.xml.rels") {
            let mut rels = String::from_utf8_lossy(rels_data).to_string();
            if let Some(pos) = rels.find("</Relationships>") {
                rels.insert_str(pos, &rel_entry);
            }
            files.insert(
                "ppt/_rels/presentation.xml.rels".to_string(),
                rels.into_bytes(),
            );
        }
    }
}

#[cfg(feature = "execute")]
fn next_rid(rels_data: &Option<Vec<u8>>) -> String {
    let mut max = 0u32;
    if let Some(data) = rels_data {
        let content = String::from_utf8_lossy(data);
        for cap in content.match_indices("rId") {
            let rest = &content[cap.0 + 3..];
            if let Some(end) = rest.find('"')
                && let Ok(n) = rest[..end].parse::<u32>()
            {
                max = max.max(n);
            }
        }
    }
    format!("rId{}", max + 1)
}

#[cfg(feature = "execute")]
fn next_sld_id(pres_content: &str) -> u32 {
    let mut max = 256u32;
    let section = if let Some(start) = pres_content.find("<p:sldIdLst>") {
        let offset = start + "<p:sldIdLst>".len();
        pres_content[offset..]
            .find("</p:sldIdLst>")
            .map(|end| &pres_content[start..offset + end])
            .unwrap_or("")
    } else {
        ""
    };
    for cap in section.match_indices("id=\"") {
        let rest = &section[cap.0 + 4..];
        if let Some(end) = rest.find('"')
            && let Ok(n) = rest[..end].parse::<u32>()
            && n >= 256
        {
            max = max.max(n);
        }
    }
    max + 1
}

#[cfg(feature = "execute")]
fn update_content_types(files: &mut std::collections::HashMap<String, Vec<u8>>, slide_num: u32) {
    if let Some(ct_data) = files.get("[Content_Types].xml") {
        let mut content = String::from_utf8_lossy(ct_data).to_string();
        let entry = format!(
            r#"<Override PartName="/ppt/slides/slide{}.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.slide+xml"/>"#,
            slide_num
        );
        if let Some(pos) = content.find("</Types>") {
            content.insert_str(pos, &entry);
        }
        files.insert("[Content_Types].xml".to_string(), content.into_bytes());
    }
}
