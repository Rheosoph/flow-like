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
pub struct PptxAddTableToSlideNode;

impl PptxAddTableToSlideNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for PptxAddTableToSlideNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "pptx_add_table_to_slide",
            "Add Table to Slide",
            "Add a branded table to a PPTX slide. Header row uses #FF4343 with white text.",
            "Document/PPTX",
        );
        node.add_icon("/flow/icons/table.svg");
        node.set_scores(
            NodeScores::new()
                .set_privacy(9)
                .set_security(8)
                .set_performance(6)
                .set_governance(8)
                .set_reliability(8)
                .set_cost(10)
                .build(),
        );

        node.add_input_pin("exec_in", "Input", "Trigger", VariableType::Execution);
        node.add_input_pin("template", "Template", "PPTX file", VariableType::Struct)
            .set_schema::<FlowPath>()
            .set_options(PinOptions::new().set_enforce_schema(true).build());
        node.add_input_pin("slide_number", "Slide Number", "1-based slide index", VariableType::Integer)
            .set_default_value(Some(json!(1)));
        node.add_input_pin("headers", "Headers", "Column headers", VariableType::String)
            .set_value_type(flow_like::flow::pin::ValueType::Array);
        node.add_input_pin("rows", "Rows", "Table data as JSON array of arrays", VariableType::String)
            .set_value_type(flow_like::flow::pin::ValueType::Array);
        node.add_input_pin("x", "X", "X position in cm", VariableType::Float)
            .set_default_value(Some(json!(2.0)));
        node.add_input_pin("y", "Y", "Y position in cm", VariableType::Float)
            .set_default_value(Some(json!(2.0)));
        node.add_input_pin("width", "Width", "Table width in cm", VariableType::Float)
            .set_default_value(Some(json!(28.0)));
        node.add_input_pin("row_height", "Row Height", "Row height in cm", VariableType::Float)
            .set_default_value(Some(json!(1.0)));
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
        let slide_number: i64 = context.evaluate_pin("slide_number").await?;
        let headers: Vec<String> = context.evaluate_pin("headers").await?;
        let rows: Vec<String> = context.evaluate_pin("rows").await?;
        let x: f64 = context.evaluate_pin("x").await?;
        let y: f64 = context.evaluate_pin("y").await?;
        let width: f64 = context.evaluate_pin("width").await?;
        let row_height: f64 = context.evaluate_pin("row_height").await?;
        let output: FlowPath = context.evaluate_pin("output").await?;

        let bytes = template.get(context, false).await?;
        let mut files = read_zip(&bytes)?;

        let slide_path = format!("ppt/slides/slide{}.xml", slide_number);
        let slide_data = files
            .get(&slide_path)
            .ok_or_else(|| flow_like_types::anyhow!("Slide {} not found", slide_number))?
            .clone();
        let mut slide_xml = String::from_utf8_lossy(&slide_data).to_string();

        let col_count = headers.len().max(1);
        let col_width = cm_to_emu(width as f32) / col_count as i64;
        let rh = cm_to_emu(row_height as f32);
        let total_rows = 1 + rows.len();
        let total_height = rh * total_rows as i64;
        let next_id = max_id(&slide_xml) + 1;

        let mut grid_cols = String::new();
        for _ in 0..col_count {
            grid_cols.push_str(&format!(r#"<a:gridCol w="{}"/>"#, col_width));
        }

        let mut rows_xml = String::new();

        // Header row
        rows_xml.push_str(&format!(r#"<a:tr h="{}">"#, rh));
        for h in &headers {
            rows_xml.push_str(&build_cell(&xml_escape(h), "FF4343", "FFFFFF", true));
        }
        rows_xml.push_str("</a:tr>");

        // Data rows
        for (row_idx, row_data) in rows.iter().enumerate() {
            let cells: Vec<String> = flow_like_types::json::from_str(row_data).unwrap_or_default();
            let bg = if row_idx % 2 == 0 { "F9FAFB" } else { "FFFFFF" };
            rows_xml.push_str(&format!(r#"<a:tr h="{}">"#, rh));
            for i in 0..col_count {
                let val = cells.get(i).map(|s| s.as_str()).unwrap_or("");
                rows_xml.push_str(&build_cell(&xml_escape(val), bg, "1A1A1A", false));
            }
            rows_xml.push_str("</a:tr>");
        }

        let table_xml = format!(
            r#"<p:graphicFrame>
  <p:nvGraphicFramePr>
    <p:cNvPr id="{id}" name="Table {id}"/>
    <p:cNvGraphicFramePr><a:graphicFrameLocks noGrp="1"/></p:cNvGraphicFramePr>
    <p:nvPr/>
  </p:nvGraphicFramePr>
  <p:xfrm>
    <a:off x="{ox}" y="{oy}"/>
    <a:ext cx="{cx}" cy="{cy}"/>
  </p:xfrm>
  <a:graphic>
    <a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/table">
      <a:tbl>
        <a:tblPr firstRow="1" bandRow="1"/>
        <a:tblGrid>{grid}</a:tblGrid>
        {rows}
      </a:tbl>
    </a:graphicData>
  </a:graphic>
</p:graphicFrame>"#,
            id = next_id,
            ox = cm_to_emu(x as f32),
            oy = cm_to_emu(y as f32),
            cx = cm_to_emu(width as f32),
            cy = total_height,
            grid = grid_cols,
            rows = rows_xml,
        );

        if let Some(pos) = slide_xml.find("</p:spTree>") {
            slide_xml.insert_str(pos, &table_xml);
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
fn build_cell(text: &str, bg_color: &str, text_color: &str, bold: bool) -> String {
    let bold_attr = if bold { r#" b="1""# } else { "" };
    format!(
        r#"<a:tc>
  <a:txBody>
    <a:bodyPr/>
    <a:lstStyle/>
    <a:p>
      <a:r>
        <a:rPr lang="en-US" sz="1400" dirty="0"{bold}>
          <a:solidFill><a:srgbClr val="{tc}"/></a:solidFill>
          <a:latin typeface="Calibri"/>
        </a:rPr>
        <a:t>{text}</a:t>
      </a:r>
    </a:p>
  </a:txBody>
  <a:tcPr>
    <a:solidFill><a:srgbClr val="{bg}"/></a:solidFill>
  </a:tcPr>
</a:tc>"#,
        bold = bold_attr,
        tc = text_color,
        bg = bg_color,
        text = text,
    )
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
        if let Some(end) = rest.find('"') {
            if let Ok(n) = rest[..end].parse::<u32>() {
                max = max.max(n);
            }
        }
    }
    max
}
