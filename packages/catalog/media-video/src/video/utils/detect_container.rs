use super::*;

#[crate::register_node]
#[derive(Default)]
pub struct DetectVideoContainerNode;

impl DetectVideoContainerNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for DetectVideoContainerNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "video_detect_container",
            "Detect Video Container",
            "Detect the media container for a FlowPath object",
            "Video/Inspect",
        );
        node.set_flowscript_name("video", "detectContainer");
        add_video_icon_and_scores(&mut node);
        add_exec_pins(&mut node);
        add_flow_path_input(&mut node, "source", "Source", "Media FlowPath to inspect");
        node.add_output_pin(
            "container",
            "Container",
            "Detected media container",
            VariableType::Struct,
        )
        .set_schema::<ContainerDetectionInfo>();
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        let source: FlowPath = context.evaluate_pin("source").await?;
        let (store, location) = flow_path_object(context, &source).await?;
        let format =
            video_utils_rs::detect_object_container_format(store.as_ref(), &location).await?;
        let container = ContainerDetectionInfo {
            format: format.as_str().to_owned(),
            display_name: format.display_name().to_owned(),
            extensions: format
                .extensions()
                .iter()
                .map(|extension| (*extension).to_owned())
                .collect(),
        };

        context.set_pin_value("container", json!(container)).await?;
        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(execute_feature_error())
    }
}
