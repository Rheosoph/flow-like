use flow_like::flow::{
    board::Board,
    execution::{LogLevel, context::ExecutionContext, internal_node::InternalNode},
    node::{Node, NodeLogic},
    pin::{PinOptions, ValueType},
    variable::VariableType,
};
use flow_like_types::{Value, async_trait, json::json};
use futures::stream::{FuturesUnordered, StreamExt};
use std::sync::Arc;

#[crate::register_node]
#[derive(Default)]
pub struct ParLoopNode {}

impl ParLoopNode {
    pub fn new() -> Self {
        ParLoopNode {}
    }
}

#[async_trait]
impl NodeLogic for ParLoopNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "control_par_for_each",
            "Parallel For Each",
            "Loops over an Array in Parallel",
            "Control",
        );
        node.set_flowscript_name("control", "parallelForEach");
        node.add_icon("/flow/icons/for-each.svg");

        node.add_input_pin("exec_in", "Input", "Trigger Pin", VariableType::Execution);
        node.add_input_pin("array", "Array", "Array to Loop", VariableType::Generic)
            .set_value_type(ValueType::Array)
            .set_options(
                PinOptions::new()
                    .set_enforce_generic_value_type(true)
                    .build(),
            );

        node.add_input_pin(
            "max_concurrent",
            "Max Concurrent",
            "Maximum number of concurrent executions (0 = unlimited)",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(30)));

        node.add_output_pin(
            "exec_out",
            "For Each Element",
            "Executes the current item",
            VariableType::Execution,
        );
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
        context.deactivate_exec_pin_ref(&done).await?;

        let array = context.get_pin_by_name("array").await?;
        let value_pin = context.get_pin_by_name("value").await?;
        let index_pin = context.get_pin_by_name("index").await?;
        let exec_item = context.get_pin_by_name("exec_out").await?;
        let connected = exec_item.get_connected_nodes();
        let max_concurrent: i64 = context.evaluate_pin("max_concurrent").await?;

        // Initialize pins with dummy values so dependency system sees them as "having a value"
        value_pin.set_value(Value::Null).await;
        index_pin.set_value(Value::from(0)).await;

        let value_pin_id = value_pin.id.clone();
        let index_pin_id = index_pin.id.clone();

        let array_value: Value = context.evaluate_pin_ref(array).await?;
        let array_value = array_value
            .as_array()
            .ok_or(flow_like_types::anyhow!("Array value is not an array"))?;

        context.activate_exec_pin_ref(&exec_item).await?;

        let results = if max_concurrent > 0 {
            self.run_with_bounded_concurrency(
                context,
                array_value,
                &connected,
                &value_pin_id,
                &index_pin_id,
                max_concurrent as usize,
            )
            .await?
        } else {
            self.run_with_unlimited_concurrency(
                context,
                array_value,
                &connected,
                &value_pin_id,
                &index_pin_id,
            )
            .await?
        };

        for (mut sub_context, run, i) in results {
            context.push_sub_context(&mut sub_context);

            if let Err(error) = run {
                context.log_message(
                    &format!("Error: {:?} in iteration {}", error, i),
                    LogLevel::Error,
                );
            }
        }

        context.deactivate_exec_pin_ref(&exec_item).await?;
        context.activate_exec_pin_ref(&done).await?;

        return Ok(());
    }

    async fn on_update(&self, node: &mut Node, board: &Board) {
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

impl ParLoopNode {
    async fn run_with_bounded_concurrency(
        &self,
        context: &ExecutionContext,
        array_value: &[Value],
        connected: &[Arc<InternalNode>],
        value_pin_id: &str,
        index_pin_id: &str,
        max_concurrent: usize,
    ) -> flow_like_types::Result<Vec<(ExecutionContext, Result<(), String>, usize)>> {
        let mut stream = FuturesUnordered::new();
        let mut results = Vec::new();

        for (i, item) in array_value.iter().enumerate() {
            let item = item.to_owned();

            for node in connected.iter() {
                let mut sub_context = context.create_sub_context(node).await;

                sub_context.override_pin_value(value_pin_id, item.clone());
                sub_context.override_pin_value(index_pin_id, Value::from(i));

                let task = async move {
                    let run = InternalNode::trigger(&mut sub_context, &mut None, true).await;
                    sub_context.end_trace();
                    let result = run.map_err(|e| format!("{:?}", e));
                    (sub_context, result, i)
                };

                stream.push(task);

                while stream.len() >= max_concurrent {
                    if let Some(result) = stream.next().await {
                        results.push(result);
                    }
                }
            }
        }

        while let Some(result) = stream.next().await {
            results.push(result);
        }

        Ok(results)
    }

    async fn run_with_unlimited_concurrency(
        &self,
        context: &ExecutionContext,
        array_value: &[Value],
        connected: &[Arc<InternalNode>],
        value_pin_id: &str,
        index_pin_id: &str,
    ) -> flow_like_types::Result<Vec<(ExecutionContext, Result<(), String>, usize)>> {
        let mut parallel_tasks = Vec::with_capacity(array_value.len() * connected.len());

        for (i, item) in array_value.iter().enumerate() {
            let item = item.to_owned();

            for node in connected.iter() {
                let mut sub_context = context.create_sub_context(node).await;

                sub_context.override_pin_value(value_pin_id, item.clone());
                sub_context.override_pin_value(index_pin_id, Value::from(i));

                let task = async move {
                    let run = InternalNode::trigger(&mut sub_context, &mut None, true).await;
                    sub_context.end_trace();
                    let result = run.map_err(|e| format!("{:?}", e));
                    (sub_context, result, i)
                };

                parallel_tasks.push(task);
            }
        }

        let results = futures::future::join_all(parallel_tasks).await;
        Ok(results)
    }
}
