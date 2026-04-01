use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json};

#[crate::register_node]
#[derive(Default)]
pub struct LogActionNode {}

impl LogActionNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for LogActionNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "rpa_log_action",
            "Log Action",
            "Logs an automation action for debugging and auditing",
            "Automation/RPA",
        );
        node.add_icon("/flow/icons/rpa.svg");

        node.set_scores(
            flow_like::flow::node::NodeScores::new()
                .set_privacy(5)
                .set_security(6)
                .set_performance(9)
                .set_governance(6)
                .set_reliability(9)
                .set_cost(9)
                .build(),
        );
        node.set_only_offline(true);

        node.add_input_pin("exec_in", "▶", "Trigger", VariableType::Execution);

        node.add_input_pin(
            "action",
            "Action",
            "Action being performed",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_input_pin(
            "details",
            "Details",
            "Additional details about the action",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_input_pin("level", "Level", "Log level", VariableType::String)
            .set_options(
                PinOptions::new()
                    .set_valid_values(vec![
                        "Debug".to_string(),
                        "Info".to_string(),
                        "Warning".to_string(),
                        "Error".to_string(),
                    ])
                    .build(),
            )
            .set_default_value(Some(json!("Info")));

        node.add_output_pin("exec_out", "▶", "Continue", VariableType::Execution);

        node.add_output_pin(
            "log_entry",
            "Log Entry",
            "Formatted log entry",
            VariableType::String,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        use chrono::Utc;

        context.deactivate_exec_pin("exec_out").await?;

        let action: String = context.evaluate_pin("action").await?;
        let details: String = context.evaluate_pin("details").await?;
        let level: String = context.evaluate_pin("level").await?;

        let log_entry = format!(
            "[{}] [{}] {} - {}",
            Utc::now().to_rfc3339(),
            level,
            action,
            details
        );

        match level.as_str() {
            "Debug" => tracing::debug!("{}", log_entry),
            "Warning" => tracing::warn!("{}", log_entry),
            "Error" => tracing::error!("{}", log_entry),
            _ => tracing::info!("{}", log_entry),
        }

        context.set_pin_value("log_entry", json!(log_entry)).await?;
        context.activate_exec_pin("exec_out").await?;

        Ok(())
    }
}
