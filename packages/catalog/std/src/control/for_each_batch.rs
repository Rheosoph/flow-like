use flow_like::flow::{
    board::Board,
    execution::{LogLevel, context::ExecutionContext, internal_node::InternalNode},
    node::{Node, NodeLogic, NodeScores},
    pin::{PinOptions, ValueType},
    variable::VariableType,
};
use flow_like_types::{Value, async_trait, json::json};

#[crate::register_node]
#[derive(Default)]
pub struct BatchLoopNode {}

impl BatchLoopNode {
    pub fn new() -> Self {
        BatchLoopNode {}
    }
}

#[async_trait]
impl NodeLogic for BatchLoopNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "control_for_each_batch",
            "For Each (Batch)",
            "Loops over an Array in batches, running the body once per slice of up to Batch Size elements",
            "Control",
        );
        node.add_icon("/flow/icons/for-each.svg");
        node.set_scores(
            NodeScores::new()
                .set_privacy(10)
                .set_security(10)
                .set_performance(8)
                .set_governance(10)
                .set_reliability(9)
                .set_cost(10)
                .build(),
        );

        node.add_input_pin("exec_in", "Input", "Trigger Pin", VariableType::Execution);
        node.add_input_pin("array", "Array", "Array to Loop", VariableType::Generic)
            .set_value_type(ValueType::Array)
            .set_options(
                PinOptions::new()
                    .set_enforce_generic_value_type(true)
                    .build(),
            );
        node.add_input_pin(
            "batch_size",
            "Batch Size",
            "Maximum number of elements per batch. Values below 1 are clamped to 1.",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(10)));

        node.add_output_pin(
            "exec_out",
            "For Each Batch",
            "Executes once for the current batch",
            VariableType::Execution,
        );
        node.add_output_pin(
            "batch",
            "Batch",
            "The current slice, holding up to Batch Size elements",
            VariableType::Generic,
        )
        .set_value_type(ValueType::Array);
        node.add_output_pin(
            "index",
            "Batch Index",
            "Zero based index of the current batch",
            VariableType::Integer,
        );
        node.add_output_pin(
            "start_index",
            "Start Index",
            "Index of the first element of this batch inside the source array",
            VariableType::Integer,
        );
        node.add_output_pin(
            "done",
            "Done",
            "Executes once the array is dealt with.",
            VariableType::Execution,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let done = context.get_pin_by_name("done").await?;
        let exec_item = context.get_pin_by_name("exec_out").await?;
        context.deactivate_exec_pin_ref(&done).await?;
        context.deactivate_exec_pin_ref(&exec_item).await?;

        let batch_pin = context.get_pin_by_name("batch").await?;
        let index_pin = context.get_pin_by_name("index").await?;
        let start_index_pin = context.get_pin_by_name("start_index").await?;
        let connected = exec_item.get_connected_nodes();

        let batch_size = context.evaluate_pin::<i64>("batch_size").await?.max(1) as usize;

        let array_value = context.evaluate_pin_to_ref("array").await?;
        let array_value = array_value
            .as_array()
            .ok_or_else(|| flow_like_types::anyhow!("Array value is not an array"))?;

        let mut cancelled = false;
        context.activate_exec_pin_ref(&exec_item).await?;
        for (i, chunk) in array_value.chunks(batch_size).enumerate() {
            if context.is_cancelled() {
                context.log_message(
                    &format!("Execution cancelled, stopping batch loop at batch {}", i),
                    LogLevel::Warn,
                );
                cancelled = true;
                break;
            }

            batch_pin.set_value(Value::Array(chunk.to_vec())).await;
            index_pin.set_value(Value::from(i)).await;
            start_index_pin.set_value(Value::from(i * batch_size)).await;

            for node in connected.iter() {
                let mut sub_context = context.create_sub_context(node).await;
                let run = InternalNode::trigger(&mut sub_context, &mut None, true).await;
                sub_context.end_trace();
                context.push_sub_context(&mut sub_context);

                if let Err(error) = run {
                    context.log_message(
                        &format!("Error: {:?} in batch {}", error, i),
                        LogLevel::Error,
                    );
                }
            }
        }

        context.deactivate_exec_pin_ref(&exec_item).await?;

        if cancelled {
            return Err(flow_like_types::anyhow!("Execution was cancelled"));
        }

        context.activate_exec_pin_ref(&done).await?;

        Ok(())
    }

    async fn on_update(&self, node: &mut Node, board: &Board) {
        let _ = node.match_type(
            "array",
            board,
            Some(ValueType::Array),
            Some(ValueType::Array),
        );
        let _ = node.match_type(
            "batch",
            board,
            Some(ValueType::Array),
            Some(ValueType::Array),
        );
        node.harmonize_type(vec!["array", "batch"], true);
    }
}
