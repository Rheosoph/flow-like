//! Platform assistant runner — makes profile ("Bits") models tool-capable for the global FlowPilot
//! assistant, reusing the exact machinery the board copilot uses for Bits: resolve the profile model
//! into a rig completion client, build a rig agent with tool schemas + a system prompt, and run the
//! manual tool-call loop, streaming the same XML tag protocol (`tool_call:`, `<plan_step>`,
//! `tool_result:done`) that the shared frontend parser renders.
//!
//! The platform tools act on the host app (navigate, create app, delegate to the board copilot, ask
//! the user), which is desktop-only, so core defines only the tool *schemas* + a `PlatformToolBridge`
//! trait; the concrete execution + approval lives in the desktop crate.

use std::sync::Arc;

use async_trait::async_trait;
use flow_like_model_provider::llm::CompletionClientDyn;
use flow_like_model_provider::provider::ModelProvider;
use flow_like_types::Result;
use futures::StreamExt;
use rig::{
    OneOrMany,
    completion::{Completion, ToolDefinition},
    message::{
        AssistantContent, DocumentSourceKind, Image, ImageDetail, ImageMediaType,
        ToolResult as RigToolResult, ToolResultContent, UserContent,
    },
    streaming::StreamedAssistantContent,
    tool::Tool,
};
use serde_json::{Value, json};

use super::memory::AssistantMemory;
use super::types::{ChatImage, ChatMessage, ChatRole, PlanStep, PlanStepStatus, StreamEvent};
use crate::bit::{Bit, BitModelPreference, BitTypes, LLMParameters};
use crate::profile::Profile;
use crate::state::FlowLikeState;

#[derive(Debug, thiserror::Error)]
#[error("Platform tool dispatched to the host bridge")]
pub struct PlatformToolError;

/// Executes the global assistant's platform tools. Implemented by the desktop crate over the
/// FrontendToolBridge (with per-tool approval); returns the tool result as a string for the model.
#[async_trait]
pub trait PlatformToolBridge: Send + Sync {
    async fn call(&self, tool_name: &str, arguments: Value) -> String;
}

// The tool structs below only supply schemas to the model; the manual loop dispatches every call
// through the PlatformToolBridge, so `call` is never invoked by rig (kept minimal for the trait).
macro_rules! platform_tool {
    ($struct_name:ident, $name:literal, $description:expr, $params:expr) => {
        pub struct $struct_name;

        impl Tool for $struct_name {
            const NAME: &'static str = $name;
            type Error = PlatformToolError;
            type Args = Value;
            type Output = String;

            async fn definition(&self, _prompt: String) -> ToolDefinition {
                ToolDefinition {
                    name: $name.to_string(),
                    description: $description.to_string(),
                    parameters: $params,
                }
            }

            async fn call(&self, _args: Self::Args) -> std::result::Result<Self::Output, Self::Error> {
                Err(PlatformToolError)
            }
        }
    };
}

platform_tool!(
    ListAppsTool,
    "list_apps",
    "List the apps visible in the user's current profile and the callable interfaces each exposes (e.g. a chat event). Use before call_app_chat / flowpilot_board. Only current-profile apps are returned.",
    json!({ "type": "object", "properties": {} })
);

platform_tool!(
    DescribeAppInterfaceTool,
    "describe_app_interface",
    "Read the full, user-readable configuration of one app event/interface (chat, MCP, REST, simple chat, …). Use after list_apps to understand HOW to call an interface. Read-only.",
    json!({
        "type": "object",
        "properties": {
            "app_id": { "type": "string", "description": "App id (from list_apps)." },
            "event_id": { "type": "string", "description": "Event id (from list_apps)." }
        },
        "required": ["app_id", "event_id"]
    })
);

platform_tool!(
    OpenAppChatTool,
    "open_app_chat",
    "Open an app's chat event as an inline chat card in the user's current view, so the USER can talk to that app directly. Prefer over call_app_chat when the user should take over. Non-destructive.",
    json!({
        "type": "object",
        "properties": {
            "app_id": { "type": "string", "description": "App id (from list_apps)." },
            "event_id": { "type": "string", "description": "Chat event id (from list_apps). Optional; defaults to the app's first chat event." }
        },
        "required": ["app_id"]
    })
);

platform_tool!(
    NavigateViewTool,
    "navigate_view",
    "Navigate the Flow-Like app to a view or route (e.g. an app, the store, settings, a profile). Non-destructive UI change.",
    json!({
        "type": "object",
        "properties": {
            "view": { "type": "string", "description": "Logical view id, e.g. 'home', 'apps', 'store', 'settings', 'profile', 'board'." },
            "route": { "type": "string", "description": "Explicit router path, e.g. '/store' or '/library?id=<app>'." },
            "app_id": { "type": "string", "description": "App id when the view is app-scoped." }
        },
        "required": ["view"]
    })
);

platform_tool!(
    CreateAppTool,
    "create_app",
    "Create a new Flow-Like app (project) in the current profile. Mutating: asks the user for approval.",
    json!({
        "type": "object",
        "properties": {
            "name": { "type": "string", "description": "Human-readable app name." },
            "description": { "type": "string", "description": "Short description of what the app does." }
        },
        "required": ["name"]
    })
);

platform_tool!(
    FlowPilotBoardTool,
    "flowpilot_board",
    "Delegate board- or page-internal work (add/connect/configure nodes, design a page) to the board FlowPilot for a specific app/board. Side-effecting: asks for approval.",
    json!({
        "type": "object",
        "properties": {
            "instruction": { "type": "string", "description": "Natural-language instruction for the board copilot." },
            "app_id": { "type": "string", "description": "App id." },
            "board_id": { "type": "string", "description": "Target board id within the app." }
        },
        "required": ["instruction"]
    })
);

platform_tool!(
    AskUserTool,
    "ask_user",
    "Ask the user a question and wait for their typed answer. Use when you are missing information a tool needs.",
    json!({
        "type": "object",
        "properties": {
            "question": { "type": "string", "description": "The question to show the user." }
        },
        "required": ["question"]
    })
);

platform_tool!(
    MemoryStoreTool,
    "_memory_store",
    "Store an important fact, user preference, decision, or context in your persistent profile-scoped memory. Call this immediately when you learn something worth remembering — do not merely say you will remember.",
    json!({
        "type": "object",
        "properties": {
            "content": { "type": "string", "description": "The fact/observation to remember." },
            "role": { "type": "string", "description": "One of: user, assistant, observation, summary. Default: observation." }
        },
        "required": ["content"]
    })
);

platform_tool!(
    MemorySearchTool,
    "_memory_search",
    "Search your persistent profile-scoped memory for relevant facts and context. Search at the start of a conversation and whenever prior context would help.",
    json!({
        "type": "object",
        "properties": {
            "query": { "type": "string", "description": "What to recall." }
        },
        "required": ["query"]
    })
);

platform_tool!(
    CallAppChatTool,
    "call_app_chat",
    "Talk to a Flow-Like app that exposes a chat event: send it a message and get its reply. Side-effecting; asks for approval. Use list_apps first to pick the app + chat event.",
    json!({
        "type": "object",
        "properties": {
            "app_id": { "type": "string", "description": "Id of the app whose chat event to call (from list_apps)." },
            "event_id": { "type": "string", "description": "Id of the specific chat event to call (from list_apps). Optional; defaults to the app's first chat event." },
            "message": { "type": "string", "description": "Message to send to the app's chat." }
        },
        "required": ["app_id", "message"]
    })
);

/// Global platform assistant. Holds just the state + profile needed to resolve a model; the tools and
/// prompt are supplied per call so the desktop keeps ownership of the self-awareness context.
pub struct PlatformCopilot {
    state: Arc<FlowLikeState>,
    profile: Option<Arc<Profile>>,
}

impl PlatformCopilot {
    pub fn new(state: Arc<FlowLikeState>, profile: Option<Arc<Profile>>) -> Self {
        Self { state, profile }
    }

    async fn resolve_model<'a>(
        &self,
        model_id: Option<String>,
        token: Option<String>,
    ) -> Result<(String, Box<dyn CompletionClientDyn + Send + Sync + 'a>)> {
        let bit = if let Some(profile) = &self.profile {
            if let Some(id) = model_id {
                profile
                    .find_bit(&id, self.state.http_client.clone())
                    .await?
            } else {
                let preference = BitModelPreference {
                    reasoning_weight: Some(1.0),
                    ..Default::default()
                };
                profile
                    .get_best_model(&preference, false, true, self.state.http_client.clone())
                    .await?
            }
        } else {
            Bit {
                id: "gpt-4o".to_string(),
                bit_type: BitTypes::Llm,
                parameters: serde_json::to_value(LLMParameters {
                    context_length: 128000,
                    provider: ModelProvider {
                        provider_name: "openai".to_string(),
                        model_id: None,
                        version: None,
                        params: None,
                    },
                    model_classification: Default::default(),
                })
                .unwrap(),
                ..Default::default()
            }
        };

        let model_factory = self.state.model_factory.clone();
        let model = model_factory
            .lock()
            .await
            .build(&bit, self.state.clone(), token, None)
            .await?;
        let default_model = model.default_model().await.unwrap_or("gpt-4o".to_string());
        let provider = model.provider().await?;
        Ok((default_model, provider.into_client()))
    }

    /// Run the platform assistant loop for a profile model, dispatching platform tools through the
    /// supplied bridge and streaming tag frames via `on_token`. Returns the final assistant text.
    #[allow(clippy::too_many_arguments)]
    pub async fn chat<F>(
        &self,
        mut system_prompt: String,
        user_prompt: String,
        current_images: Option<Vec<ChatImage>>,
        history: Vec<ChatMessage>,
        model_id: Option<String>,
        token: Option<String>,
        bridge: Arc<dyn PlatformToolBridge>,
        memory: Option<Arc<AssistantMemory>>,
        on_token: Option<F>,
    ) -> Result<String>
    where
        F: Fn(String) + Send + Sync + 'static,
    {
        let (model_name, completion_client) = self.resolve_model(model_id, token).await?;

        // Memory: recall relevant context into the prompt + advertise the memory tools.
        if let Some(memory) = &memory {
            if let Ok(recalled) = memory.search(&user_prompt, 6).await
                && !recalled.is_empty()
            {
                system_prompt.push_str("\n\n## RELEVANT MEMORY\nFacts you remembered that may help:\n");
                for item in &recalled {
                    system_prompt.push_str(&format!("- {item}\n"));
                }
            }
            system_prompt.push_str(
                "\n\n## MEMORY\nYou have persistent, profile-scoped memory. Use `_memory_search` to recall facts and `_memory_store` to save important user facts, preferences, and decisions. Store salient facts immediately rather than only saying you will remember them.",
            );
        }

        let mut builder = completion_client
            .agent(&model_name)
            .preamble(&system_prompt)
            .tool(ListAppsTool)
            .tool(DescribeAppInterfaceTool)
            .tool(OpenAppChatTool)
            .tool(NavigateViewTool)
            .tool(CreateAppTool)
            .tool(FlowPilotBoardTool)
            .tool(AskUserTool)
            .tool(CallAppChatTool);
        if memory.is_some() {
            builder = builder.tool(MemoryStoreTool).tool(MemorySearchTool);
        }
        let agent = builder.build();

        let parse_media_type = |s: &str| -> Option<ImageMediaType> {
            match s.to_lowercase().as_str() {
                "image/jpeg" | "jpeg" | "jpg" => Some(ImageMediaType::JPEG),
                "image/png" | "png" => Some(ImageMediaType::PNG),
                "image/gif" | "gif" => Some(ImageMediaType::GIF),
                "image/webp" | "webp" => Some(ImageMediaType::WEBP),
                _ => None,
            }
        };

        let mut prompt_contents = vec![UserContent::Text(rig::message::Text {
            text: user_prompt.clone(),
            additional_params: None,
        })];
        if let Some(images) = &current_images {
            for img in images {
                prompt_contents.push(UserContent::Image(Image {
                    data: DocumentSourceKind::Base64(img.data.clone()),
                    media_type: parse_media_type(&img.media_type),
                    detail: Some(ImageDetail::Auto),
                    additional_params: None,
                }));
            }
        }
        let prompt_message = rig::message::Message::User {
            content: OneOrMany::many(prompt_contents).unwrap_or_else(|_| {
                OneOrMany::one(UserContent::Text(rig::message::Text {
                    text: user_prompt.clone(),
                    additional_params: None,
                }))
            }),
        };

        let mut current_history: Vec<rig::message::Message> = history
            .iter()
            .filter_map(|msg| match msg.role {
                ChatRole::User => {
                    let mut contents: Vec<UserContent> = vec![UserContent::Text(rig::message::Text {
                        text: msg.content.clone(),
                        additional_params: None,
                    })];
                    if let Some(images) = &msg.images {
                        for img in images {
                            contents.push(UserContent::Image(Image {
                                data: DocumentSourceKind::Base64(img.data.clone()),
                                media_type: parse_media_type(&img.media_type),
                                detail: Some(ImageDetail::Auto),
                                additional_params: None,
                            }));
                        }
                    }
                    OneOrMany::many(contents)
                        .ok()
                        .map(|content| rig::message::Message::User { content })
                }
                ChatRole::Assistant => Some(rig::message::Message::Assistant {
                    id: None,
                    content: OneOrMany::one(AssistantContent::Text(rig::message::Text {
                        text: msg.content.clone(),
                        additional_params: None,
                    })),
                }),
            })
            .collect();

        let max_iterations = 8u64;
        let mut plan_step_counter = 0u32;
        let mut full_response = String::new();
        let mut current_prompt = prompt_message;

        for _iteration in 0..max_iterations {
            let request = agent
                .completion(current_prompt.clone(), current_history.clone())
                .await
                .map_err(|e| flow_like_types::anyhow!("Completion error: {}", e))?;
            let mut stream = request
                .stream()
                .await
                .map_err(|e| flow_like_types::anyhow!("Stream error: {}", e))?;

            let mut response_contents: Vec<AssistantContent> = Vec::new();
            let mut iteration_text = String::new();
            let mut current_reasoning = String::new();
            let mut reasoning_step_id: Option<String> = None;

            while let Some(item) = stream.next().await {
                let content = item.map_err(|e| flow_like_types::anyhow!("Stream chunk error: {}", e))?;
                match content {
                    StreamedAssistantContent::Text(text) => {
                        iteration_text.push_str(&text.text);
                        if let Some(ref callback) = on_token {
                            callback(text.text.clone());
                        }
                        response_contents.push(AssistantContent::Text(text));
                    }
                    StreamedAssistantContent::ToolCall { tool_call, .. } => {
                        response_contents.push(AssistantContent::ToolCall(tool_call));
                    }
                    StreamedAssistantContent::ToolCallDelta { .. } => {}
                    StreamedAssistantContent::Reasoning(reasoning) => {
                        let reasoning_text = reasoning
                            .content
                            .iter()
                            .filter_map(|c| match c {
                                rig::message::ReasoningContent::Text { text, .. } => Some(text.as_str()),
                                rig::message::ReasoningContent::Summary(s) => Some(s.as_str()),
                                _ => None,
                            })
                            .collect::<Vec<_>>()
                            .join("\n");
                        current_reasoning.push_str(&reasoning_text);
                        current_reasoning.push('\n');
                        if let Some(ref callback) = on_token {
                            if reasoning_step_id.is_none() {
                                plan_step_counter += 1;
                                reasoning_step_id = Some(format!("reasoning_{}", plan_step_counter));
                            }
                            emit_plan_step(
                                callback,
                                reasoning_step_id.clone().unwrap(),
                                current_reasoning.trim().to_string(),
                                PlanStepStatus::InProgress,
                                "think",
                            );
                        }
                    }
                    StreamedAssistantContent::ReasoningDelta { reasoning, .. } => {
                        current_reasoning.push_str(&reasoning);
                        if let Some(ref callback) = on_token {
                            if reasoning_step_id.is_none() {
                                plan_step_counter += 1;
                                reasoning_step_id = Some(format!("reasoning_{}", plan_step_counter));
                            }
                            emit_plan_step(
                                callback,
                                reasoning_step_id.clone().unwrap(),
                                current_reasoning.trim().to_string(),
                                PlanStepStatus::InProgress,
                                "think",
                            );
                        }
                    }
                    StreamedAssistantContent::Final(_) => {
                        if let (Some(callback), Some(step_id)) = (&on_token, &reasoning_step_id) {
                            emit_plan_step(
                                callback,
                                step_id.clone(),
                                current_reasoning.trim().to_string(),
                                PlanStepStatus::Completed,
                                "think",
                            );
                        }
                        reasoning_step_id = None;
                        current_reasoning.clear();
                    }
                }
            }

            if let (Some(callback), Some(step_id)) = (&on_token, &reasoning_step_id) {
                emit_plan_step(
                    callback,
                    step_id.clone(),
                    current_reasoning.trim().to_string(),
                    PlanStepStatus::Completed,
                    "think",
                );
            }

            let tool_calls: Vec<_> = response_contents
                .iter()
                .filter_map(|content| match content {
                    AssistantContent::ToolCall(tool_call) => Some(tool_call.clone()),
                    _ => None,
                })
                .collect();

            if tool_calls.is_empty() {
                full_response.push_str(&iteration_text);
                break;
            }

            current_history.push(current_prompt.clone());

            let mut step_ids: Vec<String> = Vec::new();
            for tool_call in &tool_calls {
                plan_step_counter += 1;
                let step_id = format!("step_{}", plan_step_counter);
                if let Some(ref callback) = on_token {
                    callback(format!("tool_call:{}", tool_call.function.name));
                    emit_plan_step(
                        callback,
                        step_id.clone(),
                        format!("Running {}", tool_call.function.name),
                        PlanStepStatus::InProgress,
                        &tool_call.function.name,
                    );
                }
                step_ids.push(step_id);
            }

            let tool_futures: Vec<_> = tool_calls
                .iter()
                .map(|tool_call| {
                    let name = tool_call.function.name.clone();
                    let arguments = tool_call.function.arguments.clone();
                    let id = tool_call.id.clone();
                    let bridge = bridge.clone();
                    let memory = memory.clone();
                    async move {
                        let output =
                            execute_platform_tool(&name, arguments, &bridge, memory.as_ref()).await;
                        (id, name, output)
                    }
                })
                .collect();
            let tool_results: Vec<(String, String, String)> =
                futures::future::join_all(tool_futures).await;

            for (i, (_id, name, _output)) in tool_results.iter().enumerate() {
                if let Some(ref callback) = on_token {
                    if let Some(step_id) = step_ids.get(i) {
                        emit_plan_step(
                            callback,
                            step_id.clone(),
                            format!("Ran {}", name),
                            PlanStepStatus::Completed,
                            name,
                        );
                    }
                    callback("tool_result:done".to_string());
                }
            }

            let assistant_msg = rig::message::Message::Assistant {
                id: None,
                content: OneOrMany::many(response_contents.clone()).unwrap_or_else(|_| {
                    OneOrMany::one(AssistantContent::Text(rig::message::Text {
                        text: String::new(),
                        additional_params: None,
                    }))
                }),
            };
            current_history.push(assistant_msg);

            let tool_result_contents: Vec<UserContent> = tool_results
                .iter()
                .map(|(tool_id, _name, tool_output)| {
                    UserContent::ToolResult(RigToolResult {
                        id: tool_id.clone(),
                        call_id: None,
                        content: OneOrMany::one(ToolResultContent::text(tool_output.clone())),
                    })
                })
                .collect();
            let combined = if tool_result_contents.len() == 1 {
                OneOrMany::one(tool_result_contents.into_iter().next().unwrap())
            } else {
                OneOrMany::many(tool_result_contents)
                    .unwrap_or_else(|_| OneOrMany::one(UserContent::text("")))
            };
            current_prompt = rig::message::Message::User { content: combined };
        }

        Ok(full_response)
    }
}

/// Dispatch a tool call: memory tools run locally (embed + LanceDB), everything else goes to the
/// host bridge (frontend). Keeps the assistant's persistent memory server-side and out of the bridge.
async fn execute_platform_tool(
    name: &str,
    arguments: Value,
    bridge: &Arc<dyn PlatformToolBridge>,
    memory: Option<&Arc<AssistantMemory>>,
) -> String {
    match name {
        "_memory_store" => match memory {
            Some(memory) => {
                let content = arguments.get("content").and_then(Value::as_str).unwrap_or("");
                let role = arguments
                    .get("role")
                    .and_then(Value::as_str)
                    .unwrap_or("observation");
                match memory.store(role, content).await {
                    Ok(count) => json!({ "status": "ok", "observation_count": count }).to_string(),
                    Err(error) => {
                        json!({ "status": "error", "error": error.to_string() }).to_string()
                    }
                }
            }
            None => json!({ "status": "error", "error": "Memory is not enabled." }).to_string(),
        },
        "_memory_search" => match memory {
            Some(memory) => {
                let query = arguments.get("query").and_then(Value::as_str).unwrap_or("");
                match memory.search(query, 10).await {
                    Ok(results) => json!({ "status": "ok", "results": results }).to_string(),
                    Err(error) => {
                        json!({ "status": "error", "error": error.to_string() }).to_string()
                    }
                }
            }
            None => json!({ "status": "error", "error": "Memory is not enabled." }).to_string(),
        },
        _ => bridge.call(name, arguments).await,
    }
}

fn emit_plan_step<F>(
    callback: &F,
    id: String,
    description: String,
    status: PlanStepStatus,
    tool_name: &str,
) where
    F: Fn(String),
{
    let event = StreamEvent::PlanStep(PlanStep {
        id,
        description,
        status,
        tool_name: Some(tool_name.to_string()),
    });
    callback(format!(
        "<plan_step>{}</plan_step>",
        serde_json::to_string(&event).unwrap_or_default()
    ));
}
