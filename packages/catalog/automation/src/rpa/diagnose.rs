use crate::types::handles::AutomationSession;
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json};

#[crate::register_node]
#[derive(Default)]
pub struct DiagnoseFailureNode {}

impl DiagnoseFailureNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for DiagnoseFailureNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "rpa_diagnose_failure",
            "Diagnose Failure",
            "Captures diagnostic info when an automation fails",
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
            "RPA session handle",
            VariableType::Struct,
        )
        .set_schema::<AutomationSession>();

        node.add_input_pin(
            "error_message",
            "Error Message",
            "The error that occurred",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_input_pin(
            "screenshot_path",
            "Screenshot Path",
            "Path to save diagnostic screenshot",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_output_pin("exec_out", "▶", "Continue", VariableType::Execution);

        node.add_output_pin(
            "diagnostic_info",
            "Diagnostic Info",
            "JSON string with diagnostic data",
            VariableType::String,
        );

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        use chrono::Utc;

        context.deactivate_exec_pin("exec_out").await?;

        let session: AutomationSession = context.evaluate_pin("session").await?;
        let error_message: String = context.evaluate_pin("error_message").await?;
        let screenshot_path: String = context.evaluate_pin("screenshot_path").await?;

        let autogui = session.get_autogui(context).await?;
        let mut gui = autogui.lock().await;

        let (screen_width, screen_height) = gui.get_screen_size();

        if !screenshot_path.is_empty() {
            let _ = gui.save_screenshot(&screenshot_path);
        }

        let diagnostic = flow_like_types::json::json!({
            "timestamp": Utc::now().to_rfc3339(),
            "error_message": error_message,
            "screen_size": {
                "width": screen_width,
                "height": screen_height
            },
            "screenshot_path": screenshot_path,
            "debug_mode": session.debug_mode
        });

        context
            .set_pin_value("diagnostic_info", json!(diagnostic.to_string()))
            .await?;
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
