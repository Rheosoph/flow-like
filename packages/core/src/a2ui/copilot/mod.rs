//! A2UI Copilot - AI-powered UI generation assistant
//!
//! This module provides the A2UICopilot struct which enables natural language
//! UI generation using the A2UI component system.
//!
//! Uses direct structured JSON output instead of tool calls for more robust
//! and efficient UI generation. The LLM outputs a JSON block containing
//! the complete component tree, which is parsed from the response text.

pub mod component_docs;
mod extract;
mod tools;
mod types;

pub use component_docs::{
    CHART_DOCUMENTATION, COMPONENT_CATALOG, GAME_COMPONENT_DOCUMENTATION, ML_VISION_DOCUMENTATION,
    STYLE_GUIDE, get_documentation_section, get_full_documentation,
};
pub use extract::{ExtractedSurface, extract_surface_from_response};
pub use tools::*;
pub use types::*;

use std::sync::Arc;

use flow_like_types::Result;
use futures::StreamExt;
use rig::{
    OneOrMany,
    client::completion::CompletionClientDyn,
    completion::Completion,
    message::{
        AssistantContent, DocumentSourceKind, Image, ImageDetail, ImageMediaType, UserContent,
    },
    streaming::StreamedAssistantContent,
    tools::ThinkTool,
};

use crate::a2ui::SurfaceComponent;
use crate::bit::{Bit, BitModelPreference, BitTypes, LLMParameters};
use crate::profile::Profile;
use crate::state::FlowLikeState;
use flow_like_model_provider::provider::ModelProvider;

/// The main A2UI Copilot struct that provides AI-powered UI generation
pub struct A2UICopilot {
    state: Arc<FlowLikeState>,
    profile: Option<Arc<Profile>>,
}

impl A2UICopilot {
    /// Create a new A2UICopilot
    pub async fn new(state: Arc<FlowLikeState>, profile: Option<Arc<Profile>>) -> Result<Self> {
        Ok(Self { state, profile })
    }

    /// Main entry point - generate or modify A2UI surfaces via structured output
    pub async fn chat<F>(
        &self,
        current_surface: Option<&Vec<SurfaceComponent>>,
        selected_component_ids: &[String],
        user_prompt: String,
        current_images: Option<Vec<A2UIChatImage>>,
        history: Vec<A2UIChatMessage>,
        model_id: Option<String>,
        token: Option<String>,
        on_token: Option<F>,
    ) -> Result<A2UICopilotResponse>
    where
        F: Fn(String) + Send + Sync + 'static,
    {
        let context = self.prepare_context(current_surface, selected_component_ids)?;
        let context_json = flow_like_types::json::to_string_pretty(&context)?;

        let (model_name, completion_client) = self.get_model(model_id, token).await?;

        let system_prompt = Self::build_system_prompt(&context_json);

        // Only ThinkTool for chain-of-thought reasoning — no UI-generation tools
        let agent = completion_client
            .agent(&model_name)
            .preamble(&system_prompt)
            .tool(ThinkTool)
            .build();

        let prompt = user_prompt.clone();

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
            text: prompt.clone(),
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
                    text: prompt.clone(),
                }))
            }),
        };

        // Convert chat history to rig message format
        let mut current_history: Vec<rig::message::Message> = history
            .iter()
            .filter_map(|msg| match msg.role {
                A2UIChatRole::User => {
                    let mut contents: Vec<UserContent> =
                        vec![UserContent::Text(rig::message::Text {
                            text: msg.content.clone(),
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

                    match OneOrMany::many(contents) {
                        Ok(content) => Some(rig::message::Message::User { content }),
                        Err(_) => None,
                    }
                }
                A2UIChatRole::Assistant => Some(rig::message::Message::Assistant {
                    id: None,
                    content: OneOrMany::one(AssistantContent::Text(rig::message::Text {
                        text: msg.content.clone(),
                    })),
                }),
            })
            .collect();

        let mut full_response = String::new();
        let mut plan_step_counter = 0u32;

        // Send analysis start event
        if let Some(ref callback) = on_token {
            plan_step_counter += 1;
            let step_event = A2UIStreamEvent::PlanStep(A2UIPlanStep {
                id: "generating_0".to_string(),
                description: "Generating UI...".to_string(),
                status: A2UIPlanStepStatus::InProgress,
                tool_name: Some("generate".to_string()),
            });
            callback(format!(
                "<plan_step>{}</plan_step>",
                serde_json::to_string(&step_event).unwrap_or_default()
            ));
        }

        // The model returns text with an embedded JSON block.
        // ThinkTool may cause multiple round-trips for reasoning (especially
        // with deeper-thinking models like Sonnet/Opus), so allow enough iterations.
        let max_iterations = 10u64;

        for _iteration in 0..max_iterations {
            let request = agent
                .completion(prompt_message.clone(), current_history.clone())
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
                let content =
                    item.map_err(|e| flow_like_types::anyhow!("Stream chunk error: {}", e))?;

                match content {
                    StreamedAssistantContent::Text(text) => {
                        iteration_text.push_str(&text.text);
                        if let Some(ref callback) = on_token {
                            callback(text.text.clone());
                        }
                        response_contents.push(AssistantContent::Text(text));
                    }
                    StreamedAssistantContent::ToolCall { tool_call, .. } => {
                        // Only ThinkTool calls are expected here
                        response_contents.push(AssistantContent::ToolCall(tool_call));
                    }
                    StreamedAssistantContent::ToolCallDelta { .. } => {}
                    StreamedAssistantContent::Reasoning(reasoning) => {
                        let reasoning_text = reasoning
                            .content
                            .iter()
                            .filter_map(|c| match c {
                                rig::message::ReasoningContent::Text { text, .. } => {
                                    Some(text.as_str())
                                }
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
                                reasoning_step_id =
                                    Some(format!("reasoning_{}", plan_step_counter));
                            }

                            let step_event = A2UIStreamEvent::PlanStep(A2UIPlanStep {
                                id: reasoning_step_id.clone().unwrap(),
                                description: current_reasoning.trim().to_string(),
                                status: A2UIPlanStepStatus::InProgress,
                                tool_name: Some("think".to_string()),
                            });
                            callback(format!(
                                "<plan_step>{}</plan_step>",
                                serde_json::to_string(&step_event).unwrap_or_default()
                            ));
                        }
                    }
                    StreamedAssistantContent::Final(_) => {
                        if let (Some(callback), Some(step_id)) = (&on_token, &reasoning_step_id) {
                            let step_event = A2UIStreamEvent::PlanStep(A2UIPlanStep {
                                id: step_id.clone(),
                                description: current_reasoning.trim().to_string(),
                                status: A2UIPlanStepStatus::Completed,
                                tool_name: Some("think".to_string()),
                            });
                            callback(format!(
                                "<plan_step>{}</plan_step>",
                                serde_json::to_string(&step_event).unwrap_or_default()
                            ));
                        }
                        reasoning_step_id = None;
                        current_reasoning.clear();
                    }
                    StreamedAssistantContent::ReasoningDelta { reasoning, .. } => {
                        current_reasoning.push_str(&reasoning);
                        if let Some(ref callback) = on_token {
                            if reasoning_step_id.is_none() {
                                plan_step_counter += 1;
                                reasoning_step_id =
                                    Some(format!("reasoning_{}", plan_step_counter));
                            }

                            let step_event = A2UIStreamEvent::PlanStep(A2UIPlanStep {
                                id: reasoning_step_id.clone().unwrap(),
                                description: current_reasoning.trim().to_string(),
                                status: A2UIPlanStepStatus::InProgress,
                                tool_name: Some("think".to_string()),
                            });
                            callback(format!(
                                "<plan_step>{}</plan_step>",
                                serde_json::to_string(&step_event).unwrap_or_default()
                            ));
                        }
                    }
                }
            }

            // Complete any remaining reasoning step
            if let (Some(callback), Some(step_id)) = (&on_token, &reasoning_step_id) {
                let step_event = A2UIStreamEvent::PlanStep(A2UIPlanStep {
                    id: step_id.clone(),
                    description: current_reasoning.trim().to_string(),
                    status: A2UIPlanStepStatus::Completed,
                    tool_name: Some("think".to_string()),
                });
                callback(format!(
                    "<plan_step>{}</plan_step>",
                    serde_json::to_string(&step_event).unwrap_or_default()
                ));
            }

            full_response.push_str(&iteration_text);

            // Check if there were tool calls (ThinkTool) that need another round
            let has_tool_calls = response_contents
                .iter()
                .any(|c| matches!(c, AssistantContent::ToolCall(_)));

            if !has_tool_calls {
                break;
            }

            // If iteration text already contains extractable components,
            // stop early — no need to burn more iterations.
            if !iteration_text.is_empty()
                && !extract_surface_from_response(&full_response)
                    .components
                    .is_empty()
            {
                break;
            }

            // Add assistant response to history for next iteration
            let assistant_content =
                OneOrMany::many(response_contents.clone()).unwrap_or_else(|_| {
                    OneOrMany::one(AssistantContent::Text(rig::message::Text {
                        text: iteration_text.clone(),
                    }))
                });

            current_history.push(rig::message::Message::Assistant {
                id: None,
                content: assistant_content,
            });

            // For ThinkTool calls, execute them and add results to history
            let tool_calls: Vec<_> = response_contents
                .iter()
                .filter_map(|c| {
                    if let AssistantContent::ToolCall(tc) = c {
                        Some(tc.clone())
                    } else {
                        None
                    }
                })
                .collect();

            let mut tool_result_contents: Vec<UserContent> = Vec::new();
            for tool_call in &tool_calls {
                let result = if tool_call.function.name == "think" {
                    "Continue.".to_string()
                } else {
                    format!("Unknown tool: {}", tool_call.function.name)
                };

                tool_result_contents.push(UserContent::ToolResult(rig::message::ToolResult {
                    id: tool_call.id.clone(),
                    call_id: None,
                    content: OneOrMany::one(rig::message::ToolResultContent::Text(
                        rig::message::Text { text: result },
                    )),
                }));
            }

            if !tool_result_contents.is_empty() {
                let combined = if tool_result_contents.len() == 1 {
                    OneOrMany::one(tool_result_contents.into_iter().next().unwrap())
                } else {
                    OneOrMany::many(tool_result_contents)
                        .expect("tool_result_contents should have at least 2 elements")
                };
                current_history.push(rig::message::Message::User { content: combined });
            }
        }

        // Parse the components from the response JSON block
        let surface = extract_surface_from_response(&full_response);
        let generated_components = surface.components;
        let canvas_settings = surface.canvas_settings;
        let root_component_id = surface.root_component_id;

        let component_count = generated_components.len();

        // Mark generation as complete
        if let Some(ref callback) = on_token {
            let step_event = A2UIStreamEvent::PlanStep(A2UIPlanStep {
                id: "generating_0".to_string(),
                description: if component_count > 0 {
                    format!("Generated {} components", component_count)
                } else {
                    "Response complete".to_string()
                },
                status: A2UIPlanStepStatus::Completed,
                tool_name: Some("generate".to_string()),
            });
            callback(format!(
                "<plan_step>{}</plan_step>",
                serde_json::to_string(&step_event).unwrap_or_default()
            ));
        }

        Ok(A2UICopilotResponse {
            message: full_response,
            components: generated_components,
            canvas_settings,
            root_component_id,
            suggestions: vec![],
        })
    }

    fn prepare_context(
        &self,
        current_surface: Option<&Vec<SurfaceComponent>>,
        selected_component_ids: &[String],
    ) -> Result<A2UIContext> {
        let components = current_surface
            .map(|s| {
                s.iter()
                    .map(|c| ComponentContext {
                        id: c.id.clone(),
                        component_type: c.get_component_type_name(),
                        style_classes: c.style.as_ref().and_then(|s| s.class_name.clone()),
                        is_selected: selected_component_ids.contains(&c.id),
                    })
                    .collect()
            })
            .unwrap_or_default();

        Ok(A2UIContext {
            components,
            selected_ids: selected_component_ids.to_vec(),
            component_count: current_surface.map(|s| s.len()).unwrap_or(0),
        })
    }

    async fn get_model<'a>(
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
        let completion = provider.into_client();

        Ok((default_model, completion))
    }

    fn build_system_prompt(context_json: &str) -> String {
        let component_docs = get_full_documentation();
        crate::copilot::prompts::frontend_system_prompt(context_json, &component_docs)
    }
}
