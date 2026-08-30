use ahash::AHashSet;
use flow_like::flow::{
    board::Board,
    execution::{LogLevel, context::ExecutionContext, internal_node::InternalNode},
    node::{Node, NodeLogic},
    pin::{PinOptions, ValueType},
    variable::VariableType,
};
use flow_like_types::{Value, async_trait};

#[crate::register_node]
#[derive(Default)]
pub struct ForEachWithBreakNode {}

impl ForEachWithBreakNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for ForEachWithBreakNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "control_for_each_with_break",
            "For Each (Break)",
            "Loops over an Array; allows breaking early from inside the loop body.",
            "Control",
        );
        node.set_flowscript_name("control", "forEachWithBreak");
        node.add_icon("/flow/icons/for-each.svg");

        // Execution inputs
        node.add_input_pin("exec_in", "Input", "Trigger Pin", VariableType::Execution);
        node.add_input_pin(
            "break",
            "Break",
            "Trigger this to terminate the active loop early (callable from inside Loop Body)",
            VariableType::Boolean,
        )
        .set_default_value(Some(Value::from(false)));

        // Data input
        node.add_input_pin("array", "Array", "Array to Loop", VariableType::Generic)
            .set_value_type(ValueType::Array)
            .set_options(
                PinOptions::new()
                    .set_enforce_generic_value_type(true)
                    .build(),
            );

        // Execution outputs
        node.add_output_pin(
            "exec_out",
            "Loop Body",
            "Executes for the current item",
            VariableType::Execution,
        );
        node.add_output_pin(
            "done",
            "Completed",
            "Executes once looping has finished or was broken.",
            VariableType::Execution,
        );

        // Data outputs
        node.add_output_pin(
            "value",
            "Value",
            "The current item Value",
            VariableType::Generic,
        );
        node.add_output_pin(
            "index",
            "Index",
            "Current Array Index",
            VariableType::Integer,
        );
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let break_pin = context.get_pin_by_name("break").await?;

        let already_break = context.evaluate_pin_ref(break_pin.clone()).await?;

        if already_break {
            context.log_message("ForEach(Break): breaking early", LogLevel::Debug);
            context.deactivate_exec_pin("exec_out").await?;
            context.activate_exec_pin("done").await?;
            return Ok(());
        }

        // Gather pins
        let done = context.get_pin_by_name("done").await?;
        let exec_item = context.get_pin_by_name("exec_out").await?;
        let value = context.get_pin_by_name("value").await?;
        let index = context.get_pin_by_name("index").await?;
        let array = context.get_pin_by_name("array").await?;

        let id = context.read_node().await.id.clone();
        let recursion_guard = AHashSet::from_iter(vec![id]);

        // Ensure proper exec pin state
        context.deactivate_exec_pin_ref(&done).await?;
        context.deactivate_exec_pin_ref(&exec_item).await?;

        // Read and validate the array
        let array_value: Value = context.evaluate_pin_ref(array).await?;
        let array = array_value
            .as_array()
            .ok_or(flow_like_types::anyhow!("Array value is not an array"))?;

        context.activate_exec_pin_ref(&exec_item).await?;

        'outer: for (i, item) in array.iter().enumerate() {
            // Publish per-iteration values
            let item = item.to_owned();
            let index_value = Value::from(i);
            context.override_pin_value_if_active(&value.id, &item);
            context.override_pin_value_if_active(&index.id, &index_value);
            value.set_value(item).await;
            index.set_value(index_value).await;

            // Trigger connected body nodes sequentially
            let connected = exec_item.get_connected_nodes();
            for node in connected.iter() {
                let mut sub_context = context.create_sub_context(node).await;
                let run = InternalNode::trigger(
                    &mut sub_context,
                    &mut Some(recursion_guard.clone()),
                    true,
                )
                .await;
                sub_context.end_trace();
                context.push_sub_context(&mut sub_context);

                if let Err(error) = run {
                    context.log_message(
                        &format!("Error: {:?} in iteration {}", error, i),
                        LogLevel::Error,
                    );
                }

                // Check if a Break was requested during the body execution
                match context.evaluate_pin_ref(break_pin.clone()).await {
                    Ok(should_break) => {
                        if should_break {
                            context.log_message(
                                &format!("ForEach(Break): breaking at index {}", i),
                                LogLevel::Debug,
                            );
                            break 'outer;
                        }
                    }
                    Err(err) => {
                        context.log_message(
                            &format!("Error checking Break pin: {:?}", err),
                            LogLevel::Error,
                        );
                    }
                }
            }
        }

        context.deactivate_exec_pin_ref(&exec_item).await?;
        context.activate_exec_pin_ref(&done).await?;
        Ok(())
    }

    async fn on_update(&self, node: &mut Node, board: &Board) {
        // Keep generics in sync between array and value
        let _ = node.match_type(
            "array",
            board,
            Some(ValueType::Array),
            Some(ValueType::Array),
        );
        let _ = node.match_type(
            "value",
            board,
            Some(ValueType::Normal),
            Some(ValueType::Normal),
        );
        node.harmonize_type(vec!["array", "value"], true);
    }
}
