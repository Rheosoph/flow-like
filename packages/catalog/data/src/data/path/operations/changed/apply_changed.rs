use super::DirDiffSession;
use crate::data::path::FlowPath;
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic, NodeScores},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json};

#[crate::register_node]
#[derive(Default)]
pub struct WriteManifestNode {}

impl WriteManifestNode {
    pub fn new() -> Self {
        WriteManifestNode {}
    }
}

#[async_trait]
impl NodeLogic for WriteManifestNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "path_write_manifest",
            "Write Directory Manifest",
            "Persists the manifest carried by a diff session, so the next diff sees the current state",
            "Data/Files/Operations",
        );
        node.add_icon("/flow/icons/path.svg");

        node.add_input_pin(
            "exec_in",
            "Input",
            "Initiate Execution",
            VariableType::Execution,
        );

        node.add_input_pin(
            "session",
            "Session",
            "Diff session produced by 'Diff Directory'",
            VariableType::Struct,
        )
        .set_schema::<DirDiffSession>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin(
            "exec_out",
            "Output",
            "Done with the Execution",
            VariableType::Execution,
        );

        node.add_output_pin(
            "manifest",
            "Manifest",
            "FlowPath of the written manifest file",
            VariableType::Struct,
        )
        .set_schema::<FlowPath>();

        node.set_scores(
            NodeScores::new()
                .set_privacy(8)
                .set_security(8)
                .set_performance(9)
                .set_governance(7)
                .set_reliability(8)
                .set_cost(9)
                .build(),
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let session: DirDiffSession = context.evaluate_pin("session").await?;
        let bytes = flow_like_types::json::to_vec_pretty(&session.manifest_content)?;

        session.manifest.put(context, bytes, false).await?;

        context
            .set_pin_value("manifest", json!(session.manifest))
            .await?;
        context.activate_exec_pin("exec_out").await?;

        Ok(())
    }
}
