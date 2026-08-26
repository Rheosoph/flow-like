use crate::data::path::FlowPath;
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json};

#[crate::register_node]
#[derive(Default)]
pub struct ReplaceSegmentNode {}

impl ReplaceSegmentNode {
    pub fn new() -> Self {
        ReplaceSegmentNode {}
    }
}

#[async_trait]
impl NodeLogic for ReplaceSegmentNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "path_replace_segment",
            "Replace Segment",
            "Replaces a segment in a FlowPath",
            "Data/Files/Path",
        );
        node.set_flowscript_name("path", "replaceSegment");
        node.set_receiver("in_path");
        node.add_icon("/flow/icons/path.svg");

        node.add_input_pin("in_path", "Path", "FlowPath", VariableType::Struct)
            .set_schema::<FlowPath>()
            .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin("from", "From", "Segment to replace", VariableType::String);

        node.add_input_pin("to", "To", "Replacement segment", VariableType::String);

        node.add_input_pin(
            "replace_all",
            "Replace All",
            "Replace all matching segments",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(true)));

        node.add_output_pin("out_path", "Path", "Updated FlowPath", VariableType::Struct)
            .set_schema::<FlowPath>();

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let mut path: FlowPath = context.evaluate_pin("in_path").await?;
        let from: String = context.evaluate_pin("from").await?;
        let to: String = context.evaluate_pin("to").await?;
        let replace_all: bool = context.evaluate_pin("replace_all").await?;

        if !from.is_empty() {
            let mut segments = path
                .path
                .split('/')
                .map(|segment| segment.to_string())
                .collect::<Vec<_>>();

            let mut replaced_any = false;
            for segment in segments.iter_mut() {
                if segment == &from {
                    *segment = to.clone();
                    replaced_any = true;
                    if !replace_all {
                        break;
                    }
                }
            }

            if replaced_any {
                path.path = segments.join("/");
            }
        }

        context.set_pin_value("out_path", json!(path)).await?;
        Ok(())
    }
}
