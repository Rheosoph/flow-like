//! Unified Copilot types - shared between flow and UI generation

use crate::a2ui::SurfaceComponent;
use serde::{Deserialize, Serialize};

/// Re-export commonly used types from the board copilot
pub use crate::flow::copilot::{
    BoardCommand, ChatImage, ChatRole, FlowIrCommitToken, NodeMetadata, NodePosition, PinMetadata,
    PlaceholderPinDef, PlanStep, PlanStepStatus, RunContext, Suggestion, TemplateInfo,
};

/// The scope of what the copilot agent can modify
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Default)]
pub enum CopilotScope {
    /// Only flow/board modifications (nodes, connections, variables)
    #[default]
    Board,
    /// Only UI modifications (A2UI components)
    Frontend,
    /// Both board and UI modifications
    Both,
    /// Only data-layer work (databases, ontologies/overlays, graph queries,
    /// analytics and ontology actions). No board or UI modifications.
    DataStudio,
    /// Read-only prior-art research: search and inspect existing apps and
    /// templates to decide what can be reused as a foundation. Modifies
    /// nothing — it returns a plan for the orchestrator to execute.
    Scout,
    /// Read-only PUBLIC-WEB research. The only scope holding the search,
    /// page-read and archive tools; it has no access to apps, data, storage or
    /// memory, so untrusted page text and private data never share a context.
    Research,
}

/// A unified chat message that can contain both text and images
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UnifiedChatMessage {
    pub role: ChatRole,
    pub content: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub images: Option<Vec<ChatImage>>,
}

impl From<crate::flow::copilot::ChatMessage> for UnifiedChatMessage {
    fn from(msg: crate::flow::copilot::ChatMessage) -> Self {
        Self {
            role: msg.role,
            content: msg.content,
            images: msg.images,
        }
    }
}

impl From<crate::a2ui::copilot::A2UIChatMessage> for UnifiedChatMessage {
    fn from(msg: crate::a2ui::copilot::A2UIChatMessage) -> Self {
        Self {
            role: match msg.role {
                crate::a2ui::copilot::A2UIChatRole::User => ChatRole::User,
                crate::a2ui::copilot::A2UIChatRole::Assistant => ChatRole::Assistant,
            },
            content: msg.content,
            images: msg.images.map(|imgs| {
                imgs.into_iter()
                    .map(|img| ChatImage {
                        data: img.data,
                        media_type: img.media_type,
                    })
                    .collect()
            }),
        }
    }
}

/// Unified response from the copilot agent
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UnifiedCopilotResponse {
    /// The assistant's message explaining what was done or what should be done
    pub message: String,

    /// Board commands to execute (for Board and Both scopes)
    #[serde(default)]
    pub commands: Vec<BoardCommand>,

    /// UI components generated (for Frontend and Both scopes)
    #[serde(default)]
    pub components: Vec<SurfaceComponent>,

    /// Canvas settings for UI components (includes customCss)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub canvas_settings: Option<serde_json::Value>,

    /// Root component ID for UI components
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub root_component_id: Option<String>,

    /// Last FlowScript document submitted by the workflow agent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub flowscript_workspace: Option<String>,

    /// Exact typed-IR batch awaiting host review. The host must preflight this token immediately
    /// before Apply and resolve it as applied or dismissed afterward.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub flow_ir_commit: Option<FlowIrCommitToken>,

    /// Suggested follow-up prompts
    #[serde(default)]
    pub suggestions: Vec<UnifiedSuggestion>,

    /// The actual scope that was used (agent may decide to focus on one area)
    pub active_scope: CopilotScope,
}

/// A suggestion for follow-up actions (works for both board and UI)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UnifiedSuggestion {
    pub label: String,
    pub prompt: String,
    /// Which scope this suggestion targets
    #[serde(default)]
    pub scope: Option<CopilotScope>,
}

impl From<Suggestion> for UnifiedSuggestion {
    fn from(s: Suggestion) -> Self {
        Self {
            label: s.node_type.clone(),
            prompt: s.reason,
            scope: Some(CopilotScope::Board),
        }
    }
}

impl From<crate::a2ui::copilot::A2UISuggestion> for UnifiedSuggestion {
    fn from(s: crate::a2ui::copilot::A2UISuggestion) -> Self {
        Self {
            label: s.label,
            prompt: s.prompt,
            scope: Some(CopilotScope::Frontend),
        }
    }
}

/// Unified context passed to the copilot
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UnifiedContext {
    /// Current scope the agent should operate in
    pub scope: CopilotScope,

    /// Optional run context for log queries (board mode)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_context: Option<RunContext>,

    /// Action context for UI component actions (frontend mode)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action_context: Option<UIActionContext>,
}

/// Context for UI actions (pages, events, etc.)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UIActionContext {
    pub app_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub board_id: Option<String>,
    #[serde(default)]
    pub pages: Vec<PageInfo>,
    #[serde(default)]
    pub workflow_events: Vec<WorkflowEventInfo>,
}

/// Basic page information for navigation actions
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PageInfo {
    pub id: String,
    pub name: String,
}

/// Basic workflow event information for triggering workflows
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowEventInfo {
    pub node_id: String,
    pub name: String,
}

/// Events that can be streamed from the unified copilot
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum UnifiedStreamEvent {
    /// A text token being generated
    Token(String),
    /// A step in the execution plan
    PlanStep(PlanStep),
    /// A tool is being called
    ToolCall { name: String, args: String },
    /// Result from a tool call
    ToolResult { name: String, result: String },
    /// Agent is thinking/reasoning
    Thinking(String),
    /// Focus on a specific node (board mode)
    FocusNode {
        node_id: String,
        description: String,
    },
    /// Preview of generated components (frontend mode)
    ComponentPreview(Vec<SurfaceComponent>),
    /// Agent determined which scope to focus on
    ScopeDecision(CopilotScope),
}
