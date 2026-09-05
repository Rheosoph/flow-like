use flow_like::flow::{
    execution::{LogLevel, context::ExecutionContext},
    node::{Node, NodeLogic},
    variable::VariableType,
};
use flow_like_types::{Value, async_trait};

/// Gate
///
/// Execution entering `Enter` will pass to `Exit` **only**
/// when the gate is open. Use the `Open`, `Close`, and `Toggle` exec inputs to control
/// the gate dynamically. Supports a `Start Closed` parameter.
///
/// Behavior:
/// - `Open` / `Close` / `Toggle` change the internal open-state and do **not** forward execution.
/// - `Enter` forwards to `Exit` if the gate is open; otherwise it is blocked.
/// - The open-state persists by writing to the `is_open` output pin.
#[crate::register_node]
#[derive(Default)]
pub struct GateNode {}

impl GateNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for GateNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "control_gate",
            "Gate",
            "Open/close a gate to conditionally pass execution.",
            "Control/Flow",
        );
        node.set_flowscript_name("control", "gate");
        node.add_icon("/flow/icons/gate.svg");

        // Execution inputs
        node.add_input_pin(
            "enter",
            "Enter",
            "Main execution input",
            VariableType::Execution,
        );
        node.add_input_pin("open", "Open", "Open the gate", VariableType::Execution);
        node.add_input_pin("close", "Close", "Close the gate", VariableType::Execution);
        node.add_input_pin(
            "toggle",
            "Toggle",
            "Toggle the gate",
            VariableType::Execution,
        );

        // Parameters
        node.add_input_pin(
            "start_closed",
            "Start Closed",
            "If true, the gate starts closed (blocked)",
            VariableType::Boolean,
        )
        .set_default_value(Some(Value::from(false)));

        // Execution outputs
        node.add_output_pin(
            "exit",
            "Exit",
            "Fires when Enter passes through",
            VariableType::Execution,
        );

        // State output (persisted across triggers via writes in `run`)
        node.add_output_pin(
            "is_open",
            "Is Open",
            "Current open/closed state after this tick",
            VariableType::Boolean,
        )
        .set_default_value(Some(Value::from(false)));

        // Optional debug counters (commented out; enable if desired)
        // node.add_output_pin("pass_count", "Pass Count", "Times execution passed", VariableType::Integer)
        //     .set_default_value(Some(Value::from(0)));
        // node.add_output_pin("block_count", "Block Count", "Times execution was blocked", VariableType::Integer)
        //     .set_default_value(Some(Value::from(0)));

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        // Deactivate outgoing exec first
        context.deactivate_exec_pin("exit").await?;

        // Read triggers
        let did_enter: bool = context.evaluate_pin("enter").await.unwrap_or(false);
        let did_open: bool = context.evaluate_pin("open").await.unwrap_or(false);
        let did_close: bool = context.evaluate_pin("close").await.unwrap_or(false);
        let did_toggle: bool = context.evaluate_pin("toggle").await.unwrap_or(false);

        // Determine default/open state
        let start_closed: bool = context.evaluate_pin("start_closed").await.unwrap_or(false);
        let default_open = !start_closed;

        // Load persisted state (falls back to parameter on first run)
        let mut is_open: bool = context
            .evaluate_pin("is_open")
            .await
            .unwrap_or(default_open);

        // Apply control inputs first (these do not forward execution)
        if did_open {
            is_open = true;
            context.log_message("Gate: opened", LogLevel::Debug);
        }
        if did_close {
            is_open = false;
            context.log_message("Gate: closed", LogLevel::Debug);
        }
        if did_toggle {
            is_open = !is_open;
            context.log_message("Gate: toggled", LogLevel::Debug);
        }

        // Now handle Enter
        if did_enter {
            if is_open {
                context.activate_exec_pin("exit").await?;
            } else {
                context.log_message("Gate: blocked (closed)", LogLevel::Debug);
            }
        }

        // Persist state
        context
            .set_pin_value("is_open", Value::from(is_open))
            .await?;

        // // Optional counters example
        // let mut pass_count: i64 = context.evaluate_pin("pass_count").await.unwrap_or(0);
        // let mut block_count: i64 = context.evaluate_pin("block_count").await.unwrap_or(0);
        // if did_enter { if is_open { pass_count += 1; } else { block_count += 1; } }
        // context.set_pin_value("pass_count", Value::from(pass_count)).await?;
        // context.set_pin_value("block_count", Value::from(block_count)).await?;

        Ok(())
    }
}
