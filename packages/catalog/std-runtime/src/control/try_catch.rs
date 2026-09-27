use flow_like::flow::{
    execution::{
        branch::{BranchResult, run_branch},
        context::ExecutionContext,
    },
    node::{Node, NodeLogic},
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
        // Keep the node ID and FlowScript name so existing flows and scripts still resolve.
        let mut node = Node::new(
            "rpa_try_catch",
            "Try Catch",
            "Executes an action branch and routes errors to Catch.",
            "Control",
        );
        node.set_version(1);
        node.set_flowscript_name("rpa", "tryCatch");
        node.add_icon("/flow/icons/split.svg");

        node.set_scores(
            flow_like::flow::node::NodeScores::new()
                .set_privacy(10)
                .set_security(10)
                .set_performance(8)
                .set_governance(10)
                .set_reliability(10)
                .set_cost(10)
                .build(),
        );

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

        let outcome = run_branch(context, "exec_try", None).await?;

        let mut error_occurred: bool = context.evaluate_pin("error_occurred").await?;
        let mut error_message: String = context.evaluate_pin("error_message").await?;
        if let BranchResult::Failed(error) = outcome {
            error_occurred = true;
            error_message = error;
        }

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
