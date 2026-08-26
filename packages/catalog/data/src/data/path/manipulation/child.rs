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
pub struct ChildNode {}

impl ChildNode {
    pub fn new() -> Self {
        ChildNode {}
    }
}

#[async_trait]
impl NodeLogic for ChildNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "child",
            "Child",
            "Creates a child path from a parent path",
            "Data/Files/Path",
        );
        node.set_flowscript_name("path", "child");
        node.set_receiver("parent_path");
        node.add_icon("/flow/icons/path.svg");

        node.add_input_pin(
            "parent_path",
            "Path",
            "Parent FlowPath",
            VariableType::Struct,
        )
        .set_schema::<FlowPath>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin(
            "child_name",
            "Child",
            "Name of the child",
            VariableType::String,
        );

        node.add_output_pin("path", "Path", "Child Path", VariableType::Struct)
            .set_schema::<FlowPath>();

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let parent_path: FlowPath = context.evaluate_pin("parent_path").await?;
        let child_name: String = context.evaluate_pin("child_name").await?;
        let child_segments = child_name
            .split('/')
            .filter(|segment| !segment.is_empty())
            .collect::<Vec<_>>();

        let mut path = parent_path.to_runtime(context).await?;
        for child_segment in child_segments {
            path.path = path.path.child(child_segment);
        }
        let path = path.serialize().await;

        context.set_pin_value("path", json!(path)).await?;
        let _ = context.activate_exec_pin("exec_out").await;
        Ok(())
    }
}
