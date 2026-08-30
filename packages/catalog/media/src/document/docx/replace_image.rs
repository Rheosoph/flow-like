#[cfg(feature = "execute")]
use crate::document::openxml::{read_zip, replace_image_in_archive, write_zip};
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic, NodeScores},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_catalog_core::FlowPath;
use flow_like_types::{async_trait, json::json};

use crate::document::ImageScaleMode;

#[crate::register_node]
#[derive(Default)]
pub struct DocxReplaceImageNode;

impl DocxReplaceImageNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for DocxReplaceImageNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "docx_replace_image",
            "Replace Image in DOCX",
            "Replace an image in a DOCX file by matching alt text or shape name",
            "Document/DOCX",
        );
        node.set_flowscript_name("docx", "replaceImage");
        node.add_icon("/flow/icons/image.svg");

        node.set_scores(
            NodeScores::new()
                .set_privacy(8)
                .set_security(7)
                .set_performance(7)
                .set_governance(8)
                .set_reliability(8)
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
            "DOCX template file",
            VariableType::Struct,
        )
        .set_schema::<FlowPath>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin(
            "identifier",
            "Identifier",
            "Alt text or shape name of the image to replace",
            VariableType::String,
        );

        node.add_input_pin(
            "image",
            "Image",
            "Replacement image file",
            VariableType::Struct,
        )
        .set_schema::<FlowPath>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin(
            "scale_mode",
            "Scale Mode",
            "How to handle dimensions: KeepWidth (proportional), KeepHeight (proportional), Stretch (force both, may distort), or None (use new image size)",
            VariableType::String,
        )
        .set_schema::<ImageScaleMode>()
        .set_options(
            PinOptions::new()
                .set_valid_values(vec![
                    "None".to_string(),
                    "KeepWidth".to_string(),
                    "KeepHeight".to_string(),
                    "Stretch".to_string(),
                ])
                .build(),
        )
        .set_default_value(Some(json!("KeepWidth")));

        node.add_input_pin(
            "output",
            "Output Path",
            "Where to save the resulting DOCX file",
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
            "Path to the output file",
            VariableType::Struct,
        )
        .set_schema::<FlowPath>();

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let template_path: FlowPath = context.evaluate_pin("template").await?;
        let identifier: String = context.evaluate_pin("identifier").await?;
        let image_path: FlowPath = context.evaluate_pin("image").await?;
        let scale_mode: ImageScaleMode = context.evaluate_pin("scale_mode").await?;
        let output_path: FlowPath = context.evaluate_pin("output").await?;

        let template_bytes = template_path.get(context, false).await?;
        let new_image_bytes = image_path.get(context, false).await?;

        let mut files = read_zip(&template_bytes)?;

        replace_image_in_archive(
            &mut files,
            "word/document.xml",
            "word/_rels/document.xml.rels",
            &identifier,
            &new_image_bytes,
            &scale_mode,
        )?;

        let result_bytes = write_zip(&files)?;
        output_path.put(context, result_bytes, false).await?;

        context.set_pin_value("result", json!(output_path)).await?;
        context.activate_exec_pin("exec_out").await?;

        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "DOCX processing requires the 'execute' feature"
        ))
    }
}
