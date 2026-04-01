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
pub struct PptxAddShapeNode;

impl PptxAddShapeNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for PptxAddShapeNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "pptx_add_shape",
            "Add Shape",
            "Add a shape (rectangle, ellipse, arrow, etc.) to a PPTX slide.",
            "Document/PPTX",
        );
        node.add_icon("/flow/icons/shape.svg");
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
        node.add_input_pin(
            "slide_number",
            "Slide Number",
            "1-based slide index",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(1)));
        node.add_input_pin(
            "shape",
            "Shape",
            "Shape preset: rect, ellipse, roundRect, rightArrow, diamond, triangle",
            VariableType::String,
        )
        .set_default_value(Some(json!("rect")));
        node.add_input_pin("x", "X", "X position in cm", VariableType::Float)
            .set_default_value(Some(json!(5.0)));
        node.add_input_pin("y", "Y", "Y position in cm", VariableType::Float)
            .set_default_value(Some(json!(5.0)));
        node.add_input_pin("width", "Width", "Width in cm", VariableType::Float)
            .set_default_value(Some(json!(6.0)));
        node.add_input_pin("height", "Height", "Height in cm", VariableType::Float)
            .set_default_value(Some(json!(4.0)));
        node.add_input_pin(
            "fill_color",
            "Fill Color",
            "Fill hex color",
            VariableType::String,
        )
        .set_default_value(Some(json!("#FF4343")));
        node.add_input_pin(
            "line_color",
            "Line Color",
            "Outline hex color (empty = no outline)",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));
        node.add_input_pin(
            "text",
            "Text",
            "Optional text inside shape",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));
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
        use crate::document::styles::{cm_to_emu, hex_to_ooxml};

        context.deactivate_exec_pin("exec_out").await?;

        let template: FlowPath = context.evaluate_pin("template").await?;
        let slide_number: i64 = context.evaluate_pin("slide_number").await?;
        let shape: String = context.evaluate_pin("shape").await?;
        let x: f64 = context.evaluate_pin("x").await?;
        let y: f64 = context.evaluate_pin("y").await?;
        let width: f64 = context.evaluate_pin("width").await?;
        let height: f64 = context.evaluate_pin("height").await?;
        let fill_color: String = context.evaluate_pin("fill_color").await?;
        let line_color: String = context.evaluate_pin("line_color").await?;
        let text: String = context.evaluate_pin("text").await?;
        let output: FlowPath = context.evaluate_pin("output").await?;

        let bytes = template.get(context, false).await?;
        let mut files = read_zip(&bytes)?;

        let slide_path = format!("ppt/slides/slide{}.xml", slide_number);
        let slide_data = files
            .get(&slide_path)
            .ok_or_else(|| flow_like_types::anyhow!("Slide {} not found", slide_number))?
            .clone();
        let mut slide_xml = String::from_utf8_lossy(&slide_data).to_string();

        let next_id = max_id(&slide_xml) + 1;
        let fill_val = hex_to_ooxml(&fill_color);

        let line_xml = if line_color.is_empty() {
            "<a:ln><a:noFill/></a:ln>".to_string()
        } else {
            let lc = hex_to_ooxml(&line_color);
            format!(
                r#"<a:ln w="12700"><a:solidFill><a:srgbClr val="{}"/></a:solidFill></a:ln>"#,
                lc
            )
        };

        let text_body = if text.is_empty() {
            "<p:txBody><a:bodyPr/><a:lstStyle/><a:p><a:endParaRPr lang=\"en-US\"/></a:p></p:txBody>"
                .to_string()
        } else {
            let escaped = xml_escape(&text);
            format!(
                r#"<p:txBody><a:bodyPr anchor="ctr"/><a:lstStyle/><a:p><a:pPr algn="ctr"/><a:r><a:rPr lang="en-US" sz="1400" dirty="0"><a:solidFill><a:srgbClr val="FFFFFF"/></a:solidFill><a:latin typeface="Calibri"/></a:rPr><a:t>{}</a:t></a:r></a:p></p:txBody>"#,
                escaped
            )
        };

        let shape_xml = format!(
            r#"<p:sp>
  <p:nvSpPr>
    <p:cNvPr id="{id}" name="Shape {id}"/>
    <p:cNvSpPr/>
    <p:nvPr/>
  </p:nvSpPr>
  <p:spPr>
    <a:xfrm>
      <a:off x="{ox}" y="{oy}"/>
      <a:ext cx="{cx}" cy="{cy}"/>
    </a:xfrm>
    <a:prstGeom prst="{prst}"><a:avLst/></a:prstGeom>
    <a:solidFill><a:srgbClr val="{fill}"/></a:solidFill>
    {line}
  </p:spPr>
  {txBody}
</p:sp>"#,
            id = next_id,
            ox = cm_to_emu(x as f32),
            oy = cm_to_emu(y as f32),
            cx = cm_to_emu(width as f32),
            cy = cm_to_emu(height as f32),
            prst = shape,
            fill = fill_val,
            line = line_xml,
            txBody = text_body,
        );

        if let Some(pos) = slide_xml.find("</p:spTree>") {
            slide_xml.insert_str(pos, &shape_xml);
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
fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

#[cfg(feature = "execute")]
fn max_id(xml: &str) -> u32 {
    let mut max = 0u32;
    for cap in xml.match_indices("id=\"") {
        let rest = &xml[cap.0 + 4..];
        if let Some(end) = rest.find('"')
            && let Ok(n) = rest[..end].parse::<u32>()
        {
            max = max.max(n);
        }
    }
    max
}
