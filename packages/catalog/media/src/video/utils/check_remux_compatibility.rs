use super::*;

#[crate::register_node]
#[derive(Default)]
pub struct CheckRemuxCompatibilityNode;

impl CheckRemuxCompatibilityNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for CheckRemuxCompatibilityNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "video_check_remux_compatibility",
            "Check Remux Compatibility",
            "Check whether source streams can be packet-copied into a target container",
            "Video/Planning",
        );
        add_video_icon_and_scores(&mut node);
        add_exec_pins(&mut node);
        add_flow_path_input(&mut node, "source", "Source", "Source media FlowPath");
        add_flow_path_input(
            &mut node,
            "target",
            "Target",
            "Target FlowPath with desired extension",
        );
        node.add_output_pin(
            "report",
            "Report",
            "Detailed remux compatibility report",
            VariableType::Struct,
        )
        .set_schema::<RemuxCompatibilityReport>();
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        let source: FlowPath = context.evaluate_pin("source").await?;
        let target: FlowPath = context.evaluate_pin("target").await?;
        let (source_store, source_location) = flow_path_object(context, &source).await?;
        let (_, target_location) = flow_path_object(context, &target).await?;

        let report = match video_utils_rs::plan_object_remux_from_probe(
            source_store.as_ref(),
            &source_location,
            &target_location,
        )
        .await
        {
            Ok(plan) => remux_plan_report(&plan),
            Err(err) => RemuxCompatibilityReport {
                compatible: false,
                packet_copy_only: false,
                requires_transcode: false,
                has_unsupported_streams: true,
                source_format: None,
                target_format: None,
                streams: Vec::new(),
                reason: Some(err.to_string()),
            },
        };

        context.set_pin_value("report", json!(report)).await?;
        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(execute_feature_error())
    }
}
