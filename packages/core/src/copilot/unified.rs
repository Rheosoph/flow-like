//! Unified Copilot - Delegates to the appropriate existing copilot implementations
//!
//! This module provides a unified `UnifiedCopilot` struct that delegates to either
//! the flow `Copilot` or A2UI `A2UICopilot` based on the requested scope.

use std::sync::Arc;

use flow_like_types::Result;

use crate::a2ui::SurfaceComponent;
use crate::a2ui::copilot::A2UICopilot;
use crate::flow::board::Board;
use crate::flow::copilot::platform::PlatformToolBridge;
use crate::flow::copilot::{
    CatalogProvider, Copilot, FlowIrDraftMutationHook, FlowIrDraftStore, RunContext,
    WorkflowSessionSnapshotSink,
};
use crate::models::llm::ModelUsageContext;
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

/// A board-primary combined run must not strand its typed claim when the UI-secondary run fails.
/// The token is transferred back into the merged response only after every requested scope has
/// completed successfully.
struct CombinedScopeFlowIrClaim {
    store: Arc<FlowIrDraftStore>,
    token: Option<FlowIrCommitToken>,
}

impl CombinedScopeFlowIrClaim {
    fn new(store: Arc<FlowIrDraftStore>, token: FlowIrCommitToken) -> Self {
        Self {
            store,
            token: Some(token),
        }
    }

    fn transfer(mut self) -> FlowIrCommitToken {
        self.token
            .take()
            .expect("combined-scope typed claim transfers at most once")
    }
}

impl Drop for CombinedScopeFlowIrClaim {
    fn drop(&mut self) {
        let Some(token) = self.token.take() else {
            return;
        };
        self.store.release_commit_if_matches(
            &token.draft_id,
            token.revision,
            &token.base_fingerprint,
            &token.claim_id,
        );
    }
}

/// The unified copilot that delegates to appropriate implementations
pub struct UnifiedCopilot {
    state: Arc<FlowLikeState>,
    catalog_provider: Option<Arc<dyn CatalogProvider>>,
    profile: Option<Arc<Profile>>,
    current_template_id: Option<String>,
    usage_context: Option<ModelUsageContext>,
    runtime_bridge: Option<Arc<dyn PlatformToolBridge>>,
    flow_ir_drafts: Arc<FlowIrDraftStore>,
    typed_flow_ir_lifecycle: bool,
    request_identity_prompt: Option<String>,
    board_context_augmentation: Option<serde_json::Value>,
    read_only: bool,
    workflow_session_snapshot_sink: Option<WorkflowSessionSnapshotSink>,
    flow_ir_draft_mutation_hook: Option<FlowIrDraftMutationHook>,
}

impl UnifiedCopilot {
    /// Create a new UnifiedCopilot
    pub async fn new(
        state: Arc<FlowLikeState>,
        catalog_provider: Option<Arc<dyn CatalogProvider>>,
        profile: Option<Arc<Profile>>,
        current_template_id: Option<String>,
        usage_context: Option<ModelUsageContext>,
    ) -> Result<Self> {
        Ok(Self {
            state,
            catalog_provider,
            profile,
            current_template_id,
            usage_context,
            runtime_bridge: None,
            flow_ir_drafts: Arc::new(FlowIrDraftStore::new()),
            typed_flow_ir_lifecycle: false,
            request_identity_prompt: None,
            board_context_augmentation: None,
            read_only: false,
            workflow_session_snapshot_sink: None,
            flow_ir_draft_mutation_hook: None,
        })
    }

    /// Bind retained-draft and acceptance-contract identity to a host-derived request identity
    /// (e.g. conversation id + immutable source prompt) instead of the raw prompt text alone.
    /// When unset, identity falls back to `raw_user_prompt`/`user_prompt` so existing callers
    /// keep their behavior; `raw_user_prompt` continues to drive routing and edit classification
    /// either way.
    pub fn with_request_identity_prompt(mut self, prompt: Option<String>) -> Self {
        self.request_identity_prompt = prompt.filter(|prompt| !prompt.trim().is_empty());
        self
    }

    /// Attach the immutable frontend-owned DB/UI/storage inventory used by every board model
    /// backend. The board delegate folds it into the provider-neutral authoring manifest.
    pub fn with_board_context_augmentation(
        mut self,
        augmentation: Option<serde_json::Value>,
    ) -> Self {
        self.board_context_augmentation = augmentation;
        self
    }

    pub fn with_read_only(mut self, read_only: bool) -> Self {
        self.read_only = read_only;
        self
    }

    /// Forward provider-neutral board lifecycle snapshots to a host-owned run-summary sink.
    pub fn with_workflow_session_snapshot_sink(
        mut self,
        sink: WorkflowSessionSnapshotSink,
    ) -> Self {
        self.workflow_session_snapshot_sink = Some(sink);
        self
    }

    /// Forward retained FlowScript lifecycle mutations to the host's crash-snapshot scheduler.
    pub fn with_flow_ir_draft_mutation_hook(mut self, hook: FlowIrDraftMutationHook) -> Self {
        self.flow_ir_draft_mutation_hook = Some(hook);
        self
    }

    /// Attach the host runtime bridge used by board-scoped profile/Bits models to execute
    /// persisted Events/nodes and query their logs. Server callers may omit it when no interactive
    /// execution host is available.
    pub fn with_runtime_bridge(mut self, bridge: Arc<dyn PlatformToolBridge>) -> Self {
        self.runtime_bridge = Some(bridge);
        self
    }

    /// Reuse a host-owned, board-scoped legacy draft store across chat invocations. Attaching the
    /// store does not advertise or enable the typed model-facing lifecycle: FlowScript remains the
    /// workflow authoring surface. The retained store only lets hosts resolve any outstanding
    /// review tokens created by older sessions.
    pub fn with_flow_ir_draft_store(mut self, store: Arc<FlowIrDraftStore>) -> Self {
        self.flow_ir_drafts = store;
        self
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
        self.chat_with_raw_user_prompt(
            scope,
            board,
            selected_node_ids,
            current_surface,
            selected_component_ids,
            user_prompt,
            None,
            current_images,
            history,
            model_id,
            token,
            context,
            on_token,
        )
        .await
    }

    /// Unified chat with an immutable copy of the user-authored request. Hosts may put execution
    /// context or mode guidance in `user_prompt`; routing, mutation classification, and typed-flow
    /// acceptance binding use `raw_user_prompt` exclusively when it is present.
    #[allow(clippy::too_many_arguments)]
    pub async fn chat_with_raw_user_prompt<F>(
        &self,
        scope: CopilotScope,
        board: Option<&Board>,
        selected_node_ids: &[String],
        current_surface: Option<&Vec<SurfaceComponent>>,
        selected_component_ids: &[String],
        user_prompt: String,
        raw_user_prompt: Option<String>,
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
        let raw_user_prompt = raw_user_prompt
            .filter(|prompt| !prompt.trim().is_empty())
            .unwrap_or_else(|| user_prompt.clone());

        // Determine effective scope based on available data
        let effective_scope = if self.read_only {
            if board.is_some() && self.catalog_provider.is_some() {
                CopilotScope::Board
            } else {
                return Err(flow_like_types::anyhow!(
                    "Read-only copilot mode requires board context; frontend authoring is disabled."
                ));
            }
        } else {
            self.determine_effective_scope(scope, board, current_surface)
        };

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
                    raw_user_prompt,
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
                    raw_user_prompt,
                    current_images,
                    history,
                    model_id,
                    token,
                    context,
                    on_token,
                )
                .await
            }
            // Data Studio is a tool-driven data agent handled by the platform tool loop
            // (desktop `copilot_chat`), not by the specialized board/UI copilots.
            CopilotScope::DataStudio => Err(flow_like_types::anyhow!(
                "Data Studio scope is served by the platform tool loop, not the UnifiedCopilot"
            )),
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
            CopilotScope::DataStudio => CopilotScope::DataStudio,
        }
    }

    /// Delegate to the flow Copilot for board operations
    async fn delegate_to_board<F>(
        &self,
        board: &Board,
        selected_node_ids: &[String],
        user_prompt: String,
        raw_user_prompt: String,
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

        let mut copilot = Copilot::new(
            self.state.clone(),
            catalog_provider.clone(),
            self.profile.clone(),
            self.current_template_id.clone(),
            self.usage_context.clone(),
        )
        .await?;
        if let Some(bridge) = &self.runtime_bridge {
            copilot = copilot.with_runtime_bridge(bridge.clone());
        }
        copilot = copilot.with_flow_ir_draft_store(self.flow_ir_drafts.clone());
        copilot = copilot.with_typed_flow_ir_enabled(self.typed_flow_ir_lifecycle);
        copilot = copilot.with_raw_user_prompt(Some(raw_user_prompt));
        copilot = copilot.with_request_identity_prompt(self.request_identity_prompt.clone());
        copilot = copilot.with_board_context_augmentation(self.board_context_augmentation.clone());
        copilot = copilot.with_read_only(self.read_only);
        if let Some(sink) = self.workflow_session_snapshot_sink.as_ref() {
            copilot = copilot.with_workflow_session_snapshot_sink(sink.clone());
        }
        if let Some(hook) = self.flow_ir_draft_mutation_hook.as_ref() {
            copilot = copilot.with_flow_ir_draft_mutation_hook(hook.clone());
        }

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
            flow_ir_commit: response.flow_ir_commit,
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
        let copilot = A2UICopilot::new(
            self.state.clone(),
            self.profile.clone(),
            self.usage_context.clone(),
        )
        .await?;

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
            flow_ir_commit: None,
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
        raw_user_prompt: String,
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
        let plan = self.plan_combined_scope(&raw_user_prompt, board, current_surface);

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
                    raw_user_prompt,
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
                let mut primary_response = if primary_is_board {
                    let board = board.ok_or_else(|| {
                        flow_like_types::anyhow!("Board is required for combined workflow requests")
                    })?;

                    self.delegate_to_board(
                        board,
                        selected_node_ids,
                        user_prompt.clone(),
                        raw_user_prompt.clone(),
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

                let primary_claim = primary_response
                    .flow_ir_commit
                    .take()
                    .map(|token| CombinedScopeFlowIrClaim::new(self.flow_ir_drafts.clone(), token));

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
                        raw_user_prompt,
                        current_images,
                        history,
                        model_id,
                        token,
                        context.and_then(|c| c.run_context),
                        on_token,
                    )
                    .await?
                };

                if let Some(claim) = primary_claim {
                    primary_response.flow_ir_commit = Some(claim.transfer());
                }

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
        Self::plan_combined_scope_with_availability(
            user_prompt,
            board.is_some(),
            self.catalog_provider.is_some(),
            current_surface.is_some(),
        )
    }

    fn plan_combined_scope_with_availability(
        user_prompt: &str,
        board_available: bool,
        catalog_available: bool,
        current_surface_available: bool,
    ) -> CombinedScopePlan {
        if !board_available || !catalog_available {
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

        if ui_score > board_score && current_surface_available {
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
        let flow_ir_commit = primary.flow_ir_commit.or(secondary.flow_ir_commit);

        UnifiedCopilotResponse {
            message: message_parts.join("\n\n"),
            commands,
            components,
            suggestions,
            active_scope: CopilotScope::Both,
            canvas_settings: secondary.canvas_settings.or(primary.canvas_settings),
            root_component_id: secondary.root_component_id.or(primary.root_component_id),
            flowscript_workspace,
            flow_ir_commit,
        }
    }

    fn scope_label(scope: CopilotScope) -> &'static str {
        match scope {
            CopilotScope::Board => "Workflow",
            CopilotScope::Frontend => "UI",
            CopilotScope::Both => "FlowPilot",
            CopilotScope::DataStudio => "Data Studio",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flow::{
        board::{Board, ExecutionMode, ExecutionStage},
        copilot::{
            BeginFlowIrDraftArgs, CommitFlowIrDraftArgs, FlowCapabilityPlanRequest,
            FlowCapabilityRequirement, FlowIrArg, FlowIrDraftMode, FlowIrLiteral, FlowIrModule,
            FlowIrProgram, FlowIrStep, FlowIrValue, FlowModuleEstimate, FlowModuleKind,
            NodeMetadata, PinMetadata,
        },
        execution::LogLevel,
    };
    use flow_like_storage::Path;
    use std::{collections::HashMap, time::SystemTime};

    fn claim_test_board() -> Board {
        Board {
            id: "combined-claim-board".to_string(),
            name: "Board".to_string(),
            description: String::new(),
            nodes: HashMap::new(),
            variables: HashMap::new(),
            comments: HashMap::new(),
            viewport: (0.0, 0.0, 1.0),
            version: (0, 0, 1),
            stage: ExecutionStage::Dev,
            log_level: LogLevel::Info,
            execution_mode: ExecutionMode::Hybrid,
            refs: HashMap::new(),
            internal_refs: HashMap::new(),
            layers: HashMap::new(),
            page_ids: Vec::new(),
            hash: None,
            created_at: SystemTime::now(),
            updated_at: SystemTime::now(),
            parent: None,
            board_dir: Path::from("/test"),
            logic_nodes: HashMap::new(),
            app_state: None,
        }
    }

    fn claim_test_pin(name: &str, data_type: &str) -> PinMetadata {
        PinMetadata {
            name: name.to_string(),
            friendly_name: name.to_string(),
            description: String::new(),
            data_type: data_type.to_string(),
            value_type: "Normal".to_string(),
            default_value: None,
            schema: None,
            is_generic: false,
            valid_values: None,
            enforce_schema: false,
        }
    }

    fn claim_test_catalog() -> Vec<NodeMetadata> {
        vec![
            NodeMetadata {
                name: "events_simple".to_string(),
                friendly_name: "events_simple".to_string(),
                description: String::new(),
                inputs: Vec::new(),
                outputs: vec![claim_test_pin("exec_out", "Execution")],
                category: None,
                required_inputs: Vec::new(),
                companion_nodes: Vec::new(),
                capability_tags: Vec::new(),
            },
            NodeMetadata {
                name: "string_format".to_string(),
                friendly_name: "string_format".to_string(),
                description: String::new(),
                inputs: vec![claim_test_pin("format_string", "String")],
                outputs: vec![claim_test_pin("string", "String")],
                category: None,
                required_inputs: Vec::new(),
                companion_nodes: Vec::new(),
                capability_tags: Vec::new(),
            },
        ]
    }

    fn claim_test_commit() -> (
        Arc<FlowIrDraftStore>,
        Board,
        Vec<NodeMetadata>,
        CommitFlowIrDraftArgs,
        FlowIrCommitToken,
    ) {
        let store = Arc::new(FlowIrDraftStore::new());
        let board = claim_test_board();
        let catalog = claim_test_catalog();
        store.begin(
            &board,
            &catalog,
            BeginFlowIrDraftArgs {
                draft_id: "combined-primary".to_string(),
                replace_existing: false,
                expected_modules: vec!["eventsSimple".to_string()],
                capability_plan: FlowCapabilityPlanRequest {
                    requirements: vec![FlowCapabilityRequirement {
                        id: "format".to_string(),
                        intent: "format a message".to_string(),
                        required: true,
                        exact_node_type: Some("string_format".to_string()),
                        inputs: Vec::new(),
                        outputs: Vec::new(),
                    }],
                    modules: vec![FlowModuleEstimate {
                        name: "eventsSimple".to_string(),
                        kind: FlowModuleKind::Event,
                        estimated_nodes: 1,
                    }],
                },
                mode: FlowIrDraftMode::Additive,
                program: FlowIrProgram {
                    modules: vec![FlowIrModule::Event {
                        name: "eventsSimple".to_string(),
                        node_type: "events_simple".to_string(),
                        params: Vec::new(),
                        steps: vec![FlowIrStep::Node {
                            id: "message".to_string(),
                            node_type: "string_format".to_string(),
                            args: vec![FlowIrArg {
                                pin: "format_string".to_string(),
                                occurrence: 0,
                                value: FlowIrValue::Literal {
                                    value: FlowIrLiteral::String("hello".to_string()),
                                },
                            }],
                            continue_from: None,
                            exec_arms: Vec::new(),
                            anchor: None,
                        }],
                        anchor: None,
                    }],
                    ..Default::default()
                },
            },
        );
        let args = CommitFlowIrDraftArgs {
            draft_id: "combined-primary".to_string(),
            expected_revision: 0,
            allow_deletions: false,
            remove_node_ids: Vec::new(),
            remove_variable_ids: Vec::new(),
            remove_layer_ids: Vec::new(),
            remove_comment_ids: Vec::new(),
            use_best_candidate: false,
        };
        let queued = store.commit(&board, &catalog, args.clone());
        assert_eq!(queued.status, "queued", "{queued:#?}");
        let token = store
            .latest_pending_commit_token(&board.id)
            .expect("queued combined-scope claim");
        (store, board, catalog, args, token)
    }

    #[test]
    fn ui_only_raw_request_stays_frontend_despite_unified_workflow_wrapper() {
        let raw = "Create a settings page with a form and two buttons";
        let wrapper = "UNIFIED MODE: generate workflow nodes and UI components. Create a settings page with a form and two buttons";

        let raw_plan = UnifiedCopilot::plan_combined_scope_with_availability(raw, true, true, true);
        let contaminated_plan =
            UnifiedCopilot::plan_combined_scope_with_availability(wrapper, true, true, true);

        assert_eq!(raw_plan.target, CombinedScopeTarget::Frontend);
        assert_eq!(contaminated_plan.target, CombinedScopeTarget::Both);
    }

    #[test]
    fn secondary_scope_error_releases_primary_typed_claim_but_success_transfers_it() {
        let (store, board, catalog, args, token) = claim_test_commit();
        {
            let _secondary_error_guard =
                CombinedScopeFlowIrClaim::new(store.clone(), token.clone());
        }
        assert!(!store.pending_commit_matches(
            &token.draft_id,
            token.revision,
            &token.base_fingerprint,
            &token.claim_id,
        ));

        let retried = store.commit(&board, &catalog, args);
        assert_eq!(retried.status, "queued", "{retried:#?}");
        let retried_token = store
            .latest_pending_commit_token(&board.id)
            .expect("retried combined-scope claim");
        let transferred =
            CombinedScopeFlowIrClaim::new(store.clone(), retried_token.clone()).transfer();
        assert_eq!(transferred, retried_token);
        assert!(store.pending_commit_matches(
            &retried_token.draft_id,
            retried_token.revision,
            &retried_token.base_fingerprint,
            &retried_token.claim_id,
        ));
    }
}
