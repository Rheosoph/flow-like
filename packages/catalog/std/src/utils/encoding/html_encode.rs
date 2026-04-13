use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json};

#[crate::register_node]
#[derive(Default)]
pub struct HtmlEncodeNode {}

impl HtmlEncodeNode {
    pub fn new() -> Self {
        HtmlEncodeNode {}
    }
}

#[async_trait]
impl NodeLogic for HtmlEncodeNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "utils_encoding_html_encode",
            "HTML Encode",
            "Encodes special characters as HTML entities (&amp; &lt; &gt; &quot; &#39;)",
            "Utils/Encoding",
        );
        node.add_icon("/flow/icons/hash.svg");

        node.add_input_pin("input", "Input", "String to encode", VariableType::String);
        node.add_output_pin(
            "output",
            "Encoded",
            "HTML-encoded string",
            VariableType::String,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let input: String = context.evaluate_pin("input").await?;
        let encoded = html_encode(&input);
        context.set_pin_value("output", json!(encoded)).await?;
        Ok(())
    }
}

fn html_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(c),
        }
    }
    out
}
