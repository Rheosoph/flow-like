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
pub struct PptxAddNotesNode;

impl PptxAddNotesNode {
    pub fn new() -> Self {
        Self
    }
}

#[cfg(feature = "execute")]
fn minimal_notes_slide_xml(text: &str) -> String {
    let escaped = text
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;");
    format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<p:notes xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"
         xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"
         xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main">
  <p:cSld>
    <p:spTree>
      <p:nvGrpSpPr>
        <p:cNvPr id="1" name=""/>
        <p:cNvGrpSpPr/>
        <p:nvPr/>
      </p:nvGrpSpPr>
      <p:grpSpPr/>
      <p:sp>
        <p:nvSpPr>
          <p:cNvPr id="2" name="Notes Placeholder"/>
          <p:cNvSpPr><a:spLocks noGrp="1"/></p:cNvSpPr>
          <p:nvPr><p:ph type="body" idx="1"/></p:nvPr>
        </p:nvSpPr>
        <p:spPr/>
        <p:txBody>
          <a:bodyPr/>
          <a:lstStyle/>
          <a:p><a:r><a:t>{escaped}</a:t></a:r></a:p>
        </p:txBody>
      </p:sp>
    </p:spTree>
  </p:cSld>
</p:notes>"#
    )
}

#[cfg(feature = "execute")]
fn notes_slide_rels_xml(slide_num: u32) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slide" Target="../slides/slide{slide_num}.xml"/>
</Relationships>"#
    )
}

#[async_trait]
impl NodeLogic for PptxAddNotesNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "pptx_add_notes",
            "Set Speaker Notes",
            "Set or replace speaker notes for a slide",
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
            "Which slide to set notes for (1-based)",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(1)));

        node.add_input_pin("notes", "Notes", "Speaker notes text", VariableType::String);

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
            "Execution continues after setting notes",
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
        let notes_text: String = context.evaluate_pin("notes").await?;
        let output: FlowPath = context.evaluate_pin("output").await?;

        if slide_index < 1 {
            return Err(flow_like_types::anyhow!("Slide index must be >= 1"));
        }
        let slide_num = slide_index as u32;

        let template_bytes = template.get(context, false).await?;
        let mut files = read_zip(&template_bytes)?;

        let slide_key = format!("ppt/slides/slide{slide_num}.xml");
        if !files.contains_key(&slide_key) {
            return Err(flow_like_types::anyhow!("Slide {slide_num} not found"));
        }

        let notes_key = format!("ppt/notesSlides/notesSlide{slide_num}.xml");
        let notes_rels_key = format!("ppt/notesSlides/_rels/notesSlide{slide_num}.xml.rels");

        if files.contains_key(&notes_key) {
            let existing = String::from_utf8_lossy(files.get(&notes_key).unwrap()).to_string();
            let escaped = notes_text
                .replace('&', "&amp;")
                .replace('<', "&lt;")
                .replace('>', "&gt;");

            let re = Regex::new(r"(?s)(<p:txBody>)(.*?)(</p:txBody>)")?;
            let new_body = format!(
                "<p:txBody><a:bodyPr/><a:lstStyle/><a:p><a:r><a:t>{escaped}</a:t></a:r></a:p></p:txBody>"
            );

            let found = re.is_match(&existing);
            let updated = if found {
                re.replace(&existing, new_body.as_str()).to_string()
            } else {
                existing
            };

            files.insert(notes_key, updated.into_bytes());
        } else {
            files.insert(
                notes_key.clone(),
                minimal_notes_slide_xml(&notes_text).into_bytes(),
            );
            files.insert(notes_rels_key, notes_slide_rels_xml(slide_num).into_bytes());

            let ct_key = "[Content_Types].xml";
            if let Some(ct_bytes) = files.get(ct_key) {
                let mut ct = String::from_utf8_lossy(ct_bytes).to_string();
                let part_entry = format!(
                    r#"<Override PartName="/{notes_key}" ContentType="application/vnd.openxmlformats-officedocument.presentationml.notesSlide+xml"/>"#
                );
                if !ct.contains(&notes_key) {
                    ct = ct.replace("</Types>", &format!("{part_entry}</Types>"));
                    files.insert(ct_key.to_string(), ct.into_bytes());
                }
            }

            let slide_rels_key = format!("ppt/slides/_rels/slide{slide_num}.xml.rels");
            if let Some(rels_bytes) = files.get(&slide_rels_key) {
                let mut rels = String::from_utf8_lossy(rels_bytes).to_string();
                if !rels.contains("notesSlide") {
                    let re = Regex::new(r#"rId(\d+)"#)?;
                    let max_id = re
                        .captures_iter(&rels)
                        .filter_map(|c| c.get(1)?.as_str().parse::<u32>().ok())
                        .max()
                        .unwrap_or(0);
                    let new_id = max_id + 1;
                    let rel_entry = format!(
                        r#"<Relationship Id="rId{new_id}" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/notesSlide" Target="../notesSlides/notesSlide{slide_num}.xml"/>"#
                    );
                    rels =
                        rels.replace("</Relationships>", &format!("{rel_entry}</Relationships>"));
                    files.insert(slide_rels_key, rels.into_bytes());
                }
            }
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
