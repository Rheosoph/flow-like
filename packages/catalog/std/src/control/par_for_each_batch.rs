use flow_like::flow::{
    board::Board,
    execution::{LogLevel, context::ExecutionContext, internal_node::InternalNode},
    node::{Node, NodeLogic, NodeScores},
    pin::{PinOptions, ValueType},
    variable::VariableType,
};
use flow_like_types::{Value, async_trait, json::json};
use futures::stream::{FuturesUnordered, StreamExt};
use std::sync::Arc;

struct BatchPins {
    batch: String,
    index: String,
    start_index: String,
}

#[crate::register_node]
#[derive(Default)]
pub struct ParBatchLoopNode {}

impl ParBatchLoopNode {
    pub fn new() -> Self {
        ParBatchLoopNode {}
    }
}

#[async_trait]
impl NodeLogic for ParBatchLoopNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "control_par_for_each_batch",
            "Parallel For Each (Batch)",
            "Loops over an Array in batches, running the body for multiple batches in parallel",
            "Control",
        );
        node.add_icon("/flow/icons/for-each.svg");
        node.set_scores(
            NodeScores::new()
                .set_privacy(10)
                .set_security(10)
                .set_performance(9)
                .set_governance(10)
                .set_reliability(8)
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
        node.add_input_pin(
            "max_concurrent",
            "Max Concurrent",
            "Maximum number of concurrent body executions (0 = unlimited)",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(30)));

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

        // Seed the pins so the dependency system sees them as "having a value";
        // per batch values are supplied through sub context overrides.
        batch_pin.set_value(Value::Array(vec![])).await;
        index_pin.set_value(Value::from(0)).await;
        start_index_pin.set_value(Value::from(0)).await;

        let pins = BatchPins {
            batch: batch_pin.id.to_string(),
            index: index_pin.id.to_string(),
            start_index: start_index_pin.id.to_string(),
        };

        let batch_size = context.evaluate_pin::<i64>("batch_size").await?.max(1) as usize;
        let max_concurrent = context.evaluate_pin::<i64>("max_concurrent").await?;
        let max_concurrent = if max_concurrent > 0 {
            max_concurrent as usize
        } else {
            usize::MAX
        };

        let array_value = context.evaluate_pin_to_ref("array").await?;
        let array_value = array_value
            .as_array()
            .ok_or_else(|| flow_like_types::anyhow!("Array value is not an array"))?;

        context.activate_exec_pin_ref(&exec_item).await?;

        let (results, cancelled) = self
            .run_batches(
                context,
                array_value,
                batch_size,
                &connected,
                &pins,
                max_concurrent,
            )
            .await;

        for (mut sub_context, run, i) in results {
            context.push_sub_context(&mut sub_context);

            if let Err(error) = run {
                context.log_message(
                    &format!("Error: {:?} in batch {}", error, i),
                    LogLevel::Error,
                );
            }
        }

        context.deactivate_exec_pin_ref(&exec_item).await?;

        if let Some(batch) = cancelled {
            context.log_message(
                &format!("Execution cancelled, stopped batch loop at batch {}", batch),
                LogLevel::Warn,
            );
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

impl ParBatchLoopNode {
    async fn run_batches(
        &self,
        context: &ExecutionContext,
        array_value: &[Value],
        batch_size: usize,
        connected: &[Arc<InternalNode>],
        pins: &BatchPins,
        max_concurrent: usize,
    ) -> (
        Vec<(ExecutionContext, Result<(), String>, usize)>,
        Option<usize>,
    ) {
        let mut stream = FuturesUnordered::new();
        let mut results = Vec::new();
        let mut cancelled = None;

        'batches: for (i, chunk) in array_value.chunks(batch_size).enumerate() {
            if context.is_cancelled() {
                cancelled = Some(i);
                break 'batches;
            }

            let batch = Value::Array(chunk.to_vec());
            let start_index = Value::from(i * batch_size);

            for node in connected.iter() {
                let mut sub_context = context.create_sub_context(node).await;

                sub_context.override_pin_value(&pins.batch, batch.clone());
                sub_context.override_pin_value(&pins.index, Value::from(i));
                sub_context.override_pin_value(&pins.start_index, start_index.clone());

                stream.push(async move {
                    let run = InternalNode::trigger(&mut sub_context, &mut None, true).await;
                    sub_context.end_trace();
                    let result = run.map_err(|e| format!("{:?}", e));
                    (sub_context, result, i)
                });

                while stream.len() >= max_concurrent {
                    match stream.next().await {
                        Some(result) => results.push(result),
                        None => break,
                    }
                }
            }
        }

        while let Some(result) = stream.next().await {
            results.push(result);
        }

        (results, cancelled)
    }
}
