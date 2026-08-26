use flow_like::flow::{
    execution::{LogLevel, context::ExecutionContext},
    node::{Node, NodeLogic},
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json};

#[crate::register_node]
#[derive(Default)]
pub struct ProgressNode {}

impl ProgressNode {
    pub fn new() -> Self {
        ProgressNode {}
    }
}

#[async_trait]
impl NodeLogic for ProgressNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "log_progress",
            "Show Progress",
            "Shows a progress toast to the user that can be updated",
            "Logging",
        );
        node.set_flowscript_name("log", "progress");
        node.add_icon("/flow/icons/log-progress.svg");

        node.add_input_pin("exec_in", "Input", "Trigger Pin", VariableType::Execution);

        node.add_input_pin(
            "id",
            "Progress ID",
            "Unique identifier for this progress. Use the same ID to update the progress.",
            VariableType::String,
        )
        .set_default_value(Some(json!("progress-1")));

        node.add_input_pin(
            "message",
            "Message",
            "The message shown to the user",
            VariableType::String,
        )
        .set_default_value(Some(json!("Processing...")));

        node.add_input_pin(
            "progress",
            "Progress %",
            "Progress value between 0 and 100. Leave empty to show indeterminate progress.",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(0)));

        node.add_output_pin(
            "exec_out",
            "Output",
            "Continue execution",
            VariableType::Execution,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let output = context.get_pin_by_name("exec_out").await?;
        context.deactivate_exec_pin_ref(&output).await?;

        let id = context.evaluate_pin::<String>("id").await?;
        let message = context.evaluate_pin::<String>("message").await?;
        let progress = context.evaluate_pin::<i64>("progress").await?;

        let progress_value = if progress < 0 {
            None
        } else {
            Some(progress.clamp(0, 100) as u8)
        };

        context
            .progress_message(&id, &message, progress_value)
            .await?;

        context.log_message(
            &format!("Progress [{}]: {} ({}%)", id, message, progress),
            LogLevel::Debug,
        );

        context.activate_exec_pin_ref(&output).await?;
        Ok(())
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct ProgressDoneNode {}

impl ProgressDoneNode {
    pub fn new() -> Self {
        ProgressDoneNode {}
    }
}

#[async_trait]
impl NodeLogic for ProgressDoneNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "log_progress_done",
            "Progress Done",
            "Completes a progress toast with a success or error state",
            "Logging",
        );
        node.set_flowscript_name("log", "progressDone");
        node.add_icon("/flow/icons/log-progress-done.svg");

        node.add_input_pin("exec_in", "Input", "Trigger Pin", VariableType::Execution);

        node.add_input_pin(
            "id",
            "Progress ID",
            "The ID of the progress toast to complete (must match the ID used in Show Progress)",
            VariableType::String,
        )
        .set_default_value(Some(json!("progress-1")));

        node.add_input_pin(
            "message",
            "Message",
            "Final message to show (e.g., 'Completed!' or 'Failed')",
            VariableType::String,
        )
        .set_default_value(Some(json!("Done!")));

        node.add_input_pin(
            "success",
            "Success?",
            "Whether the operation was successful (true shows success toast, false shows error)",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(true)));

        node.add_output_pin(
            "exec_out",
            "Output",
            "Continue execution",
            VariableType::Execution,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let output = context.get_pin_by_name("exec_out").await?;
        context.deactivate_exec_pin_ref(&output).await?;

        let id = context.evaluate_pin::<String>("id").await?;
        let message = context.evaluate_pin::<String>("message").await?;
        let success = context.evaluate_pin::<bool>("success").await?;

        context.progress_done(&id, &message, success).await?;

        context.log_message(
            &format!(
                "Progress [{}] completed: {} (success: {})",
                id, message, success
            ),
            LogLevel::Debug,
        );

        context.activate_exec_pin_ref(&output).await?;
        Ok(())
    }
}
