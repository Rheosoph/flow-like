use flow_like::flow::{
    execution::{LogLevel, context::ExecutionContext},
    node::{Node, NodeLogic},
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json};

#[crate::register_node]
#[derive(Default)]
pub struct DoNNode {}

impl DoNNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for DoNNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "control_do_n",
            "Do N",
            "Pass execution the first N triggers, then block; fire 'Completed' on Nth.",
            "Control/Flow",
        );
        node.set_flowscript_name("control", "doN");
        node.add_icon("/flow/icons/workflow.svg");

        // Execution
        node.add_input_pin("exec_in", "Input", "Trigger Pin", VariableType::Execution);

        // Parameters
        node.add_input_pin(
            "n",
            "N",
            "Number of times to allow execution to pass (>= 0)",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(1)));

        node.add_input_pin(
            "start_index",
            "Start Index",
            "Initial index before first pass (commonly 0)",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(0)));

        // Reset is a boolean input (reads each trigger). If true, we'll reset the counter
        // before processing the current trigger. (If you prefer a dedicated Execution
        // reset input like in UE, we can add that variant as well.)
        node.add_input_pin(
            "reset",
            "Reset",
            "If true on this trigger, reset the counter to Start Index before processing",
            VariableType::Execution,
        );
        // Outputs
        node.add_output_pin(
            "then",
            "Then",
            "Fires while index < N",
            VariableType::Execution,
        );

        // We expose the counter for debugging/branching; we also use it to persist state
        // across triggers by writing the value each run.
        node.add_output_pin(
            "index",
            "Index",
            "Current counter after this trigger",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(0)));

        // Optional convenience output: remaining = max(N - index, 0)
        node.add_output_pin(
            "remaining",
            "Remaining",
            "How many passes are left until Completed fires",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(0)));

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        // Always deactivate outgoing exec pins first (best practice)
        context.deactivate_exec_pin("then").await?;

        // Read parameters
        let mut n: i64 = context.evaluate_pin("n").await.unwrap_or(1);
        let start_index: i64 = context.evaluate_pin("start_index").await.unwrap_or(0);
        let do_reset: bool = context.evaluate_pin("reset").await.unwrap_or(false);

        // Read current state from our own output pin (persisted across triggers)
        let mut index: i64 = context.evaluate_pin("index").await.unwrap_or(start_index);

        // Normalize inputs
        if n < 0 {
            n = 0;
        }
        if index < start_index {
            index = start_index;
        }

        // Optional reset
        if do_reset {
            index = start_index;
            context.log_message("DoN: counter reset", LogLevel::Debug);
            context.deactivate_exec_pin("reset").await?;
        }

        // Compute behavior
        let mut fired_then = false;
        let mut fired_completed = false;

        if index < n {
            // We are still within the allowed passes
            context.activate_exec_pin("then").await?;
            fired_then = true;

            // Increment after firing 'then'
            index += 1;

            if index >= n {
                // Crossing/exactly reaching N -> fire 'completed' once
                fired_completed = true;
            }
        } else {
            context.log_message("DoN: limit reached, blocking execution", LogLevel::Debug);
        }

        let remaining = if n > index { n - index } else { 0 };
        context.set_pin_value("index", json!(index)).await?;
        context.set_pin_value("remaining", json!(remaining)).await?;

        context.log_message(
            &format!(
                "DoN state -> index: {}, n: {}, then: {}, completed: {}",
                index, n, fired_then, fired_completed
            ),
            LogLevel::Debug,
        );

        Ok(())
    }
}
