//! Copilot Invoke Nodes
//!
//! Nodes for sending messages and receiving responses, with history support
//! that matches the model invoke node interface.

use super::CopilotSessionHandle;
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic, NodeScores},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_model_provider::{
    history::{History, Role},
    response::Response,
    response_chunk::ResponseChunk,
};
use flow_like_types::{async_trait, json};

/// Convert Flow-Like History to Copilot message format
#[cfg(feature = "execute")]
fn history_to_copilot_context(history: &History) -> String {
    let mut context_parts = vec![];

    for msg in &history.messages {
        let role = match msg.role {
            Role::User => "User",
            Role::Assistant => "Assistant",
            Role::System => "System",
            Role::Tool => "Tool",
            Role::Function => "Function",
        };

        let content = msg.as_str();
        context_parts.push(format!("{}: {}", role, content));
    }

    context_parts.join("\n\n")
}

/// Create a Response from Copilot response text
fn create_response(text: &str, model: &str) -> Response {
    Response::from_text(text, model)
}

#[crate::register_node]
#[derive(Default)]
pub struct CopilotSendAndWaitNode {}

#[async_trait]
impl NodeLogic for CopilotSendAndWaitNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "copilot_send_and_wait",
            "Send Message",
            "Sends a message to Copilot and waits for complete response. Supports history input for context.",
            "AI/GitHub/Copilot/Chat",
        );
        node.add_icon("/flow/icons/github.svg");

        node.set_scores(
            NodeScores::new()
                .set_privacy(5)
                .set_security(7)
                .set_performance(5)
                .set_governance(7)
                .set_reliability(8)
                .set_cost(6)
                .build(),
        );

        node.add_input_pin("exec_in", "Input", "Trigger Pin", VariableType::Execution);

        node.add_input_pin(
            "session",
            "Session",
            "Copilot session",
            VariableType::Struct,
        )
        .set_schema::<CopilotSessionHandle>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin("prompt", "Prompt", "Message to send", VariableType::String);

        node.add_input_pin(
            "history",
            "History",
            "Optional chat history for context (same format as Model Invoke)",
            VariableType::Struct,
        )
        .set_schema::<History>();

        node.add_output_pin(
            "exec_out",
            "Output",
            "Continues after response is received",
            VariableType::Execution,
        );

        node.add_output_pin(
            "response",
            "Response",
            "Complete response text",
            VariableType::String,
        );

        node.add_output_pin(
            "result",
            "Result",
            "Response in standard model format (matches Model Invoke)",
            VariableType::Struct,
        )
        .set_schema::<Response>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.set_long_running(true);

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        use super::CachedCopilotSession;
        use flow_like::flow::execution::LogLevel;

        context.deactivate_exec_pin("exec_out").await?;

        let handle: CopilotSessionHandle = context.evaluate_pin("session").await?;
        let prompt: String = context.evaluate_pin("prompt").await?;
        let history: Option<History> = context.evaluate_pin("history").await.ok();

        let cached = {
            let cache = context.cache.read().await;
            cache.get(&handle.cache_key).cloned()
        };
        let cached =
            cached.ok_or_else(|| flow_like_types::anyhow!("Copilot session not found in cache"))?;
        let cached_session = cached
            .as_any()
            .downcast_ref::<CachedCopilotSession>()
            .ok_or_else(|| flow_like_types::anyhow!("Failed to downcast cached session"))?;

        let full_prompt = if let Some(hist) = history {
            let context_str = history_to_copilot_context(&hist);
            if context_str.is_empty() {
                prompt
            } else {
                format!(
                    "Context from previous conversation:\n{}\n\nCurrent request:\n{}",
                    context_str, prompt
                )
            }
        } else {
            prompt.clone()
        };

        context.log_message(
            &format!(
                "Sending message: {}...",
                &full_prompt.chars().take(50).collect::<String>()
            ),
            LogLevel::Debug,
        );

        let response = cached_session
            .session
            .send_and_wait(full_prompt.as_str())
            .await
            .map_err(|e| flow_like_types::anyhow!("Failed to send message: {}", e))?;

        context.log_message(
            &format!("Received response ({} chars)", response.len()),
            LogLevel::Debug,
        );

        let result = create_response(&response, "copilot");

        context
            .set_pin_value("response", json::json!(response))
            .await?;
        context.set_pin_value("result", json::json!(result)).await?;
        context.activate_exec_pin("exec_out").await?;

        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "GitHub Copilot integration requires the 'execute' feature"
        ))
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct CopilotSendStreamingNode {}

#[async_trait]
impl NodeLogic for CopilotSendStreamingNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "copilot_send_streaming",
            "Stream Message",
            "Sends a message to Copilot and streams the response. Supports history input and matches Model Invoke interface.",
            "AI/GitHub/Copilot/Chat",
        );
        node.add_icon("/flow/icons/github.svg");

        node.set_scores(
            NodeScores::new()
                .set_privacy(5)
                .set_security(7)
                .set_performance(6)
                .set_governance(7)
                .set_reliability(7)
                .set_cost(6)
                .build(),
        );

        node.add_input_pin("exec_in", "Input", "Trigger Pin", VariableType::Execution);

        node.add_input_pin(
            "session",
            "Session",
            "Copilot session",
            VariableType::Struct,
        )
        .set_schema::<CopilotSessionHandle>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin("prompt", "Prompt", "Message to send", VariableType::String);

        node.add_input_pin(
            "history",
            "History",
            "Optional chat history for context (same format as Model Invoke)",
            VariableType::Struct,
        )
        .set_schema::<History>();

        node.add_output_pin(
            "on_stream",
            "On Stream",
            "Fires for each streaming chunk (matches Model Invoke)",
            VariableType::Execution,
        );

        node.add_output_pin(
            "done",
            "Done",
            "Fires when streaming is complete (matches Model Invoke)",
            VariableType::Execution,
        );

        node.add_output_pin(
            "chunk",
            "Chunk",
            "Current streaming chunk (matches Model Invoke ResponseChunk format)",
            VariableType::Struct,
        )
        .set_schema::<ResponseChunk>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin(
            "result",
            "Result",
            "Complete response (matches Model Invoke Response format)",
            VariableType::Struct,
        )
        .set_schema::<Response>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin(
            "full_response",
            "Full Response",
            "Complete accumulated response text",
            VariableType::String,
        );

        node.set_long_running(true);

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        use super::CachedCopilotSession;
        use copilot_sdk::SessionEventData;
        use flow_like::flow::execution::LogLevel;

        context.deactivate_exec_pin("done").await?;
        context.deactivate_exec_pin("on_stream").await?;

        let handle: CopilotSessionHandle = context.evaluate_pin("session").await?;
        let prompt: String = context.evaluate_pin("prompt").await?;
        let history: Option<History> = context.evaluate_pin("history").await.ok();

        let cached = {
            let cache = context.cache.read().await;
            cache.get(&handle.cache_key).cloned()
        };
        let cached =
            cached.ok_or_else(|| flow_like_types::anyhow!("Copilot session not found in cache"))?;
        let cached_session = cached
            .as_any()
            .downcast_ref::<CachedCopilotSession>()
            .ok_or_else(|| flow_like_types::anyhow!("Failed to downcast cached session"))?;

        let full_prompt = if let Some(hist) = history {
            let context_str = history_to_copilot_context(&hist);
            if context_str.is_empty() {
                prompt
            } else {
                format!(
                    "Context from previous conversation:\n{}\n\nCurrent request:\n{}",
                    context_str, prompt
                )
            }
        } else {
            prompt.clone()
        };

        context.log_message("Starting streaming response...", LogLevel::Debug);

        let mut events = cached_session.session.subscribe();
        cached_session
            .session
            .send(full_prompt.as_str())
            .await
            .map_err(|e| flow_like_types::anyhow!("Failed to send message: {}", e))?;

        let mut full_response = String::new();

        loop {
            match events.recv().await {
                Ok(event) => match &event.data {
                    SessionEventData::AssistantMessageDelta(delta) => {
                        full_response.push_str(&delta.delta_content);

                        let chunk = ResponseChunk::from_text(&delta.delta_content, "copilot");

                        context.set_pin_value("chunk", json::json!(chunk)).await?;
                        context
                            .set_pin_value("full_response", json::json!(full_response))
                            .await?;
                        context.activate_exec_pin("on_stream").await?;
                    }
                    SessionEventData::AssistantMessage(msg) => {
                        full_response = msg.content.clone();
                    }
                    SessionEventData::SessionIdle(_) => {
                        break;
                    }
                    SessionEventData::SessionError(err) => {
                        return Err(flow_like_types::anyhow!("Session error: {:?}", err));
                    }
                    _ => {}
                },
                Err(e) => {
                    context.log_message(&format!("Event receive error: {}", e), LogLevel::Warn);
                    break;
                }
            }
        }

        let result = create_response(&full_response, "copilot");

        context
            .set_pin_value("full_response", json::json!(full_response))
            .await?;
        context.set_pin_value("result", json::json!(result)).await?;
        context.deactivate_exec_pin("on_stream").await?;
        context.activate_exec_pin("done").await?;

        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "GitHub Copilot integration requires the 'execute' feature"
        ))
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct CopilotAbortNode {}

#[async_trait]
impl NodeLogic for CopilotAbortNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "copilot_abort",
            "Abort",
            "Aborts the current message processing",
            "AI/GitHub/Copilot/Chat",
        );
        node.add_icon("/flow/icons/github.svg");

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

        node.add_input_pin(
            "session",
            "Session",
            "Copilot session",
            VariableType::Struct,
        )
        .set_schema::<CopilotSessionHandle>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin(
            "exec_out",
            "Output",
            "Continues after abort",
            VariableType::Execution,
        );

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        use super::CachedCopilotSession;
        use flow_like::flow::execution::LogLevel;

        context.deactivate_exec_pin("exec_out").await?;

        let handle: CopilotSessionHandle = context.evaluate_pin("session").await?;

        let cached = {
            let cache = context.cache.read().await;
            cache.get(&handle.cache_key).cloned()
        };
        let cached =
            cached.ok_or_else(|| flow_like_types::anyhow!("Copilot session not found in cache"))?;
        let cached_session = cached
            .as_any()
            .downcast_ref::<CachedCopilotSession>()
            .ok_or_else(|| flow_like_types::anyhow!("Failed to downcast cached session"))?;

        cached_session
            .session
            .abort()
            .await
            .map_err(|e| flow_like_types::anyhow!("Failed to abort: {}", e))?;

        context.log_message("Message processing aborted", LogLevel::Info);
        context.activate_exec_pin("exec_out").await?;

        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "GitHub Copilot integration requires the 'execute' feature"
        ))
    }
}
