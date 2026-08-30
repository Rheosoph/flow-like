use flow_like::flow::{
    execution::{LogLevel, context::ExecutionContext},
    node::{Node, NodeLogic},
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json};

/// Flip Flop (no reset)
/// Behavior:
/// - First open side is controlled by `start_on_a` (default: true → starts on A).
/// - Each `exec_in` fires the current side and then toggles to the other.
/// - Exposes `is_a` (the side that will fire on the **next** trigger) and `tick`
///   (how many times it has executed so far).
#[crate::register_node]
#[derive(Default)]
pub struct FlipFlopNode {}

impl FlipFlopNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for FlipFlopNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "control_flip_flop",
            "Flip Flop",
            "Alternate execution between A and B on successive triggers.",
            "Control/Flow",
        );
        node.set_flowscript_name("control", "flipFlop");
        node.add_icon("/flow/icons/workflow.svg");

        // Execution input
        node.add_input_pin("exec_in", "Input", "Trigger Pin", VariableType::Execution);

        // Parameters
        node.add_input_pin(
            "start_on_a",
            "Start On A",
            "If true, first pass goes to A; otherwise to B",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(true)));

        // Execution outputs
        node.add_output_pin("a", "A", "Fires on A side", VariableType::Execution);
        node.add_output_pin("b", "B", "Fires on B side", VariableType::Execution);

        // State/debug outputs (persist via writing in run)
        node.add_output_pin(
            "is_a",
            "Is A",
            "Side that will fire on next trigger",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(true)));
        node.add_output_pin(
            "tick",
            "Tick",
            "How many times FlipFlop has executed",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(0)));

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        // Deactivate outgoing exec pins first
        context.deactivate_exec_pin("a").await?;
        context.deactivate_exec_pin("b").await?;

        // Read inputs
        let start_on_a: bool = context.evaluate_pin("start_on_a").await.unwrap_or(true);
        let did_exec: bool = context.evaluate_pin("exec_in").await.unwrap_or(false);

        // Load persisted state (default derives from start_on_a)
        let mut is_a: bool = context.evaluate_pin("is_a").await.unwrap_or(start_on_a);
        let mut tick: i64 = context.evaluate_pin("tick").await.unwrap_or(0);

        if did_exec {
            // Fire current side
            if is_a {
                context.activate_exec_pin("a").await?;
            } else {
                context.activate_exec_pin("b").await?;
            }

            // Update state for next time
            is_a = !is_a;
            tick = tick.saturating_add(1);
        }

        // Publish state
        context.set_pin_value("is_a", json!(is_a)).await?;
        context.set_pin_value("tick", json!(tick)).await?;

        context.log_message(
            &format!("FlipFlop state -> next_is_a: {}, tick: {}", is_a, tick),
            LogLevel::Debug,
        );

        Ok(())
    }
}
