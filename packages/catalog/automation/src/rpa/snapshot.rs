use crate::types::handles::AutomationSession;
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    variable::VariableType,
};
use flow_like_catalog_core::FlowPath;
use flow_like_types::{async_trait, json::json};

#[crate::register_node]
#[derive(Default)]
pub struct TakeSnapshotNode {}

impl TakeSnapshotNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for TakeSnapshotNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "rpa_take_snapshot",
            "Take Snapshot",
            "Captures a screen snapshot and saves to file",
            "Automation/RPA",
        );
        node.add_icon("/flow/icons/rpa.svg");

        node.set_scores(
            flow_like::flow::node::NodeScores::new()
                .set_privacy(2)
                .set_security(4)
                .set_performance(6)
                .set_governance(5)
                .set_reliability(7)
                .set_cost(7)
                .build(),
        );
        node.set_only_offline(true);

        node.add_input_pin("exec_in", "▶", "Trigger", VariableType::Execution);

        node.add_input_pin(
            "session",
            "Session",
            "Automation session",
            VariableType::Struct,
        )
        .set_schema::<AutomationSession>();

        node.add_input_pin(
            "file_path",
            "File Path",
            "Path to save the snapshot image",
            VariableType::Struct,
        )
        .set_schema::<FlowPath>();

        node.add_input_pin(
            "monitor",
            "Monitor",
            "Monitor index (0 = primary)",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(0)));

        node.add_output_pin("exec_out", "▶", "Continue", VariableType::Execution);

        node.add_output_pin(
            "success",
            "Success",
            "Whether the snapshot was saved",
            VariableType::Boolean,
        );

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let session: AutomationSession = context.evaluate_pin("session").await?;
        let file_path: FlowPath = context.evaluate_pin("file_path").await?;
        let _monitor: i64 = context.evaluate_pin("monitor").await?;

        let autogui = session.get_autogui(context).await?;
        let mut gui = autogui.lock().await;

        let runtime = file_path.to_runtime(context).await?;
        let actual_path = runtime.path.to_string();
        let success = gui.save_screenshot(&actual_path).is_ok();

        context.set_pin_value("success", json!(success)).await?;
        context.activate_exec_pin("exec_out").await?;

        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "RPA automation requires the 'execute' feature"
        ))
    }
}
