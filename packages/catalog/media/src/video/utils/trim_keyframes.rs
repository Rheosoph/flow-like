use super::*;

#[crate::register_node]
#[derive(Default)]
pub struct TrimOnKeyframesNode;

impl TrimOnKeyframesNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for TrimOnKeyframesNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "video_trim_keyframes",
            "Trim On Keyframes",
            "Trim a media file using a keyframe-aligned packet range",
            "Video/Editing",
        );
        add_video_icon_and_scores(&mut node);
        add_exec_pins(&mut node);
        add_flow_path_input(&mut node, "source", "Source", "Source media FlowPath");
        add_flow_path_input(&mut node, "target", "Target", "Target media FlowPath");
        node.add_input_pin(
            "start_seconds",
            "Start Seconds",
            "Requested start time",
            VariableType::Float,
        )
        .set_default_value(Some(json!(0.0)));
        node.add_input_pin(
            "end_seconds",
            "End Seconds",
            "Requested end time",
            VariableType::Float,
        )
        .set_default_value(Some(json!(10.0)));
        node.add_input_pin(
            "track_id",
            "Track ID",
            "Boundary video track; 0 uses first video track",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(0)));
        add_flow_path_output(&mut node, "result", "Result", "Written media FlowPath");
        node.add_output_pin(
            "packet_count",
            "Packet Count",
            "Packets written",
            VariableType::Integer,
        );
        node.add_output_pin(
            "boundary_track_id",
            "Boundary Track",
            "Track used for keyframe selection",
            VariableType::Integer,
        );
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        let source: FlowPath = context.evaluate_pin("source").await?;
        let target: FlowPath = context.evaluate_pin("target").await?;
        let start_seconds: f64 = context.evaluate_pin("start_seconds").await?;
        let end_seconds: f64 = context.evaluate_pin("end_seconds").await?;
        let track_id: i64 = context.evaluate_pin("track_id").await?;
        let (source_store, source_location) = flow_path_object(context, &source).await?;
        let (target_store, target_location) = flow_path_object(context, &target).await?;

        let demuxed = video_utils_rs::demux_object(source_store.as_ref(), &source_location).await?;
        let boundary_track_id =
            select_video_track_id(&demuxed.media, optional_track_id(track_id)?)?;
        let packet_slice = video_utils_rs::select_keyframe_range(
            &demuxed.packets,
            boundary_track_id,
            start_seconds,
            end_seconds,
        )?;
        let mut packets = demuxed.packets[packet_slice.start..packet_slice.end].to_vec();
        video_utils_rs::normalize_timestamps(&mut packets)?;
        let media = media_for_packet_subset(&demuxed.media, &packets);
        let report =
            video_utils_rs::mux_object(target_store.as_ref(), &target_location, &media, &packets)
                .await?;

        context.set_pin_value("result", json!(target)).await?;
        context
            .set_pin_value("packet_count", json!(report.packet_count as i64))
            .await?;
        context
            .set_pin_value("boundary_track_id", json!(boundary_track_id as i64))
            .await?;
        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(execute_feature_error())
    }
}
