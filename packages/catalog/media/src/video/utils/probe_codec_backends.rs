use super::*;

#[crate::register_node]
#[derive(Default)]
pub struct ProbeCodecBackendsNode;

impl ProbeCodecBackendsNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for ProbeCodecBackendsNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "video_probe_codec_backends",
            "Probe Codec Backends",
            "Report compiled video-utils-rs features and recommended codec backend lanes",
            "Diagnostics",
        );
        node.add_icon("/flow/icons/info.svg");
        add_exec_pins(&mut node);
        node.add_output_pin(
            "backends",
            "Backends",
            "Recommended codec backends",
            VariableType::Struct,
        )
        .set_schema::<CodecBackendInfo>()
        .set_value_type(ValueType::Array);
        node.add_output_pin(
            "features",
            "Features",
            "Compiled video-utils-rs feature set",
            VariableType::Struct,
        )
        .set_schema::<VideoUtilsFeatureSet>();
        node.add_output_pin(
            "backend_count",
            "Backend Count",
            "Number of backend lanes",
            VariableType::Integer,
        );
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        let backends = backend_info();
        context
            .set_pin_value("backend_count", json!(backends.len() as i64))
            .await?;
        context.set_pin_value("backends", json!(backends)).await?;
        context
            .set_pin_value("features", json!(feature_set()))
            .await?;
        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(execute_feature_error())
    }
}
