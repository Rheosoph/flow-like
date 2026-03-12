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
pub struct PptxAddImageToSlideNode;

impl PptxAddImageToSlideNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for PptxAddImageToSlideNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "pptx_add_image_to_slide",
            "Add Image to Slide",
            "Place an image at a specified position on a PPTX slide.",
            "Document/PPTX",
        );
        node.add_icon("/flow/icons/image.svg");
        node.set_scores(
            NodeScores::new()
                .set_privacy(9)
                .set_security(7)
                .set_performance(6)
                .set_governance(8)
                .set_reliability(7)
                .set_cost(10)
                .build(),
        );

        node.add_input_pin("exec_in", "Input", "Trigger", VariableType::Execution);
        node.add_input_pin("template", "Template", "PPTX file", VariableType::Struct)
            .set_schema::<FlowPath>()
            .set_options(PinOptions::new().set_enforce_schema(true).build());
        node.add_input_pin("image", "Image", "Image file (PNG/JPEG)", VariableType::Struct)
            .set_schema::<FlowPath>()
            .set_options(PinOptions::new().set_enforce_schema(true).build());
        node.add_input_pin("slide_number", "Slide Number", "1-based slide index", VariableType::Integer)
            .set_default_value(Some(json!(1)));
        node.add_input_pin("x", "X", "X position in cm", VariableType::Float)
            .set_default_value(Some(json!(2.0)));
        node.add_input_pin("y", "Y", "Y position in cm", VariableType::Float)
            .set_default_value(Some(json!(2.0)));
        node.add_input_pin("width", "Width", "Width in cm", VariableType::Float)
            .set_default_value(Some(json!(10.0)));
        node.add_input_pin("height", "Height", "Height in cm", VariableType::Float)
            .set_default_value(Some(json!(7.0)));
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
        use crate::document::styles::cm_to_emu;

        context.deactivate_exec_pin("exec_out").await?;

        let template: FlowPath = context.evaluate_pin("template").await?;
        let image_path: FlowPath = context.evaluate_pin("image").await?;
        let slide_number: i64 = context.evaluate_pin("slide_number").await?;
        let x: f64 = context.evaluate_pin("x").await?;
        let y: f64 = context.evaluate_pin("y").await?;
        let width: f64 = context.evaluate_pin("width").await?;
        let height: f64 = context.evaluate_pin("height").await?;
        let output: FlowPath = context.evaluate_pin("output").await?;

        let bytes = template.get(context, false).await?;
        let image_bytes = image_path.get(context, false).await?;
        let mut files = read_zip(&bytes)?;

        let is_png = image_bytes.starts_with(&[0x89, 0x50, 0x4E, 0x47]);
        let ext = if is_png { "png" } else { "jpeg" };
        let content_type = if is_png { "image/png" } else { "image/jpeg" };

        let img_num = next_image_number(&files);
        let img_path = format!("ppt/media/image{}.{}", img_num, ext);
        files.insert(img_path.clone(), image_bytes.to_vec());

        let slide_path = format!("ppt/slides/slide{}.xml", slide_number);
        let slide_rels_path = format!("ppt/slides/_rels/slide{}.xml.rels", slide_number);

        let slide_data = files
            .get(&slide_path)
            .ok_or_else(|| flow_like_types::anyhow!("Slide {} not found", slide_number))?
            .clone();

        let rid = next_rel_id(&files, &slide_rels_path);
        add_relationship(
            &mut files,
            &slide_rels_path,
            &rid,
            "http://schemas.openxmlformats.org/officeDocument/2006/relationships/image",
            &format!("../media/image{}.{}", img_num, ext),
        );

        update_content_types_for_image(&mut files, ext, content_type);

        let mut slide_xml = String::from_utf8_lossy(&slide_data).to_string();
        let next_id = max_id(&slide_xml) + 1;

        let pic_xml = format!(
            r#"<p:pic>
  <p:nvPicPr>
    <p:cNvPr id="{id}" name="Image {id}"/>
    <p:cNvPicPr><a:picLocks noChangeAspect="1"/></p:cNvPicPr>
    <p:nvPr/>
  </p:nvPicPr>
  <p:blipFill>
    <a:blip r:embed="{rid}"/>
    <a:stretch><a:fillRect/></a:stretch>
  </p:blipFill>
  <p:spPr>
    <a:xfrm>
      <a:off x="{ox}" y="{oy}"/>
      <a:ext cx="{cx}" cy="{cy}"/>
    </a:xfrm>
    <a:prstGeom prst="rect"><a:avLst/></a:prstGeom>
  </p:spPr>
</p:pic>"#,
            id = next_id,
            rid = rid,
            ox = cm_to_emu(x as f32),
            oy = cm_to_emu(y as f32),
            cx = cm_to_emu(width as f32),
            cy = cm_to_emu(height as f32),
        );

        if let Some(pos) = slide_xml.find("</p:spTree>") {
            slide_xml.insert_str(pos, &pic_xml);
        }

        files.insert(slide_path, slide_xml.into_bytes());

        let buf = write_zip(&files)?;
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
fn next_image_number(files: &std::collections::HashMap<String, Vec<u8>>) -> u32 {
    let mut max = 0u32;
    for key in files.keys() {
        if key.starts_with("ppt/media/image") {
            if let Some(rest) = key.strip_prefix("ppt/media/image") {
                let num_part: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
                if let Ok(n) = num_part.parse::<u32>() {
                    max = max.max(n);
                }
            }
        }
    }
    max + 1
}

#[cfg(feature = "execute")]
fn next_rel_id(files: &std::collections::HashMap<String, Vec<u8>>, rels_path: &str) -> String {
    let mut max = 0u32;
    if let Some(data) = files.get(rels_path) {
        let content = String::from_utf8_lossy(data);
        for cap in content.match_indices("rId") {
            let rest = &content[cap.0 + 3..];
            if let Some(end) = rest.find('"') {
                if let Ok(n) = rest[..end].parse::<u32>() {
                    max = max.max(n);
                }
            }
        }
    }
    format!("rId{}", max + 1)
}

#[cfg(feature = "execute")]
fn add_relationship(
    files: &mut std::collections::HashMap<String, Vec<u8>>,
    rels_path: &str,
    rid: &str,
    rel_type: &str,
    target: &str,
) {
    let entry = format!(
        r#"<Relationship Id="{}" Type="{}" Target="{}"/>"#,
        rid, rel_type, target,
    );

    if let Some(data) = files.get(rels_path) {
        let mut content = String::from_utf8_lossy(data).to_string();
        if let Some(pos) = content.find("</Relationships>") {
            content.insert_str(pos, &entry);
        }
        files.insert(rels_path.to_string(), content.into_bytes());
    } else {
        let content = format!(
            r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  {}
</Relationships>"#,
            entry
        );
        files.insert(rels_path.to_string(), content.into_bytes());
    }
}

#[cfg(feature = "execute")]
fn update_content_types_for_image(
    files: &mut std::collections::HashMap<String, Vec<u8>>,
    ext: &str,
    content_type: &str,
) {
    if let Some(ct_data) = files.get("[Content_Types].xml") {
        let content = String::from_utf8_lossy(ct_data).to_string();
        if content.contains(&format!("Extension=\"{}\"", ext)) {
            return;
        }
        let entry = format!(
            r#"<Default Extension="{}" ContentType="{}"/>"#,
            ext, content_type
        );
        let mut updated = content;
        if let Some(pos) = updated.find("</Types>") {
            updated.insert_str(pos, &entry);
        }
        files.insert("[Content_Types].xml".to_string(), updated.into_bytes());
    }
}

#[cfg(feature = "execute")]
fn max_id(xml: &str) -> u32 {
    let mut max = 0u32;
    for cap in xml.match_indices("id=\"") {
        let rest = &xml[cap.0 + 4..];
        if let Some(end) = rest.find('"') {
            if let Ok(n) = rest[..end].parse::<u32>() {
                max = max.max(n);
            }
        }
    }
    max
}
