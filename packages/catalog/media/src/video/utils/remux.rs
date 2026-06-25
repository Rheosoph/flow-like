use super::*;

#[crate::register_node]
#[derive(Default)]
pub struct RemuxVideoNode;

impl RemuxVideoNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for RemuxVideoNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "video_remux",
            "Remux Video",
            "Rewrap compatible streams into another container without decoding",
            "Video/Containers",
        );
        add_video_icon_and_scores(&mut node);
        add_exec_pins(&mut node);
        add_flow_path_input(&mut node, "source", "Source", "Source media FlowPath");
        add_flow_path_input(&mut node, "target", "Target", "Target media FlowPath");
        add_flow_path_output(&mut node, "result", "Result", "Written media FlowPath");
        node.add_output_pin(
            "report",
            "Report",
            "Remux operation report",
            VariableType::Struct,
        )
        .set_schema::<VideoRemuxReport>();
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        let source: FlowPath = context.evaluate_pin("source").await?;
        let target: FlowPath = context.evaluate_pin("target").await?;
        let (source_store, source_location) = flow_path_object(context, &source).await?;
        let (target_store, target_location) = flow_path_object(context, &target).await?;
        let remux_report = video_utils_rs::remux_object_between_stores(
            source_store.as_ref(),
            &source_location,
            target_store.as_ref(),
            &target_location,
            None,
        )
        .await?;
        let report = VideoRemuxReport {
            source: source_location.to_string(),
            target: target_location.to_string(),
            operation: format!("{:?}", remux_report.operation),
            bytes_written: remux_report.bytes_written,
        };

        context.set_pin_value("result", json!(target)).await?;
        context.set_pin_value("report", json!(report)).await?;
        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(execute_feature_error())
    }
}
