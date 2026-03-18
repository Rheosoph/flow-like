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
pub struct PptxAddChartNode;

impl PptxAddChartNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for PptxAddChartNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "pptx_add_chart",
            "Add Chart",
            "Embed a simple bar chart on a PPTX slide using DrawingML chart XML.",
            "Document/PPTX",
        );
        node.add_icon("/flow/icons/chart.svg");
        node.set_scores(
            NodeScores::new()
                .set_privacy(9)
                .set_security(7)
                .set_performance(5)
                .set_governance(8)
                .set_reliability(7)
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
            "chart_type",
            "Chart Type",
            "bar, line, or pie",
            VariableType::String,
        )
        .set_default_value(Some(json!("bar")));
        node.add_input_pin(
            "categories",
            "Categories",
            "Category labels",
            VariableType::String,
        )
        .set_value_type(flow_like::flow::pin::ValueType::Array);
        node.add_input_pin("values", "Values", "Numeric values", VariableType::Float)
            .set_value_type(flow_like::flow::pin::ValueType::Array);
        node.add_input_pin(
            "series_name",
            "Series Name",
            "Series label",
            VariableType::String,
        )
        .set_default_value(Some(json!("Series 1")));
        node.add_input_pin("x", "X", "X position in cm", VariableType::Float)
            .set_default_value(Some(json!(3.0)));
        node.add_input_pin("y", "Y", "Y position in cm", VariableType::Float)
            .set_default_value(Some(json!(3.0)));
        node.add_input_pin("width", "Width", "Width in cm", VariableType::Float)
            .set_default_value(Some(json!(24.0)));
        node.add_input_pin("height", "Height", "Height in cm", VariableType::Float)
            .set_default_value(Some(json!(12.0)));
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
        let chart_type: String = context.evaluate_pin("chart_type").await?;
        let categories: Vec<String> = context.evaluate_pin("categories").await?;
        let values: Vec<f64> = context.evaluate_pin("values").await?;
        let series_name: String = context.evaluate_pin("series_name").await?;
        let x: f64 = context.evaluate_pin("x").await?;
        let y: f64 = context.evaluate_pin("y").await?;
        let width: f64 = context.evaluate_pin("width").await?;
        let height: f64 = context.evaluate_pin("height").await?;
        let output: FlowPath = context.evaluate_pin("output").await?;

        let bytes = template.get(context, false).await?;
        let mut files = read_zip(&bytes)?;

        let chart_num = next_chart_number(&files);
        let chart_path = format!("ppt/charts/chart{}.xml", chart_num);
        let chart_rels_path = format!("ppt/charts/_rels/chart{}.xml.rels", chart_num);

        let chart_xml = build_chart_xml(&chart_type, &categories, &values, &series_name);
        files.insert(chart_path.clone(), chart_xml.into_bytes());
        files.insert(
            chart_rels_path,
            r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"/>"#
                .as_bytes()
                .to_vec(),
        );

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
            "http://schemas.openxmlformats.org/officeDocument/2006/relationships/chart",
            &format!("../charts/chart{}.xml", chart_num),
        );

        update_content_types_for_chart(&mut files, chart_num);

        let mut slide_xml = String::from_utf8_lossy(&slide_data).to_string();
        let next_id = max_id(&slide_xml) + 1;

        let frame_xml = format!(
            r#"<p:graphicFrame>
  <p:nvGraphicFramePr>
    <p:cNvPr id="{id}" name="Chart {id}"/>
    <p:cNvGraphicFramePr><a:graphicFrameLocks noGrp="1"/></p:cNvGraphicFramePr>
    <p:nvPr/>
  </p:nvGraphicFramePr>
  <p:xfrm>
    <a:off x="{ox}" y="{oy}"/>
    <a:ext cx="{cx}" cy="{cy}"/>
  </p:xfrm>
  <a:graphic>
    <a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/chart">
      <c:chart xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" r:id="{rid}"/>
    </a:graphicData>
  </a:graphic>
</p:graphicFrame>"#,
            id = next_id,
            ox = cm_to_emu(x as f32),
            oy = cm_to_emu(y as f32),
            cx = cm_to_emu(width as f32),
            cy = cm_to_emu(height as f32),
            rid = rid,
        );

        if let Some(pos) = slide_xml.find("</p:spTree>") {
            slide_xml.insert_str(pos, &frame_xml);
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
fn build_chart_xml(
    chart_type: &str,
    categories: &[String],
    values: &[f64],
    series_name: &str,
) -> String {
    let mut cat_xml = String::new();
    for (i, cat) in categories.iter().enumerate() {
        cat_xml.push_str(&format!(
            r#"<c:pt idx="{}"><c:v>{}</c:v></c:pt>"#,
            i,
            xml_escape(cat),
        ));
    }

    let mut val_xml = String::new();
    for (i, val) in values.iter().enumerate() {
        val_xml.push_str(&format!(r#"<c:pt idx="{}"><c:v>{}</c:v></c:pt>"#, i, val));
    }

    let count = categories.len();

    let chart_element = match chart_type {
        "pie" => format!(
            r#"<c:pieChart>
  <c:ser>
    <c:idx val="0"/><c:order val="0"/>
    <c:tx><c:v>{sn}</c:v></c:tx>
    <c:spPr><a:solidFill><a:srgbClr val="FF4343"/></a:solidFill></c:spPr>
    <c:cat><c:strRef><c:strCache><c:ptCount val="{cnt}"/>{cats}</c:strCache></c:strRef></c:cat>
    <c:val><c:numRef><c:numCache><c:ptCount val="{cnt}"/>{vals}</c:numCache></c:numRef></c:val>
  </c:ser>
</c:pieChart>"#,
            sn = xml_escape(series_name),
            cnt = count,
            cats = cat_xml,
            vals = val_xml,
        ),
        "line" => format!(
            r#"<c:lineChart>
  <c:grouping val="standard"/>
  <c:ser>
    <c:idx val="0"/><c:order val="0"/>
    <c:tx><c:v>{sn}</c:v></c:tx>
    <c:spPr><a:ln w="28575"><a:solidFill><a:srgbClr val="FF4343"/></a:solidFill></a:ln></c:spPr>
    <c:cat><c:strRef><c:strCache><c:ptCount val="{cnt}"/>{cats}</c:strCache></c:strRef></c:cat>
    <c:val><c:numRef><c:numCache><c:ptCount val="{cnt}"/>{vals}</c:numCache></c:numRef></c:val>
  </c:ser>
</c:lineChart>"#,
            sn = xml_escape(series_name),
            cnt = count,
            cats = cat_xml,
            vals = val_xml,
        ),
        _ => format!(
            r#"<c:barChart>
  <c:barDir val="col"/>
  <c:grouping val="clustered"/>
  <c:ser>
    <c:idx val="0"/><c:order val="0"/>
    <c:tx><c:v>{sn}</c:v></c:tx>
    <c:spPr><a:solidFill><a:srgbClr val="FF4343"/></a:solidFill></c:spPr>
    <c:cat><c:strRef><c:strCache><c:ptCount val="{cnt}"/>{cats}</c:strCache></c:strRef></c:cat>
    <c:val><c:numRef><c:numCache><c:ptCount val="{cnt}"/>{vals}</c:numCache></c:numRef></c:val>
  </c:ser>
</c:barChart>"#,
            sn = xml_escape(series_name),
            cnt = count,
            cats = cat_xml,
            vals = val_xml,
        ),
    };

    format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<c:chartSpace xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart"
  xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"
  xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">
  <c:chart>
    <c:plotArea>
      <c:layout/>
      {chart}
    </c:plotArea>
    <c:legend><c:legendPos val="b"/></c:legend>
  </c:chart>
</c:chartSpace>"#,
        chart = chart_element,
    )
}

#[cfg(feature = "execute")]
fn next_chart_number(files: &std::collections::HashMap<String, Vec<u8>>) -> u32 {
    let mut max = 0u32;
    for key in files.keys() {
        if let Some(rest) = key.strip_prefix("ppt/charts/chart") {
            if let Some(num_str) = rest.strip_suffix(".xml") {
                if let Ok(n) = num_str.parse::<u32>() {
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
fn update_content_types_for_chart(
    files: &mut std::collections::HashMap<String, Vec<u8>>,
    chart_num: u32,
) {
    if let Some(ct_data) = files.get("[Content_Types].xml") {
        let mut content = String::from_utf8_lossy(ct_data).to_string();
        let entry = format!(
            r#"<Override PartName="/ppt/charts/chart{}.xml" ContentType="application/vnd.openxmlformats-officedocument.drawingml.chart+xml"/>"#,
            chart_num,
        );
        if let Some(pos) = content.find("</Types>") {
            content.insert_str(pos, &entry);
        }
        files.insert("[Content_Types].xml".to_string(), content.into_bytes());
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

#[cfg(feature = "execute")]
fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}
