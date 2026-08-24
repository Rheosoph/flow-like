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
pub struct DocxReplaceTableRowNode;

impl DocxReplaceTableRowNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for DocxReplaceTableRowNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "docx_replace_table_row",
            "Replace Table Row",
            "Find a table containing a placeholder, duplicate that row for each data item, replacing placeholders per row",
            "Document/DOCX",
        );
        node.set_flowscript_name("docx", "replaceTableRow");
        node.add_icon("/flow/icons/text.svg");
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
        node.add_input_pin(
            "template",
            "Template",
            "DOCX template file",
            VariableType::Struct,
        )
        .set_schema::<FlowPath>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());
        node.add_input_pin(
            "placeholder",
            "Placeholder",
            "Placeholder in the template row (e.g. {{item}})",
            VariableType::String,
        );
        node.add_input_pin(
            "data",
            "Data",
            "JSON array of objects — each object's keys match placeholders in the row",
            VariableType::String,
        );
        node.add_input_pin(
            "output",
            "Output Path",
            "Where to save the result",
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

        let template: FlowPath = context.evaluate_pin("template").await?;
        let placeholder: String = context.evaluate_pin("placeholder").await?;
        let data_json: String = context.evaluate_pin("data").await?;
        let output: FlowPath = context.evaluate_pin("output").await?;

        let rows: Vec<std::collections::HashMap<String, String>> =
            flow_like_types::json::from_str(&data_json)?;

        let bytes = template.get(context, false).await?;
        let mut files = read_zip(&bytes)?;

        let doc_key = "word/document.xml".to_string();
        let doc_xml = files
            .get(&doc_key)
            .ok_or_else(|| flow_like_types::anyhow!("Missing word/document.xml"))?
            .clone();
        let xml = String::from_utf8_lossy(&doc_xml).to_string();

        let updated = replace_table_rows(&xml, &placeholder, &rows)?;
        files.insert(doc_key, updated.into_bytes());

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
fn replace_table_rows(
    xml: &str,
    placeholder: &str,
    rows: &[std::collections::HashMap<String, String>],
) -> flow_like_types::Result<String> {
    let tr_open = "<w:tr";
    let tr_close = "</w:tr>";
    let mut result = xml.to_string();

    let template_row = find_row_containing(xml, placeholder)?;

    let mut new_rows = String::new();
    for row_data in rows {
        let mut row_xml = template_row.clone();
        for (key, value) in row_data {
            let ph = format!("{{{{{}}}}}", key);
            row_xml = row_xml.replace(&ph, &quick_xml::escape::escape(value));
        }
        new_rows.push_str(&row_xml);
    }

    result = result.replace(&template_row, &new_rows);
    let _ = (tr_open, tr_close);
    Ok(result)
}

#[cfg(feature = "execute")]
fn find_row_containing(xml: &str, placeholder: &str) -> flow_like_types::Result<String> {
    let mut search_pos = 0;
    loop {
        let tr_start = xml[search_pos..].find("<w:tr").map(|p| p + search_pos);
        let tr_start = match tr_start {
            Some(p) => p,
            None => break,
        };

        let tr_end = xml[tr_start..]
            .find("</w:tr>")
            .map(|p| tr_start + p + "</w:tr>".len());
        let tr_end = match tr_end {
            Some(p) => p,
            None => break,
        };

        let row = &xml[tr_start..tr_end];
        if row.contains(placeholder) {
            return Ok(row.to_string());
        }

        search_pos = tr_end;
    }
    Err(flow_like_types::anyhow!(
        "No table row containing '{}' found",
        placeholder
    ))
}
