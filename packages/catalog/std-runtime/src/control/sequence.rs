use ahash::AHashSet;
use flow_like::flow::{
    execution::{context::ExecutionContext, internal_node::InternalNode},
    node::{Node, NodeLogic},
    variable::VariableType,
};
use flow_like_types::async_trait;
use std::collections::HashSet;

#[crate::register_node]
#[derive(Default)]
pub struct SequenceNode {}

impl SequenceNode {
    pub fn new() -> Self {
        SequenceNode {}
    }
}

#[async_trait]
impl NodeLogic for SequenceNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "control_sequence",
            "Sequence",
            "Sequential Execution",
            "Control",
        );
        node.set_flowscript_name("control", "sequence");
        node.add_icon("/flow/icons/sequence.svg");

        node.add_input_pin("exec_in", "Input", "Trigger Pin", VariableType::Execution);

        node.add_output_pin("exec_out", "Output", "Output Pin", VariableType::Execution);

        node.add_output_pin("exec_out", "Output", "Output Pin", VariableType::Execution);

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let mut already_executed: std::collections::HashSet<usize> = HashSet::new();

        let mut pin_indices = {
            let exec_out_pins = context.get_pins_by_name("exec_out").await?;
            let mut pin_indices = Vec::with_capacity(exec_out_pins.len());
            for pin in exec_out_pins {
                let index = pin.index;
                pin_indices.push((pin.clone(), index));
            }
            pin_indices
        };

        pin_indices.sort_by_key(|(_, index)| *index);

        let execution_order = {
            let mut order = Vec::with_capacity(pin_indices.len());

            for (pin, _) in pin_indices {
                let _ = context.activate_exec_pin_ref(&pin).await;

                let connected_nodes = pin.get_connected_nodes();

                for node in connected_nodes {
                    let key = std::sync::Arc::as_ptr(&node) as usize;
                    if already_executed.insert(key) {
                        order.push(node);
                    }
                }
            }
            order
        };

        let mut recursion_guard = AHashSet::new();
        recursion_guard.insert(context.node.node.lock().await.id.clone());

        for node in execution_order {
            let mut sub_context = context.create_sub_context(&node).await;
            let _ =
                InternalNode::trigger(&mut sub_context, &mut Some(recursion_guard.clone()), true)
                    .await;
            sub_context.end_trace();
            context.push_sub_context(&mut sub_context);
        }

        let exec_out_pins = context.get_pins_by_name("exec_out").await?;
        for pin in exec_out_pins {
            let _ = context.deactivate_exec_pin_ref(&pin).await;
        }

        Ok(())
    }
}
