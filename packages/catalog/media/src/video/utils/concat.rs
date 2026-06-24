use super::*;

#[crate::register_node]
#[derive(Default)]
pub struct ConcatMediaNode;

impl ConcatMediaNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for ConcatMediaNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "video_concat",
            "Concatenate Videos",
            "Concatenate packet-copy-compatible media files",
            "Video/Editing",
        );
        add_video_icon_and_scores(&mut node);
        add_exec_pins(&mut node);
        node.add_input_pin(
            "sources",
            "Sources",
            "Media FlowPaths in concatenation order",
            VariableType::Struct,
        )
        .set_schema::<FlowPath>()
        .set_value_type(ValueType::Array);
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
        let sources: Vec<FlowPath> = context.evaluate_pin("sources").await?;
        let target: FlowPath = context.evaluate_pin("target").await?;
        if sources.is_empty() {
            return Err(flow_like_types::anyhow!(
                "At least one source media file is required"
            ));
        }

        let mut demuxed_media = Vec::with_capacity(sources.len());
        for source in &sources {
            let (store, location) = flow_path_object(context, source).await?;
            demuxed_media.push(video_utils_rs::demux_object(store.as_ref(), &location).await?);
        }
        let packet_groups = demuxed_media
            .iter()
            .map(|demuxed| demuxed.packets.as_slice())
            .collect::<Vec<_>>();
        let packets = video_utils_rs::concat_copy(&packet_groups)?;
        let media = demuxed_media
            .first()
            .map(|demuxed| demuxed.media.clone())
            .ok_or_else(|| flow_like_types::anyhow!("No media was demuxed"))?;
        let (target_store, target_location) = flow_path_object(context, &target).await?;
        let report =
            video_utils_rs::mux_object(target_store.as_ref(), &target_location, &media, &packets)
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
