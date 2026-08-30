use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic, NodeScores},
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json};

#[crate::register_node]
#[derive(Default)]
pub struct MarkdownToHtmlNode {}

impl MarkdownToHtmlNode {
    pub fn new() -> Self {
        MarkdownToHtmlNode {}
    }
}

#[async_trait]
impl NodeLogic for MarkdownToHtmlNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "utils_md_md_to_html",
            "Markdown to HTML",
            "Renders GitHub-flavoured Markdown as HTML",
            "Utils/Markdown",
        );
        node.set_flowscript_name("md", "toHtml");
        node.set_receiver("markdown");

        node.add_icon("/flow/icons/web.svg");

        node.set_scores(
            NodeScores::new()
                .set_privacy(10)
                .set_security(8)
                .set_performance(9)
                .set_governance(10)
                .set_reliability(9)
                .set_cost(10)
                .build(),
        );

        node.add_input_pin(
            "exec_in",
            "Input",
            "Initiate Execution",
            VariableType::Execution,
        );

        node.add_input_pin(
            "markdown",
            "Markdown",
            "Markdown source to render",
            VariableType::String,
        );

        node.add_input_pin(
            "allow_html",
            "Allow HTML",
            "Pass raw HTML in the source through to the output. Leave off for untrusted input.",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(false)));

        node.add_input_pin(
            "smart_punctuation",
            "Smart Punctuation",
            "Convert quotes, dashes and ellipses to typographic equivalents",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(false)));

        node.add_output_pin(
            "exec_out",
            "Output",
            "Finished Rendering",
            VariableType::Execution,
        );

        node.add_output_pin("html", "HTML", "The rendered HTML", VariableType::String);

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let markdown: String = context.evaluate_pin("markdown").await?;
        let allow_html: bool = context.evaluate_pin("allow_html").await?;
        let smart_punctuation: bool = context.evaluate_pin("smart_punctuation").await?;

        let html = render_markdown(&markdown, allow_html, smart_punctuation);

        context.set_pin_value("html", json!(html)).await?;
        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "Markdown rendering requires the 'execute' feature"
        ))
    }
}

#[cfg(feature = "execute")]
fn render_markdown(markdown: &str, allow_html: bool, smart_punctuation: bool) -> String {
    use pulldown_cmark::{Event, Options, Parser, html};

    let mut options = Options::ENABLE_TABLES
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_FOOTNOTES
        | Options::ENABLE_HEADING_ATTRIBUTES;
    if smart_punctuation {
        options |= Options::ENABLE_SMART_PUNCTUATION;
    }

    let parser = Parser::new_ext(markdown, options);
    let mut out = String::with_capacity(markdown.len() * 2);

    if allow_html {
        html::push_html(&mut out, parser);
    } else {
        html::push_html(
            &mut out,
            parser.filter(|event| {
                !matches!(
                    event,
                    Event::Html(_) | Event::InlineHtml(_) | Event::DisplayMath(_)
                )
            }),
        );
    }

    out
}

#[cfg(all(test, feature = "execute"))]
mod tests {
    use super::render_markdown;

    #[test]
    fn renders_gfm_tables_and_task_lists() {
        let html = render_markdown(
            "| a | b |\n| --- | --- |\n| 1 | 2 |\n\n- [x] done\n- [ ] open\n",
            false,
            false,
        );
        assert!(html.contains("<table>"));
        assert!(html.contains("<th>a</th>"));
        assert!(html.contains("type=\"checkbox\""));
    }

    #[test]
    fn strips_raw_html_unless_allowed() {
        let source = "before\n\n<script>alert(1)</script>\n\nafter";
        let stripped = render_markdown(source, false, false);
        assert!(!stripped.contains("<script>"));
        assert!(stripped.contains("before"));
        assert!(stripped.contains("after"));

        let allowed = render_markdown(source, true, false);
        assert!(allowed.contains("<script>"));
    }

    #[test]
    fn renders_strikethrough_and_code_fences() {
        let html = render_markdown("~~gone~~\n\n```rust\nlet a = 1;\n```\n", false, false);
        assert!(html.contains("<del>gone</del>"));
        assert!(html.contains("<code class=\"language-rust\">"));
    }
}
