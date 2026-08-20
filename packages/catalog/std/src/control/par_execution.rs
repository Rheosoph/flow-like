use flow_like::{
    flow::{
        execution::{LogLevel, context::ExecutionContext, internal_node::InternalNode},
        node::{Node, NodeLogic},
        pin::PinOptions,
        variable::VariableType,
    },
    num_cpus,
};
use flow_like_types::{async_trait, json::json, tokio};
use futures::{FutureExt, StreamExt, future::BoxFuture, stream::FuturesUnordered};
use std::sync::Arc;
use tokio::sync::Semaphore;

#[crate::register_node]
#[derive(Default)]
pub struct ParallelExecutionNode;

impl ParallelExecutionNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for ParallelExecutionNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "control_par_execution",
            "Parallel Execution",
            "Parallel Execution",
            "Control",
        );
        node.add_icon("/flow/icons/par_execution.svg");

        node.add_input_pin("exec_in", "Input", "Trigger Pin", VariableType::Execution);
        node.add_input_pin("thread_model", "Threads", "Threads", VariableType::String)
            .set_default_value(Some(json!("tasks")))
            .set_options(
                PinOptions::new()
                    .set_valid_values(vec!["tasks".into(), "threads".into()])
                    .build(),
            );

        node.add_output_pin(
            "exec_out",
            "Parallel Task",
            "Parallel Task Pin",
            VariableType::Execution,
        );
        node.add_output_pin(
            "exec_out",
            "Parallel Task",
            "Parallel Task Pin",
            VariableType::Execution,
        );
        node.add_output_pin("exec_done", "Done", "Done Pin", VariableType::Execution);

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_done").await?;

        let mode: String = context.evaluate_pin("thread_model").await?;
        let requested_threads = matches!(mode.as_str(), "threads");
        let use_threads = if requested_threads {
            matches!(
                tokio::runtime::Handle::try_current().map(|handle| handle.runtime_flavor()),
                Ok(tokio::runtime::RuntimeFlavor::MultiThread)
            )
        } else {
            false
        };

        if requested_threads && !use_threads {
            context.log_message(
                "Parallel Execution 'threads' mode requires a multi-thread Tokio runtime; falling back to task mode",
                LogLevel::Warn,
            );
        }

        let exec_out_pins = context.get_pins_by_name("exec_out").await?;
        let mut to_run = Vec::new();
        for pin in &exec_out_pins {
            context.activate_exec_pin_ref(pin).await?;
            let nodes = pin.get_connected_nodes();
            for node in nodes {
                let sub = context.create_sub_context(&node).await;
                to_run.push(sub);
            }
        }
        if to_run.is_empty() {
            context.activate_exec_pin("exec_done").await?;
            return Ok(());
        }

        // cap concurrency to keep stacks / memory in check
        let max_concurrency = std::cmp::max(1, num_cpus::get());
        let sem = Arc::new(Semaphore::new(max_concurrency));

        #[allow(clippy::large_enum_variant)] // output type of an already-boxed future; boxing would add one alloc per parallel branch
        enum TaskOutcome {
            Ok(ExecutionContext),
            JoinErr(String),
        }

        let mut tasks: FuturesUnordered<BoxFuture<'static, TaskOutcome>> = FuturesUnordered::new();

        if !use_threads {
            // async tasks mode
            for mut sub in to_run {
                let sem = sem.clone();
                tasks.push(
                    async move {
                        let _permit = sem.acquire().await.expect("semaphore closed");
                        if let Err(err) = InternalNode::trigger(&mut sub, &mut None, true).await {
                            sub.log_message(
                                &format!("Error running node: {err:?}"),
                                LogLevel::Error,
                            );
                        }
                        sub.end_trace();

                        TaskOutcome::Ok(sub)
                    }
                    .boxed(),
                );
            }
        } else {
            // CPU threads mode (bounded); beware small default stacks on macOS
            let rt = tokio::runtime::Handle::current();
            for mut sub in to_run {
                let sem = sem.clone();
                let h = rt.clone(); // <-- clone per iteration

                tasks.push(
                    async move {
                        let _permit = sem.acquire().await.expect("semaphore closed");

                        match tokio::task::spawn_blocking(move || {
                            // use the per-iter clone
                            h.block_on(async {
                                if let Err(err) =
                                    InternalNode::trigger(&mut sub, &mut None, true).await
                                {
                                    sub.log_message(
                                        &format!("Error running node (threads): {err:?}"),
                                        LogLevel::Error,
                                    );
                                }
                                sub.end_trace();
                                sub
                            })
                        })
                        .await
                        {
                            Ok(sub) => TaskOutcome::Ok(sub),
                            Err(join_err) => TaskOutcome::JoinErr(format!("{join_err:?}")),
                        }
                    }
                    .boxed(),
                );
            }
        }

        // collect and push into parent; log failures on the parent
        while let Some(outcome) = tasks.next().await {
            match outcome {
                TaskOutcome::Ok(mut sub) => context.push_sub_context(&mut sub),
                TaskOutcome::JoinErr(msg) => {
                    context.log_message(&format!("Thread join error: {msg}"), LogLevel::Error)
                }
            }
        }

        for pin in &exec_out_pins {
            context.deactivate_exec_pin_ref(pin).await?;
        }

        context.activate_exec_pin("exec_done").await?;
        Ok(())
    }
}
