use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic, NodeScores},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_catalog_core::FlowPath;
use flow_like_types::{async_trait, json::json};

#[cfg(feature = "execute")]
use crate::document::openxml::read_zip;

#[crate::register_node]
#[derive(Default)]
pub struct DocxListPlaceholdersNode;

impl DocxListPlaceholdersNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for DocxListPlaceholdersNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "docx_list_placeholders",
            "List Placeholders",
            "Scan document body, headers, footers for all {{...}} placeholder strings",
            "Document/DOCX",
        );
        node.set_flowscript_name("docx", "listPlaceholders");
        node.add_icon("/flow/icons/text.svg");
        node.set_scores(
            NodeScores::new()
                .set_privacy(8)
                .set_security(7)
                .set_performance(8)
                .set_governance(8)
                .set_reliability(9)
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

        node.add_output_pin("exec_out", "Done", "Continues", VariableType::Execution);
        node.add_output_pin(
            "placeholders",
            "Placeholders",
            "List of placeholder strings found",
            VariableType::String,
        )
        .set_value_type(flow_like::flow::pin::ValueType::Array);

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let template: FlowPath = context.evaluate_pin("template").await?;
        let bytes = template.get(context, false).await?;
        let files = read_zip(&bytes)?;

        let re = flow_like_types::regex::Regex::new(r"\{\{([^}]+)\}\}")?;
        let mut placeholders = std::collections::BTreeSet::new();

        for (key, data) in &files {
            if key == "word/document.xml"
                || (key.starts_with("word/header") && key.ends_with(".xml"))
                || (key.starts_with("word/footer") && key.ends_with(".xml"))
            {
                let xml = String::from_utf8_lossy(data);
                let text = extract_text_from_w_xml(&xml);
                for caps in re.captures_iter(&text) {
                    placeholders.insert(format!("{{{{{}}}}}", &caps[1]));
                }
            }
        }

        let list: Vec<String> = placeholders.into_iter().collect();
        context.set_pin_value("placeholders", json!(list)).await?;
        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!("Requires the 'execute' feature"))
    }
}

#[cfg(feature = "execute")]
fn extract_text_from_w_xml(xml: &str) -> String {
    let mut result = String::new();
    let mut in_t = false;
    let reader = quick_xml::Reader::from_str(xml);
    let mut reader = reader;
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(quick_xml::events::Event::Start(ref e) | quick_xml::events::Event::Empty(ref e))
                if e.name().as_ref() == b"w:t" =>
            {
                in_t = true;
            }
            Ok(quick_xml::events::Event::End(ref e)) if e.name().as_ref() == b"w:t" => {
                in_t = false;
            }
            Ok(quick_xml::events::Event::Text(ref e)) if in_t => {
                if let Ok(text) = e.decode() {
                    result.push_str(&text);
                }
            }
            Ok(quick_xml::events::Event::Eof) => break,
            Err(_) => break,
            _ => {}
        }
        buf.clear();
    }
    result
}
