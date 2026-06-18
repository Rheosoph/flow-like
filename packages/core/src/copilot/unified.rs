//! Unified Copilot - Delegates to the appropriate existing copilot implementations
//!
//! This module provides a unified `UnifiedCopilot` struct that delegates to either
//! the flow `Copilot` or A2UI `A2UICopilot` based on the requested scope.

use std::sync::Arc;

use flow_like_types::Result;

use crate::a2ui::SurfaceComponent;
use crate::a2ui::copilot::A2UICopilot;
use crate::flow::board::Board;
use crate::flow::copilot::{CatalogProvider, Copilot, RunContext};
use crate::profile::Profile;
use crate::state::FlowLikeState;

use super::types::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CombinedScopeTarget {
    Board,
    Frontend,
    Both,
}

#[derive(Debug, Clone)]
struct CombinedScopePlan {
    target: CombinedScopeTarget,
    primary_scope: CopilotScope,
    rationale: String,
}

/// The unified copilot that delegates to appropriate implementations
pub struct UnifiedCopilot {
    state: Arc<FlowLikeState>,
    catalog_provider: Option<Arc<dyn CatalogProvider>>,
    profile: Option<Arc<Profile>>,
    current_template_id: Option<String>,
}

impl UnifiedCopilot {
    /// Create a new UnifiedCopilot
    pub async fn new(
        state: Arc<FlowLikeState>,
        catalog_provider: Option<Arc<dyn CatalogProvider>>,
        profile: Option<Arc<Profile>>,
        current_template_id: Option<String>,
    ) -> Result<Self> {
        Ok(Self {
            state,
            catalog_provider,
            profile,
            current_template_id,
        })
    }

    /// Main entry point - unified chat that can handle board, UI, or both
    pub async fn chat<F>(
        &self,
        scope: CopilotScope,
        // Board context (optional for Frontend scope)
        board: Option<&Board>,
        selected_node_ids: &[String],
        // UI context (optional for Board scope)
        current_surface: Option<&Vec<SurfaceComponent>>,
        selected_component_ids: &[String],
        // Common parameters
        user_prompt: String,
        current_images: Option<Vec<ChatImage>>,
        history: Vec<UnifiedChatMessage>,
        model_id: Option<String>,
        token: Option<String>,
        context: Option<UnifiedContext>,
        on_token: Option<F>,
    ) -> Result<UnifiedCopilotResponse>
    where
        F: Fn(String) + Send + Sync + 'static + Clone,
    {
        // Determine effective scope based on available data
        let effective_scope = self.determine_effective_scope(scope, board, current_surface);

        // Send scope decision event
        if let Some(ref callback) = on_token {
            let event = UnifiedStreamEvent::ScopeDecision(effective_scope);
            callback(format!(
                "<scope_decision>{}</scope_decision>",
                serde_json::to_string(&event).unwrap_or_default()
            ));
        }

        match effective_scope {
            CopilotScope::Board => {
                self.delegate_to_board(
                    board.ok_or_else(|| {
                        flow_like_types::anyhow!("Board is required for Board scope")
                    })?,
                    selected_node_ids,
                    user_prompt,
                    current_images,
                    history,
                    model_id,
                    token,
                    context.and_then(|c| c.run_context),
                    on_token,
                )
                .await
            }
            CopilotScope::Frontend => {
                self.delegate_to_frontend(
                    current_surface,
                    selected_component_ids,
                    user_prompt,
                    current_images,
                    history,
                    model_id,
                    token,
                    context.and_then(|c| c.action_context),
                    on_token,
                )
                .await
            }
            CopilotScope::Both => {
                // For Both scope, we run both copilots and merge results
                self.run_both(
                    board,
                    selected_node_ids,
                    current_surface,
                    selected_component_ids,
                    user_prompt,
                    current_images,
                    history,
                    model_id,
                    token,
                    context,
                    on_token,
                )
                .await
            }
        }
    }

    /// Determine the effective scope based on available data
    fn determine_effective_scope(
        &self,
        requested_scope: CopilotScope,
        board: Option<&Board>,
        current_surface: Option<&Vec<SurfaceComponent>>,
    ) -> CopilotScope {
        match requested_scope {
            CopilotScope::Board => {
                if board.is_some() && self.catalog_provider.is_some() {
                    CopilotScope::Board
                } else {
                    CopilotScope::Frontend
                }
            }
            CopilotScope::Frontend => CopilotScope::Frontend,
            CopilotScope::Both => {
                if board.is_some() && self.catalog_provider.is_some() {
                    CopilotScope::Both
                } else if current_surface.is_some() || board.is_none() {
                    CopilotScope::Frontend
                } else {
                    CopilotScope::Board
                }
            }
        }
    }

    /// Delegate to the flow Copilot for board operations
    async fn delegate_to_board<F>(
        &self,
        board: &Board,
        selected_node_ids: &[String],
        user_prompt: String,
        current_images: Option<Vec<ChatImage>>,
        history: Vec<UnifiedChatMessage>,
        model_id: Option<String>,
        token: Option<String>,
        run_context: Option<RunContext>,
        on_token: Option<F>,
    ) -> Result<UnifiedCopilotResponse>
    where
        F: Fn(String) + Send + Sync + 'static,
    {
        let catalog_provider = self
            .catalog_provider
            .as_ref()
            .ok_or_else(|| flow_like_types::anyhow!("Catalog provider required for Board mode"))?;

        let copilot = Copilot::new(
            self.state.clone(),
            catalog_provider.clone(),
            self.profile.clone(),
            self.current_template_id.clone(),
        )
        .await?;

        // Convert history to flow ChatMessage format
        let board_history = history
            .into_iter()
            .map(|m| crate::flow::copilot::ChatMessage {
                role: m.role,
                content: m.content,
                images: m.images,
            })
            .collect();

        let response = copilot
            .chat(
                board,
                selected_node_ids,
                user_prompt,
                current_images,
                board_history,
                model_id,
                token,
                run_context,
                on_token,
            )
            .await?;

        // Convert response
        Ok(UnifiedCopilotResponse {
            message: response.message,
            commands: response.commands,
            components: vec![],
            suggestions: response
                .suggestions
                .into_iter()
                .map(|s| UnifiedSuggestion {
                    label: s.node_type,
                    prompt: s.reason,
                    scope: Some(CopilotScope::Board),
                })
                .collect(),
            active_scope: CopilotScope::Board,
            canvas_settings: None,
            root_component_id: None,
            flowscript_workspace: response.flowscript_workspace,
        })
    }

    /// Delegate to the A2UI Copilot for frontend operations
    async fn delegate_to_frontend<F>(
        &self,
        current_surface: Option<&Vec<SurfaceComponent>>,
        selected_component_ids: &[String],
        user_prompt: String,
        current_images: Option<Vec<ChatImage>>,
        history: Vec<UnifiedChatMessage>,
        model_id: Option<String>,
        token: Option<String>,
        _action_context: Option<UIActionContext>,
        on_token: Option<F>,
    ) -> Result<UnifiedCopilotResponse>
    where
        F: Fn(String) + Send + Sync + 'static,
    {
        let copilot = A2UICopilot::new(self.state.clone(), self.profile.clone()).await?;

        // Convert history
        let ui_history = history
            .into_iter()
            .map(|m| crate::a2ui::copilot::A2UIChatMessage {
                role: match m.role {
                    ChatRole::User => crate::a2ui::copilot::A2UIChatRole::User,
                    ChatRole::Assistant => crate::a2ui::copilot::A2UIChatRole::Assistant,
                },
                content: m.content,
                images: m.images.map(|imgs| {
                    imgs.into_iter()
                        .map(|img| crate::a2ui::copilot::A2UIChatImage {
                            data: img.data,
                            media_type: img.media_type,
                        })
                        .collect()
                }),
            })
            .collect();

        // Note: A2UICopilot doesn't support action_context yet, so we ignore it
        let response = copilot
            .chat(
                current_surface,
                selected_component_ids,
                user_prompt,
                current_images.map(|imgs| {
                    imgs.into_iter()
                        .map(|img| crate::a2ui::copilot::A2UIChatImage {
                            data: img.data,
                            media_type: img.media_type,
                        })
                        .collect()
                }),
                ui_history,
                model_id,
                token,
                on_token,
            )
            .await?;

        // Convert response
        Ok(UnifiedCopilotResponse {
            message: response.message,
            commands: vec![],
            components: response.components,
            suggestions: vec![],
            active_scope: CopilotScope::Frontend,
            canvas_settings: response.canvas_settings,
            root_component_id: response.root_component_id,
            flowscript_workspace: None,
        })
    }

    /// Run both copilots for unified mode
    async fn run_both<F>(
        &self,
        board: Option<&Board>,
        selected_node_ids: &[String],
        current_surface: Option<&Vec<SurfaceComponent>>,
        selected_component_ids: &[String],
        user_prompt: String,
        current_images: Option<Vec<ChatImage>>,
        history: Vec<UnifiedChatMessage>,
        model_id: Option<String>,
        token: Option<String>,
        context: Option<UnifiedContext>,
        on_token: Option<F>,
    ) -> Result<UnifiedCopilotResponse>
    where
        F: Fn(String) + Send + Sync + 'static + Clone,
    {
        let plan = self.plan_combined_scope(&user_prompt, board, current_surface);

        if let Some(ref callback) = on_token {
            let event = UnifiedStreamEvent::Thinking(plan.rationale.clone());
            callback(format!(
                "<thinking>{}</thinking>",
                serde_json::to_string(&event).unwrap_or_default()
            ));
        }

        match plan.target {
            CombinedScopeTarget::Board => {
                let board = board.ok_or_else(|| {
                    flow_like_types::anyhow!("Board is required for combined workflow requests")
                })?;

                self.delegate_to_board(
                    board,
                    selected_node_ids,
                    user_prompt,
                    current_images,
                    history,
                    model_id,
                    token,
                    context.and_then(|c| c.run_context),
                    on_token,
                )
                .await
            }
            CombinedScopeTarget::Frontend => {
                self.delegate_to_frontend(
                    current_surface,
                    selected_component_ids,
                    user_prompt,
                    current_images,
                    history,
                    model_id,
                    token,
                    context.and_then(|c| c.action_context),
                    on_token,
                )
                .await
            }
            CombinedScopeTarget::Both => {
                let primary_is_board = plan.primary_scope == CopilotScope::Board;
                let primary_response = if primary_is_board {
                    let board = board.ok_or_else(|| {
                        flow_like_types::anyhow!("Board is required for combined workflow requests")
                    })?;

                    self.delegate_to_board(
                        board,
                        selected_node_ids,
                        user_prompt.clone(),
                        current_images.clone(),
                        history.clone(),
                        model_id.clone(),
                        token.clone(),
                        context.clone().and_then(|c| c.run_context),
                        on_token.clone(),
                    )
                    .await?
                } else {
                    self.delegate_to_frontend(
                        current_surface,
                        selected_component_ids,
                        user_prompt.clone(),
                        current_images.clone(),
                        history.clone(),
                        model_id.clone(),
                        token.clone(),
                        context.clone().and_then(|c| c.action_context),
                        on_token.clone(),
                    )
                    .await?
                };

                let secondary_response = if primary_is_board {
                    self.delegate_to_frontend(
                        current_surface,
                        selected_component_ids,
                        user_prompt,
                        current_images,
                        history,
                        model_id,
                        token,
                        context.and_then(|c| c.action_context),
                        on_token,
                    )
                    .await?
                } else {
                    let board = board.ok_or_else(|| {
                        flow_like_types::anyhow!("Board is required for combined workflow requests")
                    })?;

                    self.delegate_to_board(
                        board,
                        selected_node_ids,
                        user_prompt,
                        current_images,
                        history,
                        model_id,
                        token,
                        context.and_then(|c| c.run_context),
                        on_token,
                    )
                    .await?
                };

                Ok(Self::merge_combined_responses(
                    primary_response,
                    secondary_response,
                ))
            }
        }
    }

    fn plan_combined_scope(
        &self,
        user_prompt: &str,
        board: Option<&Board>,
        current_surface: Option<&Vec<SurfaceComponent>>,
    ) -> CombinedScopePlan {
        if board.is_none() || self.catalog_provider.is_none() {
            return CombinedScopePlan {
                target: CombinedScopeTarget::Frontend,
                primary_scope: CopilotScope::Frontend,
                rationale: "Only UI context is available, so FlowPilot will stay in frontend mode."
                    .to_string(),
            };
        }

        let prompt = user_prompt.to_lowercase();
        let board_score = Self::keyword_hits(
            &prompt,
            &[
                "workflow",
                "automation",
                "node",
                "nodes",
                "connect",
                "connection",
                "flow",
                "graph",
                "trigger",
                "schedule",
                "email",
                "webhook",
                "pin",
                "variable",
                "pipeline",
                "catalog",
            ],
        );
        let ui_score = Self::keyword_hits(
            &prompt,
            &[
                "ui",
                "frontend",
                "page",
                "screen",
                "component",
                "layout",
                "button",
                "form",
                "table",
                "card",
                "modal",
                "dialog",
                "dashboard",
                "style",
                "theme",
                "chart",
                "list",
            ],
        );
        let integration_score = Self::keyword_hits(
            &prompt,
            &[
                "trigger workflow",
                "workflow status",
                "show status",
                "display result",
                "button click",
                "on click",
                "on submit",
                "submit form",
                "wire up",
                "hook up",
                "connect the ui",
                "frontend and workflow",
                "page and workflow",
                "dashboard for",
            ],
        );

        if integration_score > 0 || (board_score > 0 && ui_score > 0) {
            let primary_scope = if board_score >= ui_score {
                CopilotScope::Board
            } else {
                CopilotScope::Frontend
            };
            let rationale = if integration_score > 0 {
                "The request links workflow behavior with UI behavior, so FlowPilot will plan both scopes."
            } else {
                "The request mixes workflow and UI vocabulary, so FlowPilot will split the work across both scopes."
            };

            return CombinedScopePlan {
                target: CombinedScopeTarget::Both,
                primary_scope,
                rationale: rationale.to_string(),
            };
        }

        if ui_score > board_score && current_surface.is_some() {
            return CombinedScopePlan {
                target: CombinedScopeTarget::Frontend,
                primary_scope: CopilotScope::Frontend,
                rationale: "The request is primarily about UI structure or styling, so FlowPilot will stay in frontend mode."
                    .to_string(),
            };
        }

        if ui_score > board_score {
            return CombinedScopePlan {
                target: CombinedScopeTarget::Frontend,
                primary_scope: CopilotScope::Frontend,
                rationale: "The request is primarily about UI generation, so FlowPilot will stay in frontend mode."
                    .to_string(),
            };
        }

        CombinedScopePlan {
            target: CombinedScopeTarget::Board,
            primary_scope: CopilotScope::Board,
            rationale: "The request is primarily about workflow planning or graph edits, so FlowPilot will stay in board mode."
                .to_string(),
        }
    }

    fn keyword_hits(prompt: &str, keywords: &[&str]) -> usize {
        keywords
            .iter()
            .filter(|keyword| prompt.contains(**keyword))
            .count()
    }

    fn merge_combined_responses(
        primary: UnifiedCopilotResponse,
        secondary: UnifiedCopilotResponse,
    ) -> UnifiedCopilotResponse {
        let mut message_parts = Vec::new();

        if !primary.message.trim().is_empty() {
            message_parts.push(format!(
                "{}: {}",
                Self::scope_label(primary.active_scope),
                primary.message.trim()
            ));
        }

        if !secondary.message.trim().is_empty() {
            message_parts.push(format!(
                "{}: {}",
                Self::scope_label(secondary.active_scope),
                secondary.message.trim()
            ));
        }

        let mut commands = primary.commands;
        commands.extend(secondary.commands);

        let mut components = primary.components;
        components.extend(secondary.components);

        let mut suggestions = primary.suggestions;
        suggestions.extend(secondary.suggestions);
        let flowscript_workspace = primary
            .flowscript_workspace
            .or(secondary.flowscript_workspace);

        UnifiedCopilotResponse {
            message: message_parts.join("\n\n"),
            commands,
            components,
            suggestions,
            active_scope: CopilotScope::Both,
            canvas_settings: secondary.canvas_settings.or(primary.canvas_settings),
            root_component_id: secondary.root_component_id.or(primary.root_component_id),
            flowscript_workspace,
        }
    }

    fn scope_label(scope: CopilotScope) -> &'static str {
        match scope {
            CopilotScope::Board => "Workflow",
            CopilotScope::Frontend => "UI",
            CopilotScope::Both => "FlowPilot",
        }
    }
}
