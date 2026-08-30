use flow_like::flow::{
    execution::{LogLevel, context::ExecutionContext},
    node::{Node, NodeLogic},
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json};

/// Do Once
///
/// and then blocks until it receives a **Reset** trigger. Supports an optional
/// `Start Closed` flag that begins in the blocked state.

#[crate::register_node]
#[derive(Default)]
pub struct DoOnceNode {}

impl DoOnceNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for DoOnceNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "control_do_once",
            "Do Once",
            "Let execution pass once, then block until Reset.",
            "Control/Flow",
        );
        node.set_flowscript_name("control", "doOnce");
        node.add_icon("/flow/icons/workflow.svg");

        // Execution inputs
        node.add_input_pin("exec_in", "Input", "Trigger Pin", VariableType::Execution);
        node.add_input_pin(
            "reset",
            "Reset",
            "Trigger to reopen this node (does not forward execution)",
            VariableType::Execution,
        );

        // Parameters
        node.add_input_pin(
            "start_closed",
            "Start Closed",
            "If true, starts blocked until a Reset arrives",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(false)));

        // Outputs
        node.add_output_pin(
            "then",
            "Then",
            "Fires only the first allowed pass",
            VariableType::Execution,
        );

        node.add_output_pin(
            "has_fired",
            "Has Fired",
            "Whether this node has already allowed a pass (blocked if true)",
            VariableType::Boolean,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        // Best practice: deactivate outgoing exec pins first
        context.deactivate_exec_pin("then").await?;

        // Read inputs
        let start_closed: bool = context.evaluate_pin("start_closed").await.unwrap_or(false);
        let did_reset: bool = context.evaluate_pin("reset").await.unwrap_or(false);
        let did_exec: bool = context.evaluate_pin("exec_in").await.unwrap_or(false);

        // Load persistent state (defaults to `start_closed`):
        // - `has_fired == false`  => open (has not passed yet)
        // - `has_fired == true`   => closed (already passed once)
        let mut has_fired: bool = context
            .evaluate_pin("has_fired")
            .await
            .unwrap_or(start_closed);

        if did_reset {
            has_fired = false;
            context.log_message("DoOnce: reset -> reopened", LogLevel::Debug);
            context.deactivate_exec_pin("reset").await?;
        }

        if did_exec {
            if !has_fired {
                context.activate_exec_pin("then").await?;
                has_fired = true;
            } else {
                context.log_message("DoOnce: blocked (already fired)", LogLevel::Debug);
            }
        }

        context.set_pin_value("has_fired", json!(has_fired)).await?;

        context.log_message(
            &format!("DoOnce state -> has_fired: {}", has_fired),
            LogLevel::Debug,
        );

        Ok(())
    }
}
