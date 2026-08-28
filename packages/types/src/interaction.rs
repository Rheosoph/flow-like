use crate::Value;
use crate::channel::ChannelHandle;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Configuration for a single choice option
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ChoiceOption {
    /// Unique identifier for this option
    pub id: String,
    /// Display label
    pub label: String,
    /// Optional description
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Whether this option allows freeform text input
    #[serde(default)]
    pub freeform: bool,
}

/// Configuration for a form field
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct FormField {
    /// Unique identifier for this field
    pub id: String,
    /// Display label
    pub label: String,
    /// Optional description/help text
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Field type (text, number, boolean, select)
    pub field_type: FormFieldType,
    /// Whether the field is required
    #[serde(default)]
    pub required: bool,
    /// Default value
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_value: Option<Value>,
    /// Options for select fields
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub options: Vec<ChoiceOption>,
}

/// Type of form field
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum FormFieldType {
    Text,
    Number,
    Boolean,
    Select,
}

/// Type of interaction
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum InteractionType {
    SingleChoice {
        options: Vec<ChoiceOption>,
        #[serde(default)]
        allow_freeform: bool,
    },
    MultipleChoice {
        options: Vec<ChoiceOption>,
        #[serde(default)]
        min_selections: usize,
        #[serde(default = "default_max_selections")]
        max_selections: usize,
    },
    Form {
        #[serde(skip_serializing_if = "Option::is_none")]
        schema: Option<Value>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        fields: Vec<FormField>,
    },
}

fn default_max_selections() -> usize {
    usize::MAX
}

/// Status of an interaction
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum InteractionStatus {
    Pending,
    Responded,
    Expired,
    Cancelled,
}

/// A human-in-the-loop interaction request
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct InteractionRequest {
    /// Unique identifier
    pub id: String,
    /// Display name
    pub name: String,
    /// Description/prompt shown to the user
    pub description: String,
    /// The type and configuration of this interaction
    pub interaction_type: InteractionType,
    /// Current status
    pub status: InteractionStatus,
    /// TTL in seconds from creation
    pub ttl_seconds: u64,
    /// Unix timestamp when this expires
    pub expires_at: u64,
    /// Run ID that created this
    #[serde(skip_serializing_if = "Option::is_none")]
    pub run_id: Option<String>,
    /// App ID context
    #[serde(skip_serializing_if = "Option::is_none")]
    pub app_id: Option<String>,
    /// How to answer: present once the run has registered the request on its channel.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub channel: Option<ChannelHandle>,
}

/// Response to an interaction
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct InteractionResponse {
    /// The interaction ID this responds to
    pub interaction_id: String,
    /// The response value (interpretation depends on interaction type)
    pub value: Value,
}

/// Result of polling for an interaction response
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum InteractionPollResult {
    Pending,
    Responded { value: Value },
    Expired,
    Cancelled,
}
