use serde::{Deserialize, Serialize};

fn is_false(value: &bool) -> bool {
    !*value
}

/// A step in the execution plan.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct PlanStep {
    pub id: String,
    pub description: String,
    pub status: PlanStepStatus,
    pub tool_name: Option<String>,
}

/// Status of a plan step.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub enum PlanStepStatus {
    Pending,
    InProgress,
    Completed,
    Failed,
}

/// Events that can be streamed from the copilot.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub enum StreamEvent {
    Token(String),
    PlanStep(PlanStep),
    ToolCall {
        name: String,
        args: String,
    },
    ToolResult {
        name: String,
        result: String,
    },
    Thinking(String),
    FocusNode {
        node_id: String,
        description: String,
    },
}

/// Represents the type of response from the copilot.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum AgentType {
    /// Response that primarily explains.
    Explain,
    /// Response that includes modifications.
    Edit,
}

/// An image attachment in a chat message.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatImage {
    /// Base64-encoded image data, without a data URL prefix.
    pub data: String,
    /// MIME type, such as `image/png` or `image/jpeg`.
    pub media_type: String,
}

/// A message in the chat history.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: ChatRole,
    pub content: String,
    /// Optional images attached to this message for vision models.
    #[serde(default)]
    pub images: Option<Vec<ChatImage>>,
}

/// Role in the chat conversation.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum ChatRole {
    User,
    Assistant,
}

/// Exact typed-IR command claim retained by the host for atomic review.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct FlowIrCommitToken {
    pub board_id: String,
    pub draft_id: String,
    pub revision: u64,
    pub base_fingerprint: String,
    pub claim_id: String,
    /// Host-derived review policy. This is a UI hint only: native Apply derives the same policy
    /// again from the retained draft and fails closed unless the host passes explicit approval.
    #[serde(default, skip_serializing_if = "is_false")]
    pub requires_destructive_approval: bool,
}

/// Context for a specific run, used for log queries.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunContext {
    pub run_id: String,
    pub app_id: String,
    pub board_id: String,
}

/// Compact template information for model context.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TemplateInfo {
    pub id: String,
    pub app_id: String,
    pub name: String,
    pub description: String,
    pub tags: Vec<String>,
    pub node_count: usize,
    pub node_types: Vec<String>,
}
