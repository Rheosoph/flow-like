//! A2UI Copilot types

use crate::a2ui::SurfaceComponent;
use serde::{Deserialize, Serialize};

/// Chat message role for A2UI conversations
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum A2UIChatRole {
    User,
    Assistant,
}

/// Image attachment for chat messages
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct A2UIChatImage {
    pub data: String,
    pub media_type: String,
}

/// Chat message for A2UI conversations
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct A2UIChatMessage {
    pub role: A2UIChatRole,
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub images: Option<Vec<A2UIChatImage>>,
}

/// Response from A2UI Copilot
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct A2UICopilotResponse {
    pub message: String,
    pub components: Vec<SurfaceComponent>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub canvas_settings: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub root_component_id: Option<String>,
    pub suggestions: Vec<A2UISuggestion>,
}

/// Suggestion for follow-up actions
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct A2UISuggestion {
    pub label: String,
    pub prompt: String,
}

/// Context for the current A2UI surface
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct A2UIContext {
    pub components: Vec<ComponentContext>,
    pub selected_ids: Vec<String>,
    pub component_count: usize,
    /// The surface's persisted `canvasSettings`, `customCss` included. Serialized verbatim so the
    /// model can edit the existing stylesheet instead of overwriting one it never saw.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub canvas_settings: Option<serde_json::Value>,
}

/// Context for a single component
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComponentContext {
    pub id: String,
    pub component_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub style_classes: Option<String>,
    pub is_selected: bool,
}

/// Plan step status for streaming UI
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum A2UIPlanStepStatus {
    Pending,
    InProgress,
    Completed,
    Failed,
}

/// Plan step for streaming progress updates
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct A2UIPlanStep {
    pub id: String,
    pub description: String,
    pub status: A2UIPlanStepStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_name: Option<String>,
}

/// Stream events for real-time updates
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum A2UIStreamEvent {
    PlanStep(A2UIPlanStep),
    ComponentPreview(Vec<SurfaceComponent>),
}
