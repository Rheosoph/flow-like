use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic, NodeScores},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_catalog_core::FlowPath;
use flow_like_types::{async_trait, json::json};

use crate::document::ImageScaleMode;

#[cfg(feature = "execute")]
use crate::document::openxml::{read_zip, replace_image_in_archive, write_zip};

#[crate::register_node]
#[derive(Default)]
pub struct PptxReplaceImageNode {}

impl PptxReplaceImageNode {
    pub fn new() -> Self {
        PptxReplaceImageNode {}
    }
}

#[async_trait]
impl NodeLogic for PptxReplaceImageNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "pptx_replace_image",
            "Replace Image in PPTX",
            "Replaces images in a PowerPoint (PPTX) file by matching alt text or shape name",
            "Document/PPTX",
        );
        node.set_flowscript_name("pptx", "replaceImage");
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
            "Path to the PPTX template file",
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
            "Path to the replacement image file",
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
            "Path where the resulting PPTX file will be saved",
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
            "Path to the generated PPTX file",
            VariableType::Struct,
        )
        .set_schema::<FlowPath>();

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let template: FlowPath = context.evaluate_pin("template").await?;
        let identifier: String = context.evaluate_pin("identifier").await?;
        let image_path: FlowPath = context.evaluate_pin("image").await?;
        let scale_mode: ImageScaleMode = context.evaluate_pin("scale_mode").await?;
        let output: FlowPath = context.evaluate_pin("output").await?;

        let template_bytes = template.get(context, false).await?;
        let image_bytes = image_path.get(context, false).await?;
        let mut files = read_zip(&template_bytes)?;

        let slide_keys: Vec<String> = files
            .keys()
            .filter(|k| k.starts_with("ppt/slides/slide") && k.ends_with(".xml"))
            .cloned()
            .collect();

        for key in &slide_keys {
            let rels_key = key
                .replace("ppt/slides/", "ppt/slides/_rels/")
                .replace(".xml", ".xml.rels");

            if !files.contains_key(&rels_key) {
                continue;
            }

            replace_image_in_archive(
                &mut files,
                key,
                &rels_key,
                &identifier,
                &image_bytes,
                &scale_mode,
            )?;
        }

        let result_bytes = write_zip(&files)?;
        output.put(context, result_bytes, false).await?;

        context.set_pin_value("result", json!(output)).await?;
        context.activate_exec_pin("exec_out").await?;

        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "PPTX image replacement requires the 'execute' feature"
        ))
    }
}
