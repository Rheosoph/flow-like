use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json};

#[crate::register_node]
#[derive(Default)]
pub struct TryCatchNode {}

impl TryCatchNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for TryCatchNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "rpa_try_catch",
            "Try Catch",
            "Catches errors from automation actions. WARNING: This node reads error_occurred as a plain boolean input -- it does not actually intercept panics or Result::Err from downstream nodes. True try/catch semantics require executor-level support.",
            "Automation/RPA",
        );
        node.add_icon("/flow/icons/rpa.svg");

        node.set_scores(
            flow_like::flow::node::NodeScores::new()
                .set_privacy(6)
                .set_security(6)
                .set_performance(8)
                .set_governance(5)
                .set_reliability(8)
                .set_cost(9)
                .build(),
        );
        node.set_only_offline(true);

        node.add_input_pin("exec_in", "▶", "Trigger", VariableType::Execution);

        node.add_input_pin(
            "error_occurred",
            "Error Occurred",
            "Whether an error occurred (wire from action)",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(false)));

        node.add_input_pin(
            "error_message",
            "Error Message",
            "Error message if any (wire from action)",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_output_pin(
            "exec_try",
            "Try",
            "Execute the action",
            VariableType::Execution,
        );
        node.add_output_pin(
            "exec_success",
            "Success",
            "Action succeeded",
            VariableType::Execution,
        );
        node.add_output_pin(
            "exec_catch",
            "Catch",
            "Error occurred",
            VariableType::Execution,
        );

        node.add_output_pin("message", "Message", "Error message", VariableType::String);

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_try").await?;
        context.deactivate_exec_pin("exec_success").await?;
        context.deactivate_exec_pin("exec_catch").await?;

        context.activate_exec_pin("exec_try").await?;

        let error_occurred: bool = context.evaluate_pin("error_occurred").await?;
        let error_message: String = context.evaluate_pin("error_message").await?;

        context
            .set_pin_value("message", json!(error_message))
            .await?;

        if error_occurred {
            context.activate_exec_pin("exec_catch").await?;
        } else {
            context.activate_exec_pin("exec_success").await?;
        }

        Ok(())
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct ErrorRecoveryNode {}

impl ErrorRecoveryNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for ErrorRecoveryNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "rpa_error_recovery",
            "Error Recovery",
            "Defines recovery actions for specific error types",
            "Automation/RPA",
        );
        node.add_icon("/flow/icons/rpa.svg");

        node.set_scores(
            flow_like::flow::node::NodeScores::new()
                .set_privacy(6)
                .set_security(6)
                .set_performance(7)
                .set_governance(5)
                .set_reliability(7)
                .set_cost(8)
                .build(),
        );
        node.set_only_offline(true);

        node.add_input_pin("exec_in", "▶", "Trigger", VariableType::Execution);

        node.add_input_pin(
            "error_type",
            "Error Type",
            "Type of error to handle",
            VariableType::String,
        )
        .set_options(
            PinOptions::new()
                .set_valid_values(vec![
                    "ElementNotFound".to_string(),
                    "Timeout".to_string(),
                    "TemplateNotMatched".to_string(),
                    "ClickFailed".to_string(),
                    "Other".to_string(),
                ])
                .build(),
        )
        .set_default_value(Some(json!("Other")));

        node.add_input_pin(
            "actual_error",
            "Actual Error",
            "The actual error message to check",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_output_pin(
            "exec_matched",
            "Matched",
            "Error type matched",
            VariableType::Execution,
        );
        node.add_output_pin(
            "exec_unmatched",
            "Unmatched",
            "Error type did not match",
            VariableType::Execution,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_matched").await?;
        context.deactivate_exec_pin("exec_unmatched").await?;

        let error_type: String = context.evaluate_pin("error_type").await?;
        let actual_error: String = context.evaluate_pin("actual_error").await?;

        let actual_lower = actual_error.to_lowercase();
        let matched = match error_type.as_str() {
            "ElementNotFound" => {
                actual_lower.contains("not found") || actual_lower.contains("element")
            }
            "Timeout" => actual_lower.contains("timeout") || actual_lower.contains("timed out"),
            "TemplateNotMatched" => {
                actual_lower.contains("template") || actual_lower.contains("match")
            }
            "ClickFailed" => actual_lower.contains("click") || actual_lower.contains("mouse"),
            _ => true,
        };

        if matched {
            context.activate_exec_pin("exec_matched").await?;
        } else {
            context.activate_exec_pin("exec_unmatched").await?;
        }

        Ok(())
    }
}
