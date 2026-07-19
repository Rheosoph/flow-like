use super::*;

#[crate::register_node]
#[derive(Default)]
pub struct ProbeMediaInfoNode;

impl ProbeMediaInfoNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for ProbeMediaInfoNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "video_probe_media_info",
            "Probe Media Info",
            "Extract stream metadata from a media FlowPath",
            "Video/Inspect",
        );
        add_video_icon_and_scores(&mut node);
        add_exec_pins(&mut node);
        add_flow_path_input(&mut node, "source", "Source", "Media FlowPath to inspect");
        node.add_output_pin(
            "media",
            "Media Info",
            "Container and stream metadata",
            VariableType::Struct,
        )
        .set_schema::<VideoMediaInfo>();
        node.add_output_pin(
            "streams",
            "Streams",
            "Detected media streams",
            VariableType::Struct,
        )
        .set_schema::<VideoStreamInfo>()
        .set_value_type(ValueType::Array);
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        let source: FlowPath = context.evaluate_pin("source").await?;
        let (store, location) = flow_path_object(context, &source).await?;
        let info = match video_utils_rs::demux_object(store.as_ref(), &location).await {
            Ok(demuxed) => media_to_info_with_packets(&demuxed.media, &demuxed.packets),
            Err(_) => {
                let media =
                    video_utils_rs::probe_object_media_info(store.as_ref(), &location).await?;
                media_to_info(&media)
            }
        };
        context
            .set_pin_value("streams", json!(info.streams.clone()))
            .await?;
        context.set_pin_value("media", json!(info)).await?;
        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(execute_feature_error())
    }
}
