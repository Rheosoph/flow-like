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
pub struct PptxSetMetadataNode;

impl PptxSetMetadataNode {
    pub fn new() -> Self {
        Self
    }
}

#[cfg(feature = "execute")]
fn build_core_xml(title: &str, author: &str, subject: &str, keywords: &str) -> Vec<u8> {
    let mut xml = String::from(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<cp:coreProperties xmlns:cp="http://schemas.openxmlformats.org/package/2006/metadata/core-properties" xmlns:dc="http://purl.org/dc/elements/1.1/" xmlns:dcterms="http://purl.org/dc/terms/" xmlns:dcmitype="http://purl.org/dc/dcmitype/" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance">"#,
    );
    if !title.is_empty() {
        xml.push_str(&format!(
            "<dc:title>{}</dc:title>",
            quick_xml::escape::escape(title)
        ));
    }
    if !author.is_empty() {
        xml.push_str(&format!(
            "<dc:creator>{}</dc:creator>",
            quick_xml::escape::escape(author)
        ));
    }
    if !subject.is_empty() {
        xml.push_str(&format!(
            "<dc:subject>{}</dc:subject>",
            quick_xml::escape::escape(subject)
        ));
    }
    if !keywords.is_empty() {
        xml.push_str(&format!(
            "<cp:keywords>{}</cp:keywords>",
            quick_xml::escape::escape(keywords)
        ));
    }
    xml.push_str("</cp:coreProperties>");
    xml.into_bytes()
}

#[async_trait]
impl NodeLogic for PptxSetMetadataNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "pptx_set_metadata",
            "Set Metadata",
            "Set title, author, subject, keywords in presentation metadata",
            "Document/PPTX",
        );
        node.set_flowscript_name("pptx", "setMetadata");
        node.add_icon("/flow/icons/text.svg");
        node.set_scores(
            NodeScores::new()
                .set_privacy(7)
                .set_security(7)
                .set_performance(8)
                .set_governance(8)
                .set_reliability(8)
                .set_cost(10)
                .build(),
        );

        node.add_input_pin("exec_in", "Input", "Trigger", VariableType::Execution);
        node.add_input_pin("template", "Template", "PPTX file", VariableType::Struct)
            .set_schema::<FlowPath>()
            .set_options(PinOptions::new().set_enforce_schema(true).build());
        node.add_input_pin("title", "Title", "Document title", VariableType::String)
            .set_default_value(Some(json!("")));
        node.add_input_pin("author", "Author", "Document author", VariableType::String)
            .set_default_value(Some(json!("")));
        node.add_input_pin(
            "subject",
            "Subject",
            "Document subject",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));
        node.add_input_pin(
            "keywords",
            "Keywords",
            "Comma-separated keywords",
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
        context.deactivate_exec_pin("exec_out").await?;

        let template: FlowPath = context.evaluate_pin("template").await?;
        let title: String = context.evaluate_pin("title").await?;
        let author: String = context.evaluate_pin("author").await?;
        let subject: String = context.evaluate_pin("subject").await?;
        let keywords: String = context.evaluate_pin("keywords").await?;
        let output: FlowPath = context.evaluate_pin("output").await?;

        let bytes = template.get(context, false).await?;
        let mut files = read_zip(&bytes)?;

        files.insert(
            "docProps/core.xml".to_string(),
            build_core_xml(&title, &author, &subject, &keywords),
        );

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

#[crate::register_node]
#[derive(Default)]
pub struct PptxGetMetadataNode;

impl PptxGetMetadataNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for PptxGetMetadataNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "pptx_get_metadata",
            "Get Metadata",
            "Read presentation metadata (title, author, subject, keywords)",
            "Document/PPTX",
        );
        node.set_flowscript_name("pptx", "getMetadata");
        node.add_icon("/flow/icons/text.svg");
        node.set_scores(
            NodeScores::new()
                .set_privacy(8)
                .set_security(7)
                .set_performance(8)
                .set_governance(8)
                .set_reliability(8)
                .set_cost(10)
                .build(),
        );

        node.add_input_pin("exec_in", "Input", "Trigger", VariableType::Execution);
        node.add_input_pin("template", "Template", "PPTX file", VariableType::Struct)
            .set_schema::<FlowPath>()
            .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin("exec_out", "Done", "Continues", VariableType::Execution);
        node.add_output_pin("title", "Title", "Document title", VariableType::String);
        node.add_output_pin("author", "Author", "Document author", VariableType::String);
        node.add_output_pin(
            "subject",
            "Subject",
            "Document subject",
            VariableType::String,
        );
        node.add_output_pin(
            "keywords",
            "Keywords",
            "Document keywords",
            VariableType::String,
        );

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let template: FlowPath = context.evaluate_pin("template").await?;
        let bytes = template.get(context, false).await?;
        let files = read_zip(&bytes)?;

        let (mut title, mut author, mut subject, mut keywords) =
            (String::new(), String::new(), String::new(), String::new());

        if let Some(core_bytes) = files.get("docProps/core.xml") {
            let xml = String::from_utf8_lossy(core_bytes);
            title = extract_xml_value(&xml, "dc:title");
            author = extract_xml_value(&xml, "dc:creator");
            subject = extract_xml_value(&xml, "dc:subject");
            keywords = extract_xml_value(&xml, "cp:keywords");
        }

        context.set_pin_value("title", json!(title)).await?;
        context.set_pin_value("author", json!(author)).await?;
        context.set_pin_value("subject", json!(subject)).await?;
        context.set_pin_value("keywords", json!(keywords)).await?;
        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!("Requires the 'execute' feature"))
    }
}

#[cfg(feature = "execute")]
fn extract_xml_value(xml: &str, tag: &str) -> String {
    let open = format!("<{}>", tag);
    let close = format!("</{}>", tag);
    if let Some(start) = xml.find(&open) {
        let start = start + open.len();
        if let Some(end) = xml[start..].find(&close) {
            return xml[start..start + end].to_string();
        }
    }
    String::new()
}
