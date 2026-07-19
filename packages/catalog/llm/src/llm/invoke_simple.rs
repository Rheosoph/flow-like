use ahash::AHashSet;
use flow_like::{
    bit::Bit,
    flow::{
        execution::{
            LogLevel,
            context::ExecutionContext,
            internal_node::InternalNode,
            log::{LogMessage, LogStat},
        },
        node::{Node, NodeLogic, NodeScores},
        pin::PinOptions,
        variable::VariableType,
    },
};
use flow_like_model_provider::{
    history::{History, HistoryMessage, Role},
    llm::LLMCallback,
    response::{LLMUsageStats, Response},
    response_chunk::ResponseChunk,
};
use flow_like_types::{
    async_trait,
    json::json,
    sync::{DashMap, Mutex},
};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use std::time::Instant;

#[crate::register_node]
#[derive(Default)]
pub struct InvokeLLMSimpleNode {}

impl InvokeLLMSimpleNode {
    pub fn new() -> Self {
        InvokeLLMSimpleNode {}
    }
}

#[async_trait]
impl NodeLogic for InvokeLLMSimpleNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "ai_generative_invoke_simple",
            "Invoke Simple",
            "Invokes an LLM with a system prompt and user prompt, returning text and the full structured response.",
            "AI/Generative",
        );
        node.add_icon("/flow/icons/bot-invoke.svg");
        node.set_version(5);

        // Generic cloud/local model invocation: balanced defaults with light perf bias.
        node.set_scores(
            NodeScores::new()
                .set_privacy(5)
                .set_security(5)
                .set_performance(7)
                .set_governance(5)
                .set_reliability(6)
                .set_cost(5)
                .build(),
        );

        node.add_input_pin(
            "exec_in",
            "Input",
            "Execution trigger to start the invocation",
            VariableType::Execution,
        );

        node.add_input_pin(
            "model",
            "Model",
            "Bit describing the provider/model to execute",
            VariableType::Struct,
        )
        .set_schema::<Bit>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin(
            "system_prompt",
            "System Prompt",
            "Optional system instructions to prime the assistant",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));
        node.add_input_pin(
            "prompt",
            "Prompt",
            "User message that will be sent to the model",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_input_pin(
            "stream",
            "Stream",
            "Stream text tokens when possible. Disable to preserve structured media responses and replay them as rich chunks.",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(true)));

        node.add_output_pin(
            "on_stream",
            "On Stream",
            "Executes for every streamed token chunk",
            VariableType::Execution,
        );

        node.add_output_pin(
            "token",
            "Token",
            "Most recently streamed token or chunk",
            VariableType::String,
        );

        node.add_output_pin(
            "chunk",
            "Chunk",
            "Most recent structured stream or replay chunk, including media content parts",
            VariableType::Struct,
        )
        .set_schema::<ResponseChunk>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin(
            "done",
            "Done",
            "Signals when the invocation finished",
            VariableType::Execution,
        );

        node.add_output_pin(
            "result",
            "Result",
            "Final assistant message extracted from the response",
            VariableType::String,
        );

        node.add_output_pin(
            "response",
            "Response",
            "Full structured model response, including media content parts and reasoning",
            VariableType::Struct,
        )
        .set_schema::<Response>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin(
            "stats",
            "Stats",
            "Token usage, cost, and model statistics",
            VariableType::Struct,
        )
        .set_schema::<LLMUsageStats>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.set_long_running(true);

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("done").await?;
        let model = context.evaluate_pin::<Bit>("model").await?;
        let mut model_name = model.id.clone();
        if let Some(meta) = model.meta.get("en") {
            model_name = meta.name.clone();
        }
        let system_prompt = context.evaluate_pin::<String>("system_prompt").await?;
        let prompt = context.evaluate_pin::<String>("prompt").await?;
        let stream = context.evaluate_pin::<bool>("stream").await?;
        let model_factory = context.app_state.model_factory.clone();
        let model = model_factory
            .lock()
            .await
            .build(
                &model,
                context.app_state.clone(),
                context.token.clone(),
                context.model_usage_context(),
            )
            .await?;

        let mut history = History::new(model_name.clone(), vec![]);
        history.set_system_prompt(system_prompt.clone());
        history.push_message(HistoryMessage::from_string(Role::User, &prompt));
        history.set_stream(stream);

        let on_stream = context.get_pin_by_name("on_stream").await?;
        context.activate_exec_pin_ref(&on_stream).await?;

        let connected_nodes = Arc::new(DashMap::new());
        let connected = on_stream.get_connected_nodes();
        for node in connected {
            let context = Arc::new(Mutex::new(context.create_sub_context(&node).await));
            connected_nodes.insert(node.node.lock().await.id.clone(), context);
        }
        let has_stream_consumers = !connected_nodes.is_empty();

        let parent_node_id = context.node.node.lock().await.id.clone();
        let ctx = context.clone();
        let collection_nodes = connected_nodes.clone();
        let callback_count = Arc::new(AtomicUsize::new(0));
        let collection_callback_count = Arc::clone(&callback_count);
        const FLUSH_EVERY_N_TOKENS: usize = 50;
        let callback: LLMCallback = Arc::new(move |input: ResponseChunk| {
            let ctx = ctx.clone();
            let parent_node_id = parent_node_id.clone();
            let connected_nodes = connected_nodes.clone();
            let callback_count = Arc::clone(&callback_count);
            Box::pin(async move {
                let mut recursion_guard = AHashSet::new();
                recursion_guard.insert(parent_node_id.clone());
                let string_token = input.get_streamed_token().unwrap_or("".to_string());
                let mut ctx = ctx.clone();
                ctx.set_pin_value("token", json!(string_token)).await?;
                ctx.set_pin_value("chunk", json!(input)).await?;
                let count = callback_count.fetch_add(1, Ordering::SeqCst);
                for entry in connected_nodes.iter() {
                    let (id, context) = entry.pair();
                    let mut context = context.lock().await;
                    let mut message = LogMessage::new(
                        &format!("Tracing Token, {:?}", string_token),
                        LogLevel::Debug,
                        None,
                    );
                    let run = InternalNode::trigger(
                        &mut context,
                        &mut Some(recursion_guard.clone()),
                        true,
                    )
                    .await;
                    message.end();
                    context.log(message);
                    context.end_trace();

                    // Flush logs periodically during streaming
                    if count.is_multiple_of(FLUSH_EVERY_N_TOKENS) {
                        let _ = context.flush_logs().await;
                    }

                    match run {
                        Ok(_) => {}
                        Err(_) => {
                            println!("Error running stream node {}", id);
                        }
                    }
                }
                Ok(())
            })
        });

        let mut message = LogMessage::new(
            &format!("Invoking Model, {}", model_name),
            LogLevel::Info,
            None,
        );

        let start = Instant::now();
        let callback = has_stream_consumers.then_some(callback);
        let res = model.invoke(&history, callback).await?;
        let duration_ms = start.elapsed().as_millis() as u64;
        let mut response_string = "".to_string();

        if let Some(response) = res.last_message() {
            response_string = response.content.clone().unwrap_or("".to_string());
        }

        message.end();
        message.put_stats(LogStat::new(
            None,
            Some(collection_callback_count.load(Ordering::SeqCst) as u64),
            None,
        ));
        context.log(message);

        for entry in collection_nodes.iter() {
            let (_, sub_context) = entry.pair();
            let mut sub_context = sub_context.lock().await;
            context.push_sub_context(&mut sub_context);
        }

        let mut stats = LLMUsageStats::from_response(&res);
        stats.set_duration_ms(duration_ms);

        context
            .set_pin_value("result", json!(response_string))
            .await?;
        context.set_pin_value("response", json!(res)).await?;
        context.set_pin_value("stats", json!(stats)).await?;
        context.deactivate_exec_pin("on_stream").await?;
        context.activate_exec_pin("done").await?;

        return Ok(());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exposes_structured_response_and_stream_control() {
        let node = InvokeLLMSimpleNode::new().get_node();
        assert_eq!(node.version, Some(5));
        assert!(node.get_pin_by_name("stream").is_some());
        assert!(node.get_pin_by_name("response").is_some());
        assert!(node.get_pin_by_name("chunk").is_some());
        assert!(node.get_pin_by_name("result").is_some());
    }
}
