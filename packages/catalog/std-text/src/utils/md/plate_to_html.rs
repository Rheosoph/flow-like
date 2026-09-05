use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic, NodeScores},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json};

use super::plate_json::{ImageHandling, collect_media_urls, parse_plate_document, to_html};

#[crate::register_node]
#[derive(Default)]
pub struct PlateJsonToHtmlNode {}

impl PlateJsonToHtmlNode {
    pub fn new() -> Self {
        PlateJsonToHtmlNode {}
    }
}

#[async_trait]
impl NodeLogic for PlateJsonToHtmlNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "utils_md_plate_to_html",
            "Rich Text to HTML",
            "Converts a rich text document (plate_json) into HTML, keeping alignment, colours, columns and table spans that Markdown cannot express",
            "Utils/Markdown",
        );
        node.set_flowscript_name("md", "plateToHtml");

        node.add_icon("/flow/icons/web.svg");

        node.set_scores(
            NodeScores::new()
                .set_privacy(10)
                .set_security(9)
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
            "document",
            "Document",
            "Rich text document, with or without the plate_json:: prefix",
            VariableType::String,
        );

        node.add_input_pin(
            "images",
            "Images",
            "How to render image nodes",
            VariableType::String,
        )
        .set_default_value(Some(json!("keep")))
        .set_options(
            PinOptions::new()
                .set_valid_values(vec![
                    "keep".to_string(),
                    "alt_text".to_string(),
                    "strip".to_string(),
                ])
                .build(),
        );

        node.add_input_pin(
            "full_document",
            "Full Document",
            "Wrap the output in a complete HTML document with default styling",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(false)));

        node.add_input_pin(
            "title",
            "Title",
            "Document title, used only when Full Document is enabled",
            VariableType::String,
        )
        .set_default_value(Some(json!("Document")));

        node.add_output_pin(
            "exec_out",
            "Output",
            "Finished Conversion",
            VariableType::Execution,
        );

        node.add_output_pin("html", "HTML", "The converted HTML", VariableType::String);

        node.add_output_pin(
            "media",
            "Media",
            "Every image, video, audio and file reference found in the document",
            VariableType::String,
        )
        .set_value_type(flow_like::flow::pin::ValueType::Array);

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let document: String = context.evaluate_pin("document").await?;
        let images: String = context.evaluate_pin("images").await?;
        let full_document: bool = context.evaluate_pin("full_document").await?;
        let title: String = context.evaluate_pin("title").await?;

        let nodes = parse_plate_document(&document)?;
        let body = to_html(&nodes, ImageHandling::from_str_or_keep(&images));
        let media = collect_media_urls(&nodes);

        let html = if full_document {
            wrap_document(&title, &body)
        } else {
            body
        };

        context.set_pin_value("html", json!(html)).await?;
        context.set_pin_value("media", json!(media)).await?;
        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }
}

fn wrap_document(title: &str, body: &str) -> String {
    let escaped_title = title
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;");
    format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8" />
<meta name="viewport" content="width=device-width, initial-scale=1" />
<title>{escaped_title}</title>
<style>
:root {{ color-scheme: light dark; }}
body {{ margin: 0 auto; padding: 2rem 1.5rem; max-width: 48rem; font-family: system-ui, -apple-system, "Segoe UI", sans-serif; line-height: 1.65; }}
h1, h2, h3, h4, h5, h6 {{ line-height: 1.25; margin: 2rem 0 0.75rem; }}
h1 {{ font-size: 2rem; }}
p {{ margin: 0 0 1rem; }}
blockquote {{ margin: 0 0 1rem; padding: 0.25rem 0 0.25rem 1rem; border-left: 3px solid currentColor; opacity: 0.85; }}
pre {{ padding: 1rem; overflow-x: auto; border-radius: 0.5rem; background: rgba(127, 127, 127, 0.12); }}
code {{ font-family: ui-monospace, SFMono-Regular, Menlo, monospace; font-size: 0.9em; }}
pre code {{ font-size: 0.85em; }}
table {{ width: 100%; border-collapse: collapse; margin: 0 0 1rem; display: block; overflow-x: auto; }}
th, td {{ border: 1px solid rgba(127, 127, 127, 0.4); padding: 0.5rem 0.75rem; text-align: left; }}
img {{ max-width: 100%; height: auto; }}
figure {{ margin: 0 0 1rem; }}
figcaption {{ font-size: 0.875rem; opacity: 0.7; margin-top: 0.5rem; }}
hr {{ border: none; border-top: 1px solid rgba(127, 127, 127, 0.4); margin: 2rem 0; }}
.callout {{ display: flex; gap: 0.75rem; padding: 1rem; border-radius: 0.5rem; background: rgba(127, 127, 127, 0.12); margin: 0 0 1rem; }}
.task-list-item {{ list-style: none; margin-left: -1.25rem; }}
</style>
</head>
<body>
{body}</body>
</html>
"#
    )
}

#[cfg(test)]
mod tests {
    use super::wrap_document;

    #[test]
    fn wraps_and_escapes_the_title() {
        let html = wrap_document("A & B", "<p>hi</p>\n");
        assert!(html.starts_with("<!DOCTYPE html>"));
        assert!(html.contains("<title>A &amp; B</title>"));
        assert!(html.contains("<p>hi</p>"));
    }
}
