use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json};

#[crate::register_node]
#[derive(Default)]
pub struct HtmlDecodeNode {}

impl HtmlDecodeNode {
    pub fn new() -> Self {
        HtmlDecodeNode {}
    }
}

#[async_trait]
impl NodeLogic for HtmlDecodeNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "utils_encoding_html_decode",
            "HTML Decode",
            "Decodes HTML entities back to their original characters",
            "Utils/Encoding",
        );
        node.set_flowscript_name("encoding", "htmlDecode");
        node.set_receiver("input");
        node.add_icon("/flow/icons/hash.svg");

        node.add_input_pin(
            "input",
            "Input",
            "HTML-encoded string",
            VariableType::String,
        );
        node.add_output_pin("output", "Decoded", "Decoded string", VariableType::String);

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let input: String = context.evaluate_pin("input").await?;
        let decoded = html_decode(&input);
        context.set_pin_value("output", json!(decoded)).await?;
        Ok(())
    }
}

fn html_decode(s: &str) -> String {
    s.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&#x27;", "'")
        .replace("&#x2F;", "/")
}
