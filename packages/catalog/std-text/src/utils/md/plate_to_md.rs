use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic, NodeScores},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json};

use super::plate_json::{ImageHandling, collect_media_urls, parse_plate_document, to_markdown};

#[crate::register_node]
#[derive(Default)]
pub struct PlateJsonToMarkdownNode {}

impl PlateJsonToMarkdownNode {
    pub fn new() -> Self {
        PlateJsonToMarkdownNode {}
    }
}

#[async_trait]
impl NodeLogic for PlateJsonToMarkdownNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "utils_md_plate_to_md",
            "Rich Text to Markdown",
            "Converts a rich text document (plate_json) into GitHub-flavoured Markdown",
            "Utils/Markdown",
        );
        node.set_flowscript_name("md", "fromPlate");

        node.add_icon("/flow/icons/text.svg");

        node.set_scores(
            NodeScores::new()
                .set_privacy(10)
                .set_security(10)
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

        node.add_output_pin(
            "exec_out",
            "Output",
            "Finished Conversion",
            VariableType::Execution,
        );

        node.add_output_pin(
            "markdown",
            "Markdown",
            "The converted Markdown",
            VariableType::String,
        );

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

        let nodes = parse_plate_document(&document)?;
        let markdown = to_markdown(&nodes, ImageHandling::from_str_or_keep(&images));
        let media = collect_media_urls(&nodes);

        context.set_pin_value("markdown", json!(markdown)).await?;
        context.set_pin_value("media", json!(media)).await?;
        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }
}
