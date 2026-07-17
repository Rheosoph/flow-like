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
pub struct DocxAddImageNode;

impl DocxAddImageNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for DocxAddImageNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "docx_add_image",
            "Add Image",
            "Insert an inline image into a DOCX document",
            "Document/DOCX",
        );
        node.add_icon("/flow/icons/image.svg");
        node.set_scores(
            NodeScores::new()
                .set_privacy(8)
                .set_security(7)
                .set_performance(7)
                .set_governance(8)
                .set_reliability(7)
                .set_cost(10)
                .build(),
        );

        node.add_input_pin("exec_in", "Input", "Trigger", VariableType::Execution);
        node.add_input_pin("template", "Template", "DOCX file", VariableType::Struct)
            .set_schema::<FlowPath>()
            .set_options(PinOptions::new().set_enforce_schema(true).build());
        node.add_input_pin(
            "image",
            "Image",
            "Image file to insert",
            VariableType::Struct,
        )
        .set_schema::<FlowPath>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());
        node.add_input_pin(
            "width_cm",
            "Width (cm)",
            "Image width in cm",
            VariableType::Float,
        )
        .set_default_value(Some(json!(10.0)));
        node.add_input_pin(
            "height_cm",
            "Height (cm)",
            "Image height in cm",
            VariableType::Float,
        )
        .set_default_value(Some(json!(7.0)));
        node.add_input_pin(
            "alt_text",
            "Alt Text",
            "Accessibility alt text",
            VariableType::String,
        )
        .set_default_value(Some(json!("Image")));
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
        let width_cm: f64 = context.evaluate_pin("width_cm").await?;
        let height_cm: f64 = context.evaluate_pin("height_cm").await?;
        let alt_text: String = context.evaluate_pin("alt_text").await?;
        let output: FlowPath = context.evaluate_pin("output").await?;

        let doc_bytes = template.get(context, false).await?;
        let image_bytes = image_path.get(context, false).await?;
        let mut files = read_zip(&doc_bytes)?;

        let img_ext = detect_image_extension(&image_bytes);
        let img_idx = files
            .keys()
            .filter(|k| k.starts_with("word/media/image"))
            .count()
            + 1;
        let img_filename = format!("image{}.{}", img_idx, img_ext);
        let img_path = format!("word/media/{}", img_filename);
        files.insert(img_path, image_bytes);

        let rels_key = "word/_rels/document.xml.rels".to_string();
        let mut rels_xml = files
            .get(&rels_key)
            .map(|b| String::from_utf8_lossy(b).to_string())
            .unwrap_or_default();

        let rid_num = flow_like_types::regex::Regex::new(r#"Id="rId(\d+)""#)?
            .captures_iter(&rels_xml)
            .filter_map(|c| c[1].parse::<u32>().ok())
            .max()
            .unwrap_or(0)
            + 1;
        let r_id = format!("rId{}", rid_num);

        let new_rel = format!(
            r#"<Relationship Id="{}" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/image" Target="media/{}"/>"#,
            r_id, img_filename
        );
        rels_xml = rels_xml.replace(
            "</Relationships>",
            &format!("{}\n</Relationships>", new_rel),
        );
        files.insert(rels_key, rels_xml.into_bytes());

        let ct_key = "[Content_Types].xml".to_string();
        if let Some(ct_data) = files.get(&ct_key).cloned() {
            let mut ct_xml = String::from_utf8_lossy(&ct_data).to_string();
            let ct_type = match img_ext {
                "png" => "image/png",
                "gif" => "image/gif",
                _ => "image/jpeg",
            };
            if !ct_xml.contains(&format!("Extension=\"{}\"", img_ext)) {
                let entry = format!(
                    r#"<Default Extension="{}" ContentType="{}"/>"#,
                    img_ext, ct_type
                );
                ct_xml = ct_xml.replace("</Types>", &format!("{}\n</Types>", entry));
            }
            files.insert(ct_key, ct_xml.into_bytes());
        }

        let cx = cm_to_emu(width_cm as f32);
        let cy = cm_to_emu(height_cm as f32);
        let alt = quick_xml::escape::escape(&alt_text);

        let drawing_xml = format!(
            r#"<w:p><w:r><w:drawing><wp:inline distT="0" distB="0" distL="0" distR="0"><wp:extent cx="{cx}" cy="{cy}"/><wp:docPr id="{idx}" name="{alt}"/><a:graphic xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"><a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/picture"><pic:pic xmlns:pic="http://schemas.openxmlformats.org/drawingml/2006/picture"><pic:nvPicPr><pic:cNvPr id="{idx}" name="{alt}"/><pic:cNvPicPr/></pic:nvPicPr><pic:blipFill><a:blip r:embed="{rid}" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"/><a:stretch><a:fillRect/></a:stretch></pic:blipFill><pic:spPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="{cx}" cy="{cy}"/></a:xfrm><a:prstGeom prst="rect"><a:avLst/></a:prstGeom></pic:spPr></pic:pic></a:graphicData></a:graphic></wp:inline></w:drawing></w:r></w:p>"#,
            cx = cx,
            cy = cy,
            idx = img_idx,
            alt = alt,
            rid = r_id,
        );

        let doc_key = "word/document.xml".to_string();
        if let Some(doc_data) = files.get(&doc_key).cloned() {
            let mut xml = String::from_utf8_lossy(&doc_data).to_string();
            if let Some(pos) = xml.rfind("<w:sectPr") {
                xml.insert_str(pos, &drawing_xml);
            } else if let Some(pos) = xml.rfind("</w:body>") {
                xml.insert_str(pos, &drawing_xml);
            }
            files.insert(doc_key, xml.into_bytes());
        }

        let result_bytes = write_zip(&files)?;
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

#[cfg(feature = "execute")]
fn detect_image_extension(bytes: &[u8]) -> &'static str {
    if bytes.starts_with(&[0x89, 0x50, 0x4E, 0x47]) {
        "png"
    } else if bytes.starts_with(&[0x47, 0x49, 0x46]) {
        "gif"
    } else {
        "jpeg"
    }
}
