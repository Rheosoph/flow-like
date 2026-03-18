#[cfg(feature = "execute")]
use lopdf::{Document, Object};

use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic, NodeScores},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_catalog_core::FlowPath;
use flow_like_types::{async_trait, json::json};

// ── Set Metadata ──

#[crate::register_node]
#[derive(Default)]
pub struct PdfSetMetadataNode;

impl PdfSetMetadataNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for PdfSetMetadataNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "pdf_set_metadata",
            "Set Metadata",
            "Set title, author, subject, and keywords in a PDF's Info dictionary.",
            "Document/PDF",
        );
        node.add_icon("/flow/icons/text.svg");
        node.set_scores(
            NodeScores::new()
                .set_privacy(7)
                .set_security(7)
                .set_performance(8)
                .set_governance(9)
                .set_reliability(8)
                .set_cost(10)
                .build(),
        );

        node.add_input_pin("exec_in", "Input", "Trigger", VariableType::Execution);
        node.add_input_pin("template", "Template", "PDF file", VariableType::Struct)
            .set_schema::<FlowPath>()
            .set_options(PinOptions::new().set_enforce_schema(true).build());
        node.add_input_pin("title", "Title", "Document title", VariableType::String)
            .set_default_value(Some(json!("")));
        node.add_input_pin("author", "Author", "Author", VariableType::String)
            .set_default_value(Some(json!("")));
        node.add_input_pin("subject", "Subject", "Subject", VariableType::String)
            .set_default_value(Some(json!("")));
        node.add_input_pin("keywords", "Keywords", "Keywords", VariableType::String)
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
        let mut doc = Document::load_mem(&bytes)?;

        let trailer = doc.trailer.clone();
        if let Ok(Object::Reference(info_ref)) = trailer.get(b"Info") {
            if let Ok(Object::Dictionary(info)) = doc.get_object_mut(*info_ref) {
                set_if_not_empty(info, b"Title", &title);
                set_if_not_empty(info, b"Author", &author);
                set_if_not_empty(info, b"Subject", &subject);
                set_if_not_empty(info, b"Keywords", &keywords);
            }
        } else {
            let mut info = lopdf::Dictionary::new();
            set_if_not_empty(&mut info, b"Title", &title);
            set_if_not_empty(&mut info, b"Author", &author);
            set_if_not_empty(&mut info, b"Subject", &subject);
            set_if_not_empty(&mut info, b"Keywords", &keywords);

            let info_id = doc.add_object(Object::Dictionary(info));
            doc.trailer.set("Info", Object::Reference(info_id));
        }

        let mut buf = Vec::new();
        doc.save_to(&mut buf)?;
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
fn set_if_not_empty(dict: &mut lopdf::Dictionary, key: &[u8], value: &str) {
    if !value.is_empty() {
        dict.set(
            key,
            Object::String(value.as_bytes().to_vec(), lopdf::StringFormat::Literal),
        );
    }
}

// ── Get Metadata ──

#[crate::register_node]
#[derive(Default)]
pub struct PdfGetMetadataNode;

impl PdfGetMetadataNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for PdfGetMetadataNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "pdf_get_metadata",
            "Get Metadata",
            "Read title, author, subject, keywords, and page count from a PDF.",
            "Document/PDF",
        );
        node.add_icon("/flow/icons/text.svg");
        node.set_scores(
            NodeScores::new()
                .set_privacy(7)
                .set_security(8)
                .set_performance(9)
                .set_governance(8)
                .set_reliability(8)
                .set_cost(10)
                .build(),
        );

        node.add_input_pin("exec_in", "Input", "Trigger", VariableType::Execution);
        node.add_input_pin("template", "Template", "PDF file", VariableType::Struct)
            .set_schema::<FlowPath>()
            .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin("exec_out", "Done", "Continues", VariableType::Execution);
        node.add_output_pin("title", "Title", "Document title", VariableType::String);
        node.add_output_pin("author", "Author", "Author", VariableType::String);
        node.add_output_pin("subject", "Subject", "Subject", VariableType::String);
        node.add_output_pin("keywords", "Keywords", "Keywords", VariableType::String);
        node.add_output_pin(
            "page_count",
            "Page Count",
            "Number of pages",
            VariableType::Integer,
        );

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let template: FlowPath = context.evaluate_pin("template").await?;
        let bytes = template.get(context, false).await?;
        let doc = Document::load_mem(&bytes)?;

        let mut title = String::new();
        let mut author = String::new();
        let mut subject = String::new();
        let mut keywords = String::new();

        if let Ok(Object::Reference(info_ref)) = doc.trailer.get(b"Info") {
            if let Ok(Object::Dictionary(info)) = doc.get_object(*info_ref) {
                title = get_string_field(info, b"Title");
                author = get_string_field(info, b"Author");
                subject = get_string_field(info, b"Subject");
                keywords = get_string_field(info, b"Keywords");
            }
        }

        let page_count = doc.page_iter().count() as i64;

        context.set_pin_value("title", json!(title)).await?;
        context.set_pin_value("author", json!(author)).await?;
        context.set_pin_value("subject", json!(subject)).await?;
        context.set_pin_value("keywords", json!(keywords)).await?;
        context
            .set_pin_value("page_count", json!(page_count))
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
fn get_string_field(dict: &lopdf::Dictionary, key: &[u8]) -> String {
    match dict.get(key) {
        Ok(Object::String(bytes, _)) => String::from_utf8_lossy(bytes).to_string(),
        _ => String::new(),
    }
}
