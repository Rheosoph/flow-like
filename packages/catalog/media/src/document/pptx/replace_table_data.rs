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
pub struct PptxReplaceTableDataNode;

impl PptxReplaceTableDataNode {
    pub fn new() -> Self {
        Self
    }
}

#[cfg(feature = "execute")]
fn build_cell_xml(text: &str, template_tc_pr: &str) -> String {
    let escaped = text
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;");
    let tc_pr = if template_tc_pr.is_empty() {
        String::new()
    } else {
        format!("<a:tcPr{template_tc_pr}/>")
    };
    format!(
        "<a:tc>{tc_pr}<a:txBody><a:bodyPr/><a:lstStyle/><a:p><a:r><a:t>{escaped}</a:t></a:r></a:p></a:txBody></a:tc>"
    )
}

#[cfg(feature = "execute")]
fn build_row_xml(cells: &[String], template_tc_pr: &str) -> String {
    let mut row = String::from(r#"<a:tr h="370840">"#);
    for cell in cells {
        row.push_str(&build_cell_xml(cell, template_tc_pr));
    }
    row.push_str("</a:tr>");
    row
}

#[async_trait]
impl NodeLogic for PptxReplaceTableDataNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "pptx_replace_table_data",
            "Replace Table Data",
            "Populate a table on a slide that contains a placeholder in its first cell with structured data (JSON array of arrays). Inherits the table's existing styling.",
            "Document/PPTX",
        );
        node.add_icon("/flow/icons/text.svg");

        node.set_scores(
            NodeScores::new()
                .set_privacy(8)
                .set_security(7)
                .set_performance(6)
                .set_governance(8)
                .set_reliability(7)
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
            "Which slide contains the table (1-based)",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(1)));

        node.add_input_pin(
            "placeholder",
            "Placeholder",
            "Placeholder text to find in the table",
            VariableType::String,
        );

        node.add_input_pin(
            "data",
            "Data",
            "JSON array of arrays with table data",
            VariableType::String,
        );

        node.add_input_pin(
            "has_header",
            "Has Header",
            "Whether the first row of data is a header row",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(true)));

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
            "Execution continues after replacement",
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
        let placeholder: String = context.evaluate_pin("placeholder").await?;
        let data_json: String = context.evaluate_pin("data").await?;
        let _has_header: bool = context.evaluate_pin("has_header").await?;
        let output: FlowPath = context.evaluate_pin("output").await?;

        if slide_index < 1 {
            return Err(flow_like_types::anyhow!("Slide index must be >= 1"));
        }

        let data: Vec<Vec<String>> = flow_like_types::json::from_str(&data_json)
            .map_err(|e| flow_like_types::anyhow!("Invalid JSON data: {e}"))?;

        if data.is_empty() {
            return Err(flow_like_types::anyhow!("Data array is empty"));
        }

        let num_cols = data.iter().map(|r| r.len()).max().unwrap_or(0);

        let template_bytes = template.get(context, false).await?;
        let mut files = read_zip(&template_bytes)?;

        let slide_key = format!("ppt/slides/slide{}.xml", slide_index);
        let slide_bytes = files
            .get(&slide_key)
            .ok_or_else(|| flow_like_types::anyhow!("Slide {} not found", slide_index))?
            .clone();
        let mut slide_xml = String::from_utf8_lossy(&slide_bytes).to_string();

        let tbl_re = Regex::new(r"(?s)<a:tbl>(.*?)</a:tbl>")?;
        let mut found_table = false;

        let tables: Vec<(usize, usize, String)> = tbl_re
            .find_iter(&slide_xml)
            .map(|m| (m.start(), m.end(), m.as_str().to_string()))
            .collect();

        let tc_pr_re = Regex::new(r"<a:tcPr([^/]*)/>")?;
        let tbl_pr_re = Regex::new(r"(?s)<a:tblPr[^>]*>.*?</a:tblPr>|<a:tblPr[^/]*/>")?;

        for (start, end, tbl_content) in tables.iter().rev() {
            if !tbl_content.contains(&placeholder) {
                continue;
            }
            found_table = true;

            let template_tc_pr = tc_pr_re
                .captures(tbl_content)
                .and_then(|c| c.get(1))
                .map(|m| m.as_str().to_string())
                .unwrap_or_default();

            let tbl_pr = tbl_pr_re
                .find(tbl_content)
                .map(|m| m.as_str().to_string())
                .unwrap_or_else(|| "<a:tblPr/>".to_string());

            let grid_cols: String = (0..num_cols)
                .map(|_| r#"<a:gridCol w="1000000"/>"#)
                .collect::<Vec<_>>()
                .join("");
            let tbl_grid = format!("<a:tblGrid>{grid_cols}</a:tblGrid>");

            let mut rows_xml = String::new();
            for row in &data {
                let mut padded = row.clone();
                padded.resize(num_cols, String::new());
                rows_xml.push_str(&build_row_xml(&padded, &template_tc_pr));
            }

            let new_tbl = format!("<a:tbl>{tbl_pr}{tbl_grid}{rows_xml}</a:tbl>");
            slide_xml = format!("{}{}{}", &slide_xml[..*start], new_tbl, &slide_xml[*end..]);
            break;
        }

        if !found_table {
            return Err(flow_like_types::anyhow!(
                "No table containing placeholder '{}' found on slide {}",
                placeholder,
                slide_index
            ));
        }

        files.insert(slide_key, slide_xml.into_bytes());

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
