use super::*;

#[crate::register_node]
#[derive(Default)]
pub struct ExtractTrackNode;

impl ExtractTrackNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for ExtractTrackNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "video_extract_track",
            "Extract Track",
            "Write one encoded media track into a new container",
            "Video/Tracks",
        );
        add_video_icon_and_scores(&mut node);
        add_exec_pins(&mut node);
        add_flow_path_input(&mut node, "source", "Source", "Source media FlowPath");
        add_flow_path_input(&mut node, "target", "Target", "Target media FlowPath");
        node.add_input_pin(
            "track_id",
            "Track ID",
            "Track to keep",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(1)));
        add_flow_path_output(&mut node, "result", "Result", "Written media FlowPath");
        node.add_output_pin(
            "packet_count",
            "Packet Count",
            "Packets written",
            VariableType::Integer,
        );
        node.add_output_pin(
            "stream",
            "Stream",
            "Extracted stream metadata",
            VariableType::Struct,
        )
        .set_schema::<VideoStreamInfo>();
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        let source: FlowPath = context.evaluate_pin("source").await?;
        let target: FlowPath = context.evaluate_pin("target").await?;
        let track_id: i64 = context.evaluate_pin("track_id").await?;
        let track_id = u32::try_from(track_id)?;
        let (source_store, source_location) = flow_path_object(context, &source).await?;
        let (target_store, target_location) = flow_path_object(context, &target).await?;

        let demuxed = video_utils_rs::demux_object(source_store.as_ref(), &source_location).await?;
        let stream = demuxed
            .media
            .stream(track_id)
            .ok_or_else(|| flow_like_types::anyhow!("Track {} is missing", track_id))?;
        let packets = video_utils_rs::filter_track(&demuxed.packets, track_id);
        if packets.is_empty() {
            return Err(flow_like_types::anyhow!(
                "Track {} has no encoded packets",
                track_id
            ));
        }
        let mut media = video_utils_rs::MediaInfo {
            duration_seconds: stream.duration_seconds(),
            tags: demuxed.media.tags.clone(),
            ..Default::default()
        };
        media.push_stream(stream.clone());
        let stream_info = stream_to_info(stream);
        let report =
            video_utils_rs::mux_object(target_store.as_ref(), &target_location, &media, &packets)
                .await?;

        context.set_pin_value("result", json!(target)).await?;
        context
            .set_pin_value("packet_count", json!(report.packet_count as i64))
            .await?;
        context.set_pin_value("stream", json!(stream_info)).await?;
        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(execute_feature_error())
    }
}
