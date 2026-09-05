use super::*;

#[crate::register_node]
#[derive(Default)]
pub struct NormalizePacketTimestampsNode;

impl NormalizePacketTimestampsNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for NormalizePacketTimestampsNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "video_normalize_timestamps",
            "Normalize Timestamps",
            "Rebase packet timestamps so each track starts at zero or later",
            "Video/Packets",
        );
        node.set_flowscript_name("video", "normalizeTimestamps");
        add_video_icon_and_scores(&mut node);
        add_exec_pins(&mut node);
        add_flow_path_input(&mut node, "source", "Source", "Source media FlowPath");
        add_flow_path_input(&mut node, "target", "Target", "Target media FlowPath");
        add_flow_path_output(&mut node, "result", "Result", "Written media FlowPath");
        node.add_output_pin(
            "packet_count",
            "Packet Count",
            "Packets written",
            VariableType::Integer,
        );
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        let source: FlowPath = context.evaluate_pin("source").await?;
        let target: FlowPath = context.evaluate_pin("target").await?;
        let (source_store, source_location) = flow_path_object(context, &source).await?;
        let (target_store, target_location) = flow_path_object(context, &target).await?;
        let demuxed = video_utils_rs::demux_object(source_store.as_ref(), &source_location).await?;
        let mut packets = demuxed.packets;
        video_utils_rs::normalize_timestamps(&mut packets)?;
        let report = video_utils_rs::mux_object(
            target_store.as_ref(),
            &target_location,
            &demuxed.media,
            &packets,
        )
        .await?;

        context.set_pin_value("result", json!(target)).await?;
        context
            .set_pin_value("packet_count", json!(report.packet_count as i64))
            .await?;
        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(execute_feature_error())
    }
}
